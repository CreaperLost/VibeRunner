//! Cross-process signalling: keystroke parsing, process-tree tracking
//! with identity checks, and kill escalation.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

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

/// True when a configured Stop command is a keystroke to send into the
/// PTY rather than a script to run.
pub fn is_pure_keystroke(s: &str) -> bool {
    matches!(
        s.trim(),
        "^C" | "Ctrl-C" | "Ctrl+C"
            | "^D" | "Ctrl-D" | "Ctrl+D"
            | "^Z" | "Ctrl-Z" | "Ctrl+Z"
            | "^\\" | "Ctrl-\\" | "Ctrl+\\"
            | "^?" | "Ctrl-?" | "Ctrl+?"
    )
}

/// Stop escalation timing: keystroke/stop script → polite terminate →
/// force kill → give up and report Stopped anyway. Every step is
/// skipped as soon as the tree is observed dead.
pub const GRACE_AFTER_KEYSTROKE: Duration = Duration::from_secs(3);
pub const GRACE_AFTER_SIGTERM: Duration = Duration::from_secs(3);
pub const GRACE_AFTER_SIGKILL: Duration = Duration::from_secs(3);

// ===== Hidden child processes ==============================================
//
// Release builds are GUI-subsystem processes (`main.rs` sets
// `windows_subsystem = "windows"`), which means the app owns **no
// console of its own**. When a console-subsystem child is spawned from
// a process with no console, Windows cannot attach it to anything, so
// it allocates a brand-new console window for it — the user sees a
// terminal flash open and vanish. `netstat` and `cmd` are console
// programs, so every helper spawn would produce one of these.
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

// ===== Process identity =====================================================
//
// A bare PID is not an identity: every OS recycles them, Windows very
// aggressively. The old tracker kept every PID it had ever seen and
// treated "a process with that PID exists" as "still ours", which made
// projects stick in Running/Stopping forever (a recycled PID never dies)
// and made Stop `taskkill /T /F` unrelated processes.
//
// Every tracked process is therefore keyed by (pid, start_time). A
// process is adopted into the tree only if its parent is a tracked
// process *and* it started no earlier than that parent. Dead entries are
// pruned on every refresh, but remembered briefly so orphans (children
// whose parent already exited, e.g. `nohup … &`) can still be adopted —
// only if they started before the parent was observed dead, which a
// recycled-PID impostor's children never do.

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Fresh, cheap process-table snapshot (pid, parent, start time, status).
pub fn snapshot() -> System {
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new());
    sys
}

fn is_live(p: &Process) -> bool {
    !matches!(p.status(), ProcessStatus::Zombie | ProcessStatus::Dead)
}

/// How long a dead entry is remembered for orphan adoption.
const DEAD_MEMORY_SECS: u64 = 600;

/// The set of processes a run owns, with identity checks.
#[derive(Debug, Default)]
pub struct ProcessTree {
    root: Option<(u32, u64)>,
    live: HashMap<u32, u64>,
    /// pid → (start_time, observed_dead_at)
    dead: HashMap<u32, (u64, u64)>,
}

impl ProcessTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everything and start a new tree rooted at `pid`.
    pub fn reset(&mut self, pid: Option<u32>) {
        self.live.clear();
        self.dead.clear();
        self.root = None;
        let Some(pid) = pid else { return };
        let sys = snapshot();
        let start = sys
            .process(Pid::from_u32(pid))
            .map(|p| p.start_time())
            .filter(|&s| s != 0)
            .unwrap_or_else(now_secs);
        self.root = Some((pid, start));
        self.live.insert(pid, start);
    }

    /// Clear all tracking (after a clean stop).
    pub fn clear(&mut self) {
        self.reset(None);
    }

    pub fn pids(&self) -> Vec<u32> {
        self.live.keys().copied().collect()
    }

    /// Prune dead / recycled entries and adopt new descendants.
    /// Returns `true` if any owned process is still alive.
    pub fn refresh(&mut self, sys: &System) -> bool {
        let now = now_secs();

        // 1. Prune: an entry survives only if the same process (same
        //    start time) is still in the table and not a zombie.
        let mut died = Vec::new();
        for (&pid, &start) in &self.live {
            let same = sys
                .process(Pid::from_u32(pid))
                .map(|p| p.start_time() == start && is_live(p))
                .unwrap_or(false);
            if !same {
                died.push((pid, start));
            }
        }
        for (pid, start) in died {
            self.live.remove(&pid);
            self.dead.insert(pid, (start, now));
        }
        self.dead
            .retain(|_, (_, died_at)| now.saturating_sub(*died_at) < DEAD_MEMORY_SECS);

        // 2. Adopt descendants until a fixed point.
        let root = self.root;
        loop {
            let mut added = false;
            for (pid, proc) in sys.processes() {
                let pid = pid.as_u32();
                if self.live.contains_key(&pid) || !is_live(proc) {
                    continue;
                }
                let start = proc.start_time();
                if start == 0 {
                    // Unknown start time (no access) — can't prove identity.
                    continue;
                }
                let by_parent = proc.parent().map(|pp| pp.as_u32()).is_some_and(|pp| {
                    if let Some(&ps) = self.live.get(&pp) {
                        start >= ps
                    } else if let Some(&(ps, died_at)) = self.dead.get(&pp) {
                        // Orphan: must have started while its parent lived.
                        start >= ps && start <= died_at
                    } else {
                        false
                    }
                });
                // On Unix the PTY child is a session leader (setsid), so
                // anything still in its session belongs to the run even if
                // it was reparented to init.
                #[cfg(unix)]
                let by_session = root.is_some_and(|(rp, rs)| {
                    start >= rs && proc.session_id().map(|s| s.as_u32()) == Some(rp)
                });
                #[cfg(not(unix))]
                let by_session = {
                    let _ = root;
                    false
                };
                if by_parent || by_session {
                    self.live.insert(pid, start);
                    added = true;
                }
            }
            if !added {
                break;
            }
        }

        !self.live.is_empty()
    }

    /// Add a PID from outside the tree (a PID file), only if that process
    /// started at or after `since` — a stale PID file pointing at a
    /// long-running unrelated process is rejected.
    pub fn adopt_external(&mut self, sys: &System, pid: u32, since: u64) {
        if self.live.contains_key(&pid) {
            return;
        }
        if let Some(p) = sys.process(Pid::from_u32(pid)) {
            let start = p.start_time();
            if start != 0 && start + 1 >= since && is_live(p) {
                self.live.insert(pid, start);
            }
        }
    }

    /// Signal every owned process whose identity still checks out.
    /// `force=false` → SIGTERM on Unix; on Windows there is no polite
    /// signal for console trees, so both levels TerminateProcess.
    pub fn kill(&self, force: bool) {
        if self.live.is_empty() {
            return;
        }
        let sys = snapshot();
        for (&pid, &start) in &self.live {
            if pid == std::process::id() {
                continue;
            }
            let Some(p) = sys.process(Pid::from_u32(pid)) else { continue };
            if p.start_time() != start {
                continue; // recycled — not ours any more
            }
            if force {
                p.kill();
            } else if p.kill_with(sysinfo::Signal::Term).is_none() {
                p.kill();
            }
        }
    }
}

/// PID files (`*.pid`, `pid`) in the usual places that were written at or
/// after `since`. Callers still pass each PID through
/// [`ProcessTree::adopt_external`], which checks the process start time.
pub fn fresh_pid_files(project_path: &Path, since: SystemTime) -> Vec<u32> {
    let candidates = [
        project_path.to_path_buf(),
        project_path.join(".codex"),
        project_path.join("data"),
        project_path.join("tmp"),
        project_path.join("pids"),
        project_path.join("run"),
        project_path.join(".run"),
    ];
    let mut pids = Vec::new();
    for dir in candidates {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !(name.ends_with(".pid") || name == "pid") {
                continue;
            }
            let fresh = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|m| m >= since)
                .unwrap_or(false);
            if !fresh {
                continue;
            }
            if let Ok(pid) = std::fs::read_to_string(&path)
                .unwrap_or_default()
                .trim()
                .parse::<u32>()
            {
                if pid != 0 && !pids.contains(&pid) {
                    pids.push(pid);
                }
            }
        }
    }
    pids
}

/// Seconds since the epoch for a `SystemTime`.
pub fn to_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_sleeper() -> std::process::Child {
        #[cfg(windows)]
        let child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 30 127.0.0.1 > NUL"])
            .spawn();
        #[cfg(unix)]
        let child = std::process::Command::new("sh")
            .args(["-c", "sleep 30"])
            .spawn();
        child.expect("spawn sleeper")
    }

    #[test]
    fn keystrokes_parse() {
        assert_eq!(parse_keystrokes("Ctrl+C"), vec![0x03]);
        assert_eq!(parse_keystrokes("^C ^D"), vec![0x03, 0x04]);
        assert!(is_pure_keystroke(" ^C "));
        assert!(!is_pure_keystroke("./stop.sh"));
    }

    #[test]
    fn tree_tracks_descendants_and_kills_them() {
        let mut child = spawn_sleeper();
        let mut tree = ProcessTree::new();
        tree.reset(Some(child.id()));
        // Give the shell a moment to spawn its child.
        std::thread::sleep(Duration::from_millis(500));
        assert!(tree.refresh(&snapshot()));
        assert!(tree.pids().len() >= 2, "expected shell + child, got {:?}", tree.pids());

        tree.kill(true);
        let _ = child.wait();
        let mut alive = true;
        for _ in 0..50 {
            alive = tree.refresh(&snapshot());
            if !alive {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(!alive, "tree should be empty after kill, left: {:?}", tree.pids());
    }

    #[test]
    fn recycled_pid_is_pruned_not_adopted() {
        let mut tree = ProcessTree::new();
        // Our own PID with a bogus start time stands in for a recycled PID.
        let me = std::process::id();
        tree.live.insert(me, 1);
        tree.refresh(&snapshot());
        assert!(!tree.live.contains_key(&me));
    }

    #[test]
    fn stale_external_pid_is_rejected() {
        let mut tree = ProcessTree::new();
        let sys = snapshot();
        // This test process started before "now + 60s".
        tree.adopt_external(&sys, std::process::id(), now_secs() + 60);
        assert!(tree.pids().is_empty());
        tree.adopt_external(&sys, std::process::id(), 0);
        assert!(!tree.pids().is_empty());
    }
}
