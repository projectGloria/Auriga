//! "Try one fix, then monitor": signal counts for the device before the fix
//! (baseline) against counts since it was applied, from the same scan data.

use super::{Baseline, FixRecord, SignalCount};
use crate::analyze::{Analysis, SignalKind};
use crate::snapshot::SystemSnapshot;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// Days of data needed after a fix before before/after says anything.
pub const MIN_DAYS_AFTER: f64 = 2.0;

fn parse(t: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(t)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// Per-signal counts of events attributed to `device_id` in
/// `[from, to)` (open ends = unbounded).
fn counts(
    snap: &SystemSnapshot,
    a: &Analysis,
    device_id: &str,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
) -> BTreeMap<SignalKind, u32> {
    let mut m = BTreeMap::new();
    for (i, e) in snap.events.iter().enumerate() {
        let (Some(Some(dev)), Some(Some(kind))) = (a.event_device.get(i), a.event_signal.get(i))
        else {
            continue;
        };
        if !dev.eq_ignore_ascii_case(device_id) {
            continue;
        }
        let Some(t) = parse(&e.time) else { continue };
        if from.is_some_and(|f| t < f) || to.is_some_and(|x| t >= x) {
            continue;
        }
        *m.entry(*kind).or_insert(0) += 1;
    }
    m
}

/// Baseline for a fix applied now, from the latest scan.
pub fn baseline(device_id: &str, snap: &SystemSnapshot, a: &Analysis) -> Baseline {
    Baseline {
        collected_at: snap.collected_at.clone(),
        window_days: snap.event_window_days,
        counts: counts(snap, a, device_id, None, None)
            .into_iter()
            .map(|(kind, count)| SignalCount { kind, count })
            .collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SignalTrend {
    pub kind: SignalKind,
    pub before_per_day: f64,
    pub after_count: u32,
    pub after_per_day: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MonitorReport {
    pub days_since: f64,
    /// Under [`MIN_DAYS_AFTER`]: the numbers are shown but not judged.
    pub too_early: bool,
    pub trends: Vec<SignalTrend>,
    /// Plain-language summary.
    pub summary: String,
}

/// Before/after for an applied fix, using a scan taken after it.
pub fn compare(r: &FixRecord, snap: &SystemSnapshot, a: &Analysis) -> Option<MonitorReport> {
    let b = r.baseline.as_ref()?;
    let applied = parse(r.applied_at.as_deref()?)?;
    let scanned = parse(&snap.collected_at)?;
    if scanned < applied {
        return None;
    }
    let days_since = ((scanned - applied).num_minutes() as f64 / 1440.0).max(0.0);
    let after = counts(snap, a, &r.device_id, Some(applied), None);
    let window = f64::from(b.window_days.max(1));

    let mut kinds: Vec<SignalKind> = b.counts.iter().map(|c| c.kind).collect();
    kinds.extend(after.keys().copied());
    kinds.sort();
    kinds.dedup();

    let trends: Vec<SignalTrend> = kinds
        .into_iter()
        .map(|k| {
            let before = b.counts.iter().find(|c| c.kind == k).map_or(0, |c| c.count);
            let n = after.get(&k).copied().unwrap_or(0);
            SignalTrend {
                kind: k,
                before_per_day: f64::from(before) / window,
                after_count: n,
                after_per_day: if days_since > 0.0 {
                    f64::from(n) / days_since
                } else {
                    0.0
                },
            }
        })
        .collect();

    let too_early = days_since < MIN_DAYS_AFTER;
    let before_total: f64 = trends.iter().map(|t| t.before_per_day).sum();
    let after_total: f64 = trends.iter().map(|t| t.after_per_day).sum();
    let summary = if too_early {
        format!(
            "{days_since:.1} days since the fix. Keep using the device normally and scan again after {MIN_DAYS_AFTER:.0} days."
        )
    } else if before_total == 0.0 && after_total == 0.0 {
        "No related events before or after the fix.".to_string()
    } else if after_total <= before_total * 0.5 {
        format!("Related events dropped from {before_total:.1} to {after_total:.1} per day.")
    } else if after_total >= before_total {
        format!(
            "Related events did not drop ({before_total:.1} → {after_total:.1} per day). Consider reverting this fix before trying another."
        )
    } else {
        format!("Related events dropped a little ({before_total:.1} → {after_total:.1} per day).")
    };
    Some(MonitorReport {
        days_since,
        too_early,
        trends,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fix::testutil::DEV;
    use crate::snapshot::EventRecord;

    fn ev(time: &str) -> EventRecord {
        EventRecord {
            channel: "System".into(),
            provider: "x".into(),
            event_id: 1,
            level: 3,
            time: time.into(),
            record_id: 0,
            data: vec![],
            message: None,
        }
    }

    fn data(times: &[&str], collected: &str) -> (SystemSnapshot, Analysis) {
        let snap = SystemSnapshot {
            collected_at: collected.into(),
            event_window_days: 10,
            events: times.iter().map(|t| ev(t)).collect(),
            ..Default::default()
        };
        let a = Analysis {
            event_device: times.iter().map(|_| Some(DEV.to_string())).collect(),
            event_signal: times.iter().map(|_| Some(SignalKind::WlanDisconnect)).collect(),
            ..Default::default()
        };
        (snap, a)
    }

    #[test]
    fn baseline_then_drop() {
        let before: Vec<String> = (1..=9)
            .flat_map(|d| [format!("2026-01-0{d}T10:00:00Z"), format!("2026-01-0{d}T12:00:00Z")])
            .collect();
        let refs: Vec<&str> = before.iter().map(String::as_str).collect();
        let (snap, a) = data(&refs, "2026-01-10T00:00:00Z");
        let b = baseline(DEV, &snap, &a);
        assert_eq!(b.counts[0].count, 18);

        let mut r: FixRecord = serde_json::from_value(serde_json::json!({
            "id": 1, "createdAt": "", "findingKey": "", "ruleId": "", "ruleVersion": 1,
            "fixId": "", "title": "", "deviceId": DEV, "deviceName": "", "kind": "adapterProperty",
            "setting": "", "target": "", "from": null, "to": {"value": "3", "label": null},
            "priorRaw": null, "newRaw": "3", "state": "applied", "error": null, "notes": [],
            "restorePoint": null, "rebootRequired": false, "revertible": true,
            "appliedAt": "2026-01-10T00:00:00Z", "revertedAt": null, "baseline": null
        }))
        .unwrap();
        r.baseline = Some(b);

        let (snap, a) = data(&["2026-01-11T09:00:00Z"], "2026-01-11T00:00:00Z");
        let rep = compare(&r, &snap, &a).unwrap();
        assert!(rep.too_early);

        let mut times = refs.clone();
        times.push("2026-01-11T09:00:00Z");
        let (snap, a) = data(&times, "2026-01-14T00:00:00Z");
        let rep = compare(&r, &snap, &a).unwrap();
        assert!(!rep.too_early);
        assert_eq!(rep.trends[0].after_count, 1);
        assert!(rep.summary.contains("dropped"), "{}", rep.summary);
    }
}
