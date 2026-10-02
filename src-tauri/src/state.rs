//! Shared, mutex-guarded app state.
//!
//! Holds:
//! - the loaded `VibeConfig` and the path it was loaded from
//! - a map of `project_id → Arc<RunnerHandle>` for each project the
//!   user has started in this session (the handle is generic; "project"
//!   is just the new name for what used to be "runner")

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use crate::config::VibeConfig;
use crate::runner::{RunnerHandle, SharedRunner};

pub struct AppState {
    config: RwLock<VibeConfig>,
    config_path: RwLock<Option<PathBuf>>,
    /// Per-project runtime state. Lazy-created on first `get_or_create`.
    /// Keyed by `ProjectConfig.id` — the same id the frontend uses.
    projects: RwLock<HashMap<String, SharedRunner>>,
    /// We need a `Mutex` here too because the very first time a handle
    /// is asked for, we might race two callers; serialise the lazy insert.
    projects_init: Mutex<()>,
    /// Serializes entire config mutation transactions (read -> modify -> write).
    config_lock: tokio::sync::Mutex<()>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            config: RwLock::new(VibeConfig::default()),
            config_path: RwLock::new(None),
            projects: RwLock::new(HashMap::new()),
            projects_init: Mutex::new(()),
            config_lock: tokio::sync::Mutex::new(()),
        }
    }

    // ---- config -----------------------------------------------------------

    pub fn set_config(&self, cfg: VibeConfig) {
        *self.config.write() = cfg;
    }

    pub fn config(&self) -> VibeConfig {
        self.config.read().clone()
    }

    pub fn set_config_path(&self, path: PathBuf) {
        *self.config_path.write() = Some(path);
    }

    pub fn config_path(&self) -> Option<PathBuf> {
        self.config_path.read().clone()
    }

    pub fn config_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.config_lock
    }

    // ---- projects (formerly "runners") ------------------------------------

    /// Get (or lazily create) the handle for a given project id.
    pub fn get_or_create(&self, id: &str) -> SharedRunner {
        if let Some(h) = self.projects.read().get(id).cloned() {
            return h;
        }
        let _guard = self.projects_init.lock();
        if let Some(h) = self.projects.read().get(id).cloned() {
            return h;
        }
        let h = Arc::new(RunnerHandle::new(id.to_string()));
        self.projects.write().insert(id.to_string(), h.clone());
        h
    }

    /// Look up an existing handle without creating. Returns None if the
    /// project has never been started.
    pub fn get(&self, id: &str) -> Option<SharedRunner> {
        self.projects.read().get(id).cloned()
    }

    /// Every handle created this session.
    pub fn all(&self) -> Vec<SharedRunner> {
        self.projects.read().values().cloned().collect()
    }

    /// Drop the handle for an id (used on remove_project, after stopping
    /// any running process). Idempotent.
    pub fn forget(&self, id: &str) {
        self.projects.write().remove(id);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
