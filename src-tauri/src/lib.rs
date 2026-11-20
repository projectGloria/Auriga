//! Thin Tauri layer: typed commands + events over `auriga-core`.
//! Runs as the normal user; anything privileged goes through the helper.

mod commands;
pub mod dump;

use auriga_core::store::Store;
use std::sync::Mutex;
use tauri::Manager;
use tokio_util::sync::CancellationToken;

pub struct AppState {
    pub store: Mutex<Store>,
    /// Warning shown in the UI if the on-disk DB could not be opened.
    pub store_warning: Option<String>,
    pub scan: Mutex<Option<CancellationToken>>,
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
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::start_scan,
            commands::cancel_scan,
            commands::get_latest_snapshot,
            commands::list_scans,
            commands::helper_test,
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
