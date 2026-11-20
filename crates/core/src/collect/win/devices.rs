//! PnP device + driver inventory via SetupAPI / CfgMgr32 (no PowerShell).

use super::registry::{CLASS_ROOT, RegKey, read_dword, read_string};
use super::signature::SignatureChecker;
use super::util::{bytes_to_u16, filetime_to_utc, format_guid, from_wide, multi_sz};
use crate::collect::{CollectError, ProgressFn};
use crate::snapshot::{
    AdvancedProperty, DeviceRecord, DeviceStatus, DriverInfo, InstallDates, PowerInfo,
    PropertyOption, SourceWarning, problem_text,
};
use tokio_util::sync::CancellationToken;
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Devices::Properties::*;
use windows::Win32::Foundation::DEVPROPKEY;

pub(crate) struct DevInfoSet(pub(crate) HDEVINFO);
impl Drop for DevInfoSet {
    fn drop(&mut self) {
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

pub(crate) struct Dev<'a> {
    pub(crate) set: &'a DevInfoSet,
    pub(crate) data: SP_DEVINFO_DATA,
}

/// Opens one device by instance ID. `Ok(None)` when Windows does not know it.
pub(crate) fn open(instance_id: &str) -> Result<Option<(DevInfoSet, SP_DEVINFO_DATA)>, String> {
    let set = unsafe { SetupDiCreateDeviceInfoList(None, None) }
        .map_err(|e| format!("SetupDiCreateDeviceInfoList failed: {e}"))?;
    let set = DevInfoSet(set);
    let id = super::util::wide(instance_id);
    let mut data = SP_DEVINFO_DATA {
        cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
        ..Default::default()
    };
    match unsafe {
        SetupDiOpenDeviceInfoW(
            set.0,
            windows::core::PCWSTR(id.as_ptr()),
            None,
            0,
            Some(&mut data),
        )
    } {
        Ok(()) => Ok(Some((set, data))),
        // ERROR_NO_SUCH_DEVINST, as a Win32 code or HRESULT_FROM_SETUPAPI.
        Err(e) if matches!(e.code().0 as u32, 0xE000_020B | 0x800F_020B) => Ok(None),
        Err(e) => Err(format!("SetupDiOpenDeviceInfo failed: {e}")),
    }
}

/// Live re-read of one device, as the fix engine needs before and after a
/// change. Unlike a scan, a failure to read the adapter's settings is an
/// error here, not a warning: a fix must never act on missing data.
pub fn read_one(instance_id: &str) -> Result<Option<DeviceRecord>, String> {
    let Some((set, data)) = open(instance_id)? else {
        return Ok(None);
    };
    let dev = Dev { set: &set, data };
    let mut sig = SignatureChecker::new();
    let mut warnings = Vec::new();
    let rec = build_record(&dev, instance_id.to_string(), &mut sig, &mut warnings);
    if let Some(w) = warnings
        .iter()
        .find(|w| w.source == "net" || w.source == "power")
    {
        return Err(w.message.clone());
    }
    Ok(Some(rec))
}

impl Dev<'_> {
    fn raw(&self, key: &DEVPROPKEY) -> Option<(DEVPROPTYPE, Vec<u8>)> {
        let mut ty = DEVPROPTYPE(0);
        let mut size = 0u32;
        unsafe {
            // First call only sizes the buffer; it is expected to "fail".
            let _ = SetupDiGetDevicePropertyW(
                self.set.0,
                &self.data,
                key,
                &mut ty,
                None,
                Some(&mut size),
                0,
            );
        }
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        unsafe {
            SetupDiGetDevicePropertyW(
                self.set.0,
                &self.data,
                key,
                &mut ty,
                Some(&mut buf),
                None,
                0,
            )
            .ok()?;
        }
        Some((ty, buf))
    }

    fn string(&self, key: &DEVPROPKEY) -> Option<String> {
        let (ty, b) = self.raw(key)?;
        (ty == DEVPROP_TYPE_STRING)
            .then(|| from_wide(&bytes_to_u16(&b)))
            .filter(|s| !s.is_empty())
    }

    fn string_list(&self, key: &DEVPROPKEY) -> Vec<String> {
        match self.raw(key) {
            Some((ty, b)) if ty == DEVPROP_TYPE_STRING_LIST => multi_sz(&bytes_to_u16(&b)),
            _ => Vec::new(),
        }
    }

    fn u32(&self, key: &DEVPROPKEY) -> Option<u32> {
        let (ty, b) = self.raw(key)?;
        (ty == DEVPROP_TYPE_UINT32 && b.len() >= 4)
            .then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn bool(&self, key: &DEVPROPKEY) -> Option<bool> {
        let (ty, b) = self.raw(key)?;
        (ty == DEVPROP_TYPE_BOOLEAN && !b.is_empty()).then(|| b[0] != 0)
    }

    fn filetime(&self, key: &DEVPROPKEY) -> Option<chrono::DateTime<chrono::Utc>> {
        let (ty, b) = self.raw(key)?;
        if ty != DEVPROP_TYPE_FILETIME || b.len() < 8 {
            return None;
        }
        filetime_to_utc(u64::from_le_bytes(b[..8].try_into().ok()?))
    }

    fn guid(&self, key: &DEVPROPKEY) -> Option<String> {
        let (ty, b) = self.raw(key)?;
        (ty == DEVPROP_TYPE_GUID).then(|| format_guid(&b)).flatten()
    }

    fn instance_id(&self) -> Option<String> {
        let mut size = 0u32;
        unsafe {
            let _ = SetupDiGetDeviceInstanceIdW(self.set.0, &self.data, None, Some(&mut size));
        }
        let mut buf = vec![0u16; size.max(1) as usize];
        unsafe {
            SetupDiGetDeviceInstanceIdW(self.set.0, &self.data, Some(&mut buf), None).ok()?;
        }
        Some(from_wide(&buf))
    }

    fn status(&self) -> DeviceStatus {
        let mut flags = CM_DEVNODE_STATUS_FLAGS(0);
        let mut prob = CM_PROB(0);
        let cr = unsafe { CM_Get_DevNode_Status(&mut flags, &mut prob, self.data.DevInst, 0) };
        if cr != CR_SUCCESS {
            return DeviceStatus::default();
        }
        let has_problem = flags.contains(DN_HAS_PROBLEM);
        let code = has_problem.then_some(prob.0);
        DeviceStatus {
            problem_code: code,
            problem_text: code.map(|c| problem_text(c).to_string()),
            devnode_flags: flags.0,
            started: flags.contains(DN_STARTED),
            disabled: code == Some(CM_PROB_DISABLED.0),
            available: true,
        }
    }
}

pub fn collect(
    progress: ProgressFn<'_>,
    cancel: &CancellationToken,
    warnings: &mut Vec<SourceWarning>,
) -> Result<Vec<DeviceRecord>, CollectError> {
    // DIGCF_ALLCLASSES without DIGCF_PRESENT: include ghost devices too,
    // the UI hides them by default.
    let set = unsafe { SetupDiGetClassDevsW(None, None, None, DIGCF_ALLCLASSES) }
        .map_err(|e| CollectError::Fatal(format!("SetupDiGetClassDevs failed: {e}")))?;
    let set = DevInfoSet(set);

    // Count first so progress has a total.
    let mut total = 0u32;
    loop {
        let mut d = SP_DEVINFO_DATA {
            cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        if unsafe { SetupDiEnumDeviceInfo(set.0, total, &mut d) }.is_err() {
            break;
        }
        total += 1;
    }

    let mut sig = SignatureChecker::new();
    if let Some(e) = sig.init_error() {
        warnings.push(SourceWarning {
            source: "signature".into(),
            message: e.to_string(),
            instance_id: None,
        });
    }

    let mut out = Vec::with_capacity(total as usize);
    for i in 0..total {
        if cancel.is_cancelled() {
            return Err(CollectError::Cancelled);
        }
        if i % 16 == 0 {
            progress("devices", i, total);
        }
        let mut data = SP_DEVINFO_DATA {
            cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        if unsafe { SetupDiEnumDeviceInfo(set.0, i, &mut data) }.is_err() {
            continue;
        }
        let dev = Dev { set: &set, data };
        let Some(instance_id) = dev.instance_id() else {
            warnings.push(SourceWarning {
                source: "devices".into(),
                message: format!("could not read instance ID of device #{i}"),
                instance_id: None,
            });
            continue;
        };
        out.push(build_record(&dev, instance_id, &mut sig, warnings));
    }
    progress("devices", total, total);
    Ok(out)
}

fn build_record(
    dev: &Dev<'_>,
    instance_id: String,
    sig: &mut SignatureChecker,
    warnings: &mut Vec<SourceWarning>,
) -> DeviceRecord {
    let description = dev.string(&DEVPKEY_Device_DeviceDesc);
    let name = dev
        .string(&DEVPKEY_Device_FriendlyName)
        .or_else(|| description.clone())
        .unwrap_or_else(|| instance_id.clone());
    let class = dev.string(&DEVPKEY_Device_Class);
    let enumerator = dev.string(&DEVPKEY_Device_EnumeratorName);
    let present = dev.bool(&DEVPKEY_Device_IsPresent).unwrap_or(false);
    let status = if present {
        dev.status()
    } else {
        DeviceStatus::default()
    };

    let driver_key = dev.string(&DEVPKEY_Device_Driver);
    let inf_name = dev.string(&DEVPKEY_Device_DriverInfPath);
    let driver = (driver_key.is_some() || inf_name.is_some()).then(|| DriverInfo {
        provider: dev.string(&DEVPKEY_Device_DriverProvider),
        version: dev.string(&DEVPKEY_Device_DriverVersion),
        date: dev
            .filetime(&DEVPKEY_Device_DriverDate)
            .map(|d| d.format("%Y-%m-%d").to_string()),
        description: dev.string(&DEVPKEY_Device_DriverDesc),
        inf_section: dev.string(&DEVPKEY_Device_DriverInfSection),
        matching_device_id: dev.string(&DEVPKEY_Device_MatchingDeviceId),
        rank: dev.u32(&DEVPKEY_Device_DriverRank),
        signature: inf_name
            .as_deref()
            .map(|inf| sig.check_inf(inf))
            .unwrap_or_default(),
        driver_key: driver_key.clone(),
        inf_name,
    });

    let power = collect_power(
        dev,
        class.as_deref(),
        enumerator.as_deref(),
        driver_key.as_deref(),
    )
    .map_err(|e| {
        warnings.push(SourceWarning {
            source: "power".into(),
            message: e,
            instance_id: Some(instance_id.clone()),
        })
    })
    .ok()
    .flatten();

    let net_cfg_instance_id = match (class.as_deref(), driver_key.as_deref()) {
        (Some("Net"), Some(key)) => {
            read_string(&format!(r"{CLASS_ROOT}\{key}"), "NetCfgInstanceId").unwrap_or_else(|e| {
                warnings.push(SourceWarning {
                    source: "net".into(),
                    message: e,
                    instance_id: Some(instance_id.clone()),
                });
                None
            })
        }
        _ => None,
    };

    let physical = matches!(enumerator.as_deref(), Some("PCI" | "USB" | "SD" | "ACPI"));
    let advanced = match (class.as_deref(), driver_key.as_deref()) {
        (Some("Net"), Some(key)) if physical && present => {
            advanced_properties(key).unwrap_or_else(|e| {
                warnings.push(SourceWarning {
                    source: "net".into(),
                    message: e,
                    instance_id: Some(instance_id.clone()),
                });
                Vec::new()
            })
        }
        _ => Vec::new(),
    };

    DeviceRecord {
        advanced,
        net_cfg_instance_id,
        name,
        description,
        class,
        class_guid: dev.guid(&DEVPKEY_Device_ClassGuid),
        manufacturer: dev.string(&DEVPKEY_Device_Manufacturer),
        location: dev.string(&DEVPKEY_Device_LocationInfo),
        service: dev.string(&DEVPKEY_Device_Service),
        hardware_ids: dev.string_list(&DEVPKEY_Device_HardwareIds),
        compatible_ids: dev.string_list(&DEVPKEY_Device_CompatibleIds),
        present,
        status,
        driver,
        install: InstallDates {
            install_date: dev
                .filetime(&DEVPKEY_Device_InstallDate)
                .map(|d| d.to_rfc3339()),
            first_install_date: dev
                .filetime(&DEVPKEY_Device_FirstInstallDate)
                .map(|d| d.to_rfc3339()),
            last_arrival_date: dev
                .filetime(&DEVPKEY_Device_LastArrivalDate)
                .map(|d| d.to_rfc3339()),
        },
        power,
        enumerator,
        instance_id,
    }
}

// CM_POWER_DATA capability bits (cfgmgr32.h).
const PDCAP_D0: u32 = 0x1;
const PDCAP_D1: u32 = 0x2;
const PDCAP_D2: u32 = 0x4;
const PDCAP_D3: u32 = 0x8;
const PDCAP_WAKE_D0: u32 = 0x10;
const PDCAP_WAKE_D1: u32 = 0x20;
const PDCAP_WAKE_D2: u32 = 0x40;
const PDCAP_WAKE_D3: u32 = 0x80;
/// ndis.h NDIS_DEVICE_DISABLE_PM (0x8): "Allow the computer to turn off this
/// device to save power" is unchecked when this PnPCapabilities bit is set.
pub(crate) const NDIS_DEVICE_DISABLE_PM: u32 = 0x8;

fn collect_power(
    dev: &Dev<'_>,
    class: Option<&str>,
    enumerator: Option<&str>,
    driver_key: Option<&str>,
) -> Result<Option<PowerInfo>, String> {
    let Some((_, b)) = dev.raw(&DEVPKEY_Device_PowerData) else {
        return Ok(None);
    };
    if b.len() < 12 {
        return Ok(None);
    }
    let rd = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let current = rd(4);
    let caps = rd(8);
    let pick = |pairs: &[(u32, &str)]| {
        pairs
            .iter()
            .filter(|(bit, _)| caps & bit != 0)
            .map(|(_, n)| (*n).to_string())
            .collect::<Vec<_>>()
    };
    let mut info = PowerInfo {
        supported_states: pick(&[
            (PDCAP_D0, "D0"),
            (PDCAP_D1, "D1"),
            (PDCAP_D2, "D2"),
            (PDCAP_D3, "D3"),
        ]),
        wake_from: pick(&[
            (PDCAP_WAKE_D0, "D0"),
            (PDCAP_WAKE_D1, "D1"),
            (PDCAP_WAKE_D2, "D2"),
            (PDCAP_WAKE_D3, "D3"),
        ]),
        current_state: match current {
            1..=4 => Some(format!("D{}", current - 1)),
            _ => None,
        },
        ..Default::default()
    };

    // The NIC power-management checkbox only exists on physical adapters.
    let physical = matches!(enumerator, Some("PCI" | "USB" | "SD" | "ACPI"));
    if class == Some("Net")
        && physical
        && let Some(key) = driver_key
    {
        let raw = read_dword(&format!(r"{CLASS_ROOT}\{key}"), "PnPCapabilities")?;
        info.pnp_capabilities_raw = raw;
        info.allow_turn_off = Some(raw.unwrap_or(0) & NDIS_DEVICE_DISABLE_PM == 0);
    }
    Ok(Some(info))
}

/// Reads the driver's Advanced tab from `<class>\<NNNN>\Ndi\params\<Keyword>`
/// (definitions) and `<class>\<NNNN>\<Keyword>` (current values). Keyword based,
/// so it works on any display language; readable without admin.
fn advanced_properties(driver_key: &str) -> Result<Vec<AdvancedProperty>, String> {
    let base = format!(r"{CLASS_ROOT}\{driver_key}");
    let Some(key) = RegKey::open(&base)? else {
        return Ok(Vec::new());
    };
    let current: std::collections::HashMap<String, String> = key
        .values()
        .into_iter()
        .map(|(n, v)| (n.to_ascii_lowercase(), v))
        .collect();
    let Some(params) = RegKey::open(&format!(r"{base}\Ndi\params"))? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for kw in params.subkeys() {
        let path = format!(r"{base}\Ndi\params\{kw}");
        let Some(pk) = RegKey::open(&path)? else {
            continue;
        };
        let vals: std::collections::HashMap<String, String> = pk
            .values()
            .into_iter()
            .map(|(n, v)| (n.to_ascii_lowercase(), v))
            .collect();
        let options = match RegKey::open(&format!(r"{path}\enum"))? {
            Some(e) => e
                .values()
                .into_iter()
                .filter(|(n, _)| !n.is_empty())
                .map(|(value, label)| PropertyOption { value, label })
                .collect(),
            None => Vec::new(),
        };
        out.push(AdvancedProperty {
            current: current.get(&kw.to_ascii_lowercase()).cloned(),
            label: vals.get("paramdesc").cloned(),
            kind: vals
                .get("type")
                .map(|t| t.to_ascii_lowercase())
                .unwrap_or_default(),
            default: vals.get("default").cloned(),
            min: vals.get("min").cloned(),
            max: vals.get("max").cloned(),
            options,
            keyword: kw,
        });
    }
    out.sort_by_key(|p| p.keyword.to_ascii_lowercase());
    Ok(out)
}
