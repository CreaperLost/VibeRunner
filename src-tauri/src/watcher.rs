//! File-system watcher for `vibe.config.json`.
//!
//! Uses the `notify` crate (FSEvents on macOS, ReadDirectoryChangesW on
//! Windows, inotify on Linux) to watch the config file's parent
//! directory. We watch the directory rather than the file because some
//! editors save atomically (`write tmp` + `rename`), which can show up
//! as a delete+create pair on the file itself and be missed by a
//! file-level watcher.
//!
//! Behavior:
//! - **Debounce** — multiple events for the same file in a 200 ms
//!   window collapse into a single reload.
//! - **Skip self-writes** — when the in-app Add/Remove buttons write
//!   the file, the watcher would normally fire and reload. We set a
//!   "skip next" flag before writing; the first event after a write
//!   is ignored. The flag auto-clears after 1 s in case the OS doesn't
//!   surface the event.
//! - **Coalesce** — if the user is hammering the file (e.g. running a
//!   `for` loop that touches it), we cap at one reload per 500 ms.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use notify::{event::ModifyKind, Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter, Manager};

use crate::config;
use crate::events::{EVT_CONFIG_RELOADED, VibeConfigReloadedPayload};

/// Atomic flag set by `add_runner` / `remove_runner` to suppress the
/// next file change event (which is our own write).
static SKIP_NEXT_CHANGE: AtomicBool = AtomicBool::new(false);

/// Mark the next file-change event as "ours, ignore". Auto-clears
/// after 1 second in case the OS doesn't surface the event.
pub fn skip_next_change() {
    SKIP_NEXT_CHANGE.store(true, Ordering::SeqCst);
    let _ = thread::Builder::new()
        .name("viberunner-skip-clear".into())
        .spawn(|| {
            thread::sleep(Duration::from_secs(1));
            SKIP_NEXT_CHANGE.store(false, Ordering::SeqCst);
        });
}

/// Spawn the file watcher. Returns immediately; the watcher runs in a
/// background thread for the lifetime of the app.
pub fn spawn(app: AppHandle, path: PathBuf) {
    thread::Builder::new()
        .name("viberunner-config-watcher".into())
        .spawn(move || {
            if let Err(e) = run(app, path) {
                eprintln!("[viberunner] file watcher exited: {e}");
            }
        })
        .expect("failed to spawn file watcher thread");
}

fn run(app: AppHandle, path: PathBuf) -> Result<(), String> {
    let (tx, rx) = channel::<notify::Result<Event>>();

    let mut watcher = RecommendedWatcher::new(tx, Config::default())
        .map_err(|e| format!("create watcher: {e}"))?;

    // Watch the parent directory (not the file) so atomic-rename
    // saves are seen as the rename event.
    let watch_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    watcher
        .watch(&watch_dir, RecursiveMode::NonRecursive)
        .map_err(|e| format!("watch {}: {e}", watch_dir.display()))?;

    // Initial event — the parent dir exists by definition (we found a
    // file in it), so the watcher is now live.

    let mut last_action = Instant::now() - Duration::from_secs(1);
    let debounce = Duration::from_millis(200);
    let min_interval = Duration::from_millis(500);
    let mut pending = false;

    loop {
        // recv_timeout so we can also enforce the min-interval throttle.
        let event = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(ev)) => Some(ev),
            Ok(Err(e)) => {
                eprintln!("[viberunner] watcher error: {e}");
                None
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
            Err(e) => {
                eprintln!("[viberunner] watcher channel: {e}");
                return Err(format!("channel: {e}"));
            }
        };

        if let Some(ev) = event {
            if is_relevant(&ev, &path) {
                // Sleep for `debounce` to let a burst settle. If a fresh
                // event arrives during the wait, the next recv_timeout
                // iteration picks it up and we sleep again.
                thread::sleep(debounce);
                // Drain anything that arrived during the wait.
                while rx.try_recv().is_ok() {}
                pending = true;
            }
        }

        if !pending {
            continue;
        }

        // Throttle: don't fire more than once per min_interval.
        if last_action.elapsed() < min_interval {
            continue;
        }

        pending = false;

        // Skip flag — set by our own write paths.
        if SKIP_NEXT_CHANGE.swap(false, Ordering::SeqCst) {
            continue;
        }

        last_action = Instant::now();

        // Reload from disk and notify the frontend. Reload failures
        // are non-fatal (the file might be mid-write); we log them
        // and wait for the next change.
        match config::load_from_path(&path) {
            Ok(cfg) => {
                let resolved = config::resolve_all(&cfg);
                if let Some(state) = app.try_state::<crate::state::AppState>() {
                    state.set_config(cfg.clone());
                    state.set_config_path(path.clone());
                }
                let _ = app.emit(
                    EVT_CONFIG_RELOADED,
                    VibeConfigReloadedPayload {
                        config: Arc::new(cfg),
                        projects: resolved,
                        path: path.to_string_lossy().into_owned(),
                    },
                );
            }
            Err(e) => {
                eprintln!("[viberunner] watcher reload failed: {e}");
            }
        }
    }
}

/// `true` if the event touches our config file in a way that suggests
/// its contents may have changed.
fn is_relevant(event: &Event, target: &Path) -> bool {
    if !event.paths.iter().any(|p| p == target) {
        return false;
    }
    matches!(
        event.kind,
        EventKind::Create(_)
            | EventKind::Modify(ModifyKind::Data(_))
            | EventKind::Modify(ModifyKind::Any)
            | EventKind::Modify(ModifyKind::Name(_))
            | EventKind::Modify(ModifyKind::Metadata(_))
    )
}
