//! Dev tool: `cargo run -p auriga-core --example collect -- [snapshot.json] [preview.json] [findings.json]`
//!
//! `preview.json` has the `LatestSnapshot` shape; put it at
//! `public/dev-latest.json` to render real data in a plain browser
//! (`npm run dev`, dev builds only). `findings.json` (a `FindingsReport`
//! from the repo KB) goes at `public/dev-findings.json`.
use auriga_core::collect::collect_snapshot;
use tokio_util::sync::CancellationToken;

fn main() {
    let t = std::time::Instant::now();
    let last = std::sync::Mutex::new((String::new(), std::time::Instant::now()));
    let snap = collect_snapshot(
        &|stage, _, _| {
            let mut l = last.lock().unwrap();
            if l.0 != stage {
                if !l.0.is_empty() {
                    eprintln!("  {} took {:?}", l.0, l.1.elapsed());
                }
                *l = (stage.to_string(), std::time::Instant::now());
            }
        },
        &CancellationToken::new(),
    )
    .expect("collect");
    let present = snap.devices.iter().filter(|d| d.present).count();
    let with_driver = snap
        .devices
        .iter()
        .filter(|d| d.present && d.driver.is_some())
        .count();
    let problems: Vec<_> = snap
        .devices
        .iter()
        .filter(|d| d.status.problem_code.is_some())
        .map(|d| format!("{} -> {:?}", d.name, d.status.problem_text))
        .collect();
    let mut sig = std::collections::BTreeMap::<String, usize>::new();
    for d in snap.devices.iter().filter(|d| d.present) {
        if let Some(dr) = &d.driver {
            *sig.entry(format!("{:?}", dr.signature.status)).or_default() += 1;
        }
    }
    println!(
        "{} devices ({} present, {} present with driver) in {:?}",
        snap.devices.len(),
        present,
        with_driver,
        t.elapsed()
    );
    println!("machine: {:?}", snap.machine);
    println!("signature status counts: {sig:?}");
    println!("problems: {problems:#?}");
    let mut by = std::collections::BTreeMap::<String, usize>::new();
    for e in &snap.events {
        *by.entry(e.channel.clone()).or_default() += 1;
    }
    let msgs = snap.events.iter().filter(|e| e.message.is_some()).count();
    println!("events: {} ({msgs} with message) {by:?}", snap.events.len());
    println!(
        "setupapi sections: {}, dumps: {}",
        snap.setupapi.len(),
        snap.dumps.len()
    );
    let a = auriga_core::analyze::analyze(&snap);
    let attributed = a.event_device.iter().filter(|d| d.is_some()).count();
    println!(
        "analysis: {attributed}/{} events attributed, {} groups, {} timeline entries",
        a.event_device.len(),
        a.groups.len(),
        a.timeline.len()
    );
    for s in &a.signals {
        println!(
            "  signal {:?}: {} (24h {}, 7d {}, peak/h {}) {:?}",
            s.kind, s.count, s.last_24h, s.last_7d, s.peak_per_hour, s.breakdown
        );
    }
    let mut lv = std::collections::BTreeMap::<String, usize>::new();
    for m in &a.matches {
        *lv.entry(format!("{:?}", m.level)).or_default() += 1;
    }
    println!("match levels: {lv:?}");
    for (m, d) in a.matches.iter().zip(&snap.devices) {
        if m.level >= auriga_core::analyze::Level::Warn && d.present {
            println!("  {:?} {}: {:?}", m.level, d.name, m.notes);
        }
    }
    for t in a.timeline.iter().take(12) {
        println!("  {} x{} {:?} {}", t.time, t.count, t.kind, t.title);
    }
    println!("warnings: {}", snap.warnings.len());
    for w in snap.warnings.iter().take(5) {
        println!("  {w:?}");
    }
    if let Some(p) = std::env::args().nth(1) {
        std::fs::write(&p, serde_json::to_string_pretty(&snap).unwrap()).unwrap();
        println!("wrote {p}");
    }
    if let Some(p) = std::env::args().nth(2) {
        let preview = serde_json::json!({
            "summary": {
                "id": 0, "startedAt": snap.collected_at, "finishedAt": snap.collected_at,
                "status": "completed", "deviceCount": snap.devices.len(),
                "warningCount": snap.warnings.len(), "error": null
            },
            "snapshot": &snap,
            "analysis": auriga_core::analyze::analyze(&snap),
        });
        std::fs::write(&p, serde_json::to_string(&preview).unwrap()).unwrap();
        println!("wrote {p}");
    }
    let kb = auriga_core::kb::load_dir(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../kb"),
    );
    let findings = auriga_core::kb::eval(&snap, &a, &kb.rules);
    println!(
        "kb: {} rules, {} errors; {} findings",
        kb.rules.len(),
        kb.errors.len(),
        findings.len()
    );
    for e in &kb.errors {
        println!("  kb error {}: {}", e.file, e.message);
    }
    for f in &findings {
        println!(
            "  [{:?}/{:?}] {} — {}",
            f.severity, f.confidence, f.rule_id, f.summary
        );
        for e in &f.evidence {
            println!("      • {e}");
        }
        for x in &f.fixes {
            println!("      fix {}: {:?}", x.id, x.status);
        }
    }
    if let Some(p) = std::env::args().nth(3) {
        let report = auriga_core::kb::FindingsReport {
            kb: kb.status(),
            scan_collected_at: Some(snap.collected_at.clone()),
            findings,
        };
        std::fs::write(&p, serde_json::to_string(&report).unwrap()).unwrap();
        println!("wrote {p}");
    }
}
