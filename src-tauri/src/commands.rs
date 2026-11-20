use crate::AppState;
use auriga_core::analyze::{Analysis, analyze};
use auriga_core::collect::{CollectError, collect_snapshot};
use auriga_core::kb::{self, FindingsReport};
use auriga_core::snapshot::{ScanProgress, SystemSnapshot};
use auriga_core::store::{ScanStatus, ScanSummary};
use serde::Serialize;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

pub const EV_PROGRESS: &str = "scan://progress";
pub const EV_FINISHED: &str = "scan://finished";
pub const EV_KB_RELOADED: &str = "kb://reloaded";

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    pub version: String,
    pub store_warning: Option<String>,
    pub scan_running: bool,
}

#[derive(Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScanFinished {
    #[ts(type = "number")]
    pub scan_id: i64,
    pub status: ScanStatus,
    pub error: Option<String>,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LatestSnapshot {
    pub summary: ScanSummary,
    pub snapshot: SystemSnapshot,
    /// Computed on load (pure, cheap), never stored.
    pub analysis: Analysis,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HelperTestResult {
    pub pid: u32,
    pub elevated: bool,
    pub round_trip_ms: f64,
    pub exit_code: Option<u32>,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        store_warning: state.store_warning.clone(),
        scan_running: state.scan.lock().unwrap().is_some(),
    }
}

/// Starts a background scan and returns its id immediately. Progress is
/// streamed on `scan://progress`, completion on `scan://finished`.
#[tauri::command]
pub fn start_scan(app: AppHandle, state: State<'_, AppState>) -> CmdResult<i64> {
    let token = {
        let mut slot = state.scan.lock().unwrap();
        if slot.is_some() {
            return Err("A scan is already running.".into());
        }
        let t = CancellationToken::new();
        *slot = Some(t.clone());
        t
    };
    let id = match state.store.lock().unwrap().begin_scan() {
        Ok(id) => id,
        Err(e) => {
            *state.scan.lock().unwrap() = None;
            return Err(err(e));
        }
    };

    tauri::async_runtime::spawn_blocking(move || {
        let emitter = app.clone();
        let progress = move |stage: &str, done: u32, total: u32| {
            let _ = emitter.emit(
                EV_PROGRESS,
                ScanProgress {
                    scan_id: id as i32,
                    stage: stage.into(),
                    done,
                    total,
                },
            );
        };
        let result = collect_snapshot(&progress, &token);
        let state = app.state::<AppState>();
        let (status, snap, error) = match result {
            Ok(s) => (ScanStatus::Completed, Some(s), None),
            Err(CollectError::Cancelled) => (ScanStatus::Cancelled, None, None),
            Err(CollectError::Fatal(e)) => (ScanStatus::Failed, None, Some(e)),
        };
        let save =
            state
                .store
                .lock()
                .unwrap()
                .finish_scan(id, status, snap.as_ref(), error.as_deref());
        let (status, error) = match save {
            Ok(()) => (status, error),
            Err(e) => (
                ScanStatus::Failed,
                Some(format!("could not save scan: {e}")),
            ),
        };
        *state.scan.lock().unwrap() = None;
        let _ = app.emit(
            EV_FINISHED,
            ScanFinished {
                scan_id: id,
                status,
                error,
            },
        );
    });
    Ok(id)
}

#[tauri::command]
pub fn cancel_scan(state: State<'_, AppState>) -> bool {
    match &*state.scan.lock().unwrap() {
        Some(t) => {
            t.cancel();
            true
        }
        None => false,
    }
}

#[tauri::command]
pub fn get_latest_snapshot(state: State<'_, AppState>) -> CmdResult<Option<LatestSnapshot>> {
    Ok(state
        .store
        .lock()
        .unwrap()
        .latest_snapshot()
        .map_err(err)?
        .map(|(summary, snapshot)| LatestSnapshot {
            summary,
            analysis: analyze(&snapshot),
            snapshot,
        }))
}

/// Evaluates the current KB over the latest completed scan.
#[tauri::command]
pub fn get_findings(state: State<'_, AppState>) -> CmdResult<FindingsReport> {
    let kb = state.kb.read().unwrap().clone();
    let latest = state.store.lock().unwrap().latest_snapshot().map_err(err)?;
    let (scan_collected_at, findings) = match latest {
        Some((_, snap)) => (
            Some(snap.collected_at.clone()),
            kb::eval_snapshot(&snap, &kb.rules),
        ),
        None => (None, vec![]),
    };
    Ok(FindingsReport {
        kb: kb.status(),
        scan_collected_at,
        findings,
    })
}

#[tauri::command]
pub fn list_scans(state: State<'_, AppState>, limit: u32) -> CmdResult<Vec<ScanSummary>> {
    state
        .store
        .lock()
        .unwrap()
        .list_scans(limit.min(500))
        .map_err(err)
}

/// Launches the helper with a UAC prompt, pings it once and shuts it down.
/// Proves the elevation + IPC path end to end.
#[tauri::command]
pub async fn helper_test() -> CmdResult<HelperTestResult> {
    #[cfg(windows)]
    {
        use helper_ipc::proto::{HelperOp, HelperResult};
        use helper_ipc::win::{HelperSession, LaunchMode};
        tauri::async_runtime::spawn_blocking(|| {
            let exe = std::env::current_exe()
                .map_err(err)?
                .with_file_name("auriga-helper.exe");
            let mut s = HelperSession::launch(&exe, LaunchMode::Elevated).map_err(err)?;
            let nonce = u64::from(s.pid()) ^ 0x5A5A_5A5A;
            let t = Instant::now();
            let reply = s.request(HelperOp::Ping { nonce }).map_err(err)?;
            let round_trip_ms = t.elapsed().as_secs_f64() * 1000.0;
            if reply != (HelperResult::Pong { nonce }) {
                return Err(format!("unexpected reply: {reply:?}"));
            }
            let (pid, elevated) = (s.pid(), s.elevated());
            Ok(HelperTestResult {
                pid,
                elevated,
                round_trip_ms,
                exit_code: s.shutdown(),
            })
        })
        .await
        .map_err(err)?
    }
    #[cfg(not(windows))]
    {
        let _ = Instant::now();
        Err("Windows only".into())
    }
}
