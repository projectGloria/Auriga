//! Data collection from the live system. Everything here is read-only and
//! runs as a normal user; failures become `SourceWarning`s, never panics.

use crate::snapshot::{SNAPSHOT_SCHEMA_VERSION, SystemSnapshot};
use tokio_util::sync::CancellationToken;

#[cfg(windows)]
mod win;

#[derive(Debug, thiserror::Error)]
pub enum CollectError {
    #[error("scan cancelled")]
    Cancelled,
    #[error("{0}")]
    Fatal(String),
}

/// Progress callback: (stage, done, total).
pub type ProgressFn<'a> = &'a (dyn Fn(&str, u32, u32) + Sync);

/// Collect a full snapshot. Blocking; call from a worker thread.
pub fn collect_snapshot(
    progress: ProgressFn<'_>,
    cancel: &CancellationToken,
) -> Result<SystemSnapshot, CollectError> {
    let mut warnings = Vec::new();

    #[cfg(windows)]
    let (machine, devices) = {
        progress("machine", 0, 1);
        let machine = win::machine::collect(&mut warnings);
        let devices = win::devices::collect(progress, cancel, &mut warnings)?;
        (machine, devices)
    };
    #[cfg(not(windows))]
    let (machine, devices) = {
        let _ = (progress, cancel);
        return Err(CollectError::Fatal("only supported on Windows".into()));
    };

    Ok(SystemSnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        collected_at: chrono::Utc::now().to_rfc3339(),
        machine,
        devices,
        warnings,
    })
}
