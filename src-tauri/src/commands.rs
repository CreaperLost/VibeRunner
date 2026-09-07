//! Tauri commands callable from the React frontend via `invoke()`.
//!
//! Two families:
//! - **config / projects** — read or mutate `vibe.config.json`, list
//!   the resolved projects (with auto-discovered TOML actions merged
//!   in).
//! - **project lifecycle** — run an action, stop the current PTY,
//!   run setup, restart, send keystrokes.

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::config::{
    self, ProjectConfig, ResolvedAction, ResolvedProject, RestartPolicy, VibeConfig,
};
use crate::events::{
    EVT_CONFIG_RELOADED, EVT_OUTPUT, EVT_PORTS, EVT_RESTARTING, EVT_STATUS, OutputPayload,
    PortsPayload, RestartingPayload, StatusPayload, VibeConfigReloadedPayload,
};
use crate::ports;
use crate::process::{parse_keystrokes, GRACE_AFTER_KEYSTROKE, GRACE_AFTER_SIGTERM};
use crate::pty;
use crate::runner::{SharedRunner, Status};
use crate::state::AppState;
use crate::watcher;


// =============================================================================
// config / project commands
// =============================================================================

#[derive(Debug, Serialize)]
pub struct VibeConfigPath {
    pub path: String,
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> VibeConfig {
    state.config()
}

#[tauri::command]
pub fn get_config_path(state: State<'_, AppState>) -> VibeConfigPath {
    VibeConfigPath {
        path: state
            .config_path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

/// Return the resolved projects (TOML + manual merged). The sidebar
/// renders from this list — `get_config` only returns the raw config
/// for the editor.
#[tauri::command]
pub fn list_projects(state: State<'_, AppState>) -> Vec<ResolvedProject> {
    config::resolve_all(&state.config())
}

/// Reload `vibe.config.json` from disk and emit `config:reloaded`
/// with both the raw config and the resolved projects.
#[tauri::command]
pub async fn reload_config(app: AppHandle) -> Result<VibeConfig, String> {
    let path = config::reload_into_state(&app)
        .await
        .map_err(|e| e.to_string())?;
    let state = app.state::<AppState>();
    state.set_config_path(path.clone());
    let cfg = state.config();
    let resolved = config::resolve_all(&cfg);
    let _ = app.emit(
        EVT_CONFIG_RELOADED,
        VibeConfigReloadedPayload {
            config: std::sync::Arc::new(cfg.clone()),
            projects: resolved,
            path: path.to_string_lossy().into_owned(),
        },
    );
    Ok(cfg)
}

/// Add a new project. Reads the current config from disk, appends
/// the project, writes atomically, and reloads into app state.
/// Returns the new config on success.
#[tauri::command]
pub async fn add_project(
    project: ProjectConfig,
    app: AppHandle,
) -> Result<VibeConfigReloadedPayload, String> {
    let state = app.state::<AppState>();
    let _lock = state.config_lock().lock().await;

    let path = state
        .config_path()
        .ok_or_else(|| "config path not set — no config loaded".to_string())?;
    let path_for_io = path.clone();

    let result: Result<VibeConfig, String> =
        tauri::async_runtime::spawn_blocking(move || -> Result<VibeConfig, String> {
            let mut cfg = config::load_from_path(&path_for_io)
                .map_err(|e| format!("read config: {e}"))?;
            cfg.add_project(project)
                .map_err(|e| format!("add: {e}"))?;
            watcher::skip_next_change();
            config::write_to_path(&path_for_io, &cfg).map_err(|e| format!("write: {e}"))?;
            Ok(cfg)
        })
        .await
        .map_err(|e| format!("blocking task: {e}"))?;

    let cfg = result?;
    state.set_config(cfg.clone());
    let resolved = config::resolve_all(&cfg);
    let payload = VibeConfigReloadedPayload {
        config: std::sync::Arc::new(cfg),
        projects: resolved,
        path: path.to_string_lossy().into_owned(),
    };
    let _ = app.emit(EVT_CONFIG_RELOADED, payload.clone());
    Ok(payload)
}

/// Remove a project by id. If a process for the removed id is still
/// running, we stop it first so we don't leak a PTY. The handle is
/// then forgotten.
#[tauri::command]
pub async fn remove_project(
    id: String,
    app: AppHandle,
) -> Result<VibeConfigReloadedPayload, String> {
    let state = app.state::<AppState>();
    let _lock = state.config_lock().lock().await;

    // Stop any running process for this id (no-op if not running).
    let _ = stop_project_internal(&id, &app);

    let path = state
        .config_path()
        .ok_or_else(|| "config path not set — no config loaded".to_string())?;
    let path_for_io = path.clone();
    let id_for_blocking = id.clone();

    let result: Result<VibeConfig, String> =
        tauri::async_runtime::spawn_blocking(move || -> Result<VibeConfig, String> {
            let mut cfg = config::load_from_path(&path_for_io)
                .map_err(|e| format!("read config: {e}"))?;
            // Ignore not-found error so re-clicks or idempotent calls don't error
            let _ = cfg.remove_project(&id_for_blocking);
            watcher::skip_next_change();
            config::write_to_path(&path_for_io, &cfg).map_err(|e| format!("write: {e}"))?;
            Ok(cfg)
        })
        .await
        .map_err(|e| format!("blocking task: {e}"))?;

    let cfg = result?;
    state.set_config(cfg.clone());
    state.forget(&id);
    let resolved = config::resolve_all(&cfg);
    let payload = VibeConfigReloadedPayload {
        config: std::sync::Arc::new(cfg),
        projects: resolved,
        path: path.to_string_lossy().into_owned(),
    };
    let _ = app.emit(EVT_CONFIG_RELOADED, payload.clone());
    Ok(payload)
}

// =============================================================================
// project lifecycle commands
// =============================================================================

/// Run a named action in a fresh PTY. The action must exist in the
/// resolved project. If a PTY is already alive for this project, the
/// click is rejected (the user can Stop first).
#[tauri::command]
pub fn run_action(project_id: String, action_name: String, app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let resolved = config::resolve_all(&state.config());
    let project = resolved
        .iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| format!("project not found: {project_id}"))?
        .clone();
    let action = find_action(&project, &action_name)
        .ok_or_else(|| format!("action '{action_name}' not found in project '{project_id}'"))?
        .clone();

    let handle = state.get_or_create(&project_id);
    if handle.is_active() {
        if action.name.eq_ignore_ascii_case("stop") || action.icon.as_deref() == Some("stop") {
            stop_project_internal(&project_id, &app);
            return Ok(());
        }
        return Err(format!(
            "project '{project_id}' is already running — stop it first"
        ));
    }

    handle.reset_restart_count();
    emit_status(&app, &project_id, Status::Starting, Some(&action_name), None);

    spawn_action(&app, &project, &action, handle)
}

fn is_pure_keystroke(s: &str) -> bool {
    let t = s.trim();
    matches!(
        t,
        "^C" | "Ctrl-C" | "Ctrl+C"
            | "^D" | "Ctrl-D" | "Ctrl+D"
            | "^Z" | "Ctrl-Z" | "Ctrl+Z"
            | "^\\" | "Ctrl-\\" | "Ctrl+\\"
            | "^?" | "Ctrl-?" | "Ctrl+?"
    )
}

/// Run the project's implicit "Setup" action (if any). Returns Err
/// if the project has no setup.
#[tauri::command]
pub fn setup_project(project_id: String, app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let resolved = config::resolve_all(&state.config());
    let project = resolved
        .iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| format!("project not found: {project_id}"))?
        .clone();
    let setup = project
        .actions
        .iter()
        .find(|a| a.name == "Setup")
        .cloned()
        .ok_or_else(|| format!("project '{project_id}' has no setup action"))?;

    let handle = state.get_or_create(&project_id);
    if handle.is_active() {
        return Err(format!(
            "project '{project_id}' is already running — stop it first"
        ));
    }
    handle.reset_restart_count();
    emit_status(&app, &project_id, Status::Starting, Some("Setup"), None);
    spawn_action(&app, &project, &setup, handle)
}

/// Stop the project's current PTY (keystroke → SIGTERM → SIGKILL).
/// No-op if the project isn't running.
#[tauri::command]
pub fn stop_project(project_id: String, app: AppHandle) -> Result<(), String> {
    stop_project_internal(&project_id, &app);
    Ok(())
}

/// Restart = stop current → setup → run primary action. The sequence
/// is sequential; each step waits for the previous to settle.
#[tauri::command]
pub fn restart_project(project_id: String, app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let resolved = config::resolve_all(&state.config());
    let project = resolved
        .iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| format!("project not found: {project_id}"))?
        .clone();
    let primary = project
        .primary_action
        .clone()
        .or_else(|| project.actions.first().map(|a| a.name.clone()))
        .ok_or_else(|| format!("project '{project_id}' has no actions to restart"))?;
    let action = find_action(&project, &primary)
        .ok_or_else(|| format!("primary action '{primary}' not found"))?
        .clone();
    let setup = project
        .actions
        .iter()
        .find(|a| a.name == "Setup")
        .cloned();

    let handle = state.get_or_create(&project_id);
    let restart_guard = handle
        .acquire_restart_guard()
        .ok_or_else(|| format!("project '{project_id}' restart is already in progress"))?;
    let cancel_token = handle.create_restart_cancellation_token();

    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        let _guard = restart_guard;
        let handle = app_clone
            .state::<AppState>()
            .get_or_create(&project_id);

        // Step 1: stop whatever is running and await full shutdown.
        if handle.is_active() {
            stop_project_internal(&project_id, &app_clone);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while handle.is_active() && tokio::time::Instant::now() < deadline {
                if cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if handle.is_active() {
                let pids = handle.tracked_pids();
                crate::process::kill_pids(&pids, true);
                let force_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
                while handle.is_active() && tokio::time::Instant::now() < force_deadline {
                    if cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }

        // Await completion of any configured shutdown script so it cannot
        // kill the replacement app.
        if let Some(shutdown_task) = handle.take_shutdown_task() {
            let _ = tokio::time::timeout(Duration::from_secs(10), shutdown_task).await;
        }

        if cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }

        // Step 2: setup (if any).
        if let Some(setup_action) = setup {
            if cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            handle.reset_restart_count();
            emit_status(
                &app_clone,
                &project_id,
                Status::Starting,
                Some("Setup"),
                None,
            );
            if let Err(e) = spawn_action(&app_clone, &project, &setup_action, handle.clone()) {
                emit_status(
                    &app_clone,
                    &project_id,
                    Status::Crashed,
                    Some("Setup"),
                    Some(&e),
                );
                eprintln!("[viberunner] restart setup failed for {project_id}: {e}");
                return;
            }
            // Wait for the setup PTY to exit. Cap at 5 minutes.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
            while handle.is_active() && tokio::time::Instant::now() < deadline {
                if cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            // Require successful setup completion.
            if handle.is_active() {
                stop_project_internal(&project_id, &app_clone);
                emit_status(
                    &app_clone,
                    &project_id,
                    Status::Crashed,
                    Some("Setup"),
                    Some("setup timed out"),
                );
                eprintln!("[viberunner] restart setup timed out for {project_id}");
                return;
            }
            if handle.status() != Status::Stopped || cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
                if !cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
                    emit_status(
                        &app_clone,
                        &project_id,
                        Status::Crashed,
                        Some("Setup"),
                        Some("setup failed"),
                    );
                    eprintln!("[viberunner] restart setup failed for {project_id}");
                }
                return;
            }
        }

        if cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }

        // Step 3: primary action.
        handle.reset_restart_count();
        emit_status(
            &app_clone,
            &project_id,
            Status::Starting,
            Some(&action.name),
            None,
        );
        if let Err(e) = spawn_action(&app_clone, &project, &action, handle) {
            emit_status(
                &app_clone,
                &project_id,
                Status::Crashed,
                Some(&action.name),
                Some(&e),
            );
            eprintln!("[viberunner] restart run failed for {project_id}: {e}");
        }
    });

    Ok(())
}

#[tauri::command]
pub fn write_to_pty(project_id: String, data: String, app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let Some(handle) = state.get(&project_id) else {
        return Err(format!("project not running: {project_id}"));
    };
    handle.send_input(data.as_bytes())
}

#[tauri::command]
pub fn resize_pty(
    project_id: String,
    rows: u16,
    cols: u16,
    app: AppHandle,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    if let Some(handle) = state.get(&project_id) {
        handle.resize_pty(rows, cols)
    } else {
        Ok(())
    }
}


/// Reveal a path in the system file manager (Finder on macOS,
/// Explorer on Windows, the desktop file manager on Linux).
///
/// We use `std::process::Command::new("open")` directly instead
/// of `tauri-plugin-opener` so we can surface the real OS error
/// in the UI — the plugin flattens some macOS errors (notably
/// sandbox-related ACL denials) into generic strings, which is
/// frustrating when you want to know *why* the open failed.
///
/// On macOS this is the same `/usr/bin/open <path>` that the
/// `open` crate (and the system `open` shell command) use. If
/// this still fails, the issue is at the OS level — typically a
/// `com.apple.macl` xattr on the target folder or a missing
/// grant in the user's TCC database. The error message bubbles
/// up unchanged so the user (or the app's error banner) can see
/// it.
#[tauri::command]
pub fn reveal_in_finder(path: String, app: AppHandle) -> Result<(), String> {
    let _ = app; // not used yet, but available for future per-app state
    let target = std::path::PathBuf::from(&path);

    if !target.exists() {
        return Err(format!("path does not exist: {path}"));
    }

    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("/usr/bin/open");
        cmd.arg(&target);
        match cmd.spawn() {
            Ok(mut child) => {
                // `open` returns immediately but writes any error
                // to stderr. We don't want to block, so we let
                // it run detached — if `open` itself failed to
                // even start (e.g. bad path), spawn() returns
                // Err. The actual UI outcome (Finder opening,
                // error dialog) is handled by macOS.
                let _ = child.wait();
                Ok(())
            }
            Err(e) => Err(format!("open failed for {path}: {e}")),
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("explorer");
        cmd.arg(&target);
        match cmd.spawn() {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("explorer failed for {path}: {e}")),
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Try xdg-open first, fall back to a few common ones.
        for opener in &["xdg-open", "gio", "gnome-open", "kde-open5"] {
            if let Ok(mut child) = std::process::Command::new(opener)
                .arg(&target)
                .spawn()
            {
                let _ = child.wait();
                return Ok(());
            }
        }
        Err(format!("no file manager found (tried xdg-open, gio, gnome-open, kde-open5) for {path}"))
    }
}

/// Open a file or folder using the OS default handler.
#[tauri::command]
pub fn open_path(path: String, app: AppHandle) -> Result<(), String> {
    reveal_in_finder(path, app)
}

/// Open a URL in the user's default web browser.
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("/usr/bin/open");
        cmd.arg(&url);
        match cmd.spawn() {
            Ok(mut child) => {
                let _ = child.wait();
                Ok(())
            }
            Err(e) => Err(format!("open failed for {url}: {e}")),
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", "start", "", &url]);
        match cmd.spawn() {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("start failed for {url}: {e}")),
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for opener in &["xdg-open", "gio", "gnome-open", "kde-open5"] {
            if let Ok(mut child) = std::process::Command::new(opener)
                .arg(&url)
                .spawn()
            {
                let _ = child.wait();
                return Ok(());
            }
        }
        Err(format!("no browser launcher found for {url}"))
    }
}

// =============================================================================
// updater (unchanged)
// =============================================================================

#[derive(Debug, Serialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub notes: Option<String>,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<UpdateInfo, String> {
    use tauri_plugin_updater::UpdaterExt;

    let current = app.package_info().version.to_string();

    match app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
    {
        Ok(Some(update)) => Ok(UpdateInfo {
            available: true,
            current_version: current,
            latest_version: Some(update.version.to_string()),
            notes: update.body.clone(),
            error: None,
        }),
        Ok(None) => Ok(UpdateInfo {
            available: false,
            current_version: current,
            latest_version: None,
            notes: None,
            error: None,
        }),
        Err(e) => Ok(UpdateInfo {
            available: false,
            current_version: current,
            latest_version: None,
            notes: None,
            error: Some(e.to_string()),
        }),
    }
}

// =============================================================================
// helpers
// =============================================================================

/// Look up an action by name in a resolved project. Case-insensitive
/// fallback for user convenience.
fn find_action<'a>(project: &'a ResolvedProject, name: &str) -> Option<&'a ResolvedAction> {
    if let Some(a) = project.actions.iter().find(|a| a.name == name) {
        return Some(a);
    }
    let lower = name.to_ascii_lowercase();
    project
        .actions
        .iter()
        .find(|a| a.name.to_ascii_lowercase() == lower)
}

fn emit_status(
    app: &AppHandle,
    id: &str,
    status: Status,
    action: Option<&str>,
    reason: Option<&str>,
) {
    let _ = app.emit(
        EVT_STATUS,
        StatusPayload {
            id: id.to_string(),
            status: status_to_str(status).to_string(),
            action: action.map(|s| s.to_string()),
            reason: reason.map(|s| s.to_string()),
        },
    );
}

fn status_to_str(s: Status) -> &'static str {
    match s {
        Status::Stopped => "stopped",
        Status::Starting => "starting",
        Status::Running => "running",
        Status::Stopping => "stopping",
        Status::Crashed => "crashed",
    }
}

/// Stop the project's current PTY. Used by both the `stop_project`
/// command and the restart sequence. Synchronous from the caller's
/// POV: we set the status to Stopping and kick off the escalation
/// in the background.
fn stop_project_internal(project_id: &str, app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(handle) = state.get(project_id) else {
        return;
    };
    handle.cancel_restart();
    if !handle.is_active() {
        return;
    }

    let resolved = config::resolve_all(&state.config());
    let project = resolved.iter().find(|p| p.id == project_id).cloned();

    handle.set_status(Status::Stopping);
    emit_status(app, project_id, Status::Stopping, None, None);

    let stop_action = project.as_ref().and_then(|proj| {
        proj.actions
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case("stop") || a.icon.as_deref() == Some("stop"))
            .cloned()
    });

    if let Some(stop_act) = &stop_action {
        if is_pure_keystroke(&stop_act.command) {
            if let Some(mut writer) = handle.take_writer() {
                let bytes = parse_keystrokes(&stop_act.command);
                let _ = writer.write_all(&bytes);
                let _ = writer.flush();
            }
        } else {
            // Execute the configured shutdown script.
            if let Some(proj) = &project {
                let project_path = std::path::PathBuf::from(&proj.path);
                #[cfg(unix)]
                let mut cmd_obj = std::process::Command::new("sh");
                #[cfg(unix)]
                cmd_obj.arg("-c").arg(&stop_act.command);

                #[cfg(windows)]
                let mut cmd_obj = std::process::Command::new("cmd");
                #[cfg(windows)]
                cmd_obj.arg("/C").arg(&stop_act.command);

                cmd_obj.current_dir(&project_path);
                let full_env = pty::load_project_env(&project_path, &proj.env);
                for (k, v) in full_env {
                    cmd_obj.env(k, v);
                }
                match cmd_obj.spawn() {
                    Ok(mut child) => {
                        let task = tauri::async_runtime::spawn(async move {
                            let _ = tokio::task::spawn_blocking(move || {
                                let _ = child.wait();
                            })
                            .await;
                        });
                        handle.set_shutdown_task(task);
                    }
                    Err(e) => {
                        eprintln!("[viberunner] shutdown script spawn failed for {project_id}: {e}");
                    }
                }
            }
            // Also send Ctrl+C to writer if available.
            if let Some(mut writer) = handle.take_writer() {
                let bytes = parse_keystrokes("Ctrl+C");
                let _ = writer.write_all(&bytes);
                let _ = writer.flush();
            }
        }
    } else {
        // Step 1: send Ctrl+C into the PTY (graceful default).
        if let Some(mut writer) = handle.take_writer() {
            let bytes = parse_keystrokes("Ctrl+C");
            let _ = writer.write_all(&bytes);
            let _ = writer.flush();
        }
    }

    let gen = handle.generation();
    let mut target_pids = handle.tracked_pids();
    if let Some(proj) = &project {
        let ppath = std::path::PathBuf::from(&proj.path);
        for p in crate::process::discover_pid_files(&ppath) {
            if !target_pids.contains(&p) {
                target_pids.push(p);
            }
        }
    }
    if target_pids.is_empty() {
        if let Some(p) = handle.pid() {
            target_pids.push(p);
        }
    }
    spawn_escalation(handle, gen, target_pids);
}

/// Spawn a single action's command in a PTY and wire up the output /
/// waiter / port poller threads.
fn spawn_action(
    app: &AppHandle,
    project: &ResolvedProject,
    action: &ResolvedAction,
    handle: SharedRunner,
) -> Result<(), String> {
    let id = project.id.clone();
    let project_path = std::path::PathBuf::from(&project.path);

    if !project_path.exists() {
        let msg = format!("project path does not exist: {}", project.path);
        handle.set_status(Status::Crashed);
        emit_status(app, &id, Status::Crashed, Some(&action.name), Some(&msg));
        return Err(msg);
    }

    let pty = match pty::spawn(&action.command, &project_path, &project.env) {
        Ok(p) => p,
        Err(e) => {
            handle.set_status(Status::Crashed);
            emit_status(app, &id, Status::Crashed, Some(&action.name), Some(&e));
            return Err(e);
        }
    };

    let pid = pty.child.process_id();
    handle.install(pid, pty.writer, pty.child.clone_killer(), pty.child, pty.master);
    let gen = handle.bump_generation();

    handle.set_status(Status::Running);
    emit_status(app, &id, Status::Running, Some(&action.name), None);

    spawn_output_thread(app.clone(), id.clone(), pty.reader);
    spawn_waiter_thread(
        app.clone(),
        handle.clone(),
        id.clone(),
        action.name.clone(),
        project.auto_restart.clone(),
        action.detached,
        gen,
        project_path.clone(),
    );
    spawn_port_poller(
        app.clone(),
        handle.clone(),
        id.clone(),
        project_path,
        action.detached,
        gen,
    );

    Ok(())
}

fn spawn_output_thread<R: std::io::Read + Send + 'static>(
    app: AppHandle,
    id: String,
    mut reader: R,
) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut known_ports = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = buf[..n].to_vec();
                    let _ = app.emit(
                        EVT_OUTPUT,
                        OutputPayload {
                            id: id.clone(),
                            chunk,
                        },
                    );

                    // Scan terminal output for ports (e.g. http://localhost:8501 or http://localhost:5173)
                    let text = String::from_utf8_lossy(&buf[..n]);
                    let detected = crate::ports::extract_ports_from_text(&text);
                    if !detected.is_empty() {
                        let mut changed = false;
                        for p in detected {
                            if !known_ports.contains(&p) {
                                known_ports.push(p);
                                changed = true;
                            }
                        }
                        if changed {
                            let _ = app.emit(
                                EVT_PORTS,
                                PortsPayload {
                                    id: id.clone(),
                                    ports: known_ports.clone(),
                                },
                            );
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });
}


fn spawn_waiter_thread(
    app: AppHandle,
    handle: SharedRunner,
    id: String,
    action_name: String,
    auto_restart: Option<RestartPolicy>,
    detached: bool,
    gen: u64,
    project_path: std::path::PathBuf,
) {
    let Some(child) = handle.take_child() else {
        return;
    };
    let root_pid = handle.pid();
    std::thread::spawn(move || {
        if detached {
            detached_waiter(app, handle, id, action_name, auto_restart, root_pid, child, gen, project_path);
        } else {
            direct_waiter(app, handle, id, action_name, auto_restart, root_pid, child, gen, project_path);
        }
    });
}

/// Standard waiter: blocks on the PTY's direct child. Used for
/// long-running PTY commands (`npm run dev`, `python3 -m http.server`,
/// etc.) that don't detach.
fn direct_waiter(
    app: AppHandle,
    handle: SharedRunner,
    id: String,
    action_name: String,
    auto_restart: Option<RestartPolicy>,
    root_pid: Option<u32>,
    mut child: crate::runner::ChildBox,
    gen: u64,
    project_path: std::path::PathBuf,
) {
    let status = child.wait();
    let exit_code = status.as_ref().ok().map(|s| s.exit_code() as i32);
    let success = status.as_ref().map(|s| s.success()).unwrap_or(false);

    if handle.generation() != gen {
        // Run was superseded by a newer run. Do not mutate runtime state or emit events.
        return;
    }

    let was_stopping = handle.status() == Status::Stopping;
    if was_stopping {
        let mut tracked: std::collections::HashSet<u32> =
            handle.tracked_pids().into_iter().collect();
        let root = root_pid.unwrap_or(0);
        if root != 0 {
            tracked.insert(root);
        }
        let poll_interval = Duration::from_millis(100);
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while crate::process::update_and_check_alive(&mut tracked, root) {
            handle.add_tracked_pids(tracked.iter().copied());
            if std::time::Instant::now() >= deadline {
                crate::process::kill_pids(&handle.tracked_pids(), true);
                break;
            }
            std::thread::sleep(poll_interval);
            if handle.generation() != gen {
                return;
            }
        }
    } else if success {
        // Direct launcher script exited with code 0.
        // Check if it spawned a background daemon or detached process (nohup ... &, PID files, etc.).
        std::thread::sleep(Duration::from_millis(150));

        let root = root_pid.unwrap_or(0);
        let mut tracked: std::collections::HashSet<u32> =
            handle.tracked_pids().into_iter().collect();
        if root != 0 {
            tracked.insert(root);
        }
        let any_alive = crate::process::update_and_check_alive(&mut tracked, root);
        let pid_files = crate::process::discover_pid_files(&project_path);
        for p in &pid_files {
            tracked.insert(*p);
        }

        if any_alive || !pid_files.is_empty() {
            // A background process or PID file is still active; keep project Running.
            handle.add_tracked_pids(tracked.iter().copied());

            if let Some(recent_log) = find_recent_log_file(&project_path) {
                spawn_log_tailer(app.clone(), handle.clone(), id.clone(), recent_log, gen);
            }

            detached_waiter_loop(
                app,
                handle,
                id,
                action_name,
                auto_restart,
                root_pid,
                tracked,
                gen,
                project_path,
            );
            return;
        }

    }

    handle.clear_runtime();

    if success || was_stopping {
        handle.set_status(Status::Stopped);
        let _ = app.emit(
            EVT_STATUS,
            StatusPayload {
                id: id.clone(),
                status: "stopped".into(),
                action: Some(action_name.clone()),
                reason: None,
            },
        );
    } else {
        handle.set_status(Status::Crashed);
        let _ = app.emit(
            EVT_STATUS,
            StatusPayload {
                id: id.clone(),
                status: "crashed".into(),
                action: Some(action_name.clone()),
                reason: exit_code.map(|c| format!("exit {c}")),
            },
        );
        if let Some(policy) = auto_restart {
            if policy.enabled {
                maybe_auto_restart(&app, &handle, &id, &action_name, &policy);
            }
        }
    }
}

/// Detached waiter: ignores the PTY's direct child (which usually
/// exits within milliseconds because the launcher script detached
/// and returned). Instead polls the process tree rooted at the
/// PTY's PID — the project stays "Running" as long as any
/// descendant is alive.
///
/// We still wait on the child to reap it (so we don't leak a zombie),
/// but the lifecycle decision comes from the tree poll.
fn detached_waiter(
    app: AppHandle,
    handle: SharedRunner,
    id: String,
    action_name: String,
    auto_restart: Option<RestartPolicy>,
    root_pid: Option<u32>,
    mut child: crate::runner::ChildBox,
    gen: u64,
    project_path: std::path::PathBuf,
) {
    // Reap the direct child in another thread so it doesn't block us.
    std::thread::spawn(move || {
        let _ = child.wait();
    });

    let Some(root) = root_pid else {
        if handle.generation() == gen {
            handle.clear_runtime();
            handle.set_status(Status::Crashed);
            let _ = app.emit(
                EVT_STATUS,
                StatusPayload {
                    id: id.clone(),
                    status: "crashed".into(),
                    action: Some(action_name.clone()),
                    reason: Some("no PID recorded".into()),
                },
            );
        }
        return;
    };

    let mut tracked = std::collections::HashSet::new();
    tracked.insert(root);
    crate::process::update_and_check_alive(&mut tracked, root);
    for p in crate::process::discover_pid_files(&project_path) {
        tracked.insert(p);
    }
    handle.add_tracked_pids(tracked.iter().copied());

    if let Some(recent_log) = find_recent_log_file(&project_path) {
        spawn_log_tailer(app.clone(), handle.clone(), id.clone(), recent_log, gen);
    }

    detached_waiter_loop(

        app,
        handle,
        id,
        action_name,
        auto_restart,
        root_pid,
        tracked,
        gen,
        project_path,
    );
}

fn detached_waiter_loop(
    app: AppHandle,
    handle: SharedRunner,
    id: String,
    action_name: String,
    auto_restart: Option<RestartPolicy>,
    root_pid: Option<u32>,
    mut tracked: std::collections::HashSet<u32>,
    gen: u64,
    project_path: std::path::PathBuf,
) {
    use std::time::Duration;
    use crate::process::update_and_check_alive;

    let root = root_pid.unwrap_or(0);
    let poll_interval = Duration::from_millis(500);
    loop {
        std::thread::sleep(poll_interval);

        if handle.generation() != gen {
            return;
        }

        let is_alive = update_and_check_alive(&mut tracked, root);
        for p in crate::process::discover_pid_files(&project_path) {
            tracked.insert(p);
        }
        handle.add_tracked_pids(tracked.iter().copied());

        let mut has_live_proc = is_alive;
        if !has_live_proc {
            let mut sys = sysinfo::System::new();
            sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            has_live_proc = tracked.iter().any(|&p| sys.process(sysinfo::Pid::from_u32(p)).is_some());
        }

        if handle.status() == Status::Stopping {
            if !has_live_proc {
                handle.clear_runtime();
                handle.set_status(Status::Stopped);
                let _ = app.emit(
                    EVT_STATUS,
                    StatusPayload {
                        id: id.clone(),
                        status: "stopped".into(),
                        action: Some(action_name.clone()),
                        reason: None,
                    },
                );
                return;
            }
            continue;
        }

        if !handle.is_active() {
            return;
        }

        if !has_live_proc {
            if handle.generation() != gen {
                return;
            }
            let was_stopping = handle.status() == Status::Stopping;
            handle.clear_runtime();

            if was_stopping {
                handle.set_status(Status::Stopped);
                let _ = app.emit(
                    EVT_STATUS,
                    StatusPayload {
                        id: id.clone(),
                        status: "stopped".into(),
                        action: Some(action_name.clone()),
                        reason: None,
                    },
                );
            } else {
                handle.set_status(Status::Crashed);
                let _ = app.emit(
                    EVT_STATUS,
                    StatusPayload {
                        id: id.clone(),
                        status: "crashed".into(),
                        action: Some(action_name.clone()),
                        reason: Some("detached process exited".into()),
                    },
                );
                if let Some(policy) = auto_restart {
                    if policy.enabled {
                        maybe_auto_restart(&app, &handle, &id, &action_name, &policy);
                    }
                }
            }
            return;
        }
    }
}

fn maybe_auto_restart(
    app: &AppHandle,
    handle: &SharedRunner,
    id: &str,
    action_name: &str,
    policy: &RestartPolicy,
) {
    let attempt = handle.bump_restart_count();
    if attempt > policy.max_retries {
        return;
    }
    let scheduled_gen = handle.generation();
    let app = app.clone();
    let id = id.to_string();
    let action_name = action_name.to_string();
    let policy = policy.clone();
    let handle = handle.clone();
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
            return;
        }
        // Re-resolve the project and find the action to re-run.
        let state = app.state::<AppState>();
        let resolved = config::resolve_all(&state.config());
        let Some(project) = resolved.iter().find(|p| p.id == id).cloned() else {
            return;
        };
        let Some(action) = project
            .actions
            .iter()
            .find(|a| a.name == action_name)
            .cloned()
        else {
            return;
        };
        // Counter is not reset here, preventing infinite crash-restart loops.
        emit_status(&app, &id, Status::Starting, Some(&action_name), None);
        if let Err(e) = spawn_action(&app, &project, &action, handle.clone()) {
            emit_status(&app, &id, Status::Crashed, Some(&action_name), Some(&e));
            eprintln!("[viberunner] auto-restart failed for {id}: {e}");
            return;
        }

        // Recovery condition: if this spawn runs stably for 10 seconds, reset retry counter.
        let h = handle.clone();
        let current_gen = handle.generation();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            if h.generation() == current_gen && h.status() == Status::Running {
                h.reset_restart_count();
            }
        });
    });
}

fn find_recent_log_file(project_path: &std::path::Path) -> Option<std::path::PathBuf> {
    let candidates = [
        project_path.join("data"),
        project_path.join("logs"),
        project_path.join("log"),
        project_path.join(".codex"),
        project_path.to_path_buf(),
    ];

    let mut newest_file: Option<(std::path::PathBuf, std::time::SystemTime)> = None;

    for dir in candidates {
        if !dir.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name.ends_with(".log") || name.ends_with(".out") {
                        if let Ok(meta) = entry.metadata() {
                            if let Ok(mtime) = meta.modified() {
                                if let Ok(elapsed) = mtime.elapsed() {
                                    // Modified within the last 15 minutes
                                    if elapsed < std::time::Duration::from_secs(900) {
                                        if newest_file.as_ref().map(|(_, t)| *t < mtime).unwrap_or(true) {
                                            newest_file = Some((path, mtime));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    newest_file.map(|(p, _)| p)
}

fn spawn_log_tailer(
    app: AppHandle,
    handle: SharedRunner,
    id: String,
    log_path: std::path::PathBuf,
    gen: u64,
) {
    std::thread::spawn(move || {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = match std::fs::File::open(&log_path) {
            Ok(f) => f,
            Err(_) => return,
        };

        // Read up to recent 64KB on start so previous lines in the log file show up in the logger
        let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let start_pos = file_len.saturating_sub(65536);
        let _ = file.seek(SeekFrom::Start(start_pos));

        let mut buf = [0u8; 4096];
        let mut known_ports: Vec<u16> = Vec::new();
        while handle.generation() == gen && handle.is_active() {
            match file.read(&mut buf) {
                Ok(0) => {
                    std::thread::sleep(Duration::from_millis(250));
                }
                Ok(n) => {
                    let chunk = buf[..n].to_vec();
                    let _ = app.emit(
                        EVT_OUTPUT,
                        OutputPayload {
                            id: id.clone(),
                            chunk,
                        },
                    );
                    let text = String::from_utf8_lossy(&buf[..n]);
                    let detected = crate::ports::extract_ports_from_text(&text);
                    if !detected.is_empty() {
                        let mut changed = false;
                        for p in detected {
                            if !known_ports.contains(&p) {
                                known_ports.push(p);
                                changed = true;
                            }
                        }
                        if changed {
                            let _ = app.emit(
                                EVT_PORTS,
                                PortsPayload {
                                    id: id.clone(),
                                    ports: known_ports.clone(),
                                },
                            );
                        }
                    }
                }
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    });
}

fn spawn_port_poller(
    app: AppHandle,
    handle: SharedRunner,
    id: String,
    project_path: std::path::PathBuf,
    _detached: bool,
    gen: u64,
) {
    tauri::async_runtime::spawn(async move {
        let mut last_ports: Vec<u16> = Vec::new();
        let startup_intervals = [150, 300, 500, 1000];
        let mut step = 0;

        loop {
            let delay_ms = if step < startup_intervals.len() {
                let d = startup_intervals[step];
                step += 1;
                d
            } else {
                1500
            };
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;

            if handle.generation() != gen || !handle.is_active() {
                break;
            }

            let mut pids = handle.tracked_pids();
            if let Some(pid) = handle.pid() {
                if !pids.contains(&pid) {
                    pids.push(pid);
                }
            }
            for p in crate::process::discover_pid_files(&project_path) {
                if !pids.contains(&p) {
                    pids.push(p);
                }
            }
            if pids.is_empty() {
                continue;
            }

            // Update tracked PIDs with newly discovered descendants (Bug 5)
            let mut tracked: std::collections::HashSet<u32> = pids.into_iter().collect();
            let root = handle.pid().unwrap_or(0);
            crate::process::update_and_check_alive(&mut tracked, root);
            handle.add_tracked_pids(tracked.iter().copied());

            let pids_vec: Vec<u32> = tracked.into_iter().collect();
            let mut ports = ports::detect_ports_for_pids(&pids_vec);

            // Also check recently modified log files for any output URLs/ports
            if ports.is_empty() {
                if let Some(recent_log) = find_recent_log_file(&project_path) {
                    if let Ok(content) = std::fs::read_to_string(&recent_log) {
                        for p in crate::ports::extract_ports_from_text(&content) {
                            if !ports.contains(&p) {
                                ports.push(p);
                            }
                        }
                    }
                }
            }

            if ports != last_ports {
                let _ = app.emit(
                    EVT_PORTS,
                    PortsPayload {
                        id: id.clone(),
                        ports: ports.clone(),
                    },
                );
                last_ports = ports;
            }
        }


        // On shutdown / poller exit, emit empty port list so dead ports don't linger (Bug 6)
        if !last_ports.is_empty() && handle.generation() == gen {
            let _ = app.emit(
                EVT_PORTS,
                PortsPayload {
                    id: id.clone(),
                    ports: Vec::new(),
                },
            );
        }
    });
}

fn spawn_escalation(handle: SharedRunner, gen: u64, target_pids: Vec<u32>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(GRACE_AFTER_KEYSTROKE).await;
        if handle.generation() != gen || !handle.is_active() {
            return;
        }
        crate::process::kill_pids(&target_pids, false);

        tokio::time::sleep(GRACE_AFTER_SIGTERM).await;
        if handle.generation() != gen || !handle.is_active() {
            return;
        }
        crate::process::kill_pids(&target_pids, true);
    });
}
