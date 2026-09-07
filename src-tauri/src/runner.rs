//! Per-runner runtime state.
//!
//! Each runner the user starts gets a [`RunnerHandle`] stored in
//! `AppState.runners` keyed by id. The handle owns the child process's
//! I/O handles and tracks its current status.

use std::collections::HashSet;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use parking_lot::Mutex;
use portable_pty::{Child, ChildKiller, MasterPty, PtySize};
use tauri::async_runtime::JoinHandle;

/// Trait-object type we keep in [`RunnerHandle`]. Re-exported so
/// call sites in `commands.rs` don't have to spell out the full
/// `Box<dyn portable_pty::Child + Send + Sync>` every time.
pub type ChildBox = Box<dyn Child + Send + Sync>;

/// Lifecycle state of a runner. Mirrors the `RunnerStatus` type in
/// `src/types.ts` — keep them in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
}

impl Status {
    /// True when the runner has a live process or is in the middle of a
    /// transition. Clicks on Start are no-ops while this is true.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Status::Starting | Status::Running | Status::Stopping
        )
    }
}

pub struct RestartGuard {
    handle: Arc<RunnerHandle>,
}

impl Drop for RestartGuard {
    fn drop(&mut self) {
        self.handle.release_restart();
    }
}

pub struct RunnerHandle {
    pub id: String,

    status: Mutex<Status>,
    pid: Mutex<Option<u32>>,
    tracked_pids: Mutex<HashSet<u32>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    killer: Mutex<Option<Box<dyn ChildKiller + Send + Sync>>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    /// Kept around so we can `wait()` on it; clearing it from a separate
    /// thread is fine because the wait happens once.
    child: Mutex<Option<Box<dyn Child + Send + Sync>>>,
    /// Number of consecutive auto-restarts since the last clean start.
    /// Reset to 0 on a successful spawn.
    restart_count: AtomicU32,
    /// Generation counter identifying the current run. Incremented each time
    /// a new process is spawned to invalidate stale background tasks.
    generation: AtomicU64,
    /// Operation guard preventing concurrent restart sequences on the same project.
    restart_in_progress: AtomicBool,
    /// Cancellation token for the active restart sequence. Tripped on Stop.
    restart_cancel: Mutex<Option<Arc<AtomicBool>>>,
    /// Active shutdown script task (if any) being executed by `stop_project_internal`.
    shutdown_task: Mutex<Option<JoinHandle<()>>>,
}


impl RunnerHandle {
    pub fn new(id: String) -> Self {
        Self {
            id,
            status: Mutex::new(Status::Stopped),
            pid: Mutex::new(None),
            tracked_pids: Mutex::new(HashSet::new()),
            writer: Mutex::new(None),
            killer: Mutex::new(None),
            master: Mutex::new(None),
            child: Mutex::new(None),
            restart_count: AtomicU32::new(0),
            generation: AtomicU64::new(0),
            restart_in_progress: AtomicBool::new(false),
            restart_cancel: Mutex::new(None),
            shutdown_task: Mutex::new(None),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn bump_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn status(&self) -> Status {
        *self.status.lock()
    }

    pub fn set_status(&self, s: Status) {
        *self.status.lock() = s;
    }

    pub fn is_active(&self) -> bool {
        self.status().is_active()
    }

    pub fn pid(&self) -> Option<u32> {
        *self.pid.lock()
    }

    pub fn tracked_pids(&self) -> Vec<u32> {
        self.tracked_pids.lock().iter().copied().collect()
    }

    pub fn add_tracked_pids(&self, pids: impl IntoIterator<Item = u32>) {
        let mut set = self.tracked_pids.lock();
        for p in pids {
            set.insert(p);
        }
    }

    /// Atomically replace the I/O handles. Used by `start_runner` after
    /// a successful spawn.
    pub fn install(
        &self,
        pid: Option<u32>,
        writer: Box<dyn Write + Send>,
        killer: Box<dyn ChildKiller + Send + Sync>,
        child: Box<dyn Child + Send + Sync>,
        master: Box<dyn MasterPty + Send>,
    ) {
        *self.pid.lock() = pid;
        {
            let mut tracked = self.tracked_pids.lock();
            tracked.clear();
            if let Some(p) = pid {
                tracked.insert(p);
            }
        }
        *self.writer.lock() = Some(writer);
        *self.killer.lock() = Some(killer);
        *self.child.lock() = Some(child);
        *self.master.lock() = Some(master);
    }

    /// Resize the active PTY window dimensions.
    pub fn resize_pty(&self, rows: u16, cols: u16) -> Result<(), String> {
        if let Some(master) = self.master.lock().as_mut() {
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| format!("resize pty failed: {e}"))?;
        }
        Ok(())
    }

    /// Take the writer so we can send keystrokes during stop. If the
    /// writer has already been taken (e.g. stop is racing itself), this
    /// returns None and the caller can skip the keystroke step.
    pub fn take_writer(&self) -> Option<Box<dyn Write + Send>> {
        self.writer.lock().take()
    }

    /// Take the child so the waiter thread can call `.wait()` without
    /// holding a lock across a blocking call.
    pub fn take_child(&self) -> Option<Box<dyn Child + Send + Sync>> {
        self.child.lock().take()
    }

    /// Clear all runtime handles — used after the process has exited.
    pub fn clear_runtime(&self) {
        *self.pid.lock() = None;
        self.tracked_pids.lock().clear();
        *self.writer.lock() = None;
        *self.killer.lock() = None;
        *self.master.lock() = None;
        *self.child.lock() = None;
    }


    /// Write `data` into the PTY (keystrokes from the user). Returns
    /// `Err` if the runner is not running or the write fails.
    pub fn send_input(&self, data: &[u8]) -> Result<(), String> {
        if !self.is_active() {
            return Err(format!("runner '{}' is not running", self.id));
        }
        let mut writer = self
            .writer
            .lock()
            .take()
            .ok_or_else(|| format!("runner '{}' has no writer", self.id))?;
        writer
            .write_all(data)
            .map_err(|e| format!("write failed: {e}"))?;
        writer.flush().map_err(|e| format!("flush failed: {e}"))?;
        // Re-install so subsequent writes still work.
        *self.writer.lock() = Some(writer);
        Ok(())
    }

    // ---- restart counter -------------------------------------------------

    /// Reset to 0. Called on a fresh user-initiated start (so a new
    /// manual start doesn't inherit the previous crash's retry count).
    pub fn reset_restart_count(&self) {
        self.restart_count.store(0, Ordering::SeqCst);
    }

    /// Increment and return the new value. Called from the waiter thread
    /// when a crash happens and we decide to auto-restart.
    pub fn bump_restart_count(&self) -> u32 {
        self.restart_count.fetch_add(1, Ordering::SeqCst) + 1
    }

    // ---- restart operation guard & cancellation --------------------------

    /// Attempt to acquire the restart lock. Returns an RAII `RestartGuard`
    /// that releases the lock on drop, or `None` if a restart is already active.
    pub fn acquire_restart_guard(self: &Arc<Self>) -> Option<RestartGuard> {
        if self
            .restart_in_progress
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Some(RestartGuard {
                handle: self.clone(),
            })
        } else {
            None
        }
    }

    pub fn release_restart(&self) {
        self.restart_in_progress.store(false, Ordering::SeqCst);
        *self.restart_cancel.lock() = None;
    }

    /// Set up a new cancellation token for an upcoming restart.
    pub fn create_restart_cancellation_token(&self) -> Arc<AtomicBool> {
        let token = Arc::new(AtomicBool::new(false));
        *self.restart_cancel.lock() = Some(token.clone());
        token
    }

    /// Signal cancellation to any running restart sequence.
    pub fn cancel_restart(&self) {
        if let Some(token) = self.restart_cancel.lock().as_ref() {
            token.store(true, Ordering::SeqCst);
        }
    }

    #[allow(dead_code)]
    pub fn is_restart_cancelled(&self) -> bool {
        self.restart_cancel
            .lock()
            .as_ref()
            .map(|t| t.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    // ---- shutdown script tracking ----------------------------------------

    pub fn set_shutdown_task(&self, task: JoinHandle<()>) {
        *self.shutdown_task.lock() = Some(task);
    }

    pub fn take_shutdown_task(&self) -> Option<JoinHandle<()>> {
        self.shutdown_task.lock().take()
    }
}

/// Convenience: `Arc<RunnerHandle>` is what we hand out of `AppState` so
/// the same handle can be shared across the start command, the stop
/// command, the spawned reader/waiter threads, and event emitters.
pub type SharedRunner = Arc<RunnerHandle>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_guard_prevents_concurrency_and_releases_on_drop() {
        let handle = Arc::new(RunnerHandle::new("test-project".into()));

        let guard1 = handle.acquire_restart_guard();
        assert!(guard1.is_some(), "First restart guard should be acquired");

        let guard2 = handle.acquire_restart_guard();
        assert!(guard2.is_none(), "Concurrent restart guard should be rejected");

        drop(guard1);

        let guard3 = handle.acquire_restart_guard();
        assert!(guard3.is_some(), "Restart guard should be acquirable after previous dropped");
    }

    #[test]
    fn restart_cancellation_token_trips_on_cancel() {
        let handle = Arc::new(RunnerHandle::new("test-project".into()));
        let token = handle.create_restart_cancellation_token();

        assert!(!token.load(Ordering::SeqCst));
        assert!(!handle.is_restart_cancelled());

        handle.cancel_restart();

        assert!(token.load(Ordering::SeqCst));
        assert!(handle.is_restart_cancelled());
    }
}
