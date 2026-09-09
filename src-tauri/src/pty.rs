//! PTY spawning via `portable-pty`.
//!
//! We open a real pseudoterminal (ConPTY on Windows, forkpty on
//! macOS/Linux), run the user's command in it, and hand back a
//! reader + writer + child handle. Bytes flowing through the reader
//! are what we stream to the UI as `project:output` events; bytes
//! written to the writer are what the user sends back (keystrokes
//! for interactive prompts, Ctrl+C, etc.).
//!
//! ## Environment
//!
//! The spawned process inherits the parent's environment. We then
//! layer:
//! 1. `<project>/.env` values (lowest priority, set first so the
//!    higher-priority layers can override).
//! 2. Project-level env (from `vibe.config.json` for manual projects).
//!
//! This lets repos that ship a `.env.example` work out of the box
//! without the user having to declare every key in VibeRunner.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, PtySize};

pub struct SpawnedPty {
    /// Read PTY output from here. EOF = process closed its end.
    pub reader: Box<dyn Read + Send>,
    /// Write user input / keystrokes here.
    pub writer: Box<dyn Write + Send>,
    /// Use to wait() for exit and clone_killer() for termination.
    pub child: Box<dyn Child + Send + Sync>,
    /// Master PTY controller for dynamic window resizing.
    pub master: Box<dyn portable_pty::MasterPty + Send>,
}


/// Spawn a command in a fresh PTY. The command is interpreted by a
/// shell: `sh -c <cmd>` on Unix, `cmd /C <cmd>` on Windows.
///
/// The spawned process's CWD is `project_path` (must exist; caller
/// should have already validated). The process's environment is the
/// parent process's env, layered with:
///   1. Values from `<project_path>/.env` (if it exists).
///   2. `project_env` overrides (if any keys collide).
///
/// `.env` parsing is intentionally simple — `dotenvy` handles the
/// usual quoting and comment rules.
/// Load environment variables layered from:
///   1. `<project_path>/.env` (lowest priority)
///   2. `project_env` (highest priority)
pub fn load_project_env(
    project_path: &Path,
    project_env: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    let env_path = project_path.join(".env");
    if env_path.exists() {
        match dotenvy::from_path_iter(&env_path) {
            Ok(iter) => {
                for item in iter.flatten() {
                    env.insert(item.0, item.1);
                }
            }
            Err(e) => {
                eprintln!(
                    "[viberunner] could not parse {}: {e}",
                    env_path.display()
                );
            }
        }
    }
    for (k, v) in project_env {
        env.insert(k.clone(), v.clone());
    }
    env
}

/// Spawn a command in a fresh PTY. The command is interpreted by a
/// shell: `sh -c <cmd>` on Unix, `cmd /C <cmd>` on Windows.
///
/// The spawned process's CWD is `project_path` (must exist; caller
/// should have already validated). The process's environment is the
/// parent process's env, layered with:
///   1. Values from `<project_path>/.env` (if it exists).
///   2. `project_env` overrides (if any keys collide).
///
/// `.env` parsing is intentionally simple — `dotenvy` handles the
/// usual quoting and comment rules.
pub fn spawn(
    command: &str,
    project_path: &Path,
    project_env: &HashMap<String, String>,
) -> Result<SpawnedPty, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 32,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("openpty failed: {e}"))?;

    #[cfg(unix)]
    ensure_script_executable(project_path, command);

    let mut cmd = shell_command(command);
    cmd.cwd(project_path);

    let full_env = load_project_env(project_path, project_env);
    for (k, v) in full_env {
        cmd.env(k, v);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("spawn failed: {e}"))?;

    // The slave end must be dropped in the parent after spawn;
    // otherwise on Unix the child will block waiting for the parent
    // to close the slave.
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("clone_reader failed: {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("take_writer failed: {e}"))?;

    Ok(SpawnedPty {
        reader,
        writer,
        child,
        master: pair.master,
    })

}

/// Build a `CommandBuilder` that runs `<command>` through the
/// platform's default shell. We pipe through a shell so users can
/// write things like `npm run dev && echo done` or use shell
/// variables and globs.
fn shell_command(command: &str) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(shell_argv0());
    cmd.arg(shell_flag());
    cmd.arg(command);
    cmd
}

#[cfg(unix)]
fn shell_argv0() -> &'static str {
    "sh"
}

#[cfg(unix)]
fn shell_flag() -> &'static str {
    "-c"
}

#[cfg(windows)]
fn shell_argv0() -> &'static str {
    "cmd"
}

#[cfg(windows)]
fn shell_flag() -> &'static str {
    "/C"
}

#[cfg(unix)]
fn ensure_script_executable(project_path: &Path, command: &str) {
    use std::os::unix::fs::PermissionsExt;

    for word in command.split_whitespace() {
        let clean = word
            .trim_matches(|c| c == '\'' || c == '"')
            .trim_start_matches("./");
        if clean.ends_with(".sh") || clean.ends_with(".bash") {
            let script_path = project_path.join(clean);
            if script_path.is_file() {
                if let Ok(metadata) = std::fs::metadata(&script_path) {
                    let mut perms = metadata.permissions();
                    let mode = perms.mode();
                    if mode & 0o111 == 0 {
                        perms.set_mode(mode | 0o755);
                        let _ = std::fs::set_permissions(&script_path, perms);
                    }
                }
            }
        }
    }

    for dir_name in ["script", "scripts"] {
        let dir = project_path.join(dir_name);
        if dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("sh") {
                        if let Ok(metadata) = std::fs::metadata(&path) {
                            let mut perms = metadata.permissions();
                            let mode = perms.mode();
                            if mode & 0o111 == 0 {
                                perms.set_mode(mode | 0o755);
                                let _ = std::fs::set_permissions(&path, perms);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(not(unix))]
fn ensure_script_executable(_project_path: &Path, _command: &str) {}

// `Child` and `ChildKiller` aren't used directly here, but re-exporting
// them from this module keeps the call sites tidy.
#[allow(unused_imports)]
use {Child as _, ChildKiller as _};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_project_env_layers_correctly() {
        let temp_dir = std::env::temp_dir().join(format!("viberunner_env_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let env_file = temp_dir.join(".env");
        std::fs::write(&env_file, "PORT=8000\nSECRET=from_env\nOVERRIDE_ME=old\n").unwrap();

        let mut project_env = HashMap::new();
        project_env.insert("OVERRIDE_ME".into(), "new".into());
        project_env.insert("EXTRA".into(), "extra_val".into());

        let loaded = load_project_env(&temp_dir, &project_env);
        assert_eq!(loaded.get("PORT").map(|s| s.as_str()), Some("8000"));
        assert_eq!(loaded.get("SECRET").map(|s| s.as_str()), Some("from_env"));
        assert_eq!(loaded.get("OVERRIDE_ME").map(|s| s.as_str()), Some("new"));
        assert_eq!(loaded.get("EXTRA").map(|s| s.as_str()), Some("extra_val"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
