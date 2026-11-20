//! Thin Tauri layer: typed commands + events over `auriga-core`.
//! Runs as the normal user; anything privileged goes through the helper.

mod commands;
pub mod dump;

use auriga_core::kb::{self, Kb, watch::KbWatcher};
use auriga_core::store::Store;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tauri::{Emitter, Manager};
use tokio_util::sync::CancellationToken;

pub struct AppState {
    pub store: Mutex<Store>,
    /// Warning shown in the UI if the on-disk DB could not be opened.
    pub store_warning: Option<String>,
    pub scan: Mutex<Option<CancellationToken>>,
    /// Current knowledge base; swapped whole on hot reload.
    pub kb: RwLock<Arc<Kb>>,
    pub kb_watcher: Mutex<Option<KbWatcher>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let (store, store_warning) = open_store(app.path().app_data_dir().ok());
            app.manage(AppState {
                store: Mutex::new(store),
                store_warning,
                scan: Mutex::new(None),
                kb: RwLock::new(Arc::new(Kb::default())),
                kb_watcher: Mutex::new(None),
            });
            start_kb(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::start_scan,
            commands::cancel_scan,
            commands::get_latest_snapshot,
            commands::list_scans,
            commands::helper_test,
            commands::get_findings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Auriga");
}

/// Open the on-disk DB; on failure keep going with an in-memory DB and a
/// visible warning instead of refusing to start.
fn open_store(dir: Option<std::path::PathBuf>) -> (Store, Option<String>) {
    let attempt = dir
        .ok_or_else(|| "no app data directory".to_string())
        .and_then(|d| Store::open(&d.join("auriga.db")).map_err(|e| e.to_string()));
    match attempt {
        Ok(s) => {
            let _ = s.recover_interrupted();
            (s, None)
        }
        Err(e) => (
            Store::open_in_memory().expect("in-memory sqlite"),
            Some(format!(
                "History database unavailable ({e}); results won't be kept after exit."
            )),
        ),
    }
}

/// KB location: `AURIGA_KB_DIR`, else the bundled `kb` resource, else (dev
/// builds) the repo's `kb` folder.
fn kb_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    let ok = |p: PathBuf| p.join("rules").is_dir().then_some(p);
    if let Some(p) = std::env::var_os("AURIGA_KB_DIR")
        .map(PathBuf::from)
        .and_then(ok)
    {
        return Some(p);
    }
    if let Some(p) = app
        .path()
        .resource_dir()
        .ok()
        .map(|r| r.join("kb"))
        .and_then(ok)
    {
        return Some(p);
    }
    if cfg!(debug_assertions) {
        return ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../kb"));
    }
    None
}

/// Load the KB and watch it; edits apply without a restart.
fn start_kb(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let Some(dir) = kb_dir(app) else {
        *state.kb.write().unwrap() = Arc::new(Kb::missing(
            "Knowledge base folder not found; no rules are loaded.".into(),
        ));
        return;
    };
    *state.kb.write().unwrap() = Arc::new(kb::load_dir(&dir));
    let handle = app.clone();
    let watcher = kb::watch::watch(&dir, move |fresh| {
        let status = fresh.status();
        *handle.state::<AppState>().kb.write().unwrap() = Arc::new(fresh);
        let _ = handle.emit(commands::EV_KB_RELOADED, status);
    });
    match watcher {
        Ok(w) => *state.kb_watcher.lock().unwrap() = Some(w),
        Err(e) => {
            // Rules still work; only live reload is lost.
            let mut kb = (**state.kb.read().unwrap()).clone();
            kb.errors.push(kb::KbError {
                file: String::new(),
                message: format!("{e} Restart the app to pick up rule edits."),
            });
            *state.kb.write().unwrap() = Arc::new(kb);
        }
    }
}
