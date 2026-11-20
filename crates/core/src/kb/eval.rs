//! Pure rule evaluation: `(snapshot, analysis, rules) -> findings`.
//! No I/O and no clock: "now" is the snapshot's `collected_at`.

use super::loader::LoadedRule;
use super::rule::{
    Condition, DeviceRef, DriverCond, EventCond, FixAction, FixDef, PropertyCond, Rule, Scope,
    Selector,
};
use super::{Finding, FixStatus, FixSuggestion, ValueText};
use crate::analyze::{Analysis, SignalKind, analyze, patterns};
use crate::snapshot::{DeviceRecord, SetupApiStatus, SystemSnapshot, problem_text};
use chrono::{DateTime, NaiveDate, Utc};
use std::cmp::Reverse;
use std::collections::BTreeMap;

/// Analyse the snapshot, then evaluate the rules.
pub fn eval_snapshot(snap: &SystemSnapshot, rules: &[LoadedRule]) -> Vec<Finding> {
    eval(snap, &analyze(snap), rules)
}

pub fn eval(snap: &SystemSnapshot, analysis: &Analysis, rules: &[LoadedRule]) -> Vec<Finding> {
    let cx = Ctx::new(snap, analysis);
    let mut out = Vec::new();
    for lr in rules {
        let r = &lr.rule;
        match r.scope {
            Scope::System => {
                let mut ev = Vec::new();
                if cx.cond_opt(r.when.as_ref(), None, &mut ev) {
                    out.push(cx.finding(lr, None, ev));
                }
            }
            Scope::Device => {
                let sel = r.selector.clone().unwrap_or_default();
                for (i, d) in snap.devices.iter().enumerate() {
                    if !selects(&sel, d) {
                        continue;
                    }
                    let mut ev = Vec::new();
                    if cx.cond_opt(r.when.as_ref(), Some(i), &mut ev) {
                        out.push(cx.finding(lr, Some(i), ev));
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| {
        (Reverse(a.severity), Reverse(a.confidence), &a.title, &a.key).cmp(&(
            Reverse(b.severity),
            Reverse(b.confidence),
            &b.title,
            &b.key,
        ))
    });
    out
}

fn eq_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Device selector. Every given field must match.
pub fn selects(s: &Selector, d: &DeviceRecord) -> bool {
    if !s.include_not_present && !d.present {
        return false;
    }
    fn opt(v: &Option<String>) -> &str {
        v.as_deref().unwrap_or("")
    }
    if let Some(cls) = &s.class
        && !cls.iter().any(|c| eq_ci(c, opt(&d.class)))
    {
        return false;
    }
    if let Some(en) = &s.enumerator
        && !en.iter().any(|c| eq_ci(c, opt(&d.enumerator)))
    {
        return false;
    }
    if let Some(p) = &s.hardware_id
        && !d.hardware_ids.iter().any(|h| p.any(h))
    {
        return false;
    }
    if let Some(p) = &s.exclude_hardware_id
        && d.hardware_ids.iter().any(|h| p.any(h))
    {
        return false;
    }
    if let Some(p) = &s.compatible_id
        && !d.compatible_ids.iter().any(|h| p.any(h))
    {
        return false;
    }
    if let Some(p) = &s.service
        && !d.service.as_deref().is_some_and(|x| p.any(x))
    {
        return false;
    }
    if let Some(p) = &s.driver_provider
        && !d
            .driver
            .as_ref()
            .and_then(|x| x.provider.as_deref())
            .is_some_and(|x| p.any(x))
    {
        return false;
    }
    true
}

struct Ctx<'a> {
    snap: &'a SystemSnapshot,
    an: &'a Analysis,
    now: DateTime<Utc>,
}

/// Result of counting events/sections in a window.
#[derive(Default)]
struct Count {
    n: u32,
    last: Option<String>,
    breakdown: BTreeMap<String, u32>,
}

impl Count {
    fn add(&mut self, time: &str, detail: Option<String>) {
        self.n += 1;
        if self.last.as_deref().is_none_or(|l| time > l) {
            self.last = Some(time.to_string());
        }
        if let Some(d) = detail {
            *self.breakdown.entry(d).or_default() += 1;
        }
    }

    fn describe(&self, what: &str, days: Option<u32>, detail_name: &str) -> String {
        let mut s = format!("{} {what}", self.n);
        if let Some(d) = days {
            s.push_str(&format!(
                " in the last {d} day{}",
                if d == 1 { "" } else { "s" }
            ));
        } else {
            s.push_str(" in the collected logs");
        }
        if let Some(l) = &self.last {
            s.push_str(&format!(", most recently {}", short_time(l)));
        }
        if !self.breakdown.is_empty() && !detail_name.is_empty() {
            let mut v: Vec<_> = self.breakdown.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let parts: Vec<String> = v.iter().take(4).map(|(k, n)| format!("{k} ×{n}")).collect();
            s.push_str(&format!(" ({detail_name}: {})", parts.join(", ")));
        }
        s
    }
}

fn short_time(t: &str) -> String {
    DateTime::parse_from_rfc3339(t)
        .map(|d| {
            d.with_timezone(&Utc)
                .format("%Y-%m-%d %H:%M UTC")
                .to_string()
        })
        .unwrap_or_else(|_| t.to_string())
}

fn signal_label(k: SignalKind) -> (&'static str, &'static str) {
    use SignalKind::*;
    match k {
        Bugcheck => ("blue-screen crashes", "stop codes"),
        UnexpectedShutdown => ("unexpected shutdowns", ""),
        Whea => ("hardware error reports (WHEA)", ""),
        LiveKernelEvent => ("live kernel events", ""),
        DisplayTdr => ("display driver resets (TDR)", ""),
        StorageReset => ("storage resets or timeouts", "source"),
        DeviceStartFailure => ("device start failures", "problem"),
        DriverInstallFailure => ("failed driver installs", "status"),
        WlanDisconnect => ("Wi-Fi disconnects", "reason codes"),
        WifiDriverWarning => ("Wi-Fi driver warnings", "source"),
        UpdateInstallFailure => ("failed Windows Update installs", "error"),
    }
}

/// Compare dotted numeric versions ("31.0.101.5590"). Missing parts are 0.
fn cmp_version(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let parse = |s: &str| -> Option<Vec<u64>> {
        s.trim()
            .split('.')
            .map(|p| p.trim().parse::<u64>().ok())
            .collect()
    };
    let (mut x, mut y) = (parse(a)?, parse(b)?);
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    Some(x.cmp(&y))
}

impl<'a> Ctx<'a> {
    fn new(snap: &'a SystemSnapshot, an: &'a Analysis) -> Self {
        let now = DateTime::parse_from_rfc3339(&snap.collected_at)
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or(DateTime::<Utc>::MIN_UTC);
        Self { snap, an, now }
    }

    fn dev(&self, i: Option<usize>) -> Option<&'a DeviceRecord> {
        i.and_then(|i| self.snap.devices.get(i))
    }

    fn in_window(&self, time: &str, days: Option<u32>) -> bool {
        let Some(d) = days else { return true };
        DateTime::parse_from_rfc3339(time).is_ok_and(|t| {
            let t = t.with_timezone(&Utc);
            t <= self.now && (self.now - t).num_seconds() <= i64::from(d) * 86_400
        })
    }

    /// True when the event at index `j` concerns the device (or any device).
    fn event_is_for(&self, j: usize, who: DeviceRef, dev: Option<usize>) -> bool {
        match who {
            DeviceRef::Any => true,
            DeviceRef::This => {
                let (Some(d), Some(Some(id))) = (self.dev(dev), self.an.event_device.get(j)) else {
                    return false;
                };
                eq_ci(id, &d.instance_id)
            }
        }
    }

    fn section_is_for(&self, id: Option<&str>, who: DeviceRef, dev: Option<usize>) -> bool {
        match who {
            DeviceRef::Any => true,
            DeviceRef::This => self
                .dev(dev)
                .zip(id)
                .is_some_and(|(d, id)| eq_ci(id, &d.instance_id)),
        }
    }

    fn signal_count(
        &self,
        k: SignalKind,
        days: Option<u32>,
        who: DeviceRef,
        dev: Option<usize>,
    ) -> Count {
        let mut c = Count::default();
        for (j, e) in self.snap.events.iter().enumerate() {
            if self.an.event_signal.get(j).copied().flatten() != Some(k)
                || patterns::is_duplicate_shutdown(self.snap, e, k)
                || !self.in_window(&e.time, days)
                || !self.event_is_for(j, who, dev)
            {
                continue;
            }
            c.add(&e.time, patterns::detail(k, e));
        }
        if k == SignalKind::DriverInstallFailure {
            self.add_setupapi_failures(&mut c, days, who, dev);
        }
        c
    }

    fn add_setupapi_failures(
        &self,
        c: &mut Count,
        days: Option<u32>,
        who: DeviceRef,
        dev: Option<usize>,
    ) {
        for s in &self.snap.setupapi {
            if s.status != SetupApiStatus::Failure {
                continue;
            }
            let t = s.start.as_deref().unwrap_or("");
            if days.is_some() && !self.in_window(t, days) {
                continue;
            }
            if !self.section_is_for(s.instance_id.as_deref(), who, dev) {
                continue;
            }
            c.add(t, s.status_code.clone());
        }
    }

    fn event_count(&self, ec: &EventCond, dev: Option<usize>) -> Count {
        let mut c = Count::default();
        for (j, e) in self.snap.events.iter().enumerate() {
            let ok = ec.provider.any(&e.provider)
                && ec.ids.as_ref().is_none_or(|ids| ids.contains(&e.event_id))
                && ec.channel.as_deref().is_none_or(|ch| eq_ci(ch, &e.channel))
                && ec.max_level.is_none_or(|m| e.level != 0 && e.level <= m)
                && ec
                    .data
                    .iter()
                    .all(|(k, v)| e.field(k).is_some_and(|x| eq_ci(x, v)))
                && self.in_window(&e.time, ec.days)
                && self.event_is_for(j, ec.device, dev);
            if ok {
                c.add(&e.time, Some(format!("{} {}", e.provider, e.event_id)));
            }
        }
        c
    }

    fn cond_opt(&self, c: Option<&Condition>, dev: Option<usize>, ev: &mut Vec<String>) -> bool {
        match c {
            None => true,
            Some(c) => self.cond(c, dev, ev),
        }
    }

    /// Evaluate `c`; on success, append human-readable evidence to `ev`.
    fn cond(&self, c: &Condition, dev: Option<usize>, ev: &mut Vec<String>) -> bool {
        match c {
            Condition::All(v) => {
                let mut tmp = Vec::new();
                for c in v {
                    if !self.cond(c, dev, &mut tmp) {
                        return false;
                    }
                }
                ev.extend(tmp);
                true
            }
            Condition::Any(v) => {
                // No short-circuit: every true branch contributes evidence.
                let mut any = false;
                for c in v {
                    any |= self.cond(c, dev, ev);
                }
                any
            }
            Condition::Not(c) => !self.cond(c, dev, &mut Vec::new()),
            Condition::Signal(s) => {
                let n = self.signal_count(s.kind, s.days, s.device, dev);
                let ok = n.n >= s.min;
                if ok {
                    let (what, detail) = signal_label(s.kind);
                    ev.push(n.describe(what, s.days, detail));
                }
                ok
            }
            Condition::Event(e) => {
                let n = self.event_count(e, dev);
                let ok = n.n >= e.min;
                if ok {
                    ev.push(n.describe("matching events", e.days, "events"));
                }
                ok
            }
            Condition::SetupapiFailure(w) => {
                let mut n = Count::default();
                self.add_setupapi_failures(&mut n, w.days, w.device, dev);
                let ok = n.n >= w.min;
                if ok {
                    ev.push(n.describe(
                        "failed driver installs in setupapi.dev.log",
                        w.days,
                        "status",
                    ));
                }
                ok
            }
            Condition::Property(p) => self.property(p, dev, ev),
            Condition::ProblemCode(codes) => {
                let Some(code) = self.dev(dev).and_then(|d| d.status.problem_code) else {
                    return false;
                };
                let ok = codes.is_empty() || codes.contains(&code);
                if ok {
                    ev.push(format!("Device Manager reports: {}", problem_text(code)));
                }
                ok
            }
            Condition::Power(p) => {
                let Some(v) = self
                    .dev(dev)
                    .and_then(|d| d.power.as_ref())
                    .and_then(|p| p.allow_turn_off)
                else {
                    return false;
                };
                let ok = v == p.allow_turn_off;
                if ok {
                    ev.push(format!(
                        "\"Allow the computer to turn off this device to save power\" is {}",
                        if v { "checked" } else { "unchecked" }
                    ));
                }
                ok
            }
            Condition::Driver(dc) => self.driver(dc, dev, ev),
            Condition::MatchLevel(levels) => {
                let Some(m) = dev.and_then(|i| self.an.matches.get(i)) else {
                    return false;
                };
                let ok = levels.contains(&m.level);
                if ok {
                    let note = m.notes.first().cloned().unwrap_or_default();
                    ev.push(
                        format!("Driver match check: {:?}. {note}", m.level)
                            .trim_end()
                            .to_string(),
                    );
                }
                ok
            }
            Condition::DevicePresent(sel) => {
                let found = self.snap.devices.iter().find(|d| selects(sel, d));
                if let Some(d) = found {
                    ev.push(format!("{} is installed", d.name));
                }
                found.is_some()
            }
            Condition::DumpFiles(dc) => {
                let n = self.snap.dumps.len() as u32;
                let ok = n >= dc.min;
                if ok {
                    ev.push(format!(
                        "{n} crash dump file{} on disk",
                        if n == 1 { "" } else { "s" }
                    ));
                }
                ok
            }
        }
    }

    fn property(&self, p: &PropertyCond, dev: Option<usize>, ev: &mut Vec<String>) -> bool {
        let Some(d) = self.dev(dev) else { return false };
        let prop = d.advanced(&p.keyword);
        if let Some(want) = p.exists
            && prop.is_some() != want
        {
            return false;
        }
        let value_checks = p.equals.is_some()
            || p.not_equals.is_some()
            || p.one_of.is_some()
            || p.not_in.is_some();
        if !value_checks {
            if prop.is_some() {
                ev.push(format!("The driver exposes {}", p.keyword));
            } else {
                ev.push(format!("The driver does not expose {}", p.keyword));
            }
            return true;
        }
        let Some(prop) = prop else { return false };
        let Some(v) = prop.effective() else {
            return false;
        };
        let ok = p.equals.as_deref().is_none_or(|x| x == v)
            && p.not_equals.as_deref().is_none_or(|x| x != v)
            && p.one_of.as_ref().is_none_or(|xs| xs.iter().any(|x| x == v))
            && p.not_in
                .as_ref()
                .is_none_or(|xs| !xs.iter().any(|x| x == v));
        if ok {
            let name = prop.label.as_deref().unwrap_or(&prop.keyword);
            let shown = prop
                .option_label(v)
                .map_or(v.to_string(), |l| format!("{l} ({v})"));
            let dflt = if prop.current.is_none() {
                " (driver default)"
            } else {
                ""
            };
            ev.push(format!("{name} is {shown}{dflt}"));
        }
        ok
    }

    fn driver(&self, dc: &DriverCond, dev: Option<usize>, ev: &mut Vec<String>) -> bool {
        let Some(d) = self.dev(dev) else { return false };
        let Some(drv) = d.driver.as_ref() else {
            return false;
        };
        let mut notes = Vec::new();
        if let Some(p) = &dc.provider {
            match drv.provider.as_deref() {
                Some(x) if p.any(x) => notes.push(format!("driver provider is {x}")),
                _ => return false,
            }
        }
        if let Some(want) = dc.inbox {
            let Some(m) = dev.and_then(|i| self.an.matches.get(i)) else {
                return false;
            };
            if m.inbox != want {
                return false;
            }
            notes.push(if want {
                "driver ships with Windows".into()
            } else {
                "driver is not an inbox driver".into()
            });
        }
        let ver = drv.version.as_deref();
        if let Some(b) = &dc.version_below {
            match ver.and_then(|v| cmp_version(v, b)) {
                Some(std::cmp::Ordering::Less) => {
                    notes.push(format!("driver version {} is below {b}", ver.unwrap_or("")))
                }
                _ => return false,
            }
        }
        if let Some(b) = &dc.version_at_least {
            match ver.and_then(|v| cmp_version(v, b)) {
                Some(o) if o != std::cmp::Ordering::Less => notes.push(format!(
                    "driver version {} is {b} or later",
                    ver.unwrap_or("")
                )),
                _ => return false,
            }
        }
        if let Some(sigs) = &dc.signature {
            if !sigs.contains(&drv.signature.status) {
                return false;
            }
            notes.push(format!("driver signature: {:?}", drv.signature.status));
        }
        if let Some(days) = dc.older_than_days {
            let Some(date) = drv
                .date
                .as_deref()
                .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            else {
                return false;
            };
            let age = (self.now.date_naive() - date).num_days();
            if age < i64::from(days) {
                return false;
            }
            notes.push(format!("driver is dated {date} ({age} days old)"));
        }
        if !notes.is_empty() {
            let mut s = notes.join("; ");
            if let Some(f) = s.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            ev.push(s);
        }
        true
    }

    fn finding(&self, lr: &LoadedRule, dev: Option<usize>, evidence: Vec<String>) -> Finding {
        let r: &Rule = &lr.rule;
        let d = self.dev(dev);
        let name = d.map(|d| d.name.as_str()).unwrap_or("This PC");
        Finding {
            key: match d {
                Some(d) => format!("{}@{}", r.id, d.instance_id),
                None => r.id.clone(),
            },
            rule_id: r.id.clone(),
            rule_version: r.version,
            title: r.title.clone(),
            severity: r.severity,
            confidence: r.confidence,
            confidence_note: r.confidence_note.clone(),
            category: r.category.clone(),
            device_id: d.map(|d| d.instance_id.clone()),
            device_name: d.map(|d| d.name.clone()),
            summary: r.summary.replace("{device}", name),
            explanation: r.explanation.clone(),
            evidence,
            sources: r.sources.clone(),
            fixes: r.fixes.iter().map(|f| suggestion(f, d)).collect(),
            source_file: lr.file.clone(),
        }
    }
}

fn suggestion(f: &FixDef, d: Option<&DeviceRecord>) -> FixSuggestion {
    FixSuggestion {
        id: f.id.clone(),
        title: f.title.clone(),
        why: f.why.clone(),
        tradeoff: f.tradeoff.clone(),
        risk: f.risk,
        optional: f.optional,
        requires_admin: f.action.requires_admin(),
        action: f.action.clone(),
        status: fix_status(&f.action, d),
    }
}

/// Checks a fix against what this device actually offers. A fix whose
/// keyword or value the driver does not have is `Unavailable`, never guessed.
pub fn fix_status(a: &FixAction, d: Option<&DeviceRecord>) -> FixStatus {
    let un = |r: String| FixStatus::Unavailable { reason: r };
    match a {
        FixAction::Guided { .. } => FixStatus::Manual,
        FixAction::AdapterProperty { keyword, value } => {
            let Some(d) = d else {
                return un("No device to change.".into());
            };
            let Some(p) = d.advanced(keyword) else {
                return un(format!(
                    "This adapter's driver does not have the setting {keyword}."
                ));
            };
            let name = p.label.as_deref().unwrap_or(&p.keyword);
            if !p.accepts(value) {
                let offered = if p.options.is_empty() {
                    match (&p.min, &p.max) {
                        (Some(lo), Some(hi)) => format!(" It accepts {lo}–{hi}."),
                        _ => String::new(),
                    }
                } else {
                    let v: Vec<String> = p
                        .options
                        .iter()
                        .map(|o| format!("{} ({})", o.label, o.value))
                        .collect();
                    format!(" It offers: {}.", v.join(", "))
                };
                return un(format!(
                    "{name} on this driver has no value {value}.{offered}"
                ));
            }
            let vt = |v: &str| ValueText {
                value: v.to_string(),
                label: p.option_label(v).map(str::to_string),
            };
            match p.effective() {
                Some(cur) if cur == value => FixStatus::AlreadySet { value: vt(cur) },
                cur => FixStatus::Applicable {
                    from: cur.map(vt),
                    to: vt(value),
                },
            }
        }
        FixAction::DevicePower { allow_turn_off } => {
            let Some(cur) = d
                .and_then(|d| d.power.as_ref())
                .and_then(|p| p.allow_turn_off)
            else {
                return un("Windows does not report this power setting for this device.".into());
            };
            let vt = |b: bool| ValueText {
                value: b.to_string(),
                label: Some(if b { "Allowed" } else { "Not allowed" }.into()),
            };
            if cur == *allow_turn_off {
                FixStatus::AlreadySet { value: vt(cur) }
            } else {
                FixStatus::Applicable {
                    from: Some(vt(cur)),
                    to: vt(*allow_turn_off),
                }
            }
        }
        FixAction::RollbackDriver {} => {
            let Some(drv) = d.and_then(|d| d.driver.as_ref()) else {
                return un("This device has no driver to roll back.".into());
            };
            // Whether a previous driver exists is checked live by the fix engine.
            FixStatus::Applicable {
                from: drv.version.as_ref().map(|v| ValueText {
                    value: v.clone(),
                    label: drv.provider.clone(),
                }),
                to: ValueText {
                    value: "previous".into(),
                    label: Some("Previously installed driver".into()),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{AdvancedProperty, PropertyOption};

    #[test]
    fn versions_compare_numerically() {
        use std::cmp::Ordering::*;
        assert_eq!(cmp_version("23.10.0.8", "23.9.1"), Some(Greater));
        assert_eq!(cmp_version("1.2", "1.2.0.0"), Some(Equal));
        assert_eq!(cmp_version("x", "1"), None);
    }

    fn prop() -> AdvancedProperty {
        AdvancedProperty {
            keyword: "MIMOPowerSaveMode".into(),
            label: Some("MIMO Power Save Mode".into()),
            kind: "enum".into(),
            default: Some("0".into()),
            current: None,
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
        }
    }

    fn device(p: Vec<AdvancedProperty>) -> DeviceRecord {
        let mut d = crate::analyze::test_util::dev("PCI\\X", None, None);
        d.advanced = p;
        d
    }

    #[test]
    fn fix_status_reads_live_values() {
        let set = |v: &str| FixAction::AdapterProperty {
            keyword: "mimopowersavemode".into(),
            value: v.into(),
        };
        let d = device(vec![prop()]);
        assert!(matches!(
            fix_status(&set("3"), Some(&d)),
            FixStatus::Applicable { from: Some(ValueText { ref value, .. }), .. } if value == "0"
        ));
        assert!(matches!(
            fix_status(&set("0"), Some(&d)),
            FixStatus::AlreadySet { .. }
        ));
        let FixStatus::Unavailable { reason } = fix_status(&set("9"), Some(&d)) else {
            panic!()
        };
        assert!(reason.contains("No SMPS (3)"), "{reason}");
        let none = device(vec![]);
        assert!(matches!(
            fix_status(&set("3"), Some(&none)),
            FixStatus::Unavailable { .. }
        ));
    }
}
