//! "What changed, then what broke": driver installs, problems, crashes,
//! disconnects, boots and updates on one newest-first list. Repeats of the
//! same thing within 10 minutes collapse into one entry with a count.

use super::patterns::{SignalKind, is_duplicate_shutdown};
use crate::snapshot::{EventRecord, SetupApiStatus, SystemSnapshot};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

const COLLAPSE_SECS: i64 = 600;
const MAX_ENTRIES: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TimelineKind {
    DriverInstall,
    DriverInstallFailed,
    DeviceProblem,
    Bugcheck,
    UnexpectedShutdown,
    LiveKernelEvent,
    DisplayTdr,
    Whea,
    StorageReset,
    WlanDisconnect,
    Boot,
    Sleep,
    Wake,
    Update,
    UpdateFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TimelineEntry {
    /// Newest occurrence, RFC 3339 UTC.
    pub time: String,
    /// Oldest occurrence when `count > 1`.
    pub first_time: Option<String>,
    pub kind: TimelineKind,
    pub title: String,
    pub detail: Option<String>,
    pub device_id: Option<String>,
    pub count: u32,
}

fn entry(
    e: &EventRecord,
    kind: TimelineKind,
    title: String,
    detail: Option<String>,
    dev: &Option<String>,
) -> TimelineEntry {
    TimelineEntry {
        time: e.time.clone(),
        first_time: None,
        kind,
        title,
        detail,
        device_id: dev.clone(),
        count: 1,
    }
}

fn from_event(
    e: &EventRecord,
    sig: Option<SignalKind>,
    dev: &Option<String>,
) -> Option<TimelineEntry> {
    use TimelineKind as K;
    let is = |p: &str| e.provider.eq_ignore_ascii_case(p);
    let id = e.event_id;
    if let Some(s) = sig {
        let (kind, title) = match s {
            SignalKind::Bugcheck => (K::Bugcheck, "System crash (bugcheck)".to_string()),
            SignalKind::UnexpectedShutdown => (K::UnexpectedShutdown, "Unexpected shutdown".into()),
            SignalKind::LiveKernelEvent => (
                K::LiveKernelEvent,
                "Live kernel event (hardware/driver hang)".into(),
            ),
            SignalKind::DisplayTdr => (
                K::DisplayTdr,
                "Display driver stopped responding and recovered".into(),
            ),
            SignalKind::Whea => (K::Whea, "Hardware error reported (WHEA)".into()),
            SignalKind::StorageReset => (
                K::StorageReset,
                format!("Storage reset/timeout ({} {id})", e.provider),
            ),
            SignalKind::DeviceStartFailure => (K::DeviceProblem, "Device failed to start".into()),
            SignalKind::WlanDisconnect => (K::WlanDisconnect, "Wi-Fi disconnected".into()),
            SignalKind::UpdateInstallFailure => {
                (K::UpdateFailed, "Windows Update install failed".into())
            }
            // Too frequent and low-value on a timeline; shown on the Logs signal cards.
            SignalKind::WifiDriverWarning | SignalKind::DriverInstallFailure => return None,
        };
        let detail = match s {
            SignalKind::WlanDisconnect => e.field("ReasonCode").map(|r| format!("Reason code {r}")),
            SignalKind::UpdateInstallFailure => e.field("updateTitle").map(str::to_string),
            SignalKind::DeviceStartFailure => e.field("Problem").map(|p| format!("Problem {p}")),
            SignalKind::Bugcheck => e
                .field("BugcheckCode")
                .map(|c| format!("Bugcheck code {c}")),
            _ => None,
        };
        return Some(entry(e, kind, title, detail, dev));
    }
    if is("Microsoft-Windows-Kernel-PnP") && id == 400 && e.field("DeviceUpdated") == Some("true") {
        let name = e.field("DriverName").unwrap_or("?");
        let ver = e.field("DriverVersion").unwrap_or("?");
        let prov = e.field("DriverProvider").unwrap_or("?");
        return Some(entry(
            e,
            K::DriverInstall,
            format!("Driver installed: {name} {ver}"),
            Some(prov.to_string()),
            dev,
        ));
    }
    if is("Microsoft-Windows-UserPnp") && id == 20001 {
        let detail = e.field("DriverName").map(str::to_string);
        return Some(entry(
            e,
            K::DriverInstall,
            "Driver install (UserPnp)".into(),
            detail,
            dev,
        ));
    }
    if is("Microsoft-Windows-Kernel-General") && id == 12 {
        return Some(entry(e, K::Boot, "System started".into(), None, &None));
    }
    if is("Microsoft-Windows-Kernel-Power") && id == 42 {
        return Some(entry(e, K::Sleep, "Entered sleep".into(), None, &None));
    }
    if is("Microsoft-Windows-Power-Troubleshooter") && id == 1 {
        return Some(entry(e, K::Wake, "Resumed from sleep".into(), None, &None));
    }
    if is("Microsoft-Windows-WindowsUpdateClient") && id == 19 {
        let t = e.field("updateTitle").unwrap_or("update");
        return Some(entry(
            e,
            K::Update,
            format!("Update installed: {t}"),
            None,
            &None,
        ));
    }
    None
}

pub fn build(
    s: &SystemSnapshot,
    devices: &[Option<String>],
    kinds: &[Option<SignalKind>],
) -> Vec<TimelineEntry> {
    let mut v: Vec<TimelineEntry> = s
        .events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let k = kinds.get(i).copied().flatten();
            if k.is_some_and(|k| is_duplicate_shutdown(s, e, k)) {
                return None;
            }
            from_event(e, k, devices.get(i).unwrap_or(&None))
        })
        .collect();
    for sec in &s.setupapi {
        if sec.status != SetupApiStatus::Failure && sec.error_count == 0 {
            continue;
        }
        let Some(time) = sec.start.clone() else {
            continue;
        };
        let failed = sec.status == SetupApiStatus::Failure;
        v.push(TimelineEntry {
            time,
            first_time: None,
            kind: if failed {
                TimelineKind::DriverInstallFailed
            } else {
                TimelineKind::DeviceProblem
            },
            title: if failed {
                format!(
                    "Install failed{}",
                    sec.status_code
                        .as_deref()
                        .map(|c| format!(" ({c})"))
                        .unwrap_or_default()
                )
            } else {
                format!("Setup reported {} error(s)", sec.error_count)
            },
            detail: Some(
                sec.errors
                    .first()
                    .cloned()
                    .unwrap_or_else(|| sec.title.clone()),
            ),
            device_id: sec.instance_id.clone(),
            count: 1,
        });
    }
    v.sort_by(|a, b| b.time.cmp(&a.time));
    collapse(v)
}

fn ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// Input newest first. Merges an entry into an already-open one of the same
/// kind/title/device whose oldest occurrence is within the window.
fn collapse(v: Vec<TimelineEntry>) -> Vec<TimelineEntry> {
    let mut out: Vec<TimelineEntry> = Vec::new();
    for e in v {
        let t = ts(&e.time);
        let merge = out.iter_mut().rev().take(8).find(|o| {
            o.kind == e.kind
                && o.title == e.title
                && o.device_id == e.device_id
                && match (ts(o.first_time.as_deref().unwrap_or(&o.time)), t) {
                    (Some(a), Some(b)) => (a - b).num_seconds().abs() <= COLLAPSE_SECS,
                    _ => false,
                }
        });
        match merge {
            Some(o) => {
                o.count += 1;
                o.first_time = Some(e.time);
                if o.detail != e.detail {
                    // Keep only details shared by every merged occurrence.
                    o.detail = None;
                }
            }
            None => out.push(e),
        }
        if out.len() >= MAX_ENTRIES {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::patterns::classify;
    use crate::analyze::test_util::ev;

    #[test]
    fn installs_disconnects_and_collapse() {
        let wl = "Microsoft-Windows-WLAN-AutoConfig/Operational";
        let p = "Microsoft-Windows-WLAN-AutoConfig";
        let pnp = "Microsoft-Windows-Kernel-PnP";
        let events = vec![
            ev(wl, p, 8003, "2026-11-20T11:09:00Z", &[("ReasonCode", "7")]),
            ev(wl, p, 8003, "2026-11-20T11:04:00Z", &[("ReasonCode", "7")]),
            ev(wl, p, 8003, "2026-11-20T11:00:00Z", &[("ReasonCode", "7")]),
            ev(wl, p, 8003, "2026-11-20T10:00:00Z", &[("ReasonCode", "7")]),
            ev(
                "x",
                pnp,
                400,
                "2026-11-20T09:00:00Z",
                &[
                    ("DriverName", "oem48.inf"),
                    ("DriverVersion", "24.80.0.2"),
                    ("DeviceUpdated", "true"),
                ],
            ),
            ev(
                "x",
                pnp,
                400,
                "2026-11-20T08:00:00Z",
                &[("DriverName", "usb.inf"), ("DeviceUpdated", "false")],
            ),
        ];
        let s = SystemSnapshot {
            events,
            ..Default::default()
        };
        let kinds: Vec<_> = s.events.iter().map(classify).collect();
        let devs = vec![None; s.events.len()];
        let t = build(&s, &devs, &kinds);
        assert_eq!(t.len(), 3, "{t:#?}");
        assert_eq!((t[0].kind, t[0].count), (TimelineKind::WlanDisconnect, 3));
        assert_eq!(t[0].first_time.as_deref(), Some("2026-11-20T11:00:00Z"));
        assert_eq!(t[0].detail.as_deref(), Some("Reason code 7"));
        assert_eq!(t[1].count, 1, "an hour apart is a separate entry");
        assert_eq!(t[2].kind, TimelineKind::DriverInstall);
        assert_eq!(t[2].title, "Driver installed: oem48.inf 24.80.0.2");
    }

    #[test]
    fn kernel_power_41_and_6008_are_one_shutdown() {
        let s = SystemSnapshot {
            events: vec![
                ev(
                    "System",
                    "Microsoft-Windows-Kernel-Power",
                    41,
                    "2026-11-20T11:00:10Z",
                    &[("BugcheckCode", "0")],
                ),
                ev("System", "EventLog", 6008, "2026-11-20T11:00:05Z", &[]),
            ],
            ..Default::default()
        };
        let kinds: Vec<_> = s.events.iter().map(classify).collect();
        let t = build(&s, &[None, None], &kinds);
        assert_eq!(t.len(), 1, "{t:#?}");
        assert_eq!(
            (t[0].kind, t[0].count),
            (TimelineKind::UnexpectedShutdown, 1)
        );
    }
}
