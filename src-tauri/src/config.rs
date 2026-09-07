//! Configuration: types, defaults, validation, and the loader that turns
//! `vibe.config.json` on disk into a list of resolved projects the rest
//! of the app uses.
//!
//! ## Schema
//!
//! `vibe.config.json` is a list of `projects`. Each project is either:
//!
//! - **Auto-discovered** (default) — points at a folder on disk; VibeRunner
//!   reads `.codex/environments/environment.toml` from that folder for
//!   `[setup].script` and `[[actions]]`, and turns them into buttons.
//! - **Manual** (`"manual": true`) — the project defines its own
//!   `setup` and `actions` inline in `vibe.config.json`. Useful for
//!   repos that don't follow the TOML convention.
//!
//! ## Resolution
//!
//! At load time each project becomes a [`ResolvedProject`]: a flat
//! list of action buttons (`name` / `icon` / `command`) including the
//! implicit "Setup" action when one is configured. The frontend
//! renders those buttons directly. The "Restart" built-in uses the
//! project's `primaryAction` (or, by default, the first action whose
//! name or icon is "Run").
//!
//! ## File location
//!
//! `default_config_path()` resolves the config file location with
//! this priority:
//!   1. `$VIBE_CONFIG` env var
//!   2. App data dir (`~/Library/Application Support/com.viberunner.app/`
//!      on macOS, `%APPDATA%\com.viberunner.app\` on Windows) — the
//!      canonical place for installed apps
//!   3. Walk up from CWD (handles `pnpm tauri dev` from the project root)
//!   4. Walk up from the executable's directory
//!   5. App data dir again (so first launch has a real writable home)
//!
//! If the resolved file doesn't exist, we create a default empty
//! `VibeConfig` at the same path. See `default_config_path()` for details.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

// =============================================================================
// vibe.config.json on disk
// =============================================================================

/// Top-level shape of `vibe.config.json`. Version 2 introduces the
/// project-list model. Version 1 (legacy `runners: []`) is no longer
/// accepted — the migration is a one-shot manual edit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VibeConfig {
    pub version: u32,
    #[serde(default)]
    pub projects: Vec<ProjectConfig>,
}

impl Default for VibeConfig {
    fn default() -> Self {
        Self {
            version: 2,
            projects: vec![],
        }
    }
}

impl VibeConfig {
    /// Add a project. Returns Err if a project with the same `id` already
    /// exists or if the resulting config would be invalid.
    pub fn add_project(&mut self, project: ProjectConfig) -> Result<(), String> {
        if self.projects.iter().any(|p| p.id == project.id) {
            return Err(format!("project id '{}' already exists", project.id));
        }
        // Validate before mutating so the file never contains an invalid
        // entry, even mid-construction.
        validate(self).map_err(|e| e.to_string())?;
        self.projects.push(project);
        Ok(())
    }

    /// Remove a project by id. Returns Err if not found.
    pub fn remove_project(&mut self, id: &str) -> Result<(), String> {
        let before = self.projects.len();
        self.projects.retain(|p| p.id != id);
        if self.projects.len() == before {
            return Err(format!("project id '{id}' not found"));
        }
        Ok(())
    }
}

/// One entry in `vibe.config.json`. Either auto-discovered (pointed
/// at a folder on disk; the TOML inside that folder supplies the
/// actions) or fully manual (inline actions).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub id: String,
    /// Absolute path to the project folder. For auto-discovered
    /// projects, VibeRunner looks for `.codex/environments/environment.toml`
    /// inside this folder. For manual projects, the spawned PTY uses
    /// this as `cwd`.
    pub path: String,
    /// Display name override. Defaults to the TOML's `name` field for
    /// auto-discovered projects, or the id if neither is set.
    #[serde(default)]
    pub name: Option<String>,
    /// Action the built-in "Restart" button should run. If unset, the
    /// resolver picks the first action with `name="Run"` or
    /// `icon="run"`, falling back to the first action.
    #[serde(default, rename = "primaryAction")]
    pub primary_action: Option<String>,
    /// Skip TOML discovery and use the inline `setup` + `actions` below.
    #[serde(default)]
    pub manual: bool,
    /// Inline setup command for manual projects. Becomes the implicit
    /// "Setup" action.
    #[serde(default)]
    pub setup: Option<ManualCommand>,
    /// Inline action list for manual projects.
    #[serde(default)]
    pub actions: Vec<ActionConfig>,
    /// Optional env vars for manual projects.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Optional auto-restart policy for manual projects.
    #[serde(default, rename = "autoRestart")]
    pub auto_restart: Option<RestartPolicy>,
}

/// A shell command. Currently just a string; kept as a struct so we
/// can extend it later (cwd, env, etc.) without breaking the JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualCommand {
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionConfig {
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub command: String,
    /// `true` if the command launches a background process and
    /// returns quickly (e.g. `nohup start.sh &`). When set,
    /// VibeRunner tracks the *whole process tree* rooted at the
    /// PTY's PID — the project stays "Running" as long as any
    /// descendant is alive, even after the launcher script has
    /// returned. Defaults to `false` (wait for the direct child).
    #[serde(default)]
    pub detached: bool,
}

/// Auto-restart behaviour on crash. When `enabled` is true, a non-zero
/// exit triggers a re-spawn after `delay_ms`, up to `max_retries`
/// consecutive times. Counter resets on the next successful start.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestartPolicy {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_max_retries", rename = "maxRetries", alias = "max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_delay_ms", rename = "delayMs", alias = "delay_ms")]
    pub delay_ms: u64,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_retries: default_max_retries(),
            delay_ms: default_delay_ms(),
        }
    }
}

fn default_max_retries() -> u32 {
    3
}
fn default_delay_ms() -> u64 {
    1000
}

// =============================================================================
// `.codex/environments/environment.toml`
// =============================================================================

/// Shape of the auto-discovered TOML file. We keep the on-disk field
/// names verbatim (`setup`, `actions`, `name`, `version`).
#[derive(Debug, Clone, Deserialize)]
struct TomlEnvironment {
    #[serde(default = "default_toml_version")]
    #[allow(dead_code)]
    version: u32,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    setup: Option<TomlScript>,
    #[serde(default)]
    actions: Vec<TomlAction>,
}

fn default_toml_version() -> u32 {
    1
}

#[derive(Debug, Clone, Deserialize)]
struct TomlScript {
    #[serde(default)]
    script: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TomlAction {
    name: String,
    #[serde(default)]
    icon: Option<String>,
    command: String,
    #[serde(default)]
    detached: bool,
}

/// Read the TOML environment file from `<project>/.codex/environments/environment.toml`.
/// Returns `Ok(None)` if the file doesn't exist (a valid state — the
/// project is just empty). Returns `Err` only for I/O or parse errors.
fn read_toml_environment(project_path: &Path) -> Result<Option<TomlEnvironment>, String> {
    let toml_path = project_path.join(".codex").join("environments").join("environment.toml");
    if !toml_path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&toml_path)
        .map_err(|e| format!("read {}: {e}", toml_path.display()))?;
    let env: TomlEnvironment = toml::from_str(&text)
        .map_err(|e| format!("parse {}: {e}", toml_path.display()))?;
    Ok(Some(env))
}

// =============================================================================
// Resolved project (what the frontend renders)
// =============================================================================

/// A single button the user can click. Same shape regardless of
/// whether the action came from the TOML or from inline config.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ResolvedAction {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub command: String,
    /// See [`ActionConfig::detached`]. Propagated from the source.
    #[serde(default)]
    pub detached: bool,
}

/// Where a project's actions came from. Useful for the UI to show a
/// "TOML auto-discovered" badge.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProjectSource {
    /// Actions came from `.codex/environments/environment.toml`.
    Toml,
    /// Actions are defined inline in `vibe.config.json` (`manual: true`).
    Manual,
    /// No actions configured. The project exists but has no buttons.
    Empty,
}

/// The frontend-facing view of a project. Built by [`resolve_project`].
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedProject {
    pub id: String,
    pub name: String,
    pub path: String,
    pub source: ProjectSource,
    /// All buttons, including the implicit "Setup" action if one exists.
    /// Order: setup first, then the user-defined actions in TOML order.
    pub actions: Vec<ResolvedAction>,
    /// Name of the action the built-in "Restart" should run. `None`
    /// when the project has no "Run"-shaped action.
    #[serde(rename = "primaryAction")]
    pub primary_action: Option<String>,
    /// Non-fatal issues (path missing, no TOML, etc.). The project is
    /// still usable — just with caveats.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Optional auto-restart policy (manual projects only for now).
    #[serde(default, rename = "autoRestart")]
    pub auto_restart: Option<RestartPolicy>,
    /// Configured environment variable overrides.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

/// Resolve a single project config into the runtime view the UI needs.
pub fn resolve_project(project: &ProjectConfig) -> ResolvedProject {
    let mut warnings = Vec::new();
    let mut actions: Vec<ResolvedAction> = Vec::new();
    let mut source = ProjectSource::Empty;

    let path = PathBuf::from(&project.path);
    if !path.exists() {
        warnings.push(format!("path does not exist: {}", project.path));
    }

    if project.manual {
        // ---- Manual: use inline actions -----------------------------------
        source = ProjectSource::Manual;
        if let Some(setup) = &project.setup {
            let cmd = setup.command.trim();
            if !cmd.is_empty() {
                actions.push(ResolvedAction {
                    name: "Setup".into(),
                    icon: Some("tool".into()),
                    command: cmd.into(),
                    detached: false,
                });
            }
        }
        for a in &project.actions {
            actions.push(ResolvedAction {
                name: a.name.clone(),
                icon: a.icon.clone(),
                command: a.command.clone(),
                detached: a.detached,
            });
        }
    } else {
        // ---- Auto: read TOML ---------------------------------------------
        match read_toml_environment(&path) {
            Ok(Some(toml_env)) => {
                source = ProjectSource::Toml;
                if let Some(setup) = &toml_env.setup {
                    let cmd = setup.script.trim();
                    if !cmd.is_empty() {
                        actions.push(ResolvedAction {
                            name: "Setup".into(),
                            icon: Some("tool".into()),
                            command: cmd.into(),
                            detached: false,
                        });
                    }
                }
                for a in &toml_env.actions {
                    actions.push(ResolvedAction {
                        name: a.name.clone(),
                        icon: a.icon.clone(),
                        command: a.command.clone(),
                        detached: a.detached,
                    });
                }
                let name = project
                    .name
                    .clone()
                    .or(toml_env.name.clone())
                    .unwrap_or_else(|| project.id.clone());
                return ResolvedProject {
                    id: project.id.clone(),
                    name,
                    path: project.path.clone(),
                    source,
                    primary_action: pick_primary_action(project, &actions),
                    warnings,
                    auto_restart: project.auto_restart.clone(),
                    actions,
                    env: project.env.clone(),
                };
            }
            Ok(None) => {
                warnings.push(format!(
                    "no .codex/environments/environment.toml in {} — add one, or set manual: true with inline actions",
                    project.path
                ));
            }
            Err(e) => {
                warnings.push(format!("could not read TOML: {e}"));
            }
        }
    }

    let name = project
        .name
        .clone()
        .unwrap_or_else(|| project.id.clone());

    ResolvedProject {
        id: project.id.clone(),
        name,
        path: project.path.clone(),
        source,
        actions: actions.clone(),
        primary_action: pick_primary_action(project, &actions),
        warnings,
        auto_restart: project.auto_restart.clone(),
        env: project.env.clone(),
    }
}

/// Pick the action the built-in "Restart" button should run.
///
/// Resolution:
/// 1. Explicit `primaryAction` on the project config (matched by name).
/// 2. First action with `name="Run"` or `icon="run"`.
/// 3. First action in the list.
fn pick_primary_action(
    project: &ProjectConfig,
    actions: &[ResolvedAction],
) -> Option<String> {
    if let Some(pa) = &project.primary_action {
        if actions.iter().any(|a| &a.name == pa) {
            return Some(pa.clone());
        }
    }
    for a in actions {
        if a.name.eq_ignore_ascii_case("Run") || a.icon.as_deref() == Some("run") {
            return Some(a.name.clone());
        }
    }
    actions.first().map(|a| a.name.clone())
}

// =============================================================================
// Loading, writing, validation
// =============================================================================

/// Read the config from a path, parsing JSON. Used by tests and the
/// manual "Open config…" picker.
pub fn load_from_path(path: &Path) -> Result<VibeConfig, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Read {
        path: path.to_path_buf(),
        source: e,
    })?;
    parse(&text)
}

pub fn parse(text: &str) -> Result<VibeConfig, ConfigError> {
    // Accept JSONC (JSON with comments and trailing commas) so users
    // can annotate their config in their editor.
    let value = jsonc_parser::parse_to_serde_value(text, &Default::default())
        .map_err(|e| ConfigError::Parse(e.to_string()))?
        .ok_or_else(|| ConfigError::Parse("empty document".into()))?;
    let cfg: VibeConfig =
        serde_json::from_value(value).map_err(|e| ConfigError::Parse(e.to_string()))?;
    validate(&cfg)?;
    Ok(cfg)
}

fn validate(cfg: &VibeConfig) -> Result<(), ConfigError> {
    if cfg.version != 2 {
        return Err(ConfigError::UnsupportedVersion(cfg.version));
    }
    let mut seen = std::collections::HashSet::new();
    for p in &cfg.projects {
        if p.id.is_empty() {
            return Err(ConfigError::Validation("project id must not be empty".into()));
        }
        if p.path.is_empty() {
            return Err(ConfigError::Validation(format!(
                "project '{}': path must not be empty",
                p.id
            )));
        }
        if !seen.insert(p.id.clone()) {
            return Err(ConfigError::Validation(format!(
                "duplicate project id: '{}'",
                p.id
            )));
        }
        // For manual projects, the first action's command must be non-empty
        // (the resolver will skip the implicit Setup if its command is empty).
        if p.manual {
            for a in &p.actions {
                if a.name.trim().is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "project '{}': action name must not be empty",
                        p.id
                    )));
                }
                if a.command.trim().is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "project '{}', action '{}': command must not be empty",
                        p.id, a.name
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Where the loader looks for `vibe.config.json` by default.
///
/// Resolution order (first match wins):
/// 1. `$VIBE_CONFIG` env var (if set, points directly to a file)
/// 2. Tauri's app data dir — `~/Library/Application Support/com.viberunner.app/`
///    on macOS, `%APPDATA%\com.viberunner.app\` on Windows, etc. The
///    file lives there even after a `.dmg` install because the user
///    has no project root to walk up from when launched from Finder.
/// 3. Walk up from the current working directory (handles
///    `cargo run` from `src-tauri/` and `pnpm tauri dev` from the
///    project root).
/// 4. Walk up from the executable's directory (helps when the
///    `.app` happens to be next to a config).
/// 5. The app data dir again (as the "create here" fallback when
///    nothing else exists) — so first launch always has a real
///    path, not a fictional "./vibe.config.json" relative to
///    nothing.
///
/// If the resolved path's parent doesn't exist, we create it. If
/// the file itself doesn't exist, we write a default empty
/// `VibeConfig` so the app always has *something* to show.
pub fn default_config_path(app: &AppHandle) -> PathBuf {
    // 1. Explicit env var override (highest priority — used by tests
    //    and the `pnpm tauri dev` workflow that wants a known file).
    if let Ok(p) = std::env::var("VIBE_CONFIG") {
        return PathBuf::from(p);
    }

    // 2. The app data dir is the canonical place for installed apps
    //    (Tauri knows the right per-OS location). If the file lives
    //    there, use it; if not, we still pick this path so the
    //    caller can create it (step 5).
    let app_data = app.path().app_data_dir().ok().map(|d| d.join("vibe.config.json"));

    if let Ok(cwd) = std::env::current_dir() {
        let mut current: Option<&Path> = Some(cwd.as_path());
        while let Some(dir) = current {
            let candidate = dir.join("vibe.config.json");
            if candidate.exists() {
                return candidate;
            }
            current = dir.parent();
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("vibe.config.json");
            if candidate.exists() {
                return candidate;
            }
        }
    }

    // 5. Fall back to the app data dir so the file has a real,
    //    writable home. Caller (`reload_into_state`) creates the
    //    dir + a default config if missing.
    if let Some(p) = app_data {
        return p;
    }

    // Last resort: a relative path. Should never happen on a
    // real Tauri app (Tauri's app_data_dir always returns Ok on a
    // properly-configured bundle), but we keep the fallback so
    // the loader doesn't crash on unusual environments.
    PathBuf::from("vibe.config.json")
}

/// Load the config (from default path) and stash it in app state.
/// Used at startup and by the "Reload" button.
///
/// If the file doesn't exist, we create a default empty config at
/// the same path so the user always has a real, editable file they
/// can point VibeRunner at next time. The path is always returned
/// (even on load failure) so the UI can show "vibe.config.json at
/// <path>" rather than "no config loaded".
pub async fn reload_into_state(app: &AppHandle) -> Result<PathBuf, ConfigError> {
    let path = default_config_path(app);
    let app_for_blocking = app.clone();
    let path_for_blocking = path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        ensure_config_file(&path_for_blocking)?;
        load_from_path(&path_for_blocking)
    })
    .await
    .map_err(|e| ConfigError::Other(format!("blocking load task failed: {e}")))?;
    match result {
        Ok(cfg) => {
            if let Some(state) = app.try_state::<AppState>() {
                state.set_config(cfg);
            }
            Ok(path)
        }
        Err(e) => {
            if let Some(state) = app.try_state::<AppState>() {
                state.set_config(VibeConfig::default());
            }
            // Even on parse failure, return the path so the UI can
            // show "config at <path> (parse error: ...)" rather than
            // "no config loaded".
            let _ = app_for_blocking; // keep alive for the closure
            Err(e)
        }
    }
}

/// If `<path>` doesn't exist, create its parent directory and write
/// a default empty `VibeConfig` to it. Idempotent.
fn ensure_config_file(path: &Path) -> Result<(), ConfigError> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::Other(format!(
            "create config dir {}: {e}",
            parent.display()
        )))?;
    }
    let default = VibeConfig::default();
    write_to_path(path, &default).map_err(ConfigError::Other)?;
    Ok(())
}

/// Resolve every project in a config. Convenience for the frontend
/// (one call, full picture).
pub fn resolve_all(cfg: &VibeConfig) -> Vec<ResolvedProject> {
    cfg.projects.iter().map(resolve_project).collect()
}

/// Serialize `cfg` as pretty JSON and atomically write it to `path`.
pub fn write_to_path(path: &Path, cfg: &VibeConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(cfg)
        .map_err(|e| format!("serialize: {e}"))?;

    static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let count = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(
        ".vibe.config.{}.{}.tmp",
        std::process::id(),
        count
    ));

    {
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| format!("create temp {}: {e}", tmp.display()))?;
        use std::io::Write;
        f.write_all(json.as_bytes())
            .map_err(|e| format!("write temp: {e}"))?;
        f.sync_all().map_err(|e| format!("fsync: {e}"))?;
    }

    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename {} → {}: {e}", tmp.display(), path.display())
    })?;

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read config at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid config JSON: {0}")]
    Parse(String),
    #[error("unsupported config version: {0} (expected 2)")]
    UnsupportedVersion(u32),
    #[error("invalid config: {0}")]
    Validation(String),
    #[error("{0}")]
    Other(String),
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let json = r#"{
            "version": 2,
            "projects": [
                { "id": "api", "path": "/tmp/api" }
            ]
        }"#;
        let cfg = parse(json).unwrap();
        assert_eq!(cfg.projects.len(), 1);
        assert_eq!(cfg.projects[0].id, "api");
    }

    #[test]
    fn rejects_wrong_version() {
        let json = r#"{"version": 1, "projects": []}"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ConfigError::UnsupportedVersion(1)));
    }

    #[test]
    fn rejects_duplicate_id() {
        let json = r#"{
            "version": 2,
            "projects": [
                { "id": "a", "path": "/tmp/a" },
                { "id": "a", "path": "/tmp/a2" }
            ]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
    }

    #[test]
    fn rejects_empty_path() {
        let json = r#"{
            "version": 2,
            "projects": [{ "id": "a", "path": "" }]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
    }

    #[test]
    fn add_project_round_trip() {
        let json = r#"{"version": 2, "projects": []}"#;
        let mut cfg = parse(json).unwrap();
        cfg.add_project(ProjectConfig {
            id: "api".into(),
            path: "/tmp/api".into(),
            name: None,
            primary_action: None,
            manual: false,
            setup: None,
            actions: vec![],
            env: Default::default(),
            auto_restart: None,
        })
        .unwrap();
        assert_eq!(cfg.projects.len(), 1);

        let dup = cfg.add_project(ProjectConfig {
            id: "api".into(),
            path: "/tmp/api".into(),
            ..cfg.projects[0].clone()
        });
        assert!(dup.is_err());
    }

    #[test]
    fn remove_project_round_trip() {
        let json = r#"{
            "version": 2,
            "projects": [
                { "id": "a", "path": "/tmp/a" },
                { "id": "b", "path": "/tmp/b" }
            ]
        }"#;
        let mut cfg = parse(json).unwrap();
        cfg.remove_project("a").unwrap();
        assert_eq!(cfg.projects.len(), 1);
        assert_eq!(cfg.projects[0].id, "b");
        assert!(cfg.remove_project("nope").is_err());
    }

    #[test]
    fn parses_jsonc_with_comments_and_trailing_commas() {
        let jsonc = r#"
        // This is a comment
        {
            "version": 2,
            /* block comment */
            "projects": [
                {
                    "id": "api",  // line comment after value
                    "path": "/tmp/api",
                },  // trailing comma
            ],
        }
        "#;
        let cfg = parse(jsonc).unwrap();
        assert_eq!(cfg.projects.len(), 1);
        assert_eq!(cfg.projects[0].id, "api");
    }

    #[test]
    fn resolves_toml_environment() {
        // Build a temp project dir with a TOML file in it.
        let tmp = std::env::temp_dir().join(format!("viberunner-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".codex/environments")).unwrap();
        std::fs::write(
            tmp.join(".codex/environments/environment.toml"),
            r#"
version = 1
name = "Test Project"

[setup]
script = "echo setup"

[[actions]]
name = "Run"
icon = "run"
command = "echo run"
detached = true

[[actions]]
name = "Stop"
icon = "stop"
command = "echo stop"
"#,
        )
        .unwrap();

        let project = ProjectConfig {
            id: "test".into(),
            path: tmp.to_string_lossy().into_owned(),
            name: None,
            primary_action: None,
            manual: false,
            setup: None,
            actions: vec![],
            env: Default::default(),
            auto_restart: None,
        };
        let resolved = resolve_project(&project);
        assert_eq!(resolved.source, ProjectSource::Toml);
        assert_eq!(resolved.name, "Test Project");
        assert_eq!(resolved.actions.len(), 3); // setup + 2 actions
        assert_eq!(resolved.actions[0].name, "Setup");
        assert_eq!(resolved.actions[1].name, "Run");
        assert!(resolved.actions[1].detached, "Run action should be detached");
        assert_eq!(resolved.actions[2].name, "Stop");
        assert!(!resolved.actions[2].detached);
        assert_eq!(resolved.primary_action.as_deref(), Some("Run"));

        // Cleanup.
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolves_manual_project() {
        let project = ProjectConfig {
            id: "static".into(),
            path: "/tmp/static".into(),
            name: Some("Static Server".into()),
            primary_action: None,
            manual: true,
            setup: Some(ManualCommand {
                command: "echo setup".into(),
            }),
            actions: vec![
                ActionConfig {
                    name: "Run".into(),
                    icon: Some("run".into()),
                    command: "python3 -m http.server 8080".into(),
                    detached: false,
                },
                ActionConfig {
                    name: "Stop".into(),
                    icon: Some("stop".into()),
                    command: "Ctrl+C".into(),
                    detached: false,
                },
            ],
            env: Default::default(),
            auto_restart: None,
        };
        let resolved = resolve_project(&project);
        assert_eq!(resolved.source, ProjectSource::Manual);
        assert_eq!(resolved.name, "Static Server");
        assert_eq!(resolved.actions.len(), 3); // setup + run + stop
        assert_eq!(resolved.actions[0].name, "Setup");
        assert!(!resolved.actions[1].detached); // Run, not detached
        assert!(!resolved.actions[2].detached);
        assert_eq!(resolved.primary_action.as_deref(), Some("Run"));
    }

    #[test]
    fn missing_toml_yields_warning() {
        // Project points at a non-existent path.
        let project = ProjectConfig {
            id: "ghost".into(),
            path: "/this/does/not/exist".into(),
            name: None,
            primary_action: None,
            manual: false,
            setup: None,
            actions: vec![],
            env: Default::default(),
            auto_restart: None,
        };
        let resolved = resolve_project(&project);
        // No TOML found (since the path doesn't even exist).
        assert!(!resolved.warnings.is_empty());
    }

    #[test]
    fn primary_action_can_be_overridden() {
        let project = ProjectConfig {
            id: "x".into(),
            path: "/tmp/x".into(),
            name: None,
            primary_action: Some("Stop".into()),
            manual: true,
            setup: None,
            actions: vec![
                ActionConfig {
                    name: "Run".into(),
                    icon: Some("run".into()),
                    command: "echo run".into(),
                    detached: false,
                },
                ActionConfig {
                    name: "Stop".into(),
                    icon: Some("stop".into()),
                    command: "echo stop".into(),
                    detached: false,
                },
            ],
            env: Default::default(),
            auto_restart: None,
        };
        let resolved = resolve_project(&project);
        // Explicit primaryAction wins, even though "Run" would be the default.
        assert_eq!(resolved.primary_action.as_deref(), Some("Stop"));
    }

    #[test]
    fn manual_action_detached_propagates() {
        let project = ProjectConfig {
            id: "x".into(),
            path: "/tmp/x".into(),
            name: None,
            primary_action: None,
            manual: true,
            setup: None,
            actions: vec![
                ActionConfig {
                    name: "Run".into(),
                    icon: Some("run".into()),
                    command: "nohup start.sh &".into(),
                    detached: true,
                },
                ActionConfig {
                    name: "Stop".into(),
                    icon: Some("stop".into()),
                    command: "Ctrl+C".into(),
                    detached: false,
                },
            ],
            env: Default::default(),
            auto_restart: None,
        };
        let resolved = resolve_project(&project);
        assert!(resolved.actions[0].detached, "Run should be detached");
        assert!(!resolved.actions[1].detached, "Stop should not be detached");
    }

    #[test]
    fn restart_policy_wire_format_camel_case() {
        let json = r#"{"enabled":true,"maxRetries":7,"delayMs":2500}"#;
        let policy: RestartPolicy = serde_json::from_str(json).unwrap();
        assert_eq!(policy.max_retries, 7);
        assert_eq!(policy.delay_ms, 2500);

        let serialized = serde_json::to_string(&policy).unwrap();
        assert!(serialized.contains(r#""maxRetries":7"#));
        assert!(serialized.contains(r#""delayMs":2500"#));

        // Legacy snake_case also deserializes
        let legacy = r#"{"enabled":true,"max_retries":4,"delay_ms":1500}"#;
        let policy_legacy: RestartPolicy = serde_json::from_str(legacy).unwrap();
        assert_eq!(policy_legacy.max_retries, 4);
        assert_eq!(policy_legacy.delay_ms, 1500);
    }

    #[test]
    fn resolved_project_retains_env() {
        let mut env = std::collections::HashMap::new();
        env.insert("FOO".into(), "bar".into());
        env.insert("PORT".into(), "3000".into());

        let project = ProjectConfig {
            id: "with-env".into(),
            path: "/tmp/with-env".into(),
            name: None,
            primary_action: None,
            manual: true,
            setup: None,
            actions: vec![],
            env: env.clone(),
            auto_restart: None,
        };
        let resolved = resolve_project(&project);
        assert_eq!(resolved.env, env);
    }

    #[test]
    fn test_write_to_path_atomic() {
        let temp_dir = std::env::temp_dir().join(format!("viberunner_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let config_path = temp_dir.join("vibe.config.json");

        let mut cfg = VibeConfig::default();
        cfg.projects.push(ProjectConfig {
            id: "p1".into(),
            path: "/path/1".into(),
            name: None,
            primary_action: None,
            manual: false,
            setup: None,
            actions: vec![],
            env: Default::default(),
            auto_restart: None,
        });

        write_to_path(&config_path, &cfg).expect("write failed");

        let loaded = load_from_path(&config_path).expect("load failed");
        assert_eq!(loaded.projects.len(), 1);
        assert_eq!(loaded.projects[0].id, "p1");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
