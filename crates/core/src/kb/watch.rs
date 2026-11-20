//! Hot reload: watch the KB directory and reload after edits settle.

use super::loader::{Kb, load_dir};
use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

/// Editors often write a file in several steps; wait this long for quiet.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// Keeps the watch alive; dropping it stops watching.
pub struct KbWatcher {
    _watcher: notify::RecommendedWatcher,
}

/// Calls `on_reload` with a freshly loaded KB whenever files under `dir` change.
pub fn watch(dir: &Path, on_reload: impl Fn(Kb) + Send + 'static) -> Result<KbWatcher, String> {
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res
            && !ev.kind.is_access()
        {
            let _ = tx.send(());
        }
    })
    .map_err(|e| format!("cannot watch the knowledge base: {e}"))?;
    watcher
        .watch(dir, RecursiveMode::Recursive)
        .map_err(|e| format!("cannot watch {}: {e}", dir.display()))?;
    let dir: PathBuf = dir.to_path_buf();
    std::thread::Builder::new()
        .name("kb-watch".into())
        .spawn(move || {
            // Ends when the watcher (and with it `tx`) is dropped.
            while rx.recv().is_ok() {
                while rx.recv_timeout(DEBOUNCE).is_ok() {}
                on_reload(load_dir(&dir));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(KbWatcher { _watcher: watcher })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn reloads_after_a_change() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("rules")).unwrap();
        let got: Arc<Mutex<Vec<usize>>> = Arc::default();
        let g = got.clone();
        let _w = watch(d.path(), move |kb| g.lock().unwrap().push(kb.errors.len())).unwrap();
        std::fs::write(d.path().join("rules").join("bad.yaml"), "id: [").unwrap();
        for _ in 0..50 {
            if !got.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(got.lock().unwrap().last(), Some(&1));
    }
}
