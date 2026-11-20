//! Preview → apply → verify → revert. Every step re-reads the device through
//! the backend; nothing is decided from the UI or from an old scan.

use super::validate::{check_pnp_write, check_property_write, pnp_target, require_net_adapter};
use super::*;
use crate::kb::Finding;
use crate::store::Store;
use std::sync::Mutex;

pub const POWER_SETTING: &str = "Allow the computer to turn off this device to save power";

/// Rebuilds the fix from the current findings (the UI only names it).
pub fn target_from_findings(
    findings: &[Finding],
    finding_key: &str,
    fix_id: &str,
) -> Option<FixTarget> {
    let f = findings.iter().find(|f| f.key == finding_key)?;
    let s = f.fixes.iter().find(|s| s.id == fix_id)?;
    Some(FixTarget {
        finding_key: f.key.clone(),
        rule_id: f.rule_id.clone(),
        rule_version: f.rule_version,
        fix_id: s.id.clone(),
        title: s.title.clone(),
        risk: s.risk,
        device_id: f.device_id.clone()?,
        device_name: f.device_name.clone().unwrap_or_default(),
        action: s.action.clone(),
    })
}

fn vt(p: &crate::snapshot::AdvancedProperty, v: &str) -> ValueText {
    ValueText {
        value: v.to_string(),
        label: p.option_label(v).map(str::to_string),
    }
}

fn power_vt(allow: bool) -> ValueText {
    ValueText {
        value: allow.to_string(),
        label: Some(if allow { "Allowed" } else { "Not allowed" }.into()),
    }
}

/// Pure: what applying `t` to the live device `d` would do.
pub fn plan(t: &FixTarget, d: Option<&DeviceRecord>) -> Preview {
    let refuse = |r: String| Preview::Refused { reason: r };
    let Some(d) = d else {
        return refuse(format!("{} is not on this system any more.", t.device_name));
    };
    if d.instance_id != t.device_id {
        return refuse("Device mismatch.".into());
    }
    match &t.action {
        FixAction::Guided { .. } => refuse("This fix is a set of manual steps.".into()),
        FixAction::AdapterProperty { keyword, value } => {
            let kw = match check_property_write(d, keyword, Some(value)) {
                Ok(k) => k.to_string(),
                Err(e) => return refuse(e),
            };
            let Some(p) = d.advanced(&kw) else {
                return refuse(format!("The driver does not have {kw}."));
            };
            if p.effective() == Some(value.as_str()) {
                return Preview::AlreadySet {
                    value: vt(p, value),
                };
            }
            let mut notes = vec![
                "The adapter restarts to apply this; the connection drops for a few seconds."
                    .to_string(),
            ];
            if p.current.is_none() {
                notes.push(
                    "No value is stored now (the driver uses its default); Revert removes the value again."
                        .into(),
                );
            }
            Preview::Ready {
                plan: FixPlan {
                    kind: ActionKind::AdapterProperty,
                    setting: p.label.clone().unwrap_or_else(|| kw.clone()),
                    from: p.effective().map(|v| vt(p, v)),
                    to: vt(p, value),
                    prior_raw: p.current.clone(),
                    new_raw: Some(value.clone()),
                    target: kw,
                    restarts_device: true,
                    revertible: true,
                    restore_point_default: false,
                    notes,
                },
            }
        }
        FixAction::DevicePower { allow_turn_off } => {
            if let Err(e) = require_net_adapter(d) {
                return refuse(e);
            }
            let Some(pw) = d.power.as_ref() else {
                return refuse("Windows does not report power settings for this device.".into());
            };
            let Some(cur) = pw.allow_turn_off else {
                return refuse("Windows does not report this power setting for this device.".into());
            };
            if cur == *allow_turn_off {
                return Preview::AlreadySet {
                    value: power_vt(cur),
                };
            }
            let raw = pw.pnp_capabilities_raw;
            let new = pnp_target(raw, *allow_turn_off);
            if let Err(e) = check_pnp_write(raw, new) {
                return refuse(e);
            }
            Preview::Ready {
                plan: FixPlan {
                    kind: ActionKind::DevicePower,
                    setting: POWER_SETTING.into(),
                    target: "PnPCapabilities".into(),
                    from: Some(power_vt(cur)),
                    to: power_vt(*allow_turn_off),
                    prior_raw: raw.map(|v| v.to_string()),
                    new_raw: new.map(|v| v.to_string()),
                    restarts_device: true,
                    revertible: true,
                    restore_point_default: false,
                    notes: vec![
                        "The adapter restarts to apply this; the connection drops for a few seconds."
                            .into(),
                    ],
                },
            }
        }
        FixAction::RollbackDriver {} => {
            let Some(drv) = d.driver.as_ref().filter(|_| d.present) else {
                return refuse(format!("{} has no installed driver to roll back.", d.name));
            };
            Preview::Ready {
                plan: FixPlan {
                    kind: ActionKind::RollbackDriver,
                    setting: "Driver".into(),
                    target: "driver".into(),
                    from: drv.version.as_ref().map(|v| ValueText {
                        value: v.clone(),
                        label: drv.provider.clone(),
                    }),
                    to: ValueText {
                        value: "previous".into(),
                        label: Some("The driver Windows kept as the previous one".into()),
                    },
                    prior_raw: drv.version.clone(),
                    new_raw: None,
                    restarts_device: false,
                    revertible: false,
                    restore_point_default: true,
                    notes: vec![
                        "Windows keeps at most one previous driver. If none is kept, the rollback fails and nothing changes."
                            .into(),
                        "Revert is not available for a rollback; going forward again means reinstalling the newer driver from the vendor."
                            .into(),
                    ],
                },
            }
        }
    }
}

pub fn preview(be: &mut dyn SystemBackend, t: &FixTarget) -> Preview {
    match be.read_device(&t.device_id) {
        Ok(d) => plan(t, d.as_ref()),
        Err(e) => Preview::Refused {
            reason: format!("Could not read the device: {e}"),
        },
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn lock(store: &Mutex<Store>) -> Result<std::sync::MutexGuard<'_, Store>, String> {
    store.lock().map_err(|_| "store lock poisoned".to_string())
}

/// Restore point text: "Auriga: <fix title>", within the helper's limit.
pub fn restore_point_description(title: &str) -> String {
    let s: String = format!("Auriga: {title}")
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect();
    s
}

/// What is stored now for the plan's target (raw text), from a live read.
fn stored_now(kind: ActionKind, target: &str, d: &DeviceRecord) -> Option<String> {
    match kind {
        ActionKind::AdapterProperty => d.advanced(target).and_then(|p| p.current.clone()),
        ActionKind::DevicePower => d
            .power
            .as_ref()
            .and_then(|p| p.pnp_capabilities_raw)
            .map(|v| v.to_string()),
        ActionKind::RollbackDriver => d.driver.as_ref().and_then(|x| x.version.clone()),
    }
}

fn parse_pnp(raw: Option<&str>) -> Result<Option<u32>, String> {
    raw.map(|s| s.parse::<u32>().map_err(|_| format!("bad PnPCapabilities value {s}")))
        .transpose()
}

/// Write `raw` (or delete when `None`) for the plan's target. Returns the
/// previous stored value the system reported.
fn write(
    be: &mut dyn SystemBackend,
    kind: ActionKind,
    device: &str,
    target: &str,
    raw: Option<&str>,
) -> Result<Option<String>, String> {
    match kind {
        ActionKind::AdapterProperty => be.set_adapter_property(device, target, raw),
        ActionKind::DevicePower => Ok(be
            .set_pnp_capabilities(device, parse_pnp(raw)?)?
            .map(|v| v.to_string())),
        ActionKind::RollbackDriver => Err("A rollback cannot be written as a value.".into()),
    }
}

/// Restart after a write. A failed restart is not fatal: the value is
/// written and applies after a reboot.
fn restart(be: &mut dyn SystemBackend, r: &mut FixRecord) {
    match be.restart_device(&r.device_id) {
        Ok(reboot) => r.reboot_required |= reboot,
        Err(e) => {
            r.reboot_required = true;
            r.notes.push(format!(
                "The adapter could not be restarted ({e}); the change applies after a reboot."
            ));
        }
    }
}

pub fn apply(
    be: &mut dyn SystemBackend,
    store: &Mutex<Store>,
    t: &FixTarget,
    expected_from: Option<&ValueText>,
    opts: &ApplyOptions,
    baseline: Option<Baseline>,
) -> Result<ApplyOutcome, String> {
    let p = match preview(be, t) {
        Preview::Ready { plan } => plan,
        Preview::AlreadySet { value } => {
            return Ok(ApplyOutcome::ChangedSincePreview {
                message: format!(
                    "The device already has {}; nothing to do.",
                    value.label.as_deref().unwrap_or(&value.value)
                ),
            });
        }
        Preview::Refused { reason } => return Ok(ApplyOutcome::Refused { reason }),
    };
    if p.from.as_ref() != expected_from {
        return Ok(ApplyOutcome::ChangedSincePreview {
            message: "The current value changed since the preview. Review the new preview before applying."
                .into(),
        });
    }

    let mut notes = Vec::new();
    let mut rp = None;
    if opts.restore_point {
        if opts.continue_without_restore_point {
            notes.push("Applied without a restore point (Windows did not create one).".into());
        } else {
            let o = be.create_restore_point(&restore_point_description(&t.title));
            if !matches!(o, RestorePointOutcome::Created { .. }) {
                return Ok(ApplyOutcome::RestorePointNotCreated { outcome: o });
            }
            rp = Some(o);
        }
    }

    let mut r = FixRecord {
        id: 0,
        created_at: now(),
        finding_key: t.finding_key.clone(),
        rule_id: t.rule_id.clone(),
        rule_version: t.rule_version,
        fix_id: t.fix_id.clone(),
        title: t.title.clone(),
        device_id: t.device_id.clone(),
        device_name: t.device_name.clone(),
        kind: p.kind,
        setting: p.setting.clone(),
        target: p.target.clone(),
        from: p.from.clone(),
        to: p.to.clone(),
        prior_raw: p.prior_raw.clone(),
        new_raw: p.new_raw.clone(),
        state: FixState::Pending,
        error: None,
        notes,
        restore_point: rp,
        reboot_required: false,
        revertible: p.revertible,
        applied_at: None,
        reverted_at: None,
        baseline,
    };
    r.id = lock(store)?.insert_fix(&r).map_err(|e| e.to_string())?;

    let wrote = if p.kind == ActionKind::RollbackDriver {
        be.rollback_driver(&t.device_id).map(|reboot| {
            r.reboot_required = reboot;
        })
    } else {
        write(be, p.kind, &t.device_id, &p.target, p.new_raw.as_deref()).map(|prev| {
            // What the system had just before the write is the true prior.
            r.prior_raw = prev;
        })
    };
    if let Err(e) = wrote {
        r.state = FixState::Failed;
        r.error = Some(e);
        lock(store)?.update_fix(&r).map_err(|e| e.to_string())?;
        return Ok(ApplyOutcome::Done { record: Box::new(r) });
    }
    r.applied_at = Some(now());
    if p.restarts_device {
        restart(be, &mut r);
    }

    r.state = match be.read_device(&t.device_id) {
        Ok(Some(d)) => {
            let now_v = stored_now(p.kind, &p.target, &d);
            match p.kind {
                ActionKind::RollbackDriver if now_v != p.prior_raw => FixState::Applied,
                ActionKind::RollbackDriver if r.reboot_required => {
                    r.notes
                        .push("Windows finishes the rollback after a restart.".into());
                    FixState::Applied
                }
                _ if p.kind != ActionKind::RollbackDriver && now_v == p.new_raw => {
                    FixState::Applied
                }
                _ => {
                    r.error = Some(format!(
                        "After the change the device reports {}, not {}.",
                        now_v.as_deref().unwrap_or("no stored value"),
                        p.new_raw.as_deref().unwrap_or("the previous driver")
                    ));
                    FixState::VerifyFailed
                }
            }
        }
        Ok(None) => {
            r.error = Some("The device disappeared after the change.".into());
            FixState::VerifyFailed
        }
        Err(e) => {
            r.error = Some(format!("Could not re-read the device: {e}"));
            FixState::VerifyFailed
        }
    };
    lock(store)?.update_fix(&r).map_err(|e| e.to_string())?;
    Ok(ApplyOutcome::Done { record: Box::new(r) })
}

pub fn revert(
    be: &mut dyn SystemBackend,
    store: &Mutex<Store>,
    id: i64,
) -> Result<RevertOutcome, String> {
    let refuse = |r: &str| Ok(RevertOutcome::Refused { reason: r.into() });
    let Some(mut r) = lock(store)?.get_fix(id).map_err(|e| e.to_string())? else {
        return refuse("No such fix in history.");
    };
    if !r.revertible {
        return refuse("This change cannot be reverted automatically.");
    }
    if !matches!(
        r.state,
        FixState::Applied | FixState::VerifyFailed | FixState::RevertFailed | FixState::Unknown
    ) {
        return refuse("There is nothing to revert for this entry.");
    }
    let d = match be.read_device(&r.device_id) {
        Ok(Some(d)) => d,
        Ok(None) => return refuse("The device is not on this system any more."),
        Err(e) => {
            return Ok(RevertOutcome::Refused {
                reason: format!("Could not read the device: {e}"),
            });
        }
    };
    let cur = stored_now(r.kind, &r.target, &d);
    if cur == r.prior_raw {
        r.state = FixState::Reverted;
        r.reverted_at = Some(now());
        r.notes
            .push("The device already had the previous value; nothing was written.".into());
        lock(store)?.update_fix(&r).map_err(|e| e.to_string())?;
        return Ok(RevertOutcome::Done { record: Box::new(r) });
    }
    if cur != r.new_raw {
        return Ok(RevertOutcome::Refused {
            reason: format!(
                "{} is now {}, not the value this fix set ({}). It was changed since; not overwriting it.",
                r.setting,
                cur.as_deref().unwrap_or("not stored"),
                r.new_raw.as_deref().unwrap_or("not stored"),
            ),
        });
    }
    let (kind, dev, target, prior) = (
        r.kind,
        r.device_id.clone(),
        r.target.clone(),
        r.prior_raw.clone(),
    );
    if let Err(e) = write(be, kind, &dev, &target, prior.as_deref()) {
        r.state = FixState::RevertFailed;
        r.error = Some(e);
        lock(store)?.update_fix(&r).map_err(|e| e.to_string())?;
        return Ok(RevertOutcome::Done { record: Box::new(r) });
    }
    restart(be, &mut r);
    match be.read_device(&dev) {
        Ok(Some(d)) if stored_now(kind, &target, &d) == prior => {
            r.state = FixState::Reverted;
            r.error = None;
            r.reverted_at = Some(now());
        }
        Ok(_) => {
            r.state = FixState::RevertFailed;
            r.error = Some("After reverting, the device does not report the previous value.".into());
        }
        Err(e) => {
            r.state = FixState::RevertFailed;
            r.error = Some(format!("Could not re-read the device: {e}"));
        }
    }
    lock(store)?.update_fix(&r).map_err(|e| e.to_string())?;
    Ok(RevertOutcome::Done { record: Box::new(r) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fix::testutil::{DEV, adapter};
    use crate::kb::Risk;

    /// In-memory machine: one adapter, every call logged.
    struct Mock {
        dev: Option<DeviceRecord>,
        log: Vec<String>,
        rp: RestorePointOutcome,
        fail_write: bool,
        /// Simulate a driver that ignores writes.
        ignore_writes: bool,
        fail_restart: bool,
        rollback_to: Option<String>,
    }

    impl Mock {
        fn new() -> Self {
            Self {
                dev: Some(adapter()),
                log: Vec::new(),
                rp: RestorePointOutcome::Created { sequence: 5 },
                fail_write: false,
                ignore_writes: false,
                fail_restart: false,
                rollback_to: None,
            }
        }
        fn prop(&self, kw: &str) -> Option<String> {
            self.dev.as_ref()?.advanced(kw)?.current.clone()
        }
    }

    impl SystemBackend for Mock {
        fn read_device(&mut self, id: &str) -> Result<Option<DeviceRecord>, String> {
            self.log.push("read".into());
            Ok(self.dev.clone().filter(|d| d.instance_id == id))
        }
        fn set_adapter_property(
            &mut self,
            _: &str,
            kw: &str,
            v: Option<&str>,
        ) -> Result<Option<String>, String> {
            self.log.push(format!("set {kw}={v:?}"));
            if self.fail_write {
                return Err("access denied".into());
            }
            let d = self.dev.as_mut().unwrap();
            let p = d.advanced.iter_mut().find(|p| p.keyword == kw).unwrap();
            let prev = p.current.clone();
            if !self.ignore_writes {
                p.current = v.map(String::from);
            }
            Ok(prev)
        }
        fn set_pnp_capabilities(&mut self, _: &str, v: Option<u32>) -> Result<Option<u32>, String> {
            self.log.push(format!("pnp {v:?}"));
            let pw = self.dev.as_mut().unwrap().power.as_mut().unwrap();
            let prev = pw.pnp_capabilities_raw;
            pw.pnp_capabilities_raw = v;
            pw.allow_turn_off = Some(v.unwrap_or(0) & 0x8 == 0);
            Ok(prev)
        }
        fn restart_device(&mut self, _: &str) -> Result<bool, String> {
            self.log.push("restart".into());
            if self.fail_restart {
                Err("device busy".into())
            } else {
                Ok(false)
            }
        }
        fn create_restore_point(&mut self, d: &str) -> RestorePointOutcome {
            self.log.push(format!("rp {d}"));
            self.rp.clone()
        }
        fn rollback_driver(&mut self, _: &str) -> Result<bool, String> {
            self.log.push("rollback".into());
            match &self.rollback_to {
                Some(v) => {
                    self.dev.as_mut().unwrap().driver.as_mut().unwrap().version = Some(v.clone());
                    Ok(false)
                }
                None => Err("no previous driver".into()),
            }
        }
    }

    fn target(action: FixAction) -> FixTarget {
        FixTarget {
            finding_key: format!("intel-ax201@{DEV}"),
            rule_id: "intel-ax201".into(),
            rule_version: 1,
            fix_id: "f".into(),
            title: "Set MIMO power save to No SMPS".into(),
            risk: Risk::Low,
            device_id: DEV.into(),
            device_name: "AX201".into(),
            action,
        }
    }

    fn mimo() -> FixTarget {
        target(FixAction::AdapterProperty {
            keyword: "MIMOPowerSaveMode".into(),
            value: "3".into(),
        })
    }

    fn store() -> Mutex<Store> {
        Mutex::new(Store::open_in_memory().unwrap())
    }

    fn from_of(be: &mut Mock, t: &FixTarget) -> Option<ValueText> {
        match preview(be, t) {
            Preview::Ready { plan } => plan.from,
            p => panic!("{p:?}"),
        }
    }

    const NO_RP: ApplyOptions = ApplyOptions {
        restore_point: false,
        continue_without_restore_point: false,
    };

    fn done(o: ApplyOutcome) -> FixRecord {
        match o {
            ApplyOutcome::Done { record } => *record,
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn refuses_missing_keyword_or_value() {
        let mut be = Mock::new();
        let t = target(FixAction::AdapterProperty {
            keyword: "RoamAggressiveness".into(),
            value: "1".into(),
        });
        assert!(matches!(preview(&mut be, &t), Preview::Refused { reason } if reason.contains("does not have")));
        let t = target(FixAction::AdapterProperty {
            keyword: "MIMOPowerSaveMode".into(),
            value: "9".into(),
        });
        assert!(matches!(preview(&mut be, &t), Preview::Refused { .. }));
        let s = store();
        let o = apply(&mut be, &s, &t, None, &NO_RP, None).unwrap();
        assert!(matches!(o, ApplyOutcome::Refused { .. }));
        assert!(be.log.iter().all(|l| l == "read"), "{:?}", be.log);
        assert!(s.lock().unwrap().list_fixes(10).unwrap().is_empty());

        be.dev = None;
        assert!(matches!(preview(&mut be, &mimo()), Preview::Refused { .. }));
    }

    #[test]
    fn refuses_non_net_device() {
        let mut be = Mock::new();
        be.dev.as_mut().unwrap().class_guid = Some("{4d36e968-e325-11ce-bfc1-08002be10318}".into());
        assert!(matches!(preview(&mut be, &mimo()), Preview::Refused { .. }));
    }

    #[test]
    fn apply_and_revert_property() {
        let mut be = Mock::new();
        let s = store();
        let t = mimo();
        let from = from_of(&mut be, &t);
        assert_eq!(from.as_ref().unwrap().label.as_deref(), Some("Auto SMPS"));
        let r = done(apply(&mut be, &s, &t, from.as_ref(), &NO_RP, None).unwrap());
        assert_eq!(r.state, FixState::Applied, "{r:?}");
        assert_eq!(r.prior_raw.as_deref(), Some("0"));
        assert_eq!(be.prop("MIMOPowerSaveMode").as_deref(), Some("3"));
        assert!(be.log.contains(&"restart".to_string()));
        assert!(!be.log.iter().any(|l| l.starts_with("rp ")));
        let stored = s.lock().unwrap().get_fix(r.id).unwrap().unwrap();
        assert_eq!(stored.state, FixState::Applied);

        assert!(matches!(preview(&mut be, &t), Preview::AlreadySet { .. }));

        let rv = revert(&mut be, &s, r.id).unwrap();
        let RevertOutcome::Done { record } = rv else {
            panic!("{rv:?}")
        };
        assert_eq!(record.state, FixState::Reverted);
        assert_eq!(be.prop("MIMOPowerSaveMode").as_deref(), Some("0"));
    }

    #[test]
    fn revert_deletes_value_that_was_absent() {
        let mut be = Mock::new();
        let s = store();
        let t = target(FixAction::AdapterProperty {
            keyword: "ChannelWidth24".into(),
            value: "0".into(),
        });
        let from = from_of(&mut be, &t);
        assert_eq!(from.as_ref().unwrap().value, "1"); // driver default
        let r = done(apply(&mut be, &s, &t, from.as_ref(), &NO_RP, None).unwrap());
        assert_eq!(r.state, FixState::Applied);
        assert_eq!(r.prior_raw, None);
        revert(&mut be, &s, r.id).unwrap();
        assert!(be.log.contains(&"set ChannelWidth24=None".to_string()));
        assert_eq!(be.prop("ChannelWidth24"), None);
    }

    #[test]
    fn changed_since_preview_changes_nothing() {
        let mut be = Mock::new();
        let s = store();
        let stale = ValueText {
            value: "1".into(),
            label: None,
        };
        let o = apply(&mut be, &s, &mimo(), Some(&stale), &NO_RP, None).unwrap();
        assert!(matches!(o, ApplyOutcome::ChangedSincePreview { .. }));
        assert!(be.log.iter().all(|l| l == "read"));
    }

    #[test]
    fn restore_point_skipped_asks_then_continues() {
        let mut be = Mock::new();
        be.rp = RestorePointOutcome::Skipped;
        let s = store();
        let t = mimo();
        let from = from_of(&mut be, &t);
        let opts = ApplyOptions {
            restore_point: true,
            continue_without_restore_point: false,
        };
        let o = apply(&mut be, &s, &t, from.as_ref(), &opts, None).unwrap();
        assert!(matches!(
            o,
            ApplyOutcome::RestorePointNotCreated {
                outcome: RestorePointOutcome::Skipped
            }
        ));
        assert_eq!(be.prop("MIMOPowerSaveMode").as_deref(), Some("0"));
        assert!(s.lock().unwrap().list_fixes(10).unwrap().is_empty());

        let opts = ApplyOptions {
            restore_point: true,
            continue_without_restore_point: true,
        };
        let r = done(apply(&mut be, &s, &t, from.as_ref(), &opts, None).unwrap());
        assert_eq!(r.state, FixState::Applied);
        assert!(r.restore_point.is_none());
        assert_eq!(be.log.iter().filter(|l| l.starts_with("rp ")).count(), 1);
    }

    #[test]
    fn restore_point_created_is_recorded() {
        let mut be = Mock::new();
        let s = store();
        let t = mimo();
        let from = from_of(&mut be, &t);
        let opts = ApplyOptions {
            restore_point: true,
            continue_without_restore_point: false,
        };
        let r = done(apply(&mut be, &s, &t, from.as_ref(), &opts, None).unwrap());
        assert_eq!(
            r.restore_point,
            Some(RestorePointOutcome::Created { sequence: 5 })
        );
        let i_rp = be.log.iter().position(|l| l.starts_with("rp ")).unwrap();
        let i_set = be.log.iter().position(|l| l.starts_with("set ")).unwrap();
        assert!(i_rp < i_set);
    }

    #[test]
    fn write_failure_is_recorded_as_failed() {
        let mut be = Mock::new();
        be.fail_write = true;
        let s = store();
        let t = mimo();
        let from = from_of(&mut be, &t);
        let r = done(apply(&mut be, &s, &t, from.as_ref(), &NO_RP, None).unwrap());
        assert_eq!(r.state, FixState::Failed);
        assert!(r.error.as_deref().unwrap().contains("access denied"));
        assert!(!be.log.contains(&"restart".to_string()));
        let rv = revert(&mut be, &s, r.id).unwrap();
        assert!(matches!(rv, RevertOutcome::Refused { .. }));
    }

    #[test]
    fn verify_mismatch_is_flagged() {
        let mut be = Mock::new();
        be.ignore_writes = true;
        let s = store();
        let t = mimo();
        let from = from_of(&mut be, &t);
        let r = done(apply(&mut be, &s, &t, from.as_ref(), &NO_RP, None).unwrap());
        assert_eq!(r.state, FixState::VerifyFailed);
    }

    #[test]
    fn restart_failure_is_not_fatal() {
        let mut be = Mock::new();
        be.fail_restart = true;
        let s = store();
        let t = mimo();
        let from = from_of(&mut be, &t);
        let r = done(apply(&mut be, &s, &t, from.as_ref(), &NO_RP, None).unwrap());
        assert_eq!(r.state, FixState::Applied);
        assert!(r.reboot_required);
        assert!(!r.notes.is_empty());
    }

    #[test]
    fn revert_refuses_when_value_changed_since() {
        let mut be = Mock::new();
        let s = store();
        let t = mimo();
        let from = from_of(&mut be, &t);
        let r = done(apply(&mut be, &s, &t, from.as_ref(), &NO_RP, None).unwrap());
        // Someone picks another value in Device Manager afterwards.
        be.dev.as_mut().unwrap().advanced[0].current = Some("1".into());
        let n = be.log.len();
        let rv = revert(&mut be, &s, r.id).unwrap();
        assert!(matches!(rv, RevertOutcome::Refused { reason } if reason.contains("changed since")));
        assert!(be.log[n..].iter().all(|l| l == "read"));
    }

    #[test]
    fn power_fix_keeps_other_bits() {
        let mut be = Mock::new();
        {
            let pw = be.dev.as_mut().unwrap().power.as_mut().unwrap();
            pw.pnp_capabilities_raw = Some(0x10);
            pw.allow_turn_off = Some(true);
        }
        let s = store();
        let t = target(FixAction::DevicePower {
            allow_turn_off: false,
        });
        let from = from_of(&mut be, &t);
        let r = done(apply(&mut be, &s, &t, from.as_ref(), &NO_RP, None).unwrap());
        assert_eq!(r.state, FixState::Applied, "{r:?}");
        assert!(be.log.contains(&"pnp Some(24)".to_string()));
        revert(&mut be, &s, r.id).unwrap();
        assert!(be.log.contains(&"pnp Some(16)".to_string()));
    }

    #[test]
    fn power_fix_revert_deletes_absent_value() {
        let mut be = Mock::new();
        let s = store();
        let t = target(FixAction::DevicePower {
            allow_turn_off: false,
        });
        let from = from_of(&mut be, &t);
        let r = done(apply(&mut be, &s, &t, from.as_ref(), &NO_RP, None).unwrap());
        assert_eq!(r.prior_raw, None);
        revert(&mut be, &s, r.id).unwrap();
        assert!(be.log.contains(&"pnp None".to_string()));
    }

    #[test]
    fn rollback_verified_and_not_revertible() {
        let mut be = Mock::new();
        let s = store();
        let t = target(FixAction::RollbackDriver {});
        let Preview::Ready { plan } = preview(&mut be, &t) else {
            panic!()
        };
        assert!(plan.restore_point_default && !plan.revertible);
        let r = done(apply(&mut be, &s, &t, plan.from.as_ref(), &NO_RP, None).unwrap());
        assert_eq!(r.state, FixState::Failed); // no previous driver kept

        be.rollback_to = Some("22.250.0.2".into());
        let r = done(apply(&mut be, &s, &t, plan.from.as_ref(), &NO_RP, None).unwrap());
        assert_eq!(r.state, FixState::Applied);
        assert!(matches!(
            revert(&mut be, &s, r.id).unwrap(),
            RevertOutcome::Refused { .. }
        ));
    }

    #[test]
    fn guided_fix_is_never_applied() {
        let mut be = Mock::new();
        let t = target(FixAction::Guided {
            steps: vec!["x".into()],
            page: None,
        });
        assert!(matches!(preview(&mut be, &t), Preview::Refused { .. }));
    }

    #[test]
    fn restore_point_text_fits_helper_limit() {
        let d = restore_point_description(&"x".repeat(200));
        assert!(helper_ipc::proto::valid_text(&d, 64));
    }

    #[test]
    fn pending_rows_become_unknown() {
        let s = store();
        let mut be = Mock::new();
        let t = mimo();
        let from = from_of(&mut be, &t);
        let r = done(apply(&mut be, &s, &t, from.as_ref(), &NO_RP, None).unwrap());
        let mut pending = r.clone();
        pending.state = FixState::Pending;
        let st = s.lock().unwrap();
        st.update_fix(&pending).unwrap();
        assert_eq!(st.mark_pending_unknown().unwrap(), 1);
        assert_eq!(st.get_fix(r.id).unwrap().unwrap().state, FixState::Unknown);
    }
}
