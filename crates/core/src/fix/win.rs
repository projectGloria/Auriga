//! Live system access for fixes. Reads work as a normal user; the write
//! functions need admin and are called only by the elevated helper, each
//! after re-reading the device and re-running the `validate` checks.

use super::validate::{check_pnp_write, check_property_write, require_net_adapter};
use super::{MinidumpInfo, RestorePointOutcome};
use crate::collect::win::devices::{Dev, open, read_one};
use crate::collect::win::registry::{CLASS_ROOT, read_dword};
use crate::collect::win::util::wide;
use crate::snapshot::DeviceRecord;
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Foundation::{ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::*;
use windows::core::PCWSTR;

pub fn read_device(instance_id: &str) -> Result<Option<DeviceRecord>, String> {
    read_one(instance_id)
}

fn live(instance_id: &str) -> Result<DeviceRecord, String> {
    read_one(instance_id)?.ok_or_else(|| "The device is no longer present on this system.".into())
}

/// An HKLM key opened for writing values. Closed on drop.
struct WriteKey(HKEY);
impl Drop for WriteKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

impl WriteKey {
    /// Opens `CLASS_ROOT\<driver_key>` only; `driver_key` was checked by
    /// `require_net_adapter` to be `{net guid}\NNNN`.
    fn open_driver_key(driver_key: &str) -> Result<Self, String> {
        let path = wide(&format!(r"{CLASS_ROOT}\{driver_key}"));
        let mut h = HKEY::default();
        let rc = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(path.as_ptr()),
                None,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                &mut h,
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(win32_msg("open the driver key", rc));
        }
        Ok(Self(h))
    }

    fn value_type(&self, name: &str) -> Option<REG_VALUE_TYPE> {
        let n = wide(name);
        let mut ty = REG_VALUE_TYPE::default();
        let rc = unsafe {
            RegQueryValueExW(self.0, PCWSTR(n.as_ptr()), None, Some(&mut ty), None, None)
        };
        (rc == ERROR_SUCCESS).then_some(ty)
    }

    fn set_dword(&self, name: &str, v: u32) -> Result<(), String> {
        let n = wide(name);
        let rc = unsafe {
            RegSetValueExW(
                self.0,
                PCWSTR(n.as_ptr()),
                None,
                REG_DWORD,
                Some(&v.to_le_bytes()),
            )
        };
        (rc == ERROR_SUCCESS)
            .then_some(())
            .ok_or_else(|| win32_msg(&format!("write {name}"), rc))
    }

    fn set_sz(&self, name: &str, v: &str) -> Result<(), String> {
        let n = wide(name);
        let data: Vec<u8> = wide(v).iter().flat_map(|c| c.to_le_bytes()).collect();
        let rc = unsafe { RegSetValueExW(self.0, PCWSTR(n.as_ptr()), None, REG_SZ, Some(&data)) };
        (rc == ERROR_SUCCESS)
            .then_some(())
            .ok_or_else(|| win32_msg(&format!("write {name}"), rc))
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        let n = wide(name);
        let rc = unsafe { RegDeleteValueW(self.0, PCWSTR(n.as_ptr())) };
        match rc.0 {
            0 | 2 => Ok(()), // deleted, or already absent
            _ => Err(win32_msg(&format!("delete {name}"), rc)),
        }
    }
}

fn win32_msg(what: &str, e: WIN32_ERROR) -> String {
    let hint = match e.0 {
        5 => " (access denied; this needs the elevated helper)",
        _ => "",
    };
    format!(
        "Could not {what}: {}{hint}",
        windows::core::Error::from(e.to_hresult()).message()
    )
}

/// Sets or deletes one Advanced-tab value. Keeps the value's existing
/// registry type (some drivers store DWORDs); new values are REG_SZ, as the
/// Device Manager Advanced tab writes them. Returns the previous stored value.
pub fn set_adapter_property(
    instance_id: &str,
    keyword: &str,
    value: Option<&str>,
) -> Result<Option<String>, String> {
    let d = live(instance_id)?;
    let key = require_net_adapter(&d)?.to_string();
    let kw = check_property_write(&d, keyword, value)?.to_string();
    let previous = d.advanced(&kw).and_then(|p| p.current.clone());
    let k = WriteKey::open_driver_key(&key)?;
    match value {
        None => k.delete(&kw)?,
        Some(v) => match k.value_type(&kw) {
            Some(REG_DWORD) => {
                let n: u32 = v
                    .parse()
                    .map_err(|_| format!("{kw} is stored as a number; {v} is not one."))?;
                k.set_dword(&kw, n)?
            }
            _ => k.set_sz(&kw, v)?,
        },
    }
    Ok(previous)
}

/// Writes or deletes `PnPCapabilities`; only the power-management bit may
/// change. Returns the previous stored value.
pub fn set_pnp_capabilities(instance_id: &str, value: Option<u32>) -> Result<Option<u32>, String> {
    let d = live(instance_id)?;
    let key = require_net_adapter(&d)?.to_string();
    let previous = read_dword(&format!(r"{CLASS_ROOT}\{key}"), "PnPCapabilities")?;
    check_pnp_write(previous, value)?;
    let k = WriteKey::open_driver_key(&key)?;
    match value {
        None => k.delete("PnPCapabilities")?,
        Some(v) => k.set_dword("PnPCapabilities", v)?,
    }
    Ok(previous)
}

fn needs_reboot(dev: &Dev<'_>) -> bool {
    let mut p = SP_DEVINSTALL_PARAMS_W {
        cbSize: size_of::<SP_DEVINSTALL_PARAMS_W>() as u32,
        ..Default::default()
    };
    unsafe { SetupDiGetDeviceInstallParamsW(dev.set.0, Some(&dev.data), &mut p) }.is_ok()
        && (p.Flags.0 & (DI_NEEDREBOOT.0 | DI_NEEDRESTART.0)) != 0
}

/// Restarts a network adapter (DIF_PROPERTYCHANGE / DICS_PROPCHANGE, the
/// same as disable+enable) so the driver re-reads its settings. Returns
/// whether Windows asked for a reboot instead.
pub fn restart_device(instance_id: &str) -> Result<bool, String> {
    let d = live(instance_id)?;
    require_net_adapter(&d)?;
    let (set, data) = open(instance_id)?.ok_or("The device is no longer present.")?;
    let params = SP_PROPCHANGE_PARAMS {
        ClassInstallHeader: SP_CLASSINSTALL_HEADER {
            cbSize: size_of::<SP_CLASSINSTALL_HEADER>() as u32,
            InstallFunction: DIF_PROPERTYCHANGE,
        },
        StateChange: DICS_PROPCHANGE,
        Scope: DICS_FLAG_CONFIGSPECIFIC,
        HwProfile: 0,
    };
    unsafe {
        SetupDiSetClassInstallParamsW(
            set.0,
            Some(&data),
            Some(&params.ClassInstallHeader),
            size_of::<SP_PROPCHANGE_PARAMS>() as u32,
        )
        .map_err(|e| format!("Could not prepare the device restart: {}", e.message()))?;
        SetupDiCallClassInstaller(DIF_PROPERTYCHANGE, set.0, Some(&data))
            .map_err(|e| format!("Could not restart the device: {}", e.message()))?;
    }
    Ok(needs_reboot(&Dev {
        set: &set,
        data,
    }))
}

/// Rolls back to the driver Windows kept as the previous one for this
/// device. Installs nothing else; fails when there is no previous driver.
pub fn rollback_driver(instance_id: &str) -> Result<bool, String> {
    let d = live(instance_id)?;
    if !d.present || d.driver.is_none() {
        return Err(format!("{} has no installed driver to roll back.", d.name));
    }
    let (set, data) = open(instance_id)?.ok_or("The device is no longer present.")?;
    let mut reboot = windows::core::BOOL(0);
    unsafe { DiRollbackDriver(set.0, &data, None, ROLLBACK_FLAG_NO_UI, Some(&mut reboot)) }
        .map_err(|e| {
            format!(
                "Windows could not roll back the driver (there may be no previous driver kept): {}",
                e.message()
            )
        })?;
    Ok(reboot.as_bool())
}

/// Creates a restore point and checks it really appeared: when one was made
/// in the last 24 h (the default `SystemRestorePointCreationFrequency`),
/// Windows reports success without creating anything.
pub fn create_restore_point(description: &str) -> RestorePointOutcome {
    use windows::Win32::System::Restore::*;
    let before = wmi::max_restore_sequence();

    let mut info = RESTOREPOINTINFOW {
        dwEventType: BEGIN_SYSTEM_CHANGE,
        dwRestorePtType: MODIFY_SETTINGS,
        llSequenceNumber: 0,
        szDescription: [0; 256],
    };
    for (i, c) in description.encode_utf16().take(255).enumerate() {
        info.szDescription[i] = c;
    }
    let mut st = STATEMGRSTATUS::default();
    let ok = unsafe { SRSetRestorePointW(&info, &mut st) }.as_bool();
    let status = st.nStatus;
    if !ok {
        return match status.0 {
            1058 => RestorePointOutcome::Disabled, // ERROR_SERVICE_DISABLED
            _ => RestorePointOutcome::Failed {
                message: windows::core::Error::from(status.to_hresult())
                    .message()
                    .to_string(),
            },
        };
    }
    let seq = st.llSequenceNumber;
    let end = RESTOREPOINTINFOW {
        dwEventType: END_SYSTEM_CHANGE,
        dwRestorePtType: MODIFY_SETTINGS,
        llSequenceNumber: seq,
        szDescription: [0; 256],
    };
    let mut st2 = STATEMGRSTATUS::default();
    let _ = unsafe { SRSetRestorePointW(&end, &mut st2) };

    match (before, wmi::max_restore_sequence()) {
        (Ok(b), Ok(Some(a))) if Some(a) > b => RestorePointOutcome::Created { sequence: a },
        (Ok(_), Ok(_)) => RestorePointOutcome::Skipped,
        (Err(e), _) | (_, Err(e)) => RestorePointOutcome::Unconfirmed {
            message: format!(
                "Windows accepted the request (sequence {seq}), but the restore point list could not be read: {e}"
            ),
        },
    }
}

mod wmi {
    use windows::Win32::System::Com::*;
    use windows::Win32::System::Variant::{VARIANT, VT_I4, VT_UI4, VariantClear};
    use windows::Win32::System::Wmi::*;
    use windows::core::{BSTR, w};

    /// Highest `SequenceNumber` in `root\default:SystemRestore`.
    pub fn max_restore_sequence() -> Result<Option<u32>, String> {
        unsafe {
            let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
            let uninit = hr.is_ok();
            let r = query();
            if uninit {
                CoUninitialize();
            }
            r
        }
    }

    unsafe fn query() -> Result<Option<u32>, String> {
        unsafe {
            // Fails harmlessly when the process already set security.
            let _ = CoInitializeSecurity(
                None,
                -1,
                None,
                None,
                RPC_C_AUTHN_LEVEL_DEFAULT,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
                None,
            );
            let loc: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.message())?;
            let svc = loc
                .ConnectServer(
                    &BSTR::from(r"root\default"),
                    &BSTR::new(),
                    &BSTR::new(),
                    &BSTR::new(),
                    0,
                    &BSTR::new(),
                    None,
                )
                .map_err(|e| e.message())?;
            let en = svc
                .ExecQuery(
                    &BSTR::from("WQL"),
                    &BSTR::from("SELECT SequenceNumber FROM SystemRestore"),
                    WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                    None,
                )
                .map_err(|e| e.message())?;
            let mut max: Option<u32> = None;
            loop {
                let mut objs = [None];
                let mut n = 0u32;
                let hr = en.Next(WBEM_INFINITE, &mut objs, &mut n);
                if hr.is_err() {
                    return Err(windows::core::Error::from(hr).message());
                }
                if n == 0 {
                    break;
                }
                let Some(o) = objs[0].take() else { break };
                let mut v = VARIANT::default();
                if o.Get(w!("SequenceNumber"), 0, &mut v, None, None).is_ok() {
                    let inner = &v.Anonymous.Anonymous;
                    let val = if inner.vt == VT_I4 || inner.vt == VT_UI4 {
                        Some(inner.Anonymous.lVal as u32)
                    } else {
                        None
                    };
                    let _ = VariantClear(&mut v);
                    if let Some(s) = val {
                        max = Some(max.map_or(s, |m| m.max(s)));
                    }
                }
            }
            Ok(max)
        }
    }
}

/// Reads the bugcheck header of each `*.dmp` in `%SystemRoot%\Minidump`
/// (newest first, at most 50). The folder is fixed here, not requested.
pub fn list_minidumps() -> (Vec<MinidumpInfo>, Vec<String>) {
    use super::dump::{HEADER_LEN, parse_dump_header};
    use std::io::Read;
    let root = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| r"C:\Windows".into())
        .join("Minidump");
    let mut warnings = Vec::new();
    let rd = match std::fs::read_dir(&root) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), warnings),
        Err(e) => {
            warnings.push(format!("Could not list {}: {e}", root.display()));
            return (Vec::new(), warnings);
        }
    };
    let mut files: Vec<(std::path::PathBuf, std::fs::Metadata)> = rd
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("dmp"))
        })
        .filter_map(|e| Some((e.path(), e.metadata().ok()?)))
        .filter(|(_, m)| m.is_file())
        .collect();
    files.sort_by_key(|(_, m)| std::cmp::Reverse(m.modified().ok()));
    if files.len() > 50 {
        warnings.push(format!("Showing the newest 50 of {} dump files.", files.len()));
        files.truncate(50);
    }
    let out = files
        .into_iter()
        .map(|(p, m)| {
            let mut info = MinidumpInfo {
                file: p
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                size: m.len(),
                modified: m
                    .modified()
                    .ok()
                    .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
                bugcheck_code: None,
                parameters: Vec::new(),
                error: None,
            };
            let mut buf = vec![0u8; HEADER_LEN];
            let read = std::fs::File::open(&p).and_then(|mut f| f.read_exact(&mut buf));
            match read
                .map_err(|e| e.to_string())
                .and_then(|_| parse_dump_header(&buf))
            {
                Ok(h) => {
                    info.bugcheck_code = Some(h.bugcheck_code);
                    info.parameters = h.parameters.iter().map(|x| format!("{x:#x}")).collect();
                }
                Err(e) => info.error = Some(e),
            }
            info
        })
        .collect();
    (out, warnings)
}
