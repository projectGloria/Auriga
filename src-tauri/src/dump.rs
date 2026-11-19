//! `auriga.exe --dump-snapshot <path|->`: collect a snapshot headlessly and
//! write it as JSON (for fixtures and bug reports). No window, no DB.

use auriga_core::collect::collect_snapshot;
use tokio_util::sync::CancellationToken;

pub fn dump_snapshot(path: Option<&str>) -> i32 {
    let Some(path) = path else {
        eprintln!("usage: auriga --dump-snapshot <out.json | ->");
        return 2;
    };
    let snap = match collect_snapshot(
        &|stage, done, total| eprint!("\r{stage}: {done}/{total}      "),
        &CancellationToken::new(),
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("\nscan failed: {e:?}");
            return 1;
        }
    };
    eprintln!();
    let json = match serde_json::to_string_pretty(&snap) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("serialise failed: {e}");
            return 1;
        }
    };
    if path == "-" {
        println!("{json}");
    } else if let Err(e) = std::fs::write(path, json) {
        eprintln!("cannot write {path}: {e}");
        return 1;
    } else {
        eprintln!(
            "wrote {} devices, {} warnings to {path}",
            snap.devices.len(),
            snap.warnings.len()
        );
    }
    0
}
