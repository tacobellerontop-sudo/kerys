//! Hot reload: watch the config file and call back when it changes.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

/// Editors often write a file in several steps; wait this long after the last
/// change before reloading.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// Starts watching `path` and calls `changed` after each save. The returned
/// watcher must be kept alive.
pub fn watch_config(
    path: &Path,
    changed: impl Fn() + Send + 'static,
) -> notify::Result<impl Watcher> {
    let file_name = path.file_name().map(ToOwned::to_owned);
    let dir: PathBuf = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let (tx, rx) = mpsc::channel::<()>();

    // Watch the directory, not the file: many editors replace the file on save.
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            let ours = ev
                .paths
                .iter()
                .any(|p| p.file_name() == file_name.as_deref());
            if ours && (ev.kind.is_modify() || ev.kind.is_create()) {
                let _ = tx.send(());
            }
        }
    })?;
    watcher.watch(&dir, RecursiveMode::NonRecursive)?;

    std::thread::Builder::new()
        .name("config-watch".into())
        .spawn(move || {
            while rx.recv().is_ok() {
                // Swallow the burst of events from a single save.
                while rx.recv_timeout(DEBOUNCE).is_ok() {}
                changed();
            }
        })
        .map_err(|e| notify::Error::generic(&e.to_string()))?;
    Ok(watcher)
}
