//! `SystemSnapshot`: the single, serialisable input to every analysis step.
//!
//! Collectors fill it; analysis and the rule engine only read it. It is also the
//! exact JSON written by `--dump-snapshot`, so fields are stable and additive.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// v2: events, setupapi sections, dump files, `net_cfg_instance_id`.
/// New fields are `serde(default)` so older stored snapshots still load.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SystemSnapshot {
    pub schema_version: u32,
    /// RFC 3339, UTC.
    pub collected_at: String,
    pub machine: MachineInfo,
    pub devices: Vec<DeviceRecord>,
    /// Event log entries of interest, newest first, within `event_window_days`.
    #[serde(default)]
    pub events: Vec<EventRecord>,
    /// Parsed sections of `%windir%\INF\setupapi.dev.log`, newest first.
    #[serde(default)]
    pub setupapi: Vec<SetupApiSection>,
    /// Crash dump files (metadata only; contents need admin).
    #[serde(default)]
    pub dumps: Vec<DumpFile>,
    #[serde(default)]
    pub event_window_days: u32,
    /// Non-fatal problems while collecting (access denied, missing data, ...).
    pub warnings: Vec<SourceWarning>,
}

/// One Windows event, from its XML rendering (locale independent). Rules
/// match on `channel`/`provider`/`event_id`/`data`, never on `message`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventRecord {
    pub channel: String,
    pub provider: String,
    pub event_id: u32,
    /// 1 critical, 2 error, 3 warning, 4 information, 0/5 other.
    pub level: u8,
    /// RFC 3339, UTC.
    pub time: String,
    #[ts(type = "number")]
    pub record_id: u64,
    /// `<EventData>`/`<UserData>` values in document order. `name` is `None`
    /// for classic providers that log unnamed `<Data>` items.
    pub data: Vec<EventField>,
    /// Rendered message in the OS display language (display only).
    pub message: Option<String>,
}

impl EventRecord {
    /// Case-insensitive lookup of a named data field.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.data
            .iter()
            .find(|f| {
                f.name
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .map(|f| f.value.as_str())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventField {
    pub name: Option<String>,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SetupApiStatus {
    Success,
    Failure,
    /// Section has no `Exit status` line (still running or truncated log).
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetupApiSection {
    /// Text inside `>>>  [...]`, e.g. `Device Install (Hardware initiated) - PCI\VEN_...`.
    pub title: String,
    /// Device instance ID when the title names one.
    pub instance_id: Option<String>,
    /// RFC 3339 UTC (the log itself is written in local time).
    pub start: Option<String>,
    pub end: Option<String>,
    pub status: SetupApiStatus,
    /// e.g. `0xe0000203` from `FAILURE(0xe0000203)`.
    pub status_code: Option<String>,
    pub error_count: u32,
    pub warning_count: u32,
    /// First few `!!!` lines, trimmed.
    pub errors: Vec<String>,
    /// INF files mentioned in the section (lower-case file names).
    pub infs: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DumpFile {
    pub path: String,
    #[ts(type = "number")]
    pub size: u64,
    /// Last write time, RFC 3339 UTC.
    pub modified: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MachineInfo {
    pub os_build: Option<String>,
    pub os_display_version: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SourceWarning {
    /// Which collector, e.g. `devices`, `signature`, `power`.
    pub source: String,
    pub message: String,
    pub instance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceRecord {
    pub instance_id: String,
    /// Friendly name if set, else device description.
    pub name: String,
    pub description: Option<String>,
    pub class: Option<String>,
    pub class_guid: Option<String>,
    pub manufacturer: Option<String>,
    pub enumerator: Option<String>,
    pub location: Option<String>,
    pub service: Option<String>,
    pub hardware_ids: Vec<String>,
    pub compatible_ids: Vec<String>,
    /// False for "ghost" devices: installed before, not currently connected.
    pub present: bool,
    pub status: DeviceStatus,
    pub driver: Option<DriverInfo>,
    pub install: InstallDates,
    pub power: Option<PowerInfo>,
    /// Network adapters only: `NetCfgInstanceId` GUID from the driver key,
    /// used to tie WLAN/NDIS events (which log `InterfaceGuid`) to the device.
    #[serde(default)]
    pub net_cfg_instance_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceStatus {
    /// CM_PROB_* code, e.g. 10, 28, 43. `None` when the device has no problem.
    pub problem_code: Option<u32>,
    pub problem_text: Option<String>,
    /// Raw DN_* flags from CM_Get_DevNode_Status (0 when unavailable).
    pub devnode_flags: u32,
    pub started: bool,
    pub disabled: bool,
    /// False when the status could not be read (e.g. ghost device).
    pub available: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DriverInfo {
    pub provider: Option<String>,
    pub version: Option<String>,
    /// Date stamped in the INF (`DriverVer`), YYYY-MM-DD. Not the install date.
    pub date: Option<String>,
    pub description: Option<String>,
    /// e.g. `oem42.inf` or `netwtw08.inf` for inbox drivers.
    pub inf_name: Option<String>,
    pub inf_section: Option<String>,
    /// Hardware/compatible ID the driver was matched on.
    pub matching_device_id: Option<String>,
    /// DEVPKEY_Device_DriverRank (lower is better).
    pub rank: Option<u32>,
    /// Driver software key, e.g. `{4d36e972-e325-11ce-bfc1-08002be10318}\0003`.
    pub driver_key: Option<String>,
    pub signature: SignatureInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SignatureStatus {
    /// Catalog found and its signature chains to a trusted root.
    Valid,
    /// Catalog signed but chain is a test root / untrusted.
    TestSigned,
    /// Catalog found but signature check failed.
    Invalid,
    /// No catalog contains this INF.
    NotSigned,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SignatureInfo {
    pub status: SignatureStatus,
    pub signer: Option<String>,
    pub catalog: Option<String>,
    /// HRESULT/Win32 detail when not `Valid`, human readable.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallDates {
    /// When the current driver was installed on this device (RFC 3339).
    pub install_date: Option<String>,
    /// When this device was first installed on this machine.
    pub first_install_date: Option<String>,
    pub last_arrival_date: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PowerInfo {
    /// e.g. ["D0","D3"].
    pub supported_states: Vec<String>,
    pub wake_from: Vec<String>,
    pub current_state: Option<String>,
    /// "Allow the computer to turn off this device to save power" for network
    /// adapters (from PnPCapabilities). `None` when not applicable/unknown.
    pub allow_turn_off: Option<bool>,
    pub pnp_capabilities_raw: Option<u32>,
}

/// Progress events sent while a scan runs.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScanProgress {
    pub scan_id: i32,
    pub stage: String,
    pub done: u32,
    pub total: u32,
}

/// Human-readable names for CM_PROB_* codes (Device Manager "Code N").
pub fn problem_text(code: u32) -> &'static str {
    match code {
        1 => "Not configured correctly (Code 1)",
        3 => "Driver may be corrupted or out of memory (Code 3)",
        9 => "Invalid hardware ID reported by firmware (Code 9)",
        10 => "Device cannot start (Code 10)",
        12 => "Not enough free resources (Code 12)",
        14 => "Restart required (Code 14)",
        16 => "Cannot identify all resources (Code 16)",
        18 => "Reinstall the drivers (Code 18)",
        19 => "Registry configuration incomplete or damaged (Code 19)",
        21 => "Device is being removed (Code 21)",
        22 => "Device is disabled (Code 22)",
        24 => "Device not present or not working (Code 24)",
        28 => "Drivers are not installed (Code 28)",
        29 => "Disabled by firmware (Code 29)",
        31 => "Windows cannot load the drivers (Code 31)",
        32 => "Driver service disabled (Code 32)",
        33 => "Cannot determine required resources (Code 33)",
        34 => "Cannot determine settings (Code 34)",
        35 => "Firmware lacks information (Code 35)",
        36 => "IRQ translation failed (Code 36)",
        37 => "Driver returned failure (Code 37)",
        38 => "Previous driver instance still in memory (Code 38)",
        39 => "Driver corrupted or missing (Code 39)",
        40 => "Service registry key missing or invalid (Code 40)",
        41 => "Driver loaded but device not found (Code 41)",
        42 => "Duplicate device detected (Code 42)",
        43 => "Device reported a problem and was stopped (Code 43)",
        44 => "Application or service shut down the device (Code 44)",
        45 => "Device not connected (Code 45)",
        46 => "Windows is shutting down (Code 46)",
        47 => "Prepared for safe removal (Code 47)",
        48 => "Driver blocked from starting (Code 48)",
        49 => "System hive too large (Code 49)",
        50 => "Cannot apply all properties (Code 50)",
        51 => "Waiting on another device to start (Code 51)",
        52 => "Cannot verify digital signature (Code 52)",
        53 => "Reserved for kernel debugger (Code 53)",
        54 => "Device failed and is resetting (Code 54)",
        56 => "Waiting for network configuration (Code 56)",
        57 => "Device failed to start (Code 57)",
        _ => "Device problem",
    }
}
