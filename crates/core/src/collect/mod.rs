//! Data collection from the live system. Everything here is read-only and
//! runs as a normal user; failures become `SourceWarning`s, never panics.

use crate::snapshot::{SNAPSHOT_SCHEMA_VERSION, SystemSnapshot};
use tokio_util::sync::CancellationToken;

pub mod eventxml;
pub mod setupapi_log;
#[cfg(windows)]
pub(crate) mod win;

/// How far back event logs and setupapi.dev.log are read.
pub const EVENT_WINDOW_DAYS: u32 = 14;

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
    let (machine, devices, events, setupapi, dumps) = {
        progress("machine", 0, 1);
        let machine = win::machine::collect(&mut warnings);
        let devices = win::devices::collect(progress, cancel, &mut warnings)?;
        let events = win::events::collect(EVENT_WINDOW_DAYS, progress, cancel, &mut warnings)?;
        progress("setupapi", 0, 1);
        let setupapi = win::dumps::setupapi(EVENT_WINDOW_DAYS, &mut warnings);
        if cancel.is_cancelled() {
            return Err(CollectError::Cancelled);
        }
        progress("dumps", 0, 1);
        let dumps = win::dumps::collect(&mut warnings);
        (machine, devices, events, setupapi, dumps)
    };
    #[cfg(not(windows))]
    let (machine, devices, events, setupapi, dumps) = {
        let _ = (progress, cancel);
        return Err(CollectError::Fatal("only supported on Windows".into()));
    };

    Ok(SystemSnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        collected_at: chrono::Utc::now().to_rfc3339(),
        machine,
        devices,
        events,
        setupapi,
        dumps,
        event_window_days: EVENT_WINDOW_DAYS,
        warnings,
    })
}
