//! Plugin directory watcher: hot reload for dev plugins and reactive store
//! refresh when user plugins change on disk (spec §87: hot reload where
//! possible).

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

use crate::Kernel;

/// Start watching plugin directories. Changes trigger re-discovery and a
/// `plugin-state` push so the shell reloads affected plugins.
pub fn start(kernel: std::sync::Arc<Kernel>) {
    let (tx, rx) = mpsc::channel();
    let mut watcher = match notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(error = %e, "filesystem watcher unavailable, hot reload disabled");
            return;
        }
    };

    let mut watched_any = false;
    for dir in [&kernel.dirs.dev_plugins, &kernel.dirs.plugins] {
        if dir.is_dir() {
            if let Err(e) = watcher.watch(Path::new(dir), RecursiveMode::Recursive) {
                tracing::warn!(dir = %dir.display(), error = %e, "cannot watch plugin dir");
            } else {
                watched_any = true;
            }
        }
    }
    if !watched_any {
        return;
    }

    // Keep the watcher alive for the process lifetime.
    std::thread::Builder::new()
        .name("ed-plugin-watcher".into())
        .spawn(move || {
            // `watcher` must outlive the loop; move it here.
            let _watcher = watcher;
            let mut last_change = std::time::Instant::now() - Duration::from_secs(10);
            let mut dirty = false;
            loop {
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(_event) => {
                        dirty = true;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if dirty && last_change.elapsed() > Duration::from_millis(400) {
                            dirty = false;
                            last_change = std::time::Instant::now();
                            match kernel.plugins.discover() {
                                Ok(_) => {
                                    kernel.sync_registries();
                                    kernel
                                        .push
                                        .push("plugin-state", None, serde_json::json!({ "reason": "fs-change" }));
                                    tracing::info!("plugin directories changed, rediscovered");
                                }
                                Err(e) => tracing::warn!(error = %e, "plugin rediscovery failed"),
                            }
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        })
        .expect("spawn plugin watcher thread");
}
