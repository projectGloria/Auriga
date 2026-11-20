//! The shipped KB against JSON fixture snapshots (`fixtures/snapshots`).
//! Fixtures reuse a real AX201's advanced-property table; events are synthetic.

use auriga_core::kb::{self, Finding, FixStatus, Kb};
use auriga_core::snapshot::SystemSnapshot;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn repo_kb() -> Kb {
    let kb = kb::load_dir(&root().join("kb"));
    assert!(kb.errors.is_empty(), "KB errors: {:#?}", kb.errors);
    kb
}

fn findings(fixture: &str) -> Vec<Finding> {
    let path = root().join("fixtures/snapshots").join(fixture);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let snap: SystemSnapshot = serde_json::from_str(&text).unwrap();
    kb::eval_snapshot(&snap, &repo_kb().rules)
}

fn ids(f: &[Finding]) -> Vec<&str> {
    f.iter().map(|f| f.rule_id.as_str()).collect()
}

fn status<'a>(f: &'a Finding, fix: &str) -> &'a FixStatus {
    &f.fixes
        .iter()
        .find(|x| x.id == fix)
        .unwrap_or_else(|| panic!("no fix {fix}"))
        .status
}

#[test]
fn repo_kb_loads_cleanly() {
    let kb = repo_kb();
    assert!(kb.rules.len() >= 5);
    assert!(kb.kb_version.is_some());
}

#[test]
fn ax201_with_disconnects_fires() {
    let f = findings("ax201-disconnects.json");
    let ax = f
        .iter()
        .find(|f| f.rule_id == "intel.ax201.disconnects")
        .unwrap_or_else(|| panic!("AX201 rule did not fire: {:?}", ids(&f)));
    assert!(ax.device_name.as_deref().unwrap().contains("AX201"));
    assert!(ax.summary.contains("AX201"), "{}", ax.summary);
    assert!(
        ax.evidence
            .iter()
            .any(|e| e.starts_with("8 Wi-Fi disconnects in the last 14 days") && e.contains("7 ×")),
        "{:?}",
        ax.evidence
    );
    // Statuses come from this adapter's real values:
    // MIMO already No SMPS (3), 802.11 mode already "ac" (2), the rest still on.
    assert!(matches!(
        status(ax, "mimo-no-smps"),
        FixStatus::AlreadySet { .. }
    ));
    assert!(matches!(
        status(ax, "mode-80211ac"),
        FixStatus::AlreadySet { .. }
    ));
    match status(ax, "width24-20mhz") {
        FixStatus::Applicable {
            from: Some(from),
            to,
        } => {
            assert_eq!((from.value.as_str(), to.value.as_str()), ("1", "0"));
            assert_eq!(to.label.as_deref(), Some("20 MHz Only"));
        }
        s => panic!("{s:?}"),
    }
    assert!(matches!(
        status(ax, "packet-coalescing-off"),
        FixStatus::Applicable { .. }
    ));
    assert!(matches!(
        status(ax, "gtk-rekey-off"),
        FixStatus::Applicable { .. }
    ));
    assert_eq!(status(ax, "check-driver"), &FixStatus::Manual);

    // The generic Intel rule excludes the AX201 so it is not reported twice.
    assert!(
        !ids(&f).contains(&"intel.wifi.disconnects"),
        "{:?}",
        ids(&f)
    );
    // Power-off allowed + disconnects → NIC power rule too, on the same adapter only.
    let nic: Vec<_> = f
        .iter()
        .filter(|f| f.rule_id == "nic.power.allow-turn-off")
        .collect();
    assert_eq!(nic.len(), 1, "{:?}", ids(&f));
    assert_eq!(nic[0].device_id, ax.device_id);
    assert!(matches!(
        status(nic[0], "disallow-turn-off"),
        FixStatus::Applicable { .. }
    ));
}

#[test]
fn ax201_healthy_does_not_fire() {
    let f = findings("ax201-healthy.json");
    assert!(f.is_empty(), "unexpected findings: {:?}", ids(&f));
}

#[test]
fn ax210_does_not_fire_ax201_rule() {
    let f = findings("ax210-disconnects.json");
    let got = ids(&f);
    assert!(!got.contains(&"intel.ax201.disconnects"), "{got:?}");
    let generic = f
        .iter()
        .find(|f| f.rule_id == "intel.wifi.disconnects")
        .unwrap_or_else(|| panic!("generic Intel rule did not fire: {got:?}"));
    assert!(generic.device_name.as_deref().unwrap().contains("AX210"));
    assert!(got.contains(&"nic.power.allow-turn-off"), "{got:?}");
}

#[test]
fn findings_sorted_by_severity_then_confidence() {
    let f = findings("ax201-disconnects.json");
    let keys: Vec<_> = f.iter().map(|f| (f.severity, f.confidence)).collect();
    let mut sorted = keys.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(keys, sorted);
}

#[test]
fn invalid_rule_samples_are_rejected() {
    let dir = root().join("fixtures/invalid-rules");
    let mut n = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let text = std::fs::read_to_string(&p).unwrap();
        let err = kb::parse_rule(&text).err();
        assert!(err.is_some(), "{} should be rejected", p.display());
        n += 1;
    }
    assert!(n >= 3);
}
