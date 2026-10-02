//! Cross-process signalling: keystroke parsing + kill escalation +
//! process-tree walk.

use std::collections::HashSet;
use std::time::Duration;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Parse a user-friendly keystroke string into raw bytes for the PTY.
///
/// Supports the common forms: `^C`, `Ctrl-C`, `Ctrl+C`, `^D`, `^Z`, `^\`.
/// Anything else is passed through as literal UTF-8 bytes.
pub fn parse_keystrokes(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for tok in s.split(' ').filter(|t| !t.is_empty()) {
        match tok {
            "^C" | "Ctrl-C" | "Ctrl+C" => out.push(0x03),
            "^D" | "Ctrl-D" | "Ctrl+D" => out.push(0x04),
            "^Z" | "Ctrl-Z" | "Ctrl+Z" => out.push(0x1A),
            "^\\" | "Ctrl-\\" | "Ctrl+\\" => out.push(0x1C),
            "^?" | "Ctrl-?" | "Ctrl+?" => out.push(0x7F),
            other => out.extend_from_slice(other.as_bytes()),
        }
    }
    out
}

/// How long to wait between escalation steps. Tunable later; these are
/// sane defaults that match npm/python ergonomics.
pub const GRACE_AFTER_KEYSTROKE: Duration = Duration::from_secs(3);
pub const GRACE_AFTER_SIGTERM: Duration = Duration::from_secs(3);

// ===== Hidden child processes ==============================================
//
// Release builds are GUI-subsystem processes (`main.rs` sets
// `windows_subsystem = "windows"`), which means the app owns **no
// console of its own**. When a console-subsystem child is spawned from
// a process with no console, Windows cannot attach it to anything, so
// it allocates a brand-new console window for it — the user sees a
// terminal flash open and vanish. `taskkill` and `netstat` are both
// console programs, so every helper spawn produced one of these.
//
// `CREATE_NO_WINDOW` tells Windows to run the child with its console
// hidden instead. It is a no-op on other platforms.
//
// Use this for **every** helper process we spawn. Do not use it for
// things the user is meant to see or interact with.
#[allow(unused_mut)]
pub fn hidden_command(program: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

// ===== Process-tree kill ===================================================
//
// `npm run dev` typically spawns a chain like:
//     sh -c "npm run dev" → npm → sh -c "node …" → node server.js
// Killing just the top PID leaves the grandchildren alive, hanging the
// port and frustrating the user. The fix is to walk the process tree
// rooted at the runner's PID and signal each descendant.
//
// We use `sysinfo` rather than `setsid`/process groups because:
//   - `setsid` is not universally available (e.g. macOS has no
//     `/usr/bin/setsid` on a default install).
//   - `sysinfo` works identically on Unix and Windows.

/// Walk the live process tree rooted at `root_pid` and return every
/// descendant's PID (including the root itself). Order is unspecified.
///
/// We use `refresh_processes(All, /*remove_dead=*/ false)` so a
/// tree-walk can still see recently-killed zombies.
#[allow(dead_code)]
pub fn collect_descendants(root_pid: u32) -> Vec<u32> {
    let mut sys = System::new_all();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        false,
        ProcessRefreshKind::everything(),
    );
    collect_descendants_in(&sys, root_pid)
}

/// Same as [`collect_descendants`] but reuses an existing `System`
/// snapshot — useful when you want to do a tree-walk plus other
/// queries on the same view (e.g. "is anything in the tree alive?").
fn collect_descendants_in(sys: &System, root_pid: u32) -> Vec<u32> {
    let mut all = vec![root_pid];
    let mut to_visit = vec![root_pid];
    while let Some(current) = to_visit.pop() {
        let current_pid = Pid::from_u32(current);
        for (child_pid, proc) in sys.processes() {
            if let Some(parent) = proc.parent() {
                if parent == current_pid && !all.contains(&child_pid.as_u32()) {
                    let child_u32 = child_pid.as_u32();
                    all.push(child_u32);
                    to_visit.push(child_u32);
                }
            }
        }
    }
    all
}

/// `true` if `root_pid` itself or any of its descendants is still in
/// the live process table. Used by the "detached" waiter to know
/// when the actual app has exited (even after the launcher script
/// has returned and the PTY is dead).
///
/// Uses `remove_dead_processes: true` so the table is the source of
/// truth for "alive".
#[allow(dead_code)]
pub fn any_descendant_alive(root_pid: u32) -> bool {
    if root_pid == 0 {
        return false;
    }
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::everything(),
    );
    let tree = collect_descendants_in(&sys, root_pid);
    tree.iter()
        .any(|pid| sys.process(Pid::from_u32(*pid)).is_some())
}

/// Discover new descendants of `root_pid` or any previously tracked PID.
/// Updates `tracked` with newly discovered children and session members.
/// Returns `true` if `root_pid` (if alive) or any discovered descendant is still running.
pub fn update_and_check_alive(tracked: &mut HashSet<u32>, root_pid: u32) -> bool {
    if root_pid == 0 && tracked.is_empty() {
        return false;
    }
    if root_pid != 0 {
        tracked.insert(root_pid);
    }
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::everything(),
    );

    let mut added = true;
    while added {
        added = false;
        for (child_pid, proc) in sys.processes() {
            let child_u32 = child_pid.as_u32();
            if tracked.contains(&child_u32) {
                continue;
            }
            let is_child = proc.parent().map(|p| tracked.contains(&p.as_u32())).unwrap_or(false);
            #[cfg(unix)]
            let is_session = proc.session_id().map(|s| root_pid != 0 && s.as_u32() == root_pid).unwrap_or(false);
            #[cfg(not(unix))]
            let is_session = false;

            if is_child || is_session {
                tracked.insert(child_u32);
                added = true;
            }
        }
    }

    let root_alive = root_pid != 0 && sys.process(Pid::from_u32(root_pid)).is_some();
    let any_descendant_alive = tracked
        .iter()
        .any(|&p| p != root_pid && sys.process(Pid::from_u32(p)).is_some());

    root_alive || any_descendant_alive
}

/// Inspect common PID file locations in `project_path` and return any valid, alive PIDs.
pub fn discover_pid_files(project_path: &std::path::Path) -> Vec<u32> {
    let mut pids = Vec::new();
    let candidates = [
        project_path.join(".codex"),
        project_path.join(".codex/environments"),
        project_path.join("data"),
        project_path.join("tmp"),
        project_path.join("pids"),
        project_path.join("run"),
        project_path.join(".run"),
        project_path.to_path_buf(),
    ];

    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    for dir in candidates {
        if !dir.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name.ends_with(".pid") || name == "pid" {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Ok(pid) = content.trim().parse::<u32>() {
                                if pid != 0 {
                                    #[cfg(unix)]
                                    let alive = unsafe { libc::kill(pid as i32, 0) == 0 }
                                        || sys.process(Pid::from_u32(pid)).is_some();
                                    #[cfg(not(unix))]
                                    let alive = sys.process(Pid::from_u32(pid)).is_some();

                                    if alive && !pids.contains(&pid) {
                                        pids.push(pid);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    pids
}


/// Terminate all processes in `pids` and their descendants.
pub fn kill_pids(pids: &[u32], force: bool) {
    let set: HashSet<u32> = pids.iter().copied().filter(|&p| p != 0).collect();
    if set.is_empty() {
        return;
    }

    // Callers hand us a *flat* set — the root plus every descendant we
    // have ever tracked. Signalling all of them is not just wasteful,
    // it is user-visible on Windows: each `taskkill` spawn used to pop
    // a console window, so one stop produced one terminal per process
    // in the tree, most of them for PIDs that had already exited.
    //
    // On Windows `taskkill /T` already walks and terminates the whole
    // subtree, so signalling the *roots* is sufficient and equivalent.
    // On Unix we send a raw signal to a single PID with no group
    // semantics, so every node still has to be signalled explicitly.
    #[cfg(windows)]
    let targets: Vec<u32> = {
        let mut sys = System::new_all();
        sys.refresh_processes(ProcessesToUpdate::All, false);
        set.iter()
            .copied()
            .filter(|&pid| {
                // Skip anything whose parent is also in the set — the
                // ancestor's `/T` covers it.
                !sys.process(Pid::from_u32(pid))
                    .and_then(|p| p.parent())
                    .map(|parent| set.contains(&parent.as_u32()))
                    .unwrap_or(false)
            })
            .collect()
    };
    #[cfg(not(windows))]
    let targets: Vec<u32> = set.iter().copied().collect();

    for pid in targets {
        terminate_pid(pid, force);
    }
}

/// Walk the tree rooted at `root_pid` and terminate every descendant.
///
/// `force = false` sends SIGTERM (Unix) or `taskkill /T` (Windows) —
/// the polite ask, leaves a window for graceful shutdown.
///
/// `force = true` sends SIGKILL (Unix) or `taskkill /T /F` (Windows) —
/// unconditional, used as the last step of escalation.
#[allow(dead_code)]
pub fn kill_tree(root_pid: u32, force: bool) {
    if root_pid == 0 {
        return;
    }
    kill_pids(&[root_pid], force);
}

#[cfg(unix)]
fn terminate_pid(pid: u32, force: bool) {
    let sig = if force { libc::SIGKILL } else { libc::SIGTERM };
    // SAFETY: `libc::kill` is async-signal-safe. ESRCH (no such process)
    // is the expected outcome for an already-dead descendant, so we
    // intentionally ignore the return value.
    unsafe {
        libc::kill(pid as libc::pid_t, sig);
    }
}

#[cfg(windows)]
fn terminate_pid(pid: u32, _force: bool) {
    // Windows has no real SIGTERM. `taskkill /T` walks the tree and
    // `TerminateProcess`es each node. We always pass /F because
    // graceful termination of a console tree is unreliable without a
    // shared console handle, which `portable_pty` doesn't expose.
    //
    // `hidden_command` is what keeps this from flashing a console
    // window at the user on every single call.
    let _ = hidden_command("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .output();
}
