//! Per-runner runtime state.
//!
//! Each project the user starts gets a [`RunnerHandle`] stored in
//! `AppState` keyed by id. The handle owns the child process's I/O
//! handles, the identity-checked process tree, and the current status.
//!
//! ## Status ownership
//!
//! Exactly one party finalizes a run:
//! - the run's monitor thread, when the process exits on its own
//!   (→ Stopped / Crashed), or
//! - the stop supervisor, once the user asked to stop (→ Stopped).
//!
//! Both go through [`RunnerHandle::finish`], which checks the run
//! generation and the current status under one lock, so a late waiter
//! can never overwrite a newer run's state.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use portable_pty::{Child, ChildKiller, MasterPty, PtySize};
use tauri::async_runtime::JoinHandle;

use crate::process::ProcessTree;

/// Trait-object type we keep in [`RunnerHandle`].
pub type ChildBox = Box<dyn Child + Send + Sync>;

/// Lifecycle state of a runner. Mirrors the `ProjectStatus` type in
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
    /// transition. Clicks on Start are rejected while this is true.
    pub fn is_active(self) -> bool {
        matches!(self, Status::Starting | Status::Running | Status::Stopping)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Stopped => "stopped",
            Status::Starting => "starting",
            Status::Running => "running",
            Status::Stopping => "stopping",
            Status::Crashed => "crashed",
        }
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

#[derive(Debug, Clone, Default)]
struct Meta {
    action: Option<String>,
    reason: Option<String>,
    started_at_ms: Option<u64>,
}

pub struct RunnerHandle {
    pub id: String,

    status: Mutex<Status>,
    meta: Mutex<Meta>,
    tree: Mutex<ProcessTree>,
    /// When the current run was spawned. PID files / log files older
    /// than this are ignored.
    run_started: Mutex<Option<SystemTime>>,
    /// Last emitted listening ports (for rehydration).
    ports: Mutex<Vec<u16>>,
    /// Ports mentioned as URLs in the run's output, in first-seen order.
    port_hints: Mutex<Vec<u16>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    killer: Mutex<Option<Box<dyn ChildKiller + Send + Sync>>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    child: Mutex<Option<ChildBox>>,
    restart_count: AtomicU32,
    /// Incremented on every spawn; background tasks capture it and bail
    /// once it changes.
    generation: AtomicU64,
    restart_in_progress: AtomicBool,
    restart_cancel: Mutex<Option<Arc<AtomicBool>>>,
    shutdown_task: Mutex<Option<JoinHandle<()>>>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl RunnerHandle {
    pub fn new(id: String) -> Self {
        Self {
            id,
            status: Mutex::new(Status::Stopped),
            meta: Mutex::new(Meta::default()),
            tree: Mutex::new(ProcessTree::new()),
            run_started: Mutex::new(None),
            ports: Mutex::new(Vec::new()),
            port_hints: Mutex::new(Vec::new()),
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

    // ---- generation / status ---------------------------------------------

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

    /// Atomically claim the runner for a new run: succeeds only from an
    /// idle state (Stopped / Crashed). Prevents two clicks or a click
    /// racing an auto-restart from spawning twice.
    pub fn try_begin(&self, action: &str) -> bool {
        let mut st = self.status.lock();
        if st.is_active() {
            return false;
        }
        *st = Status::Starting;
        let mut m = self.meta.lock();
        m.action = Some(action.to_string());
        m.reason = None;
        m.started_at_ms = Some(now_ms());
        true
    }

    /// Finalize the run `gen` into `to`. Returns false (and changes
    /// nothing) if a newer run exists, or if the run is stopping and the
    /// caller is not the stop supervisor (`allow_from_stopping`).
    pub fn finish(&self, gen: u64, to: Status, reason: Option<String>, allow_from_stopping: bool) -> bool {
        let mut st = self.status.lock();
        if self.generation() != gen {
            return false;
        }
        let ok = match *st {
            Status::Running | Status::Starting => true,
            Status::Stopping => allow_from_stopping,
            Status::Crashed => allow_from_stopping && to == Status::Stopped,
            Status::Stopped => false,
        };
        if !ok {
            return false;
        }
        *st = to;
        self.meta.lock().reason = reason;
        true
    }

    /// Mark the run as failed before/at spawn (no generation yet).
    pub fn fail_start(&self, reason: &str) {
        *self.status.lock() = Status::Crashed;
        self.meta.lock().reason = Some(reason.to_string());
    }

    pub fn action(&self) -> Option<String> {
        self.meta.lock().action.clone()
    }

    pub fn reason(&self) -> Option<String> {
        self.meta.lock().reason.clone()
    }

    pub fn started_at_ms(&self) -> Option<u64> {
        self.meta.lock().started_at_ms
    }

    // ---- process tree ------------------------------------------------------

    pub fn run_started(&self) -> Option<SystemTime> {
        *self.run_started.lock()
    }

    /// Run `f` with the process tree locked.
    pub fn with_tree<R>(&self, f: impl FnOnce(&mut ProcessTree) -> R) -> R {
        f(&mut self.tree.lock())
    }

    // ---- ports ---------------------------------------------------------------

    pub fn ports(&self) -> Vec<u16> {
        self.ports.lock().clone()
    }

    pub fn set_ports(&self, ports: Vec<u16>) {
        *self.ports.lock() = ports;
    }

    pub fn port_hints(&self) -> Vec<u16> {
        self.port_hints.lock().clone()
    }

    pub fn add_port_hints(&self, hints: &[u16]) {
        let mut h = self.port_hints.lock();
        for &p in hints {
            if !h.contains(&p) && h.len() < 16 {
                h.push(p);
            }
        }
    }

    // ---- I/O handles -------------------------------------------------------

    /// Atomically replace the I/O handles after a successful spawn and
    /// start a fresh process tree rooted at `pid`.
    pub fn install(
        &self,
        pid: Option<u32>,
        writer: Box<dyn Write + Send>,
        killer: Box<dyn ChildKiller + Send + Sync>,
        child: ChildBox,
        master: Box<dyn MasterPty + Send>,
    ) {
        self.tree.lock().reset(pid);
        *self.run_started.lock() = Some(SystemTime::now());
        self.ports.lock().clear();
        self.port_hints.lock().clear();
        *self.writer.lock() = Some(writer);
        *self.killer.lock() = Some(killer);
        *self.child.lock() = Some(child);
        *self.master.lock() = Some(master);
    }

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

    pub fn take_child(&self) -> Option<ChildBox> {
        self.child.lock().take()
    }

    /// Drop the PTY handles once the process has exited. The process
    /// tree is kept so a Crashed run's leftovers can still be cleaned up
    /// by Stop; it is cleared on the next spawn or a clean stop.
    pub fn clear_io(&self) {
        *self.writer.lock() = None;
        *self.killer.lock() = None;
        *self.master.lock() = None;
        *self.child.lock() = None;
        self.ports.lock().clear();
    }

    /// Write raw bytes into the PTY regardless of status (used by the
    /// stop path for keystrokes).
    pub fn write_raw(&self, data: &[u8]) -> Result<(), String> {
        let mut guard = self.writer.lock();
        let writer = guard
            .as_mut()
            .ok_or_else(|| format!("runner '{}' has no writer", self.id))?;
        writer
            .write_all(data)
            .map_err(|e| format!("write failed: {e}"))?;
        writer.flush().map_err(|e| format!("flush failed: {e}"))
    }

    /// Write user keystrokes into the PTY.
    pub fn send_input(&self, data: &[u8]) -> Result<(), String> {
        if !self.is_active() {
            return Err(format!("runner '{}' is not running", self.id));
        }
        self.write_raw(data)
    }

    // ---- restart counter -------------------------------------------------

    pub fn reset_restart_count(&self) {
        self.restart_count.store(0, Ordering::SeqCst);
    }

    pub fn bump_restart_count(&self) -> u32 {
        self.restart_count.fetch_add(1, Ordering::SeqCst) + 1
    }

    // ---- restart operation guard & cancellation --------------------------

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

    pub fn create_restart_cancellation_token(&self) -> Arc<AtomicBool> {
        let token = Arc::new(AtomicBool::new(false));
        *self.restart_cancel.lock() = Some(token.clone());
        token
    }

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

/// `Arc<RunnerHandle>` is what we hand out of `AppState` so the same
/// handle can be shared across commands, threads, and emitters.
pub type SharedRunner = Arc<RunnerHandle>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_guard_prevents_concurrency_and_releases_on_drop() {
        let handle = Arc::new(RunnerHandle::new("test-project".into()));
        let guard1 = handle.acquire_restart_guard();
        assert!(guard1.is_some());
        assert!(handle.acquire_restart_guard().is_none());
        drop(guard1);
        assert!(handle.acquire_restart_guard().is_some());
    }

    #[test]
    fn restart_cancellation_token_trips_on_cancel() {
        let handle = Arc::new(RunnerHandle::new("test-project".into()));
        let token = handle.create_restart_cancellation_token();
        assert!(!token.load(Ordering::SeqCst));
        handle.cancel_restart();
        assert!(token.load(Ordering::SeqCst));
        assert!(handle.is_restart_cancelled());
    }

    #[test]
    fn status_is_active_categorizes_states() {
        assert!(Status::Running.is_active());
        assert!(Status::Starting.is_active());
        assert!(Status::Stopping.is_active());
        assert!(!Status::Stopped.is_active());
        assert!(!Status::Crashed.is_active());
    }

    #[test]
    fn try_begin_is_exclusive() {
        let h = RunnerHandle::new("x".into());
        assert!(h.try_begin("Run"));
        assert!(!h.try_begin("Run"));
        assert_eq!(h.status(), Status::Starting);
        assert_eq!(h.action().as_deref(), Some("Run"));
    }

    #[test]
    fn finish_respects_generation_and_stop_ownership() {
        let h = RunnerHandle::new("x".into());
        assert!(h.try_begin("Run"));
        let gen = h.bump_generation();
        h.set_status(Status::Running);

        // Stale generation is ignored.
        assert!(!h.finish(gen - 1, Status::Crashed, None, false));
        assert_eq!(h.status(), Status::Running);

        // While stopping, only the supervisor may finalize.
        h.set_status(Status::Stopping);
        assert!(!h.finish(gen, Status::Crashed, None, false));
        assert!(h.finish(gen, Status::Stopped, None, true));
        assert_eq!(h.status(), Status::Stopped);

        // Already final: no double transition.
        assert!(!h.finish(gen, Status::Stopped, None, true));
    }
}
