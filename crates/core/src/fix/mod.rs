//! Guided, reversible fixes. The engine (`engine`) works over a
//! [`SystemBackend`] so it can be tested without touching the machine; the
//! real backend reads live state as the user and writes only through the
//! elevated helper, which re-runs the same checks (`validate`) itself.

pub mod dump;
pub mod engine;
pub mod monitor;
pub mod validate;
#[cfg(windows)]
pub mod win;

use crate::analyze::SignalKind;
use crate::kb::{FixAction, Risk, ValueText};
use crate::snapshot::DeviceRecord;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use helper_ipc::proto::{MinidumpInfo, RestorePointOutcome};

/// What the system side must provide. Reads run as the user; every write
/// goes to the elevated helper in the real implementation.
pub trait SystemBackend {
    /// Live read of one device (advanced properties and power included).
    fn read_device(&mut self, instance_id: &str) -> Result<Option<DeviceRecord>, String>;
    /// Set (`Some`) or delete (`None`) an adapter value; returns the previous
    /// stored value.
    fn set_adapter_property(
        &mut self,
        instance_id: &str,
        keyword: &str,
        value: Option<&str>,
    ) -> Result<Option<String>, String>;
    /// Returns the previous stored `PnPCapabilities`.
    fn set_pnp_capabilities(
        &mut self,
        instance_id: &str,
        value: Option<u32>,
    ) -> Result<Option<u32>, String>;
    /// Returns whether Windows says a reboot is needed.
    fn restart_device(&mut self, instance_id: &str) -> Result<bool, String>;
    fn create_restore_point(&mut self, description: &str) -> RestorePointOutcome;
    /// Returns whether Windows says a reboot is needed.
    fn rollback_driver(&mut self, instance_id: &str) -> Result<bool, String>;
}

/// A fix as suggested by a finding; built by the app from the current KB
/// evaluation, never from UI input.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FixTarget {
    pub finding_key: String,
    pub rule_id: String,
    pub rule_version: u32,
    pub fix_id: String,
    pub title: String,
    pub risk: Risk,
    pub device_id: String,
    pub device_name: String,
    pub action: FixAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ActionKind {
    AdapterProperty,
    DevicePower,
    RollbackDriver,
}

/// Exactly what applying would change, computed from the live device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FixPlan {
    pub kind: ActionKind,
    /// Human name of the setting, e.g. "MIMO Power Save Mode".
    pub setting: String,
    /// Registry value name written (canonical keyword, `PnPCapabilities`), or
    /// "driver" for a rollback.
    pub target: String,
    /// Effective value now (stored or driver default).
    pub from: Option<ValueText>,
    pub to: ValueText,
    /// Raw value stored now; `None` = not stored (driver default applies).
    pub prior_raw: Option<String>,
    /// Raw value that will be written.
    pub new_raw: Option<String>,
    pub restarts_device: bool,
    pub revertible: bool,
    /// Suggested default for the restore-point checkbox.
    pub restore_point_default: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Preview {
    Ready { plan: FixPlan },
    AlreadySet { value: ValueText },
    Refused { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApplyOptions {
    pub restore_point: bool,
    /// The user saw that no restore point was created and chose to go on.
    pub continue_without_restore_point: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FixState {
    /// Written to history, change not finished (app closed mid-way → Unknown).
    Pending,
    Applied,
    /// The write failed; nothing changed.
    Failed,
    /// Written, but the re-read value is not what was set.
    VerifyFailed,
    Reverted,
    RevertFailed,
    /// Interrupted; the device's current value must be checked.
    Unknown,
}

/// Signal counts for the device when the fix was applied.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Baseline {
    pub collected_at: String,
    pub window_days: u32,
    pub counts: Vec<SignalCount>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SignalCount {
    pub kind: SignalKind,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FixRecord {
    #[ts(type = "number")]
    pub id: i64,
    pub created_at: String,
    pub finding_key: String,
    pub rule_id: String,
    pub rule_version: u32,
    pub fix_id: String,
    pub title: String,
    pub device_id: String,
    pub device_name: String,
    pub kind: ActionKind,
    pub setting: String,
    pub target: String,
    pub from: Option<ValueText>,
    pub to: ValueText,
    /// Stored value before the change; `None` = absent (driver default).
    pub prior_raw: Option<String>,
    pub new_raw: Option<String>,
    pub state: FixState,
    pub error: Option<String>,
    pub notes: Vec<String>,
    pub restore_point: Option<RestorePointOutcome>,
    pub reboot_required: bool,
    pub revertible: bool,
    pub applied_at: Option<String>,
    pub reverted_at: Option<String>,
    pub baseline: Option<Baseline>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum ApplyOutcome {
    Refused {
        reason: String,
    },
    /// The live value differs from what the preview showed; nothing changed.
    ChangedSincePreview {
        message: String,
    },
    /// Restore point requested but not created; nothing changed yet.
    RestorePointNotCreated {
        outcome: RestorePointOutcome,
    },
    /// The attempt is in history; `record.state` says how it went.
    Done {
        record: Box<FixRecord>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum RevertOutcome {
    Refused { reason: String },
    Done { record: Box<FixRecord> },
}

#[cfg(test)]
pub(crate) mod testutil {
    use crate::snapshot::{AdvancedProperty, DeviceRecord, DriverInfo, PowerInfo, PropertyOption};

    pub const NET_GUID: &str = "{4d36e972-e325-11ce-bfc1-08002be10318}";
    pub const DEV: &str = r"PCI\VEN_8086&DEV_43F0&SUBSYS_00748086&REV_00\3&11583659&0&A3";

    pub fn adapter() -> DeviceRecord {
        DeviceRecord {
            instance_id: DEV.into(),
            name: "Intel(R) Wi-Fi 6 AX201 160MHz".into(),
            class: Some("Net".into()),
            class_guid: Some(NET_GUID.into()),
            enumerator: Some("PCI".into()),
            present: true,
            driver: Some(DriverInfo {
                version: Some("23.60.0.10".into()),
                provider: Some("Intel".into()),
                driver_key: Some(format!(r"{NET_GUID}\0001")),
                ..Default::default()
            }),
            power: Some(PowerInfo {
                allow_turn_off: Some(true),
                pnp_capabilities_raw: None,
                ..Default::default()
            }),
            advanced: vec![
                AdvancedProperty {
                    keyword: "MIMOPowerSaveMode".into(),
                    label: Some("MIMO Power Save Mode".into()),
                    kind: "enum".into(),
                    default: Some("3".into()),
                    current: Some("0".into()),
                    options: vec![
                        PropertyOption {
                            value: "0".into(),
                            label: "Auto SMPS".into(),
                        },
                        PropertyOption {
                            value: "3".into(),
                            label: "No SMPS".into(),
                        },
                    ],
                    min: None,
                    max: None,
                },
                AdvancedProperty {
                    keyword: "ChannelWidth24".into(),
                    label: Some("Channel Width for 2.4GHz".into()),
                    kind: "enum".into(),
                    default: Some("1".into()),
                    current: None,
                    options: vec![
                        PropertyOption {
                            value: "0".into(),
                            label: "20 MHz Only".into(),
                        },
                        PropertyOption {
                            value: "1".into(),
                            label: "Auto".into(),
                        },
                    ],
                    min: None,
                    max: None,
                },
            ],
            ..Default::default()
        }
    }
}
