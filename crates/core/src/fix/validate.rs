//! Pure checks shared by the app (preview) and the elevated helper (before
//! every write). They decide *where* a write may go from the device itself,
//! never from the request.

use crate::snapshot::DeviceRecord;

/// Network adapter device setup class (devguid.h GUID_DEVCLASS_NET).
pub const NET_CLASS_GUID: &str = "{4d36e972-e325-11ce-bfc1-08002be10318}";

/// ndis.h NDIS_DEVICE_DISABLE_PM: when set in `PnPCapabilities`, "Allow the
/// computer to turn off this device to save power" is unchecked.
pub const DISABLE_PM_BIT: u32 = 0x8;

/// The device must be a present network adapter whose driver key is exactly
/// `{net class guid}\NNNN`. Returns the driver key (the only registry key a
/// write may touch, under `HKLM\...\Control\Class`).
pub fn require_net_adapter(d: &DeviceRecord) -> Result<&str, String> {
    let is_net = d
        .class_guid
        .as_deref()
        .is_some_and(|g| g.eq_ignore_ascii_case(NET_CLASS_GUID));
    if !is_net {
        return Err(format!("{} is not a network adapter.", d.name));
    }
    if !d.present {
        return Err(format!("{} is not connected right now.", d.name));
    }
    let key = d
        .driver
        .as_ref()
        .and_then(|x| x.driver_key.as_deref())
        .ok_or_else(|| format!("{} has no driver key.", d.name))?;
    let ok = key.len() == NET_CLASS_GUID.len() + 5
        && key.is_char_boundary(NET_CLASS_GUID.len())
        && key[..NET_CLASS_GUID.len()].eq_ignore_ascii_case(NET_CLASS_GUID)
        && key.as_bytes()[NET_CLASS_GUID.len()] == b'\\'
        && key.as_bytes()[NET_CLASS_GUID.len() + 1..]
            .iter()
            .all(u8::is_ascii_digit);
    if !ok {
        return Err(format!("Unexpected driver key {key:?} for {}.", d.name));
    }
    Ok(key)
}

/// Checks an Advanced-tab write against what the driver on this machine
/// declares (matched on registry keyword, not display name). `None` deletes
/// the stored value, which makes the driver use its default. Returns the
/// keyword with the driver's own capitalisation.
pub fn check_property_write<'a>(
    d: &'a DeviceRecord,
    keyword: &str,
    value: Option<&str>,
) -> Result<&'a str, String> {
    require_net_adapter(d)?;
    let p = d.advanced(keyword).ok_or_else(|| {
        format!(
            "The driver of {} does not have the setting {keyword}; nothing was changed.",
            d.name
        )
    })?;
    if let Some(v) = value {
        if !helper_ipc::proto::valid_text(v, 64) {
            return Err("Invalid value.".into());
        }
        if !p.accepts(v) {
            let name = p.label.as_deref().unwrap_or(&p.keyword);
            return Err(format!(
                "{name} on this driver does not accept the value {v}; nothing was changed."
            ));
        }
    }
    Ok(&p.keyword)
}

/// `PnPCapabilities` may change only in the "allow turn off" bit, and may be
/// deleted only when no other bit is set (deleting = value 0).
pub fn check_pnp_write(current: Option<u32>, new: Option<u32>) -> Result<(), String> {
    let cur = current.unwrap_or(0);
    let new_v = new.unwrap_or(0);
    if (cur ^ new_v) & !DISABLE_PM_BIT != 0 {
        return Err(format!(
            "Refusing to change PnPCapabilities from {cur:#x} to {new_v:#x}: only the power-management bit may change."
        ));
    }
    Ok(())
}

/// The `PnPCapabilities` value that gives `allow_turn_off`, keeping other
/// bits. `None` = delete (only when nothing else would remain and the value
/// was absent before, so a revert restores exactly the old state).
pub fn pnp_target(current: Option<u32>, allow_turn_off: bool) -> Option<u32> {
    let cur = current.unwrap_or(0);
    let v = if allow_turn_off {
        cur & !DISABLE_PM_BIT
    } else {
        cur | DISABLE_PM_BIT
    };
    if v == 0 && current.is_none() {
        None
    } else {
        Some(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fix::testutil::{NET_GUID, adapter};

    #[test]
    fn net_adapter_required() {
        let d = adapter();
        assert_eq!(require_net_adapter(&d).unwrap(), format!(r"{NET_GUID}\0001"));

        let mut upper = adapter();
        upper.class_guid = Some(NET_GUID.to_ascii_uppercase());
        assert!(require_net_adapter(&upper).is_ok());

        let mut disk = adapter();
        disk.class_guid = Some("{4d36e967-e325-11ce-bfc1-08002be10318}".into());
        assert!(require_net_adapter(&disk).is_err());

        let mut gone = adapter();
        gone.present = false;
        assert!(require_net_adapter(&gone).is_err());

        for bad in [
            format!(r"{NET_GUID}\0001\..\..\Services"),
            format!(r"{NET_GUID}\00a1"),
            format!(r"{NET_GUID}/0001"),
            "{4d36e967-e325-11ce-bfc1-08002be10318}\\0001".to_string(),
            String::new(),
        ] {
            let mut d = adapter();
            d.driver.as_mut().unwrap().driver_key = Some(bad.clone());
            assert!(require_net_adapter(&d).is_err(), "{bad}");
        }
    }

    #[test]
    fn property_write_checks_live_definition() {
        let d = adapter();
        assert_eq!(
            check_property_write(&d, "mimopowersavemode", Some("3")).unwrap(),
            "MIMOPowerSaveMode"
        );
        assert!(check_property_write(&d, "MIMOPowerSaveMode", None).is_ok());
        let e = check_property_write(&d, "MIMOPowerSaveMode", Some("2")).unwrap_err();
        assert!(e.contains("does not accept"), "{e}");
        let e = check_property_write(&d, "RoamAggressiveness", Some("1")).unwrap_err();
        assert!(e.contains("does not have"), "{e}");
    }

    #[test]
    fn pnp_only_power_bit() {
        assert!(check_pnp_write(None, Some(0x8)).is_ok());
        assert!(check_pnp_write(Some(0x18), Some(0x10)).is_ok());
        assert!(check_pnp_write(Some(0x8), None).is_ok());
        assert!(check_pnp_write(Some(0x18), None).is_err());
        assert!(check_pnp_write(None, Some(0x18)).is_err());
        assert!(check_pnp_write(Some(0x10), Some(0x0)).is_err());

        assert_eq!(pnp_target(None, false), Some(0x8));
        assert_eq!(pnp_target(Some(0x8), true), Some(0));
        assert_eq!(pnp_target(Some(0x18), true), Some(0x10));
        assert_eq!(pnp_target(Some(0x10), true), Some(0x10));
        assert_eq!(pnp_target(None, true), None);
    }
}
