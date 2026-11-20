//! Fix commands. Reads run here as the user; writes go to the elevated
//! helper, which is started on the first write (UAC) and kept for the fix
//! session (it exits by itself after `IDLE_TIMEOUT_SECS` idle).

use crate::AppState;
use auriga_core::analyze::analyze;
use auriga_core::fix::monitor::{self, MonitorReport};
use auriga_core::fix::{
    ApplyOptions, ApplyOutcome, FixRecord, FixTarget, MinidumpInfo, Preview, RestorePointOutcome,
    RevertOutcome, SystemBackend, engine,
};
use auriga_core::kb::{self, ValueText};
use helper_ipc::proto::{HelperOp, HelperResult};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use ts_rs::TS;

type CmdResult<T> = Result<T, String>;

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FixPreview {
    pub target: FixTarget,
    pub preview: Preview,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FixHistoryEntry {
    pub record: FixRecord,
    /// Before/after from the latest scan, when that scan is newer than the fix.
    pub monitor: Option<MonitorReport>,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MinidumpReport {
    pub files: Vec<MinidumpInfo>,
    pub warnings: Vec<String>,
}

/// The helper connection, launched lazily. One fix at a time holds it.
#[cfg(windows)]
pub type HelperSlot = Option<helper_ipc::win::HelperSession>;
#[cfg(not(windows))]
pub type HelperSlot = Option<()>;

struct AppBackend<'a> {
    slot: &'a mut HelperSlot,
}

impl AppBackend<'_> {
    #[cfg(windows)]
    fn call(&mut self, op: HelperOp) -> Result<HelperResult, String> {
        use helper_ipc::win::{HelperSession, IpcError, LaunchMode};
        if self.slot.as_ref().is_some_and(|s| !s.is_alive()) {
            *self.slot = None;
        }
        if self.slot.is_none() {
            let exe = std::env::current_exe()
                .map_err(|e| e.to_string())?
                .with_file_name("auriga-helper.exe");
            let s = HelperSession::launch(&exe, LaunchMode::Elevated).map_err(|e| match e {
                IpcError::UacDeclined => {
                    "Administrator permission was declined; nothing was changed.".to_string()
                }
                e => format!("Could not start the elevated helper: {e}"),
            })?;
            *self.slot = Some(s);
        }
        let s = self.slot.as_mut().expect("helper session");
        match s.request(op) {
            Ok(HelperResult::Error { message }) => Err(message),
            Ok(r) => Ok(r),
            Err(e) => {
                // Do not retry: the op may or may not have run.
                *self.slot = None;
                Err(format!("Lost the connection to the elevated helper: {e}"))
            }
        }
    }
    #[cfg(not(windows))]
    fn call(&mut self, _: HelperOp) -> Result<HelperResult, String> {
        Err("Windows only".into())
    }
}

fn unexpected(r: HelperResult) -> String {
    format!("Unexpected reply from the helper: {r:?}")
}

impl SystemBackend for AppBackend<'_> {
    fn read_device(
        &mut self,
        id: &str,
    ) -> Result<Option<auriga_core::snapshot::DeviceRecord>, String> {
        #[cfg(windows)]
        {
            auriga_core::fix::win::read_device(id)
        }
        #[cfg(not(windows))]
        {
            let _ = id;
            Err("Windows only".into())
        }
    }
    fn set_adapter_property(
        &mut self,
        id: &str,
        kw: &str,
        v: Option<&str>,
    ) -> Result<Option<String>, String> {
        match self.call(HelperOp::SetAdapterProperty {
            instance_id: id.into(),
            keyword: kw.into(),
            value: v.map(String::from),
        })? {
            HelperResult::PropertySet { previous } => Ok(previous),
            r => Err(unexpected(r)),
        }
    }
    fn set_pnp_capabilities(&mut self, id: &str, v: Option<u32>) -> Result<Option<u32>, String> {
        match self.call(HelperOp::SetPnpCapabilities {
            instance_id: id.into(),
            value: v,
        })? {
            HelperResult::PnpSet { previous } => Ok(previous),
            r => Err(unexpected(r)),
        }
    }
    fn restart_device(&mut self, id: &str) -> Result<bool, String> {
        match self.call(HelperOp::RestartDevice {
            instance_id: id.into(),
        })? {
            HelperResult::Restarted { reboot_required } => Ok(reboot_required),
            r => Err(unexpected(r)),
        }
    }
    fn create_restore_point(&mut self, description: &str) -> RestorePointOutcome {
        match self.call(HelperOp::CreateRestorePoint {
            description: description.into(),
        }) {
            Ok(HelperResult::RestorePoint { outcome }) => outcome,
            Ok(r) => RestorePointOutcome::Failed {
                message: unexpected(r),
            },
            Err(message) => RestorePointOutcome::Failed { message },
        }
    }
    fn rollback_driver(&mut self, id: &str) -> Result<bool, String> {
        match self.call(HelperOp::RollbackDriver {
            instance_id: id.into(),
        })? {
            HelperResult::RolledBack { reboot_required } => Ok(reboot_required),
            r => Err(unexpected(r)),
        }
    }
}

/// Rebuild the fix from the KB over the latest scan; the UI only names it.
fn find_target(state: &AppState, finding_key: &str, fix_id: &str) -> CmdResult<FixTarget> {
    let kb = state.kb.read().unwrap().clone();
    let (_, snap) = state
        .store
        .lock()
        .unwrap()
        .latest_snapshot()
        .map_err(|e| e.to_string())?
        .ok_or("No scan yet.")?;
    let findings = kb::eval_snapshot(&snap, &kb.rules);
    engine::target_from_findings(&findings, finding_key, fix_id)
        .ok_or_else(|| "This fix is no longer suggested by the latest scan. Scan again.".into())
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> CmdResult<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn preview_fix(
    app: AppHandle,
    finding_key: String,
    fix_id: String,
) -> CmdResult<FixPreview> {
    blocking(move || {
        let state = app.state::<AppState>();
        let target = find_target(&state, &finding_key, &fix_id)?;
        let mut none: HelperSlot = None;
        let preview = engine::preview(&mut AppBackend { slot: &mut none }, &target);
        Ok(FixPreview { target, preview })
    })
    .await
}

#[tauri::command]
pub async fn apply_fix(
    app: AppHandle,
    finding_key: String,
    fix_id: String,
    expected_from: Option<ValueText>,
    options: ApplyOptions,
) -> CmdResult<ApplyOutcome> {
    blocking(move || {
        let state = app.state::<AppState>();
        let target = find_target(&state, &finding_key, &fix_id)?;
        let baseline = state
            .store
            .lock()
            .unwrap()
            .latest_snapshot()
            .ok()
            .flatten()
            .map(|(_, s)| monitor::baseline(&target.device_id, &s, &analyze(&s)));
        let mut slot = state.helper.lock().unwrap();
        engine::apply(
            &mut AppBackend { slot: &mut slot },
            &state.store,
            &target,
            expected_from.as_ref(),
            &options,
            baseline,
        )
    })
    .await
}

#[tauri::command]
pub async fn revert_fix(app: AppHandle, id: i64) -> CmdResult<RevertOutcome> {
    blocking(move || {
        let state = app.state::<AppState>();
        let mut slot = state.helper.lock().unwrap();
        engine::revert(&mut AppBackend { slot: &mut slot }, &state.store, id)
    })
    .await
}

#[tauri::command]
pub fn list_fix_history(
    state: State<'_, AppState>,
    limit: u32,
) -> CmdResult<Vec<FixHistoryEntry>> {
    let store = state.store.lock().unwrap();
    let records = store.list_fixes(limit.min(500)).map_err(|e| e.to_string())?;
    let latest = store.latest_snapshot().ok().flatten();
    drop(store);
    let analysis = latest.as_ref().map(|(_, s)| analyze(s));
    Ok(records
        .into_iter()
        .map(|record| {
            let monitor = match (&latest, &analysis) {
                (Some((_, s)), Some(a)) if record.applied_at.is_some() => {
                    monitor::compare(&record, s, a)
                }
                _ => None,
            };
            FixHistoryEntry { record, monitor }
        })
        .collect())
}

/// Opt-in, read-only: reads minidump headers through the helper (the folder
/// is admin-only).
#[tauri::command]
pub async fn read_minidumps(app: AppHandle) -> CmdResult<MinidumpReport> {
    blocking(move || {
        let state = app.state::<AppState>();
        let mut slot = state.helper.lock().unwrap();
        match (AppBackend { slot: &mut slot }).call(HelperOp::ListMinidumps)? {
            HelperResult::Minidumps { files, warnings } => Ok(MinidumpReport { files, warnings }),
            r => Err(unexpected(r)),
        }
    })
    .await
}
