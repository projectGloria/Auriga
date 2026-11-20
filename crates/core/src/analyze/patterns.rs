//! Event grouping, rates and named signals. Classification uses provider,
//! event ID and named data only (locale independent).

use crate::snapshot::{EventRecord, SetupApiStatus, SystemSnapshot};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SignalKind {
    // Ordered most severe first; `signals()` sorts by this.
    Bugcheck,
    UnexpectedShutdown,
    Whea,
    LiveKernelEvent,
    DisplayTdr,
    StorageReset,
    DeviceStartFailure,
    DriverInstallFailure,
    WlanDisconnect,
    WifiDriverWarning,
    UpdateInstallFailure,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Signal {
    pub kind: SignalKind,
    pub count: u32,
    pub last_24h: u32,
    pub last_7d: u32,
    /// Most events in any single clock hour.
    pub peak_per_hour: u32,
    pub first: Option<String>,
    pub last: Option<String>,
    /// Devices involved (instance IDs), most frequent first, at most 5.
    pub device_ids: Vec<String>,
    /// Kind-specific breakdown, e.g. WLAN `ReasonCode` or bugcheck code,
    /// as (value, count), most frequent first, at most 5.
    pub breakdown: Vec<(String, u32)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventGroup {
    pub channel: String,
    pub provider: String,
    pub event_id: u32,
    /// Most severe level seen (1 critical .. 4 info; 0 treated as info).
    pub level: u8,
    pub count: u32,
    pub last_24h: u32,
    pub last_7d: u32,
    pub peak_per_hour: u32,
    pub first: String,
    pub last: String,
    pub device_ids: Vec<String>,
    pub sample_message: Option<String>,
}

const STORAGE_PROVIDERS: &[(&str, &[u32])] = &[
    ("disk", &[7, 51, 153, 157]),
    ("stornvme", &[11, 129]),
    ("storahci", &[129]),
    ("storport", &[129]),
];

fn starts_ci(s: &str, prefix: &str) -> bool {
    s.get(..prefix.len())
        .is_some_and(|x| x.eq_ignore_ascii_case(prefix))
}

pub fn classify(e: &EventRecord) -> Option<SignalKind> {
    let p = e.provider.as_str();
    let is = |name: &str| p.eq_ignore_ascii_case(name);
    let id = e.event_id;
    use SignalKind::*;
    Some(match () {
        _ if is("Microsoft-Windows-WLAN-AutoConfig") && id == 8003 => WlanDisconnect,
        _ if starts_ci(p, "netwtw") && (1..=3).contains(&e.level) => WifiDriverWarning,
        _ if is("Display") && id == 4101 => DisplayTdr,
        _ if is("Microsoft-Windows-Kernel-Power") && id == 41 => match e.field("BugcheckCode") {
            Some(c) if !c.is_empty() && c != "0" && c != "0x0" => Bugcheck,
            _ => UnexpectedShutdown,
        },
        _ if is("EventLog") && id == 6008 => UnexpectedShutdown,
        _ if (is("Microsoft-Windows-WER-SystemErrorReporting") || is("BugCheck")) && id == 1001 => {
            Bugcheck
        }
        _ if is("Windows Error Reporting") && id == 1001 => {
            if e.data
                .iter()
                .any(|f| f.value.eq_ignore_ascii_case("BlueScreen"))
            {
                Bugcheck
            } else if e
                .data
                .iter()
                .any(|f| f.value.eq_ignore_ascii_case("LiveKernelEvent"))
            {
                LiveKernelEvent
            } else {
                return None;
            }
        }
        _ if is("Microsoft-Windows-WHEA-Logger") => Whea,
        _ if STORAGE_PROVIDERS
            .iter()
            .any(|(n, ids)| is(n) && ids.contains(&id)) =>
        {
            StorageReset
        }
        _ if starts_ci(p, "iastor") && id == 129 => StorageReset,
        _ if is("Microsoft-Windows-Kernel-PnP") && (id == 411 || id == 219) => DeviceStartFailure,
        _ if is("Microsoft-Windows-WindowsUpdateClient") && id == 20 => UpdateInstallFailure,
        _ => return None,
    })
}

/// Per-kind detail value for the breakdown.
fn detail(kind: SignalKind, e: &EventRecord) -> Option<String> {
    match kind {
        SignalKind::WlanDisconnect => e.field("ReasonCode").map(str::to_string),
        SignalKind::Bugcheck => e
            .field("BugcheckCode")
            .map(|c| match c.parse::<u64>() {
                Ok(n) => format!("0x{n:08X}"),
                Err(_) => c.to_string(),
            })
            .or_else(|| {
                e.data
                    .first()
                    .map(|f| f.value.clone())
                    .filter(|v| v.starts_with("0x"))
            }),
        SignalKind::UpdateInstallFailure => e.field("errorCode").map(str::to_string),
        SignalKind::DeviceStartFailure => e.field("Problem").map(str::to_string),
        SignalKind::StorageReset | SignalKind::WifiDriverWarning => {
            Some(format!("{} {}", e.provider, e.event_id))
        }
        _ => None,
    }
}

#[derive(Default)]
struct Acc {
    count: u32,
    d1: u32,
    d7: u32,
    hours: HashMap<i64, u32>,
    first: Option<String>,
    last: Option<String>,
    devices: BTreeMap<String, u32>,
    breakdown: BTreeMap<String, u32>,
}

impl Acc {
    fn add(&mut self, time: &str, now: DateTime<Utc>, device: Option<&str>) {
        self.count += 1;
        if let Ok(t) = DateTime::parse_from_rfc3339(time) {
            let t = t.with_timezone(&Utc);
            let age = now - t;
            if age <= Duration::hours(24) {
                self.d1 += 1;
            }
            if age <= Duration::days(7) {
                self.d7 += 1;
            }
            *self
                .hours
                .entry(t.timestamp().div_euclid(3600))
                .or_default() += 1;
        }
        if self.first.as_deref().is_none_or(|f| time < f) {
            self.first = Some(time.to_string());
        }
        if self.last.as_deref().is_none_or(|l| time > l) {
            self.last = Some(time.to_string());
        }
        if let Some(d) = device {
            *self.devices.entry(d.to_string()).or_default() += 1;
        }
    }
    fn peak(&self) -> u32 {
        self.hours.values().copied().max().unwrap_or(0)
    }
}

fn top(m: BTreeMap<String, u32>, n: usize) -> Vec<(String, u32)> {
    let mut v: Vec<_> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

pub fn groups(
    events: &[EventRecord],
    devices: &[Option<String>],
    now: DateTime<Utc>,
) -> Vec<EventGroup> {
    // (channel, provider, id) -> (stats, most severe level, first message)
    type Key<'a> = (&'a str, &'a str, u32);
    let mut m: HashMap<Key<'_>, (Acc, u8, Option<&str>)> = HashMap::new();
    for (i, e) in events.iter().enumerate() {
        let g = m
            .entry((&e.channel, &e.provider, e.event_id))
            .or_insert_with(|| (Acc::default(), 4, None));
        g.0.add(&e.time, now, devices.get(i).and_then(|d| d.as_deref()));
        let lvl = if e.level == 0 { 4 } else { e.level };
        g.1 = g.1.min(lvl);
        if g.2.is_none() {
            g.2 = e.message.as_deref();
        }
    }
    let mut out: Vec<EventGroup> = m
        .into_iter()
        .map(
            |((channel, provider, event_id), (acc, level, msg))| EventGroup {
                channel: channel.into(),
                provider: provider.into(),
                event_id,
                level,
                count: acc.count,
                last_24h: acc.d1,
                last_7d: acc.d7,
                peak_per_hour: acc.peak(),
                first: acc.first.clone().unwrap_or_default(),
                last: acc.last.clone().unwrap_or_default(),
                device_ids: top(acc.devices, 5).into_iter().map(|x| x.0).collect(),
                sample_message: msg.map(str::to_string),
            },
        )
        .collect();
    out.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| a.event_id.cmp(&b.event_id))
    });
    out
}

/// One incident logs both Kernel-Power 41 and EventLog 6008: count the 6008
/// only when no Kernel-Power 41 is within 10 minutes of it.
pub(crate) fn is_duplicate_shutdown(s: &SystemSnapshot, e: &EventRecord, k: SignalKind) -> bool {
    k == SignalKind::UnexpectedShutdown
        && e.provider.eq_ignore_ascii_case("EventLog")
        && has_kp41_near(s, e)
}

fn has_kp41_near(s: &SystemSnapshot, e: &EventRecord) -> bool {
    let Ok(t) = DateTime::parse_from_rfc3339(&e.time) else {
        return false;
    };
    s.events.iter().any(|o| {
        o.event_id == 41
            && o.provider
                .eq_ignore_ascii_case("Microsoft-Windows-Kernel-Power")
            && DateTime::parse_from_rfc3339(&o.time)
                .is_ok_and(|ot| (ot - t).num_seconds().abs() <= 600)
    })
}

pub fn signals(
    s: &SystemSnapshot,
    kinds: &[Option<SignalKind>],
    devices: &[Option<String>],
    now: DateTime<Utc>,
) -> Vec<Signal> {
    let mut m: BTreeMap<SignalKind, Acc> = BTreeMap::new();
    for (i, e) in s.events.iter().enumerate() {
        let Some(k) = kinds.get(i).copied().flatten() else {
            continue;
        };
        if is_duplicate_shutdown(s, e, k) {
            continue;
        }
        let acc = m.entry(k).or_default();
        acc.add(&e.time, now, devices.get(i).and_then(|d| d.as_deref()));
        if let Some(d) = detail(k, e) {
            *acc.breakdown.entry(d).or_default() += 1;
        }
    }
    for sec in s
        .setupapi
        .iter()
        .filter(|x| x.status == SetupApiStatus::Failure)
    {
        let acc = m.entry(SignalKind::DriverInstallFailure).or_default();
        acc.add(
            sec.start.as_deref().unwrap_or(""),
            now,
            sec.instance_id.as_deref(),
        );
        if let Some(c) = &sec.status_code {
            *acc.breakdown.entry(c.clone()).or_default() += 1;
        }
    }
    m.into_iter()
        .map(|(kind, acc)| Signal {
            kind,
            count: acc.count,
            last_24h: acc.d1,
            last_7d: acc.d7,
            peak_per_hour: acc.peak(),
            first: acc.first.clone().filter(|x| !x.is_empty()),
            last: acc.last.clone().filter(|x| !x.is_empty()),
            device_ids: top(acc.devices, 5).into_iter().map(|x| x.0).collect(),
            breakdown: top(acc.breakdown, 5),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::test_util::ev;

    const WLAN: &str = "Microsoft-Windows-WLAN-AutoConfig/Operational";

    #[test]
    fn classify_known_events() {
        let t = "2026-11-20T10:00:00Z";
        let k = |e: EventRecord| classify(&e);
        assert_eq!(
            k(ev(WLAN, "Microsoft-Windows-WLAN-AutoConfig", 8003, t, &[])),
            Some(SignalKind::WlanDisconnect)
        );
        let mut w = ev("System", "Netwtw10", 6062, t, &[]);
        w.level = 3;
        assert_eq!(classify(&w), Some(SignalKind::WifiDriverWarning));
        w.level = 4;
        assert_eq!(
            classify(&w),
            None,
            "info-level driver events are not warnings"
        );
        assert_eq!(
            k(ev(
                "System",
                "Microsoft-Windows-Kernel-Power",
                41,
                t,
                &[("BugcheckCode", "0")]
            )),
            Some(SignalKind::UnexpectedShutdown)
        );
        assert_eq!(
            k(ev(
                "System",
                "Microsoft-Windows-Kernel-Power",
                41,
                t,
                &[("BugcheckCode", "278")]
            )),
            Some(SignalKind::Bugcheck)
        );
        assert_eq!(
            k(ev(
                "Application",
                "Windows Error Reporting",
                1001,
                t,
                &[("", "LiveKernelEvent")]
            )),
            Some(SignalKind::LiveKernelEvent)
        );
        assert_eq!(
            k(ev(
                "Application",
                "Windows Error Reporting",
                1001,
                t,
                &[("", "APPCRASH")]
            )),
            None
        );
        assert_eq!(
            k(ev("System", "stornvme", 129, t, &[])),
            Some(SignalKind::StorageReset)
        );
        assert_eq!(
            k(ev("System", "iaStorAC", 129, t, &[])),
            Some(SignalKind::StorageReset)
        );
        assert_eq!(k(ev("System", "disk", 1, t, &[])), None);
    }

    #[test]
    fn rates_peaks_and_breakdown() {
        let now: DateTime<Utc> = "2026-11-20T12:00:00Z".parse().unwrap();
        let p = "Microsoft-Windows-WLAN-AutoConfig";
        let events = vec![
            ev(
                WLAN,
                p,
                8003,
                "2026-11-20T11:50:00Z",
                &[("ReasonCode", "7")],
            ),
            ev(
                WLAN,
                p,
                8003,
                "2026-11-20T11:10:00Z",
                &[("ReasonCode", "7")],
            ),
            ev(
                WLAN,
                p,
                8003,
                "2026-11-20T11:05:00Z",
                &[("ReasonCode", "2")],
            ),
            ev(
                WLAN,
                p,
                8003,
                "2026-11-16T09:00:00Z",
                &[("ReasonCode", "7")],
            ),
            ev(
                WLAN,
                p,
                8003,
                "2026-11-08T09:00:00Z",
                &[("ReasonCode", "7")],
            ),
        ];
        let s = SystemSnapshot {
            events,
            ..Default::default()
        };
        let kinds: Vec<_> = s.events.iter().map(classify).collect();
        let devs = vec![Some("WIFI".to_string()); 5];
        let sig = signals(&s, &kinds, &devs, now);
        assert_eq!(sig.len(), 1);
        let w = &sig[0];
        assert_eq!(
            (w.count, w.last_24h, w.last_7d, w.peak_per_hour),
            (5, 3, 4, 3)
        );
        assert_eq!(w.breakdown[0], ("7".to_string(), 4));
        assert_eq!(w.device_ids, vec!["WIFI"]);
        assert_eq!(w.first.as_deref(), Some("2026-11-08T09:00:00Z"));
        assert_eq!(w.last.as_deref(), Some("2026-11-20T11:50:00Z"));

        let g = groups(&s.events, &devs, now);
        assert_eq!(g.len(), 1);
        assert_eq!((g[0].count, g[0].level), (5, 4));
    }

    #[test]
    fn setupapi_failures_become_signal() {
        use crate::snapshot::SetupApiSection;
        let s = SystemSnapshot {
            setupapi: vec![
                SetupApiSection {
                    status: SetupApiStatus::Failure,
                    status_code: Some("0xe0000203".into()),
                    instance_id: Some("PCI\\X".into()),
                    start: Some("2026-11-20T10:00:00.000Z".into()),
                    ..Default::default()
                },
                SetupApiSection {
                    status: SetupApiStatus::Success,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let sig = signals(&s, &[], &[], "2026-11-20T12:00:00Z".parse().unwrap());
        assert_eq!(sig[0].kind, SignalKind::DriverInstallFailure);
        assert_eq!((sig[0].count, sig[0].last_24h), (1, 1));
    }
}
