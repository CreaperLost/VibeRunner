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

use crate::artifacts;
use crate::config::{self, ProjectConfig, ResolvedAction, ResolvedProject, VibeConfig};
use crate::events::{StatusSnapshot, VibeConfigReloadedPayload, EVT_CONFIG_RELOADED};
use crate::lifecycle;
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

    // Stop any running process for this id and wait for it, so the
    // handle isn't forgotten while its processes are still alive.
    if let Some(handle) = state.get(&id) {
        lifecycle::stop(&app, &id);
        lifecycle::wait_idle(&handle, Duration::from_secs(15)).await;
    }

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

fn resolve(app: &AppHandle, project_id: &str) -> Result<ResolvedProject, String> {
    config::resolve_all(&app.state::<AppState>().config())
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| format!("project not found: {project_id}"))
}

/// Claim the runner and spawn `action`. Rejected if anything is already
/// running for this project.
fn start(app: &AppHandle, project: &ResolvedProject, action: &ResolvedAction) -> Result<(), String> {
    let handle = app.state::<AppState>().get_or_create(&project.id);
    if !handle.try_begin(&action.name) {
        return Err(format!(
            "project '{}' is already running — stop it first",
            project.id
        ));
    }
    handle.reset_restart_count();
    lifecycle::emit_status(app, &handle);
    lifecycle::spawn_action(app, project, action, handle)
}

/// Run a named action in a fresh PTY. Clicking the project's stop
/// action stops the current run instead.
#[tauri::command]
pub fn run_action(project_id: String, action_name: String, app: AppHandle) -> Result<(), String> {
    let project = resolve(&app, &project_id)?;
    let action = find_action(&project, &action_name)
        .ok_or_else(|| format!("action '{action_name}' not found in project '{project_id}'"))?
        .clone();
    if config::is_stop_action(&project, &action) {
        lifecycle::stop(&app, &project_id);
        return Ok(());
    }
    start(&app, &project, &action)
}

/// Run the project's implicit "Setup" action.
#[tauri::command]
pub fn setup_project(project_id: String, app: AppHandle) -> Result<(), String> {
    let project = resolve(&app, &project_id)?;
    let setup = project
        .actions
        .iter()
        .find(|a| a.name == "Setup")
        .cloned()
        .ok_or_else(|| format!("project '{project_id}' has no setup action"))?;
    start(&app, &project, &setup)
}

/// Run the project's Build action.
#[tauri::command]
pub fn build_project(project_id: String, app: AppHandle) -> Result<(), String> {
    let project = resolve(&app, &project_id)?;
    let build = config::build_action(&project)
        .cloned()
        .ok_or_else(|| format!("project '{project_id}' has no build action"))?;
    start(&app, &project, &build)
}

/// Stop the current run (stop script / keystroke → terminate → kill).
/// No-op if nothing is running.
#[tauri::command]
pub fn stop_project(project_id: String, app: AppHandle) -> Result<(), String> {
    lifecycle::stop(&app, &project_id);
    Ok(())
}

/// Restart = stop → setup → primary action, in the background.
#[tauri::command]
pub fn restart_project(project_id: String, app: AppHandle) -> Result<(), String> {
    let project = resolve(&app, &project_id)?;
    let primary = project
        .primary_action
        .clone()
        .ok_or_else(|| format!("project '{project_id}' has no actions to restart"))?;
    let action = find_action(&project, &primary)
        .ok_or_else(|| format!("primary action '{primary}' not found"))?
        .clone();
    lifecycle::restart(&app, project, action)
}

/// Current status + ports of every project that has run this session.
/// The UI calls this on mount so a webview reload doesn't show running
/// projects as stopped.
#[tauri::command]
pub fn get_statuses(app: AppHandle) -> Vec<StatusSnapshot> {
    lifecycle::snapshots(&app)
}

#[tauri::command]
pub fn write_to_pty(project_id: String, data: String, app: AppHandle) -> Result<(), String> {
    let Some(handle) = app.state::<AppState>().get(&project_id) else {
        return Err(format!("project not running: {project_id}"));
    };
    handle.send_input(data.as_bytes())
}

#[tauri::command]
pub fn resize_pty(project_id: String, rows: u16, cols: u16, app: AppHandle) -> Result<(), String> {
    match app.state::<AppState>().get(&project_id) {
        Some(handle) => handle.resize_pty(rows, cols),
        None => Ok(()),
    }
}

// =============================================================================
// OS integration
// =============================================================================
//
// These shell out directly instead of using `tauri-plugin-opener` so the
// real OS error reaches the UI. None of them wait on the child: `open`,
// `xdg-open` etc. can block for as long as the opened app runs.

/// Show a path in the system file manager, selected.
#[tauri::command]
pub fn reveal_in_finder(path: String) -> Result<(), String> {
    let target = std::path::PathBuf::from(&path);
    if !target.exists() {
        return Err(format!("path does not exist: {path}"));
    }

    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("/usr/bin/open").arg("-R").arg(&target).spawn();

    #[cfg(target_os = "windows")]
    let result = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", target.display()))
            .spawn()
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let result = {
        let dir = if target.is_dir() {
            target.clone()
        } else {
            target.parent().map(|p| p.to_path_buf()).unwrap_or(target.clone())
        };
        spawn_first_linux_opener(dir.as_os_str())
    };

    result
        .map(|_| ())
        .map_err(|e| format!("could not reveal {path}: {e}"))
}

/// Open a file or folder with the OS default handler (runs an `.exe`,
/// opens an `.msi` / `.dmg`, launches an `.app`).
#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
    let target = std::path::PathBuf::from(&path);
    if !target.exists() {
        return Err(format!("path does not exist: {path}"));
    }

    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("/usr/bin/open").arg(&target).spawn();

    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(&target).spawn();

    #[cfg(all(unix, not(target_os = "macos")))]
    let result = spawn_first_linux_opener(target.as_os_str());

    result
        .map(|_| ())
        .map_err(|e| format!("could not open {path}: {e}"))
}

/// Open a URL in the user's default web browser.
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("refusing to open non-http URL: {url}"));
    }

    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("/usr/bin/open").arg(&url).spawn();

    #[cfg(target_os = "windows")]
    let result = {
        use std::os::windows::process::CommandExt;
        // `cmd /C start` is a console program, so spawn it hidden.
        // Quote the URL verbatim so `&` in a query string survives cmd.
        crate::process::hidden_command("cmd")
            .args(["/D", "/C"])
            .raw_arg(format!("start \"\" \"{url}\""))
            .spawn()
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let result = spawn_first_linux_opener(std::ffi::OsStr::new(&url));

    result
        .map(|_| ())
        .map_err(|e| format!("could not open {url}: {e}"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn spawn_first_linux_opener(arg: &std::ffi::OsStr) -> std::io::Result<std::process::Child> {
    let mut last_err = None;
    for opener in ["xdg-open", "gio", "gnome-open", "kde-open5"] {
        let mut cmd = std::process::Command::new(opener);
        if opener == "gio" {
            cmd.arg("open");
        }
        match cmd.arg(arg).spawn() {
            Ok(child) => return Ok(child),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("no opener found")))
}

// =============================================================================
// helpers
// =============================================================================

/// Look up an action by name; case-insensitive fallback.
fn find_action<'a>(project: &'a ResolvedProject, name: &str) -> Option<&'a ResolvedAction> {
    project
        .actions
        .iter()
        .find(|a| a.name == name)
        .or_else(|| project.actions.iter().find(|a| a.name.eq_ignore_ascii_case(name)))
}

// =============================================================================
// artifact scanning
// =============================================================================

/// Scan a project folder for installable / portable build artifacts —
/// see `artifacts.rs` for the OS filter and ranking rules. Runs on a
/// blocking thread: a sync command would run on the main thread and
/// freeze the window while it walks a large `target/`.
#[tauri::command]
pub async fn scan_project_artifacts(path: String) -> Result<artifacts::ArtifactsScan, String> {
    let path = std::path::PathBuf::from(&path);
    if !path.is_dir() {
        return Err(format!("not a directory: {}", path.display()));
    }
    tauri::async_runtime::spawn_blocking(move || artifacts::scan_project_artifacts(&path))
        .await
        .map_err(|e| format!("scan failed: {e}"))
}
