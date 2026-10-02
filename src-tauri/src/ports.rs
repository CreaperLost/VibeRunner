//! Cross-platform TCP port detection for a run.
//!
//! Ports are attributed to a project **only** through its identity-checked
//! process tree (see `process::ProcessTree`). The old "any process whose
//! cwd or command line mentions the project path" heuristic pulled in
//! VS Code, terminals and language servers, so their ports showed up as
//! the project's — and auto-open launched browser tabs for them.
//!
//! - macOS / Linux: one `lsof -a -iTCP -sTCP:LISTEN -P -n -p <pids>` call.
//! - Windows:       one `netstat -ano` call, filtered by PID in process.
//!
//! URLs printed in the run's own output are kept as *hints*: they are
//! shown only while something is actually accepting connections on them
//! (this covers e.g. a container port-forward the tree doesn't own).

use std::collections::HashSet;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// Listening TCP ports owned by any of `pids` (no tree walk — callers
/// pass the full tracked tree).
pub fn listening_ports(pids: &[u32]) -> Vec<u16> {
    let set: HashSet<u32> = pids.iter().copied().filter(|&p| p != 0).collect();
    if set.is_empty() {
        return Vec::new();
    }
    #[cfg(unix)]
    {
        let pid_list = set.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        match lsof_command()
            .args(["-a", "-iTCP", "-sTCP:LISTEN", "-P", "-n", "-p", &pid_list])
            .output()
        {
            // lsof exits 1 when nothing matched; stdout is still valid.
            Ok(o) => parse_lsof(&String::from_utf8_lossy(&o.stdout)),
            Err(_) => Vec::new(),
        }
    }
    #[cfg(windows)]
    {
        windows_netstat(&set)
    }
}

/// True if something accepts TCP connections on `port` on loopback.
pub fn is_listening_local(port: u16) -> bool {
    let timeout = Duration::from_millis(120);
    ["127.0.0.1", "[::1]"].iter().any(|host| {
        format!("{host}:{port}")
            .parse::<SocketAddr>()
            .ok()
            .map(|addr| TcpStream::connect_timeout(&addr, timeout).is_ok())
            .unwrap_or(false)
    })
}

#[cfg(unix)]
fn lsof_command() -> std::process::Command {
    if std::path::Path::new("/usr/sbin/lsof").exists() {
        std::process::Command::new("/usr/sbin/lsof")
    } else {
        std::process::Command::new("lsof")
    }
}

/// Strip ANSI escape sequences (CSI and OSC codes) from terminal output.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    while let Some(&ch) = chars.peek() {
                        chars.next();
                        if ch.is_ascii_alphabetic() || ch == '~' {
                            break;
                        }
                    }
                    continue;
                }
                Some(']') => {
                    chars.next();
                    while let Some(ch) = chars.next() {
                        if ch == '\x07' || ch == '\x1b' {
                            break;
                        }
                    }
                    continue;
                }
                Some('(') | Some(')') | Some('#') | Some('%') => {
                    chars.next();
                    chars.next();
                    continue;
                }
                _ => continue,
            }
        }
        out.push(c);
    }
    out
}

const LOCAL_HOSTS: &[&str] = &["localhost", "127.0.0.1", "0.0.0.0", "[::1]", "[::]"];

/// Extract app ports announced in process output. Deliberately narrow:
///   - `http(s)://<local host>:PORT` URLs (vite, next, uvicorn, …)
///   - "… on port PORT" phrases ("listening on port 3000")
///
/// Bare `host:port` text, timestamps, `file.rs:120:5` locations and
/// words that merely contain "port" (report, support, import) are not
/// ports and are ignored.
pub fn extract_ports_from_text(text: &str) -> Vec<u16> {
    let clean = strip_ansi(text);
    let mut ports = Vec::new();
    let mut push = |p: u16| {
        if p >= 80 && !ports.contains(&p) {
            ports.push(p);
        }
    };

    for line in clean.lines() {
        // 1. Local URLs.
        let mut rem = line;
        while let Some(idx) = rem.find("://") {
            let scheme_ok = rem[..idx].ends_with("http") || rem[..idx].ends_with("https");
            let after = &rem[idx + 3..];
            if scheme_ok {
                for host in LOCAL_HOSTS {
                    if let Some(rest) = after.strip_prefix(host) {
                        if let Some(port_str) = rest.strip_prefix(':') {
                            let digits: String =
                                port_str.chars().take_while(|c| c.is_ascii_digit()).collect();
                            if let Ok(p) = digits.parse::<u16>() {
                                push(p);
                            }
                        }
                        break;
                    }
                }
            }
            rem = after;
        }

        // 2. "on port N".
        let lower = line.to_ascii_lowercase();
        let mut kw_rem = lower.as_str();
        while let Some(idx) = kw_rem.find("on port") {
            let after = kw_rem[idx + 7..].trim_start_matches([' ', ':', '=']);
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(p) = digits.parse::<u16>() {
                if p >= 1024 {
                    push(p);
                }
            }
            kw_rem = &kw_rem[idx + 7..];
        }
    }

    ports
}


/// Run `netstat -ano` **once** and return every listening port owned by
/// any PID in `pids`.
///
/// `netstat -ano` reports the owning PID for every listening socket on
/// the machine, so one call covers the whole process tree. We used to
/// shell out once per PID instead, which meant a process spawn per
/// tracked process on every 1.5s poll tick — and each of those spawns
/// popped a console window on Windows.
#[cfg(windows)]
fn windows_netstat(pids: &HashSet<u32>) -> Vec<u16> {
    if pids.is_empty() {
        return Vec::new();
    }
    let output = crate::process::hidden_command("netstat")
        .arg("-ano")
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            parse_netstat_many(stdout.as_ref(), pids)
        }
        _ => Vec::new(),
    }
}

/// `lsof` output looks like (one process per line, NAME is the last column):
///
///   COMMAND  PID   USER   FD   TYPE  DEVICE SIZE/OFF NODE NAME
///   node    12345  user   22u  IPv4  abcdef      0t0  TCP *:4000 (LISTEN)
///
/// We skip the header row and pull the port off the end of each line.
// Only the Unix `lsof` branch calls this, but the unit test below
// exercises it on every platform, so keep it compiled everywhere.
#[allow(dead_code)]
fn parse_lsof(output: &str) -> Vec<u16> {
    let mut ports = Vec::new();
    for (i, line) in output.lines().enumerate() {
        if i == 0 {
            continue;
        }
        if let Some(port) = extract_port(line) {
            if !ports.contains(&port) {
                ports.push(port);
            }
        }
    }
    ports
}

/// `netstat -ano` lines look like (whitespace-separated, 5+ columns):
///
///   TCP    0.0.0.0:4000    0.0.0.0:0    LISTENING    12345
///   TCP    [::]:4000       [::]:0        LISTENING    12345
#[allow(dead_code)]
fn parse_netstat(output: &str, target_pid: u32) -> Vec<u16> {
    parse_netstat_many(output, &HashSet::from([target_pid]))
}

/// Same as [`parse_netstat`] but matches any PID in `pids`, so a single
/// `netstat -ano` capture can serve a whole process tree.
fn parse_netstat_many(output: &str, pids: &HashSet<u32>) -> Vec<u16> {
    let mut ports = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 || !parts[0].eq_ignore_ascii_case("TCP") {
            continue;
        }
        // The state column is localized ("LISTENING", "ABHÖREN", …), so
        // identify listeners by their wildcard foreign address instead.
        let foreign = parts[2];
        if !(foreign.ends_with(":0") || foreign.ends_with(":*")) {
            continue;
        }
        match parts[parts.len() - 1].parse::<u32>() {
            Ok(p) if pids.contains(&p) => {
                if let Some(port) = extract_port(parts[1]) {
                    if port != 0 && !ports.contains(&port) {
                        ports.push(port);
                    }
                }
            }
            _ => {}
        }
    }
    ports
}

/// Pull the trailing port number off the end of an address like
/// `*:4000`, `127.0.0.1:4000`, or `[::]:4000`. Returns None if no
/// port-like suffix is found.
fn extract_port(s: &str) -> Option<u16> {
    let idx = s.rfind(':')?;
    let after = &s[idx + 1..];
    let digits: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_port_works() {
        assert_eq!(extract_port("*:4000 (LISTEN)"), Some(4000));
        assert_eq!(extract_port("127.0.0.1:8080"), Some(8080));
        assert_eq!(extract_port("[::]:3000"), Some(3000));
        assert_eq!(extract_port("0.0.0.0:0"), Some(0));
        assert_eq!(extract_port("no port here"), None);
        assert_eq!(extract_port(""), None);
    }

    #[test]
    fn parse_lsof_skips_header_and_dedupes() {
        let sample = "\
COMMAND  PID   USER   FD   TYPE  DEVICE SIZE/OFF NODE NAME
node    12345 user   22u  IPv4  0t0     TCP *:4000 (LISTEN)
node    12345 user   23u  IPv6  0t0     TCP [::]:4000 (LISTEN)
node    12345 user   24u  IPv4  0t0     TCP 127.0.0.1:8080 (LISTEN)
";
        let ports = parse_lsof(sample);
        assert_eq!(ports, vec![4000, 8080]);
    }

    #[test]
    fn parse_netstat_filters_by_pid_and_state() {
        let sample = "\
  TCP    0.0.0.0:4000    0.0.0.0:0    LISTENING    12345
  TCP    0.0.0.0:5000    0.0.0.0:0    LISTENING    99999
  TCP    127.0.0.1:6000  127.0.0.1:52000  ESTABLISHED  12345
  TCP    [::]:4000       [::]:0        LISTENING    12345
";
        let ports = parse_netstat(sample, 12345);
        assert_eq!(ports, vec![4000]);
    }

    /// Find a Python interpreter that actually works.
    ///
    /// On Windows `python3` is very often the Microsoft Store *App
    /// Execution Alias* stub: it sits on PATH, `spawn()` succeeds, and
    /// then it immediately exits with "Python was not found". Probing
    /// `--version` first keeps this test from failing for a reason that
    /// has nothing to do with port detection.
    fn working_python() -> Option<&'static str> {
        for candidate in ["python3", "python"] {
            let works = std::process::Command::new(candidate)
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if works {
                return Some(candidate);
            }
        }
        None
    }

    #[test]
    fn test_detect_ports_live() {
        use std::process::Command;
        let Some(python) = working_python() else {
            eprintln!("skipping test_detect_ports_live: no working python3/python on PATH");
            return;
        };
        let mut child = Command::new(python)
            .args(["-m", "http.server", "9871"])
            .spawn()
            .expect("Failed to spawn python http.server");
        let pid = child.id();
        let mut ports = Vec::new();
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            ports = listening_ports(&[pid]);
            if ports.contains(&9871) {
                break;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(ports.contains(&9871), "Expected 9871 in detected ports, got: {:?}", ports);
    }

    #[test]
    fn test_extract_ports_from_text() {
        let sample1 = "Started X-Automation (PID 45658).\nhttp://localhost:8501";
        assert_eq!(extract_ports_from_text(sample1), vec![8501]);

        let sample2 = "✨ AI Thumbnail Studio is ready!\n   👉 Open in browser: http://localhost:5173\n   👉 API Docs: http://localhost:8000/docs";
        assert_eq!(extract_ports_from_text(sample2), vec![5173, 8000]);

        let sample3 = "  ➜  Local:   http://localhost:5173/\n  ➜  Network: use --host to expose";
        assert_eq!(extract_ports_from_text(sample3), vec![5173]);

        let sample4 = "2026-09-07 21:04:00.798 Uvicorn server started on 127.0.0.1:8501\nURL: http://127.0.0.1:8501";
        assert_eq!(extract_ports_from_text(sample4), vec![8501]);

        let sample5 = "Server listening on port 3000";
        assert_eq!(extract_ports_from_text(sample5), vec![3000]);

        // Vite with ANSI bold and color codes (AeroShoot.AI scenario)
        let vite_ansi = "  \x1b[32m➜\x1b[39m  \x1b[1mLocal:\x1b[22m   \x1b[36mhttp://127.0.0.1:\x1b[1m1420\x1b[22m/\x1b[39m\n  \x1b[32m➜\x1b[39m  \x1b[1mNetwork:\x1b[22m use \x1b[32m--host\x1b[39m to expose";
        assert_eq!(extract_ports_from_text(vite_ansi), vec![1420]);

        let vite_ansi2 = "  ➜  Local:   \x1b[36mhttp://localhost:\x1b[1m1420\x1b[22m/\x1b[39m";
        assert_eq!(extract_ports_from_text(vite_ansi2), vec![1420]);
    }

    #[test]
    fn extract_ignores_non_port_noise() {
        let noise = r"error[E0425]: src/main.rs:1528:5
12:34:56 report 2048 rows, support 3000 users
postgres://localhost:5432/db
see https://github.com:443/x
Compiling foo v0.1.0 (C:\port 9000)";
        assert!(extract_ports_from_text(noise).is_empty(), "{:?}", extract_ports_from_text(noise));
    }

    #[test]
    fn parse_netstat_handles_localized_state() {
        let sample = "  TCP    0.0.0.0:4000    0.0.0.0:0    ABHÖREN    12345
  TCP    127.0.0.1:50000    127.0.0.1:4000    HERGESTELLT    12345
";
        assert_eq!(parse_netstat(sample, 12345), vec![4000]);
    }
}
