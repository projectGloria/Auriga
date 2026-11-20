//! Does the installed driver actually fit the hardware?
//!
//! We compare `DriverInfo::matching_device_id` against the device's own
//! hardware-ID and compatible-ID lists (most specific first). That is the same
//! information Windows used to pick the driver, and it needs no decoding of
//! the undocumented-in-detail `DriverRank` bit layout.

use crate::snapshot::{DeviceRecord, SignatureStatus};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Level {
    Good,
    Info,
    Warn,
    Bad,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "kind")]
#[ts(export)]
pub enum MatchKind {
    /// Matched the device's hardware ID at `index` (0 = most specific).
    HardwareId { index: u32 },
    /// Matched only a compatible ID: a class/generic driver.
    CompatibleId { index: u32 },
    /// Driver present but its matching ID is not in either list
    /// (common for software/root-enumerated devices).
    Unlisted,
    /// Device is present and has no driver.
    NoDriver,
    /// Not applicable (ghost device, or a device that never takes a driver).
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MatchAssessment {
    pub instance_id: String,
    pub level: Level,
    pub kind: MatchKind,
    /// Driver ships with Windows (INF not `oemNN.inf`).
    pub inbox: bool,
    /// Short, plain-language reasons for `level`.
    pub notes: Vec<String>,
}

/// Classes where an *inbox* driver bound by compatible ID usually means
/// missing features, so it is worth flagging. Not included on purpose:
/// NVMe/AHCI (inbox stornvme/storahci are the normal choice), and USB class
/// devices (UVC webcams, USB audio) whose class drivers are the intended
/// driver; those match `USB\Class_*` and are skipped below.
const VENDOR_DRIVER_EXPECTED: &[&str] = &["Display", "Net", "MEDIA", "Bluetooth"];

/// Buses where a less specific hardware ID can mean a generic rather than an
/// OEM-tuned driver. ACPI lists aliases of one ID (`ACPI\VEN_ACPI&DEV_0011`
/// and `ACPI\ACPI0011`), and software buses carry no hardware meaning.
const SPECIFIC_ID_BUSES: &[&str] = &["PCI", "USB", "HDAUDIO", "INTELAUDIO", "BTHENUM", "BTHLE"];

pub fn assess(d: &DeviceRecord) -> MatchAssessment {
    let mut notes = Vec::new();
    let mut level = Level::Good;
    let mut bump = |l: Level, n: String, notes: &mut Vec<String>| {
        level = level.max(l);
        notes.push(n);
    };

    let Some(drv) = d.driver.as_ref() else {
        let kind = if d.present && d.status.problem_code == Some(28) {
            bump(
                Level::Bad,
                "No driver is installed for this device.".into(),
                &mut notes,
            );
            MatchKind::NoDriver
        } else {
            MatchKind::None
        };
        return MatchAssessment {
            instance_id: d.instance_id.clone(),
            level,
            kind,
            inbox: false,
            notes,
        };
    };

    let inbox = drv
        .inf_name
        .as_deref()
        .is_some_and(|i| !i.to_ascii_lowercase().starts_with("oem"));

    let kind = match drv.matching_device_id.as_deref() {
        None => MatchKind::Unlisted,
        Some(m) => {
            let pos = |list: &[String]| list.iter().position(|h| h.eq_ignore_ascii_case(m));
            if let Some(i) = pos(&d.hardware_ids) {
                MatchKind::HardwareId { index: i as u32 }
            } else if let Some(i) = pos(&d.compatible_ids) {
                MatchKind::CompatibleId { index: i as u32 }
            } else {
                MatchKind::Unlisted
            }
        }
    };

    if d.present {
        match &kind {
            MatchKind::HardwareId { index }
                if *index > 0
                    && !only_revision_dropped(d, drv.matching_device_id.as_deref())
                    && d.enumerator.as_deref().is_some_and(|e| {
                        SPECIFIC_ID_BUSES.iter().any(|b| b.eq_ignore_ascii_case(e))
                    }) =>
            {
                bump(
                    Level::Info,
                    format!(
                        "Driver matched a less specific hardware ID ({}), not the exact one ({}). \
                     Usually fine; an OEM-specific driver may exist.",
                        drv.matching_device_id.as_deref().unwrap_or("?"),
                        d.hardware_ids.first().map(String::as_str).unwrap_or("?"),
                    ),
                    &mut notes,
                )
            }
            MatchKind::CompatibleId { .. } => {
                let class = d.class.as_deref().unwrap_or("");
                let usb_class = drv
                    .matching_device_id
                    .as_deref()
                    .and_then(|m| m.get(..10))
                    .is_some_and(|p| p.eq_ignore_ascii_case(r"USB\Class_"));
                if inbox
                    && !usb_class
                    && VENDOR_DRIVER_EXPECTED
                        .iter()
                        .any(|c| c.eq_ignore_ascii_case(class))
                {
                    bump(
                        Level::Warn,
                        format!(
                            "A generic class driver is in use (matched compatible ID {}). \
                             A vendor driver for this {class} device may add features or fix problems.",
                            drv.matching_device_id.as_deref().unwrap_or("?")
                        ),
                        &mut notes,
                    );
                } else if inbox {
                    notes
                        .push("Generic Windows class driver (normal for this device type).".into());
                } else {
                    notes.push(
                        "Vendor driver bound by a compatible ID (normal for multi-device driver packages)."
                            .into(),
                    );
                }
            }
            _ => {}
        }
        // Only a real GPU (PCI) on BasicDisplay is a problem; the root-enumerated
        // "Microsoft Basic Display Driver" device exists on every system.
        if d.service
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("BasicDisplay"))
            && d.enumerator
                .as_deref()
                .is_some_and(|e| e.eq_ignore_ascii_case("PCI"))
        {
            bump(
                Level::Bad,
                "Microsoft Basic Display Adapter is in use: the GPU driver is missing or failed."
                    .into(),
                &mut notes,
            );
        }
    }

    match drv.signature.status {
        SignatureStatus::Valid => {}
        SignatureStatus::TestSigned => bump(
            Level::Warn,
            "Driver is test-signed (not a production signature).".into(),
            &mut notes,
        ),
        SignatureStatus::Invalid => bump(
            Level::Bad,
            format!(
                "Driver signature check failed{}.",
                drv.signature
                    .detail
                    .as_deref()
                    .map(|x| format!(": {x}"))
                    .unwrap_or_default()
            ),
            &mut notes,
        ),
        SignatureStatus::NotSigned => bump(
            Level::Bad,
            "No signed catalog covers this driver's INF.".into(),
            &mut notes,
        ),
        SignatureStatus::Unknown => bump(
            Level::Info,
            "Signature could not be checked.".into(),
            &mut notes,
        ),
    }

    if let Some(code) = d.status.problem_code.filter(|_| d.present) {
        bump(
            Level::Bad,
            format!(
                "Device reports a problem: {}.",
                crate::snapshot::problem_text(code)
            ),
            &mut notes,
        );
    }

    MatchAssessment {
        instance_id: d.instance_id.clone(),
        level,
        kind,
        inbox,
        notes,
    }
}

/// `PCI\VEN_x&DEV_y&SUBSYS_z` matching a device whose top ID only adds
/// `&REV_nn`: vendors routinely omit the revision, so this is still exact.
fn only_revision_dropped(d: &DeviceRecord, matched: Option<&str>) -> bool {
    let (Some(m), Some(top)) = (matched, d.hardware_ids.first()) else {
        return false;
    };
    top.len() > m.len()
        && top.is_char_boundary(m.len())
        && top[..m.len()].eq_ignore_ascii_case(m)
        && top[m.len()..]
            .get(..5)
            .is_some_and(|r| r.eq_ignore_ascii_case("&REV_"))
        && !top[m.len() + 5..].contains('&')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{DeviceStatus, DriverInfo, SignatureInfo};

    fn dev(matching: &str, class: &str) -> DeviceRecord {
        DeviceRecord {
            instance_id: "PCI\\X\\1".into(),
            name: "x".into(),
            description: None,
            class: Some(class.into()),
            class_guid: None,
            manufacturer: None,
            enumerator: Some("PCI".into()),
            location: None,
            service: None,
            hardware_ids: vec![
                "PCI\\VEN_8086&DEV_06F0&SUBSYS_00748086&REV_00".into(),
                "PCI\\VEN_8086&DEV_06F0&SUBSYS_00748086".into(),
                "PCI\\VEN_8086&DEV_06F0".into(),
            ],
            compatible_ids: vec!["PCI\\VEN_8086&CC_028000".into(), "PCI\\CC_0280".into()],
            present: true,
            status: DeviceStatus {
                available: true,
                started: true,
                ..Default::default()
            },
            driver: Some(DriverInfo {
                inf_name: Some("oem12.inf".into()),
                matching_device_id: Some(matching.into()),
                signature: SignatureInfo {
                    status: SignatureStatus::Valid,
                    ..Default::default()
                },
                ..Default::default()
            }),
            install: Default::default(),
            power: None,
            net_cfg_instance_id: None,
        }
    }

    #[test]
    fn exact_hardware_id_is_good() {
        let a = assess(&dev("pci\\ven_8086&dev_06f0&subsys_00748086&rev_00", "Net"));
        assert_eq!(a.kind, MatchKind::HardwareId { index: 0 });
        assert_eq!(a.level, Level::Good);
        assert!(!a.inbox);
    }

    #[test]
    fn less_specific_hardware_id_is_info() {
        let a = assess(&dev("PCI\\VEN_8086&DEV_06F0", "Net"));
        assert_eq!(a.kind, MatchKind::HardwareId { index: 2 });
        assert_eq!(a.level, Level::Info);
    }

    #[test]
    fn dropped_revision_only_is_exact_enough() {
        let a = assess(&dev("PCI\\VEN_8086&DEV_06F0&SUBSYS_00748086", "Net"));
        assert_eq!(a.kind, MatchKind::HardwareId { index: 1 });
        assert_eq!(a.level, Level::Good, "{:?}", a.notes);
        assert!(a.notes.is_empty());
    }

    #[test]
    fn generic_driver_on_net_is_warn_but_on_usb_good() {
        let inbox = |m: &str, class: &str| {
            let mut d = dev(m, class);
            d.driver.as_mut().unwrap().inf_name = Some("netgeneric.inf".into());
            d
        };
        assert_eq!(assess(&inbox("PCI\\CC_0280", "Net")).level, Level::Warn);
        assert_eq!(assess(&inbox("PCI\\CC_0280", "USB")).level, Level::Good);
        // A vendor (oem) package matching a compatible ID is normal.
        assert_eq!(assess(&dev("PCI\\CC_0280", "Net")).level, Level::Good);
        // USB Audio/Video class drivers are the intended drivers.
        let mut d = inbox("USB\\Class_01", "MEDIA");
        d.compatible_ids.push("USB\\Class_01".into());
        assert_eq!(assess(&d).level, Level::Good);
    }

    #[test]
    fn basic_display_only_bad_on_a_real_gpu() {
        let mut d = dev("PCI\\CC_0300", "Display");
        d.service = Some("BasicDisplay".into());
        assert_eq!(assess(&d).level, Level::Bad);
        d.enumerator = Some("ROOT".into());
        assert_eq!(assess(&d).level, Level::Good);
    }

    #[test]
    fn acpi_alias_ids_are_not_flagged() {
        let mut d = dev("PCI\\VEN_8086&DEV_06F0", "System");
        d.enumerator = Some("ACPI".into());
        assert_eq!(assess(&d).level, Level::Good);
    }

    #[test]
    fn unsigned_and_problem_are_bad() {
        let mut d = dev("PCI\\VEN_8086&DEV_06F0", "Net");
        d.driver.as_mut().unwrap().signature.status = SignatureStatus::NotSigned;
        assert_eq!(assess(&d).level, Level::Bad);
        let mut d = dev("PCI\\VEN_8086&DEV_06F0&SUBSYS_00748086&REV_00", "Net");
        d.status.problem_code = Some(43);
        let a = assess(&d);
        assert_eq!(a.level, Level::Bad);
        assert!(a.notes.iter().any(|n| n.contains("Code 43")));
    }

    #[test]
    fn missing_driver_is_bad_only_for_code_28() {
        let mut d = dev("x", "Net");
        d.driver = None;
        assert_eq!(assess(&d).kind, MatchKind::None);
        d.status.problem_code = Some(28);
        let a = assess(&d);
        assert_eq!(a.kind, MatchKind::NoDriver);
        assert_eq!(a.level, Level::Bad);
    }
}
