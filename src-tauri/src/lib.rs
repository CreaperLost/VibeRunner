//! VibeRunner — Tauri app entry point.
//!
//! Wires up plugins, registers commands, and hands off to Tauri's
//! runtime.

mod commands;
mod config;
mod events;
mod ports;
mod process;
mod pty;
mod runner;
mod state;
mod watcher;

use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::new())
        .setup(|app| {
            // Load vibe.config.json at startup. If it fails, the app
            // still launches — the user can use "Reload" in the UI.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match config::reload_into_state(&handle).await {
                    Ok(path) => {
                        if let Some(state) = handle.try_state::<AppState>() {
                            state.set_config_path(path.clone());
                        }
                        // Start watching the config file so external
                        // edits auto-reload. We only start the watcher
                        // once we know the file path.
                        watcher::spawn(handle.clone(), path);
                    }
                    Err(e) => eprintln!("[viberunner] initial config load failed: {e}"),
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::get_config_path,
            commands::list_projects,
            commands::reload_config,
            commands::add_project,
            commands::remove_project,
            commands::run_action,
            commands::setup_project,
            commands::stop_project,
            commands::restart_project,
            commands::write_to_pty,
            commands::reveal_in_finder,
            commands::open_path,
            commands::open_url,
            commands::resize_pty,
            commands::check_for_updates,
        ])

        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
