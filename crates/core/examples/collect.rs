//! Dev tool: `cargo run -p auriga-core --example collect -- [out.json]`
use auriga_core::collect::collect_snapshot;
use tokio_util::sync::CancellationToken;

fn main() {
    let t = std::time::Instant::now();
    let snap = collect_snapshot(&|_, _, _| {}, &CancellationToken::new()).expect("collect");
    let present = snap.devices.iter().filter(|d| d.present).count();
    let with_driver = snap.devices.iter().filter(|d| d.present && d.driver.is_some()).count();
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
    println!("warnings: {}", snap.warnings.len());
    for w in snap.warnings.iter().take(5) {
        println!("  {w:?}");
    }
    if let Some(p) = std::env::args().nth(1) {
        std::fs::write(&p, serde_json::to_string_pretty(&snap).unwrap()).unwrap();
        println!("wrote {p}");
    }
}
