//! Crash dump files: metadata only. Listing `Minidump` works as a normal
//! user on default ACLs; reading contents needs admin (opt-in helper op).

use crate::snapshot::{DumpFile, SourceWarning};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

fn windir() -> PathBuf {
    std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("windir"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

pub fn collect(warnings: &mut Vec<SourceWarning>) -> Vec<DumpFile> {
    let root = windir();
    let mut out = Vec::new();
    list_dir(&root.join("Minidump"), &mut out, warnings);
    // Live kernel reports (TDR/watchdog) live one level down, e.g. WATCHDOG\.
    let lkr = root.join("LiveKernelReports");
    list_dir(&lkr, &mut out, warnings);
    if let Ok(rd) = std::fs::read_dir(&lkr) {
        for e in rd.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                list_dir(&e.path(), &mut out, warnings);
            }
        }
    }
    if let Some(f) = file_info(&root.join("MEMORY.DMP")) {
        out.push(f);
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

fn list_dir(dir: &Path, out: &mut Vec<DumpFile>, warnings: &mut Vec<SourceWarning>) {
    match std::fs::read_dir(dir) {
        Ok(rd) => {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dmp"))
                    && let Some(f) = file_info(&p)
                {
                    out.push(f);
                }
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => warnings.push(SourceWarning {
            source: "dumps".into(),
            message: format!(
                "{}: {} (listing it needs admin; other dump folders were read)",
                dir.display(),
                short_io(&e)
            ),
            instance_id: None,
        }),
    }
}

fn file_info(p: &Path) -> Option<DumpFile> {
    let m = std::fs::metadata(p).ok()?;
    Some(DumpFile {
        path: p.display().to_string(),
        size: m.len(),
        modified: m.modified().ok().map(|t| {
            chrono::DateTime::<chrono::Utc>::from(t)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        }),
    })
}

pub fn short_io(e: &std::io::Error) -> String {
    match e.kind() {
        ErrorKind::PermissionDenied => "access denied".into(),
        ErrorKind::NotFound => "not found".into(),
        _ => e.to_string(),
    }
}

/// `%windir%\INF\setupapi.dev.log`, sections within the window.
pub fn setupapi(
    days: u32,
    warnings: &mut Vec<SourceWarning>,
) -> Vec<crate::snapshot::SetupApiSection> {
    let path = windir().join("INF").join("setupapi.dev.log");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            warnings.push(SourceWarning {
                source: "setupapi".into(),
                message: format!("{}: {}", path.display(), short_io(&e)),
                instance_id: None,
            });
            return Vec::new();
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(i64::from(days.max(1))))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    crate::collect::setupapi_log::parse(&text, &chrono::Local, 400)
        .into_iter()
        .filter(|s| s.start.as_deref().is_none_or(|t| t >= cutoff.as_str()))
        .collect()
}
