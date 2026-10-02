//! Typed Tauri event names and their payload shapes.
//!
//! All events are emitted to the frontend via `app.emit(name, payload)`.
//! The React side mirrors these types in `src/types.ts`.
//!
//! ## Naming
//!
//! Events use a `project:*` prefix because the runtime unit is now a
//! project (a folder on disk) rather than a generic "runner". The
//! old `runner:*` names are gone — they were too generic and the
//! schema no longer matches.

use std::sync::Arc;

use serde::Serialize;

use crate::config::{ResolvedProject, VibeConfig};

/// `project:status` — fired whenever a project's lifecycle status
/// changes. The frontend uses this as the source of truth for the
/// status pill in the sidebar / detail view.
///
/// The `action` field is the name of the action whose PTY is now
/// alive (e.g. "Run", "Setup"), or `None` when the project is
/// idle / stopped / crashed.
pub const EVT_STATUS: &str = "project:status";

/// `project:output` — streamed PTY bytes. Sent in chunks (≤4 KiB) as
/// they arrive from the spawned process.
pub const EVT_OUTPUT: &str = "project:output";

/// `project:ports` — fired every couple of seconds while a project's
/// PTY is active, carrying the list of TCP ports the process is
/// listening on.
pub const EVT_PORTS: &str = "project:ports";

/// `project:restarting` — fired when the auto-restart policy kicks in
/// after a crash. Carries the attempt number and the max.
pub const EVT_RESTARTING: &str = "project:restarting";

/// `config:reloaded` — fired by the file watcher (or the manual
/// Reload button) when `vibe.config.json` has been re-read from disk.
/// The frontend replaces its state with the payload.
pub const EVT_CONFIG_RELOADED: &str = "config:reloaded";

#[derive(Debug, Clone, Serialize)]
pub struct VibeConfigReloadedPayload {
    /// The new raw config. For inspection / editing.
    pub config: Arc<VibeConfig>,
    /// Resolved projects (TOML + manual merged). The sidebar renders
    /// from this list — the raw config is just for the editor.
    pub projects: Vec<ResolvedProject>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusPayload {
    pub id: String,
    /// "stopped" | "starting" | "running" | "stopping" | "crashed"
    pub status: String,
    /// Name of the action that triggered this status, if any. The
    /// frontend uses it to label the status pill ("Running Run",
    /// "Running Setup") so the user knows which PTY they're looking at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Optional human-readable reason (e.g. spawn failure message).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When the current action started (ms since epoch). Lets the UI
    /// show an elapsed timer so a stuck state is obvious.
    #[serde(rename = "startedAtMs", skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
}

/// One entry of `get_statuses` — the full runtime view of a project,
/// used to rehydrate the UI after a webview reload.
#[derive(Debug, Clone, Serialize)]
pub struct StatusSnapshot {
    #[serde(flatten)]
    pub status: StatusPayload,
    pub ports: Vec<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutputPayload {
    pub id: String,
    /// Raw PTY bytes. Serializes to a JSON array of numbers; the
    /// frontend converts to `Uint8Array` and hands to xterm.js.
    pub chunk: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PortsPayload {
    pub id: String,
    pub ports: Vec<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestartingPayload {
    pub id: String,
    pub attempt: u32,
    pub max: u32,
    /// Delay in milliseconds until the restart fires.
    #[serde(rename = "delayMs")]
    pub delay_ms: u64,
}
