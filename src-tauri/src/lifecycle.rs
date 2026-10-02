//! Run lifecycle: spawn an action in a PTY, monitor it, stop it, poll
//! its ports, and auto-restart it.
//!
//! ## Who decides the final status
//!
//! - **Monitor thread** (one per run): waits on the PTY child. When the
//!   process exits on its own it finalizes the run as Stopped (exit 0)
//!   or Crashed (non-zero). Detached actions — and launchers that exit 0
//!   but leave a daemon behind that listens on a port or wrote a fresh
//!   PID file — switch to a background loop that polls the process tree.
//! - **Stop supervisor** (one per Stop click): owns the run from the
//!   moment it is Stopping. It escalates keystroke → terminate → kill and
//!   finalizes Stopped as soon as the tree is observed dead, or after a
//!   hard deadline — never "Stopping…" forever.
//!
//! Every final transition goes through `RunnerHandle::finish`, which
//! rejects stale generations, so the two can't trample each other.
//!
//! Processes are tracked with identity (pid + start time) in
//! `process::ProcessTree`; nothing here matches processes by path.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::config::{self, ResolvedAction, ResolvedProject, RestartPolicy};
use crate::events::{
    OutputPayload, PortsPayload, RestartingPayload, StatusPayload, StatusSnapshot, EVT_OUTPUT,
    EVT_PORTS, EVT_RESTARTING, EVT_STATUS,
};
use crate::ports;
use crate::process::{
    self, is_pure_keystroke, parse_keystrokes, GRACE_AFTER_KEYSTROKE, GRACE_AFTER_SIGKILL,
    GRACE_AFTER_SIGTERM,
};
use crate::pty;
use crate::runner::{ChildBox, RunnerHandle, SharedRunner, Status};
use crate::state::AppState;

// =============================================================================
// events
// =============================================================================

fn status_payload(handle: &RunnerHandle) -> StatusPayload {
    let status = handle.status();
    StatusPayload {
        id: handle.id.clone(),
        status: status.as_str().to_string(),
        action: handle.action(),
        reason: handle.reason(),
        started_at_ms: if status.is_active() {
            handle.started_at_ms()
        } else {
            None
        },
    }
}

/// Emit the handle's current status.
pub fn emit_status<R: Runtime>(app: &AppHandle<R>, handle: &RunnerHandle) {
    let _ = app.emit(EVT_STATUS, status_payload(handle));
}

fn emit_ports<R: Runtime>(app: &AppHandle<R>, id: &str, ports: Vec<u16>) {
    let _ = app.emit(
        EVT_PORTS,
        PortsPayload {
            id: id.to_string(),
            ports,
        },
    );
}

fn emit_output<R: Runtime>(app: &AppHandle<R>, id: &str, chunk: Vec<u8>) {
    let _ = app.emit(
        EVT_OUTPUT,
        OutputPayload {
            id: id.to_string(),
            chunk,
        },
    );
}

/// Full runtime view of every known project (for UI rehydration).
pub fn snapshots<R: Runtime>(app: &AppHandle<R>) -> Vec<StatusSnapshot> {
    app.state::<AppState>()
        .all()
        .iter()
        .map(|h| StatusSnapshot {
            status: status_payload(h),
            ports: h.ports(),
        })
        .collect()
}

/// Finalize run `gen`. Clears PTY handles and ports; a clean Stopped
/// also forgets the process tree, a Crashed run keeps it so Stop
/// (Cleanup) can still kill leftovers.
fn finish_run<R: Runtime>(
    app: &AppHandle<R>,
    handle: &RunnerHandle,
    gen: u64,
    to: Status,
    reason: Option<String>,
    from_stop: bool,
) -> bool {
    if !handle.finish(gen, to, reason, from_stop) {
        return false;
    }
    handle.clear_io();
    if to == Status::Stopped {
        handle.with_tree(|t| t.clear());
    }
    emit_status(app, handle);
    emit_ports(app, &handle.id, Vec::new());
    true
}

/// Refresh the run's process tree (adopting fresh PID files) and report
/// whether anything it owns is alive.
fn refresh_tree(handle: &RunnerHandle, project_path: &Path) -> bool {
    let since = handle.run_started();
    handle.with_tree(|t| {
        let sys = process::snapshot();
        if let Some(since) = since {
            let since_secs = process::to_secs(since);
            for pid in process::fresh_pid_files(project_path, since) {
                t.adopt_external(&sys, pid, since_secs);
            }
        }
        t.refresh(&sys)
    })
}

// =============================================================================
// spawn
// =============================================================================

/// Spawn `action` in a fresh PTY and wire up the output / monitor /
/// port-poller threads. The caller must have claimed the runner with
/// `try_begin` / `continue_with` (status Starting).
pub fn spawn_action<R: Runtime>(
    app: &AppHandle<R>,
    project: &ResolvedProject,
    action: &ResolvedAction,
    handle: SharedRunner,
) -> Result<(), String> {
    let project_path = PathBuf::from(&project.path);

    if !project_path.exists() {
        let msg = format!("project path does not exist: {}", project.path);
        handle.fail_start(&msg);
        emit_status(app, &handle);
        return Err(msg);
    }

    let pty = match pty::spawn(&action.command, &project_path, &project.env) {
        Ok(p) => p,
        Err(e) => {
            handle.fail_start(&e);
            emit_status(app, &handle);
            return Err(e);
        }
    };

    let pid = pty.child.process_id();
    let killer = pty.child.clone_killer();
    handle.install(pid, pty.writer, killer, pty.child, pty.master);
    let gen = handle.bump_generation();

    handle.set_status(Status::Running);
    emit_status(app, &handle);
    emit_ports(app, &project.id, Vec::new());

    spawn_output_thread(app.clone(), handle.clone(), gen, pty.reader);
    if let Some(child) = handle.take_child() {
        let (app, handle, project, action) =
            (app.clone(), handle.clone(), project.clone(), action.clone());
        std::thread::spawn(move || monitor(app, handle, gen, project, action, child));
    }
    spawn_port_poller(app.clone(), handle, gen, project_path);
    Ok(())
}

/// Stream PTY output to the UI and collect URL port hints from it.
fn spawn_output_thread<R: Runtime, T: Read + Send + 'static>(
    app: AppHandle<R>,
    handle: SharedRunner,
    gen: u64,
    mut reader: T,
) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        // Partial last line from the previous chunk, so a URL split
        // across two reads is still recognized.
        let mut tail = String::new();
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            emit_output(&app, &handle.id, buf[..n].to_vec());
            if handle.generation() != gen {
                continue;
            }

            let text = format!("{tail}{}", String::from_utf8_lossy(&buf[..n]));
            let hints = ports::extract_ports_from_text(&text);
            if !hints.is_empty() {
                handle.add_port_hints(&hints);
            }

            // `cmd /C foo.cmd` answers Ctrl+C with "Terminate batch job
            // (Y/N)?" and waits forever. While stopping, answer it.
            #[cfg(windows)]
            if handle.status() == Status::Stopping && text.contains("(Y/N)") {
                let _ = handle.write_raw(b"Y\r\n");
            }

            tail = match text.rfind('\n') {
                Some(i) => text[i + 1..].to_string(),
                None => text,
            };
            if tail.len() > 512 {
                let mut cut = tail.len() - 512;
                while !tail.is_char_boundary(cut) {
                    cut += 1;
                }
                tail = tail[cut..].to_string();
            }
        }
    });
}

// =============================================================================
// monitoring
// =============================================================================

fn monitor<R: Runtime>(
    app: AppHandle<R>,
    handle: SharedRunner,
    gen: u64,
    project: ResolvedProject,
    action: ResolvedAction,
    mut child: ChildBox,
) {
    let exit = child.wait();
    let success = exit.as_ref().map(|s| s.success()).unwrap_or(false);
    let code = exit.as_ref().ok().map(|s| s.exit_code());

    // Superseded, or the stop supervisor owns the run now.
    if handle.generation() != gen || !matches!(handle.status(), Status::Running | Status::Starting) {
        return;
    }

    let project_path = PathBuf::from(&project.path);
    if action.detached || (success && daemon_survived(&handle, &project_path)) {
        if let Some(log) = recent_log_file(&project_path, &handle) {
            spawn_log_tailer(app.clone(), handle.clone(), gen, log);
        }
        background_loop(&app, &handle, gen, &project, &action, &project_path);
        return;
    }

    if success {
        finish_run(&app, &handle, gen, Status::Stopped, None, false);
    } else {
        let reason = code.map(|c| format!("exit {c}"));
        if finish_run(&app, &handle, gen, Status::Crashed, reason, false) {
            maybe_auto_restart(&app, &handle, &project.id, &action.name, project.auto_restart.as_ref());
        }
    }
}

/// A launcher exited 0. Did it leave a daemon behind? Only counts if a
/// surviving process of ours listens on a TCP port or wrote a fresh PID
/// file — leftover toolchain helpers (e.g. MSVC's `mspdbsrv.exe`) do
/// neither, so a finished Build goes to Stopped instead of hanging in
/// "Running".
fn daemon_survived(handle: &RunnerHandle, project_path: &Path) -> bool {
    std::thread::sleep(Duration::from_millis(400));
    if !refresh_tree(handle, project_path) {
        return false;
    }
    let has_pid_file = handle
        .run_started()
        .map(|since| !process::fresh_pid_files(project_path, since).is_empty())
        .unwrap_or(false);
    has_pid_file || !ports::listening_ports(&handle.with_tree(|t| t.pids())).is_empty()
}

fn background_loop<R: Runtime>(
    app: &AppHandle<R>,
    handle: &SharedRunner,
    gen: u64,
    project: &ResolvedProject,
    action: &ResolvedAction,
    project_path: &Path,
) {
    loop {
        std::thread::sleep(Duration::from_millis(1000));
        if handle.generation() != gen || handle.status() != Status::Running {
            return; // superseded or stopping (supervisor finalizes)
        }
        if !refresh_tree(handle, project_path) {
            if finish_run(
                app,
                handle,
                gen,
                Status::Crashed,
                Some("background process exited".into()),
                false,
            ) {
                maybe_auto_restart(app, handle, &project.id, &action.name, project.auto_restart.as_ref());
            }
            return;
        }
    }
}

/// The newest `*.log` / `*.out` written since this run started, in the
/// usual places. Only used for background runs, whose output no longer
/// flows through the PTY.
fn recent_log_file(project_path: &Path, handle: &RunnerHandle) -> Option<PathBuf> {
    let since = handle.run_started()?;
    let dirs = [
        project_path.join("logs"),
        project_path.join("log"),
        project_path.join("data"),
        project_path.join(".codex"),
        project_path.to_path_buf(),
    ];
    let mut newest: Option<(PathBuf, std::time::SystemTime)> = None;
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !(name.ends_with(".log") || name.ends_with(".out")) {
                continue;
            }
            let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) else { continue };
            if mtime >= since && newest.as_ref().map(|(_, t)| *t < mtime).unwrap_or(true) {
                newest = Some((path, mtime));
            }
        }
    }
    newest.map(|(p, _)| p)
}

fn spawn_log_tailer<R: Runtime>(app: AppHandle<R>, handle: SharedRunner, gen: u64, log_path: PathBuf) {
    std::thread::spawn(move || {
        use std::io::{Seek, SeekFrom};
        let Ok(mut file) = std::fs::File::open(&log_path) else { return };
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let _ = file.seek(SeekFrom::Start(len.saturating_sub(65536)));
        let mut buf = [0u8; 8192];
        while handle.generation() == gen && handle.is_active() {
            match file.read(&mut buf) {
                Ok(0) | Err(_) => std::thread::sleep(Duration::from_millis(300)),
                Ok(n) => {
                    emit_output(&app, &handle.id, buf[..n].to_vec());
                    handle.add_port_hints(&ports::extract_ports_from_text(
                        &String::from_utf8_lossy(&buf[..n]),
                    ));
                }
            }
        }
    });
}

// =============================================================================
// ports
// =============================================================================

/// Poll the run's listening ports. Single source of truth for
/// `project:ports`: ports owned by the tracked tree, plus URL hints from
/// the output that are actually accepting connections. Hints come
/// first, in the order the app printed them, so the UI's "primary" port
/// is the URL the app announced.
fn spawn_port_poller<R: Runtime>(app: AppHandle<R>, handle: SharedRunner, gen: u64, project_path: PathBuf) {
    std::thread::spawn(move || {
        let mut last: Vec<u16> = Vec::new();
        let startup = [300u64, 600, 1000];
        let mut step = 0;
        loop {
            let delay = startup.get(step).copied().unwrap_or(1500);
            step += 1;
            std::thread::sleep(Duration::from_millis(delay));

            if handle.generation() != gen || !handle.is_active() {
                break;
            }
            if handle.status() == Status::Stopping {
                continue;
            }

            refresh_tree(&handle, &project_path);
            let pids = handle.with_tree(|t| t.pids());
            let mut owned = ports::listening_ports(&pids);
            let mut out: Vec<u16> = handle
                .port_hints()
                .into_iter()
                .filter(|h| owned.contains(h) || ports::is_listening_local(*h))
                .collect();
            owned.sort_unstable();
            for p in owned {
                if !out.contains(&p) {
                    out.push(p);
                }
            }

            if out != last && handle.generation() == gen {
                handle.set_ports(out.clone());
                emit_ports(&app, &handle.id, out.clone());
                last = out;
            }
        }
    });
}

// =============================================================================
// stop
// =============================================================================

/// User-facing Stop: cancels any restart sequence, then stops.
pub fn stop<R: Runtime>(app: &AppHandle<R>, project_id: &str) {
    if let Some(handle) = app.state::<AppState>().get(project_id) {
        handle.cancel_restart();
    }
    stop_inner(app, project_id);
}

/// Stop without touching the restart token (used *by* Restart).
///
/// Sets Stopping, runs the configured stop (keystroke or script) plus a
/// Ctrl+C, and hands the run to a supervisor that escalates and always
/// reaches Stopped.
fn stop_inner<R: Runtime>(app: &AppHandle<R>, project_id: &str) {
    let state = app.state::<AppState>();
    let Some(handle) = state.get(project_id) else { return };
    match handle.status() {
        Status::Stopped | Status::Stopping => return, // idempotent
        Status::Starting | Status::Running | Status::Crashed => {}
    }

    let project = config::resolve_all(&state.config())
        .into_iter()
        .find(|p| p.id == project_id);
    let project_path = project
        .as_ref()
        .map(|p| PathBuf::from(&p.path))
        .unwrap_or_default();

    handle.set_status(Status::Stopping);
    emit_status(app, &handle);

    let ctrl_c = parse_keystrokes("Ctrl+C");
    match project.as_ref().and_then(config::stop_action) {
        Some(a) if is_pure_keystroke(&a.command) => {
            let _ = handle.write_raw(&parse_keystrokes(&a.command));
        }
        Some(a) => {
            run_stop_script(app, &handle, project.as_ref().unwrap(), &a.command);
            let _ = handle.write_raw(&ctrl_c);
        }
        None => {
            let _ = handle.write_raw(&ctrl_c);
        }
    }

    let gen = handle.generation();
    let app = app.clone();
    std::thread::spawn(move || supervise_stop(app, handle, gen, project_path));
}

fn supervise_stop<R: Runtime>(app: AppHandle<R>, handle: SharedRunner, gen: u64, project_path: PathBuf) {
    let t0 = Instant::now();
    let term_at = GRACE_AFTER_KEYSTROKE;
    let kill_at = term_at + GRACE_AFTER_SIGTERM;
    let give_up_at = kill_at + GRACE_AFTER_SIGKILL;
    let (mut termed, mut killed) = (false, false);

    loop {
        std::thread::sleep(Duration::from_millis(150));
        if handle.generation() != gen || handle.status() != Status::Stopping {
            return;
        }
        if !refresh_tree(&handle, &project_path) {
            finish_run(&app, &handle, gen, Status::Stopped, None, true);
            return;
        }
        let elapsed = t0.elapsed();
        if !termed && elapsed >= term_at {
            handle.with_tree(|t| t.kill(false));
            termed = true;
        }
        if !killed && elapsed >= kill_at {
            handle.with_tree(|t| t.kill(true));
            killed = true;
        }
        if elapsed >= give_up_at {
            let left = handle.with_tree(|t| t.pids());
            eprintln!("[viberunner] {}: processes survived stop: {left:?}", handle.id);
            finish_run(
                &app,
                &handle,
                gen,
                Status::Stopped,
                Some(format!("{} process(es) did not exit", left.len())),
                true,
            );
            return;
        }
    }
}

/// Run a project's stop script outside the PTY. Its output is streamed
/// into the project's log so a failing stop script is visible.
fn run_stop_script<R: Runtime>(app: &AppHandle<R>, handle: &SharedRunner, project: &ResolvedProject, command: &str) {
    use std::process::Stdio;

    let project_path = PathBuf::from(&project.path);

    #[cfg(unix)]
    let mut cmd = {
        let mut c = std::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = process::hidden_command("cmd");
        // Pass the command line verbatim: std's argv quoting would turn
        // `-File ".\scripts\dev.ps1"` into `\".\scripts\dev.ps1\"`,
        // which cmd.exe does not understand.
        c.args(["/D", "/C"]).raw_arg(command);
        c
    };

    cmd.current_dir(&project_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in pty::command_env(&project_path, &project.env) {
        cmd.env(k, v);
    }

    emit_output(app, &handle.id, format!("\r\n\x1b[2m[viberunner] stop: {command}\x1b[0m\r\n").into_bytes());
    match cmd.spawn() {
        Ok(mut child) => {
            for stream in [
                child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
                child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
            ]
            .into_iter()
            .flatten()
            {
                let (app, id) = (app.clone(), handle.id.clone());
                std::thread::spawn(move || pipe_to_log(app, id, stream));
            }
            let task = tauri::async_runtime::spawn_blocking(move || {
                let _ = child.wait();
            });
            handle.set_shutdown_task(tauri::async_runtime::spawn(async move {
                let _ = task.await;
            }));
        }
        Err(e) => {
            emit_output(app, &handle.id, format!("[viberunner] stop script failed to start: {e}\r\n").into_bytes());
        }
    }
}

fn pipe_to_log<R: Runtime>(app: AppHandle<R>, id: String, mut stream: Box<dyn Read + Send>) {
    let mut buf = [0u8; 4096];
    while let Ok(n) = stream.read(&mut buf) {
        if n == 0 {
            break;
        }
        // Pipes give bare LF; the terminal needs CRLF.
        let text = String::from_utf8_lossy(&buf[..n]).replace("\r\n", "\n").replace('\n', "\r\n");
        emit_output(&app, &id, text.into_bytes());
    }
}

/// Wait until the runner is idle (Stopped / Crashed), up to `timeout`.
pub async fn wait_idle(handle: &RunnerHandle, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while handle.is_active() {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    true
}

/// Hard-kill everything every project owns. Called on app exit so dev
/// servers aren't orphaned (the next Start would otherwise find their
/// ports taken and bind to new ones).
pub fn kill_all<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppState>() else { return };
    for handle in state.all() {
        let status = handle.status();
        if status.is_active() || status == Status::Crashed {
            handle.with_tree(|t| {
                t.refresh(&process::snapshot());
                t.kill(true);
            });
        }
    }
}

// =============================================================================
// restart
// =============================================================================

/// Restart = stop → wait → setup (if any) → primary action. Runs in the
/// background; a user Stop cancels it at the next step boundary.
pub fn restart<R: Runtime>(app: &AppHandle<R>, project: ResolvedProject, primary: ResolvedAction) -> Result<(), String> {
    let handle = app.state::<AppState>().get_or_create(&project.id);
    let guard = handle
        .acquire_restart_guard()
        .ok_or_else(|| format!("project '{}' restart is already in progress", project.id))?;
    let cancel = handle.create_restart_cancellation_token();
    let setup = project.actions.iter().find(|a| a.name == "Setup").cloned();
    let app = app.clone();
    let cancelled = move || cancel.load(std::sync::atomic::Ordering::SeqCst);

    tauri::async_runtime::spawn(async move {
        let _guard = guard;

        // 1. Stop (without cancelling ourselves) and wait for it.
        if handle.is_active() || handle.status() == Status::Crashed {
            stop_inner(&app, &project.id);
            let budget = GRACE_AFTER_KEYSTROKE + GRACE_AFTER_SIGTERM + GRACE_AFTER_SIGKILL;
            if !wait_idle(&handle, budget + Duration::from_secs(3)).await {
                eprintln!("[viberunner] restart: {} did not stop", project.id);
                return;
            }
        }
        if let Some(task) = handle.take_shutdown_task() {
            let _ = tokio::time::timeout(Duration::from_secs(10), task).await;
        }
        if cancelled() {
            return;
        }

        // 2. Setup, which must finish cleanly.
        if let Some(setup) = setup {
            if !handle.try_begin(&setup.name) {
                return;
            }
            handle.reset_restart_count();
            emit_status(&app, &handle);
            if spawn_action(&app, &project, &setup, handle.clone()).is_err() {
                return;
            }
            if !wait_idle(&handle, Duration::from_secs(300)).await {
                stop_inner(&app, &project.id);
                return;
            }
            if cancelled() || handle.status() != Status::Stopped {
                return; // stopped by the user, or Setup crashed (already shown)
            }
        }

        // 3. Primary action.
        if !handle.try_begin(&primary.name) {
            return;
        }
        handle.reset_restart_count();
        emit_status(&app, &handle);
        let _ = spawn_action(&app, &project, &primary, handle.clone());
    });
    Ok(())
}

fn maybe_auto_restart<R: Runtime>(
    app: &AppHandle<R>,
    handle: &SharedRunner,
    project_id: &str,
    action_name: &str,
    policy: Option<&RestartPolicy>,
) {
    let Some(policy) = policy.filter(|p| p.enabled).cloned() else { return };
    let attempt = handle.bump_restart_count();
    if attempt > policy.max_retries {
        return;
    }
    let scheduled_gen = handle.generation();
    let (app, handle) = (app.clone(), handle.clone());
    let (id, action_name) = (project_id.to_string(), action_name.to_string());

    tauri::async_runtime::spawn(async move {
        let _ = app.emit(
            EVT_RESTARTING,
            RestartingPayload {
                id: id.clone(),
                attempt,
                max: policy.max_retries,
                delay_ms: policy.delay_ms,
            },
        );
        tokio::time::sleep(Duration::from_millis(policy.delay_ms)).await;
        if handle.generation() != scheduled_gen || handle.status() != Status::Crashed {
            return; // user intervened
        }
        let state = app.state::<AppState>();
        let Some(project) = config::resolve_all(&state.config()).into_iter().find(|p| p.id == id) else {
            return;
        };
        let Some(action) = project.actions.iter().find(|a| a.name == action_name).cloned() else {
            return;
        };
        if !handle.try_begin(&action.name) {
            return;
        }
        emit_status(&app, &handle);
        if spawn_action(&app, &project, &action, handle.clone()).is_err() {
            return;
        }
        // Stable for 10s → forgive previous crashes.
        let gen = handle.generation();
        tokio::time::sleep(Duration::from_secs(10)).await;
        if handle.generation() == gen && handle.status() == Status::Running {
            handle.reset_restart_count();
        }
    });
}

#[cfg(test)]
mod tests {
    //! End-to-end lifecycle tests: real processes in a real PTY, driven
    //! through Tauri's mock runtime (no window).

    use super::*;
    use crate::config::{ActionConfig, ProjectConfig, VibeConfig};
    use std::sync::atomic::{AtomicU64, Ordering};
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

    #[cfg(windows)]
    const LONG: &str = "ping -n 60 127.0.0.1 > NUL";
    #[cfg(unix)]
    const LONG: &str = "sleep 60";

    static N: AtomicU64 = AtomicU64::new(0);

    fn act(name: &str, icon: &str, command: &str) -> ActionConfig {
        ActionConfig {
            name: name.into(),
            icon: Some(icon.into()),
            command: command.into(),
            detached: false,
            platform: None,
        }
    }

    fn setup(actions: Vec<ActionConfig>) -> (tauri::App<MockRuntime>, ResolvedProject) {
        let dir = std::env::temp_dir().join(format!(
            "vr-lifecycle-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = VibeConfig {
            version: 1,
            projects: vec![ProjectConfig {
                id: "p".into(),
                path: dir.to_string_lossy().into_owned(),
                name: None,
                primary_action: None,
                manual: true,
                setup: None,
                build: None,
                actions,
                env: Default::default(),
                auto_restart: None,
            }],
        };
        let app = mock_builder()
            .manage(AppState::new())
            .build(mock_context(noop_assets()))
            .expect("mock app");
        app.state::<AppState>().set_config(cfg.clone());
        let project = config::resolve_all(&cfg).remove(0);
        (app, project)
    }

    fn start(app: &AppHandle<MockRuntime>, project: &ResolvedProject, action: &str) -> SharedRunner {
        let handle = app.state::<AppState>().get_or_create(&project.id);
        let action = project.actions.iter().find(|a| a.name == action).unwrap().clone();
        assert!(handle.try_begin(&action.name));
        spawn_action(app, project, &action, handle.clone()).expect("spawn");
        handle
    }

    fn wait_for(timeout: Duration, mut pred: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if pred() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        pred()
    }

    #[test]
    fn successful_exit_goes_to_stopped() {
        let (app, project) = setup(vec![act("Build", "build", "echo built")]);
        let handle = start(app.handle(), &project, "Build");
        assert!(
            wait_for(Duration::from_secs(10), || handle.status() == Status::Stopped),
            "build should finish as Stopped, is {:?}",
            handle.status()
        );
    }

    #[test]
    fn failing_exit_goes_to_crashed_with_reason() {
        let (app, project) = setup(vec![act("Fail", "test", "exit 3")]);
        let handle = start(app.handle(), &project, "Fail");
        assert!(wait_for(Duration::from_secs(10), || handle.status() == Status::Crashed));
        assert_eq!(handle.reason().as_deref(), Some("exit 3"));
    }

    #[test]
    fn stop_kills_the_tree_and_always_reaches_stopped() {
        let (app, project) = setup(vec![act("Run", "run", LONG)]);
        let handle = start(app.handle(), &project, "Run");
        assert_eq!(handle.status(), Status::Running);

        // Let the shell spawn its child, then snapshot what we own.
        let mut owned = Vec::new();
        assert!(wait_for(Duration::from_secs(5), || {
            owned = handle.with_tree(|t| {
                t.refresh(&process::snapshot());
                t.pids()
            });
            owned.len() >= 2
        }));

        stop(app.handle(), &project.id);
        assert_eq!(handle.status(), Status::Stopping);
        // Second click is a no-op, not a second stop sequence.
        stop(app.handle(), &project.id);

        let budget = GRACE_AFTER_KEYSTROKE + GRACE_AFTER_SIGTERM + GRACE_AFTER_SIGKILL + Duration::from_secs(2);
        assert!(
            wait_for(budget, || handle.status() == Status::Stopped),
            "stuck in {:?}",
            handle.status()
        );
        let sys = process::snapshot();
        let alive: Vec<u32> = owned
            .iter()
            .copied()
            .filter(|p| sys.process(sysinfo::Pid::from_u32(*p)).is_some_and(|pr| {
                !matches!(pr.status(), sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead)
            }))
            .collect();
        assert!(alive.is_empty(), "processes survived stop: {alive:?}");
    }

    #[test]
    fn stop_script_runs_and_run_still_stops() {
        let (app, project) = setup(vec![act("Run", "run", LONG), act("Stop", "stop", "echo stopping")]);
        let handle = start(app.handle(), &project, "Run");
        std::thread::sleep(Duration::from_millis(300));
        stop(app.handle(), &project.id);
        let budget = GRACE_AFTER_KEYSTROKE + GRACE_AFTER_SIGTERM + GRACE_AFTER_SIGKILL + Duration::from_secs(2);
        assert!(wait_for(budget, || handle.status() == Status::Stopped));
    }

    #[test]
    fn stop_after_crash_cleans_up_immediately() {
        let (app, project) = setup(vec![act("Fail", "test", "exit 1")]);
        let handle = start(app.handle(), &project, "Fail");
        assert!(wait_for(Duration::from_secs(10), || handle.status() == Status::Crashed));
        let t = Instant::now();
        stop(app.handle(), &project.id);
        assert!(wait_for(Duration::from_secs(5), || handle.status() == Status::Stopped));
        assert!(t.elapsed() < Duration::from_secs(2), "cleanup took {:?}", t.elapsed());
    }

    #[test]
    fn restart_from_running_actually_restarts() {
        // Regression: Restart used to cancel itself via Stop's
        // cancel_restart() and end up merely stopping the project.
        let (app, project) = setup(vec![act("Run", "run", LONG)]);
        let handle = start(app.handle(), &project, "Run");
        let gen_before = handle.generation();
        let primary = project.actions[0].clone();
        restart(app.handle(), project.clone(), primary).unwrap();

        let budget = GRACE_AFTER_KEYSTROKE + GRACE_AFTER_SIGTERM + GRACE_AFTER_SIGKILL + Duration::from_secs(5);
        assert!(
            wait_for(budget, || handle.generation() > gen_before && handle.status() == Status::Running),
            "restart did not bring the project back: {:?}",
            handle.status()
        );
        stop(app.handle(), &project.id);
        wait_for(budget, || handle.status() == Status::Stopped);
    }
}
