//! Cross-platform TCP port detection for a process.
//!
//! We poll every couple of seconds while a runner is active and emit the
//! list of listening ports as `runner:ports` events. The frontend
//! renders them as clickable links to `http://localhost:<port>`.
//!
//! - macOS / Linux: `lsof -iTCP -sTCP:LISTEN -P -n -p <pid>`.
//! - Windows:       `netstat -ano` filtered by PID + LISTENING state.

#[allow(dead_code)]
pub fn detect_ports(pid: u32) -> Vec<u16> {
    #[cfg(unix)]
    {
        unix_lsof(pid)
    }
    #[cfg(windows)]
    {
        windows_netstat(pid)
    }
}

/// Like [`detect_ports`] but aggregates listening ports from the
/// whole process tree rooted at `root_pid`. Used for "detached"
/// actions where the actual app is a grandchild of the PTY.
#[allow(dead_code)]
pub fn detect_ports_for_tree(root_pid: u32) -> Vec<u16> {
    detect_ports_for_pids(&[root_pid])
}

/// Aggregates listening ports for all given PIDs and their descendants.
pub fn detect_ports_for_pids(pids: &[u32]) -> Vec<u16> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::everything(),
    );

    // Walk the tree.
    let mut tree: Vec<u32> = Vec::new();
    let mut to_visit: Vec<u32> = Vec::new();
    for &p in pids {
        if p != 0 && !tree.contains(&p) {
            tree.push(p);
            to_visit.push(p);
        }
    }

    while let Some(cur) = to_visit.pop() {
        let cur_pid = Pid::from_u32(cur);
        for (child_pid, proc) in sys.processes() {
            if let Some(parent) = proc.parent() {
                if parent == cur_pid && !tree.contains(&child_pid.as_u32()) {
                    let child_u32 = child_pid.as_u32();
                    tree.push(child_u32);
                    to_visit.push(child_u32);
                }
            }
        }
    }

    if tree.is_empty() {
        return Vec::new();
    }

    // On Unix we can ask lsof for all PIDs at once (faster). On
    // Windows we have to call netstat once per PID and merge.
    let mut all_ports: Vec<u16> = Vec::new();
    #[cfg(unix)]
    {
        let pid_list = tree
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        if let Ok(output) = lsof_command()
            .args([
                "-a",
                "-iTCP",
                "-sTCP:LISTEN",
                "-P",
                "-n",
                "-p",
                &pid_list,
            ])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for p in parse_lsof(stdout.as_ref()) {
                    if !all_ports.contains(&p) {
                        all_ports.push(p);
                    }
                }
            }
        }
    }
    #[cfg(windows)]
    {
        for pid in &tree {
            for p in detect_ports(*pid) {
                if !all_ports.contains(&p) {
                    all_ports.push(p);
                }
            }
        }
    }
    all_ports
}

/// Detect listening ports for a project given its root folder and runner PIDs.
/// Aggregates:
/// 1. Listening ports from `pids` and their descendants in the process tree.
/// 2. Listening ports from any active process whose CWD or command line matches `project_path`.
pub fn detect_ports_for_project(project_path: &std::path::Path, pids: &[u32]) -> Vec<u16> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let mut all_ports = detect_ports_for_pids(pids);

    // Also scan all listening ports and see if any listening process belongs to `project_path`
    #[cfg(unix)]
    {
        if let Ok(output) = lsof_command()
            .args(["-iTCP", "-sTCP:LISTEN", "-P", "-n"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut sys = System::new();
                sys.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true,
                    ProcessRefreshKind::everything(),
                );

                for line in stdout.lines().skip(1) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() < 2 {
                        continue;
                    }
                    if let Ok(pid) = parts[1].parse::<u32>() {
                        if let Some(port) = extract_port(line) {
                            if all_ports.contains(&port) {
                                continue;
                            }
                            let path_str = project_path.to_string_lossy();
                            let belongs_to_project = if let Some(proc) = sys.process(Pid::from_u32(pid)) {
                                let cwd_matches = proc.cwd().map(|c| c.starts_with(project_path)).unwrap_or(false);
                                let cmd_matches = proc.cmd().iter().any(|arg| arg.to_string_lossy().contains(path_str.as_ref()));
                                cwd_matches || cmd_matches
                            } else {
                                false
                            };

                            if belongs_to_project {
                                all_ports.push(port);
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(windows)]
    {
        let mut sys = System::new();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::everything(),
        );
        let path_str = project_path.to_string_lossy();
        for (pid, proc) in sys.processes() {
            let pid_u32 = pid.as_u32();
            let cwd_matches = proc.cwd().map(|c| c.starts_with(project_path)).unwrap_or(false);
            let cmd_matches = proc.cmd().iter().any(|arg| arg.to_string_lossy().contains(path_str.as_ref()));
            if cwd_matches || cmd_matches {
                for p in detect_ports(pid_u32) {
                    if !all_ports.contains(&p) {
                        all_ports.push(p);
                    }
                }
            }
        }
    }

    all_ports
}

#[cfg(unix)]
fn lsof_command() -> std::process::Command {
    if std::path::Path::new("/usr/sbin/lsof").exists() {
        std::process::Command::new("/usr/sbin/lsof")
    } else {
        std::process::Command::new("lsof")
    }
}

#[cfg(unix)]
fn unix_lsof(pid: u32) -> Vec<u16> {
    let output = lsof_command()
        .args([
            "-a",
            "-iTCP",
            "-sTCP:LISTEN",
            "-P",  // no service-name translation
            "-n",  // no hostname resolution
            "-p",
            &pid.to_string(),
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            parse_lsof(stdout.as_ref())
        }
        _ => Vec::new(),
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

/// Extract valid TCP listening ports from process stdout / stderr or log output.
/// Matches patterns like:
///   http://localhost:8501
///   http://127.0.0.1:8000
///   Local: http://localhost:5173/
///   URL: http://127.0.0.1:8501
///   port 8080, port: 8080, PORT=8080
pub fn extract_ports_from_text(text: &str) -> Vec<u16> {
    let clean = strip_ansi(text);
    let mut ports = Vec::new();

    for line in clean.lines() {
        // 1. Scan for URLs: http://...:PORT or https://...:PORT
        let mut rem = line;
        while let Some(proto_idx) = rem.find("http://").or_else(|| rem.find("https://")) {
            let after_proto = &rem[proto_idx..];
            let skip = if after_proto.starts_with("https://") { 8 } else { 7 };
            if after_proto.len() > skip {
                let rest = &after_proto[skip..];
                if let Some(colon_idx) = rest.find(':') {
                    let after_colon = &rest[colon_idx + 1..];
                    let digits: String = after_colon
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    if let Ok(p) = digits.parse::<u16>() {
                        if (80..=65535).contains(&p) && !ports.contains(&p) {
                            ports.push(p);
                        }
                    }
                }
            }
            // Advance past this proto
            rem = &after_proto[skip..];
        }

        // 2. Scan for localhost:PORT or 127.0.0.1:PORT or 0.0.0.0:PORT
        for host in &["localhost:", "127.0.0.1:", "0.0.0.0:"] {
            let mut host_rem = line;
            while let Some(idx) = host_rem.find(host) {
                let after_host = &host_rem[idx + host.len()..];
                let digits: String = after_host
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(p) = digits.parse::<u16>() {
                    if (80..=65535).contains(&p) && !ports.contains(&p) {
                        ports.push(p);
                    }
                }
                host_rem = after_host;
            }
        }

        // 3. Scan for port keywords: "port 8080", "port: 8080", "port=8080"
        let lower = line.to_ascii_lowercase();
        for keyword in &["port:", "port=", "port "] {
            let mut kw_rem = lower.as_str();
            while let Some(idx) = kw_rem.find(keyword) {
                let after_kw = kw_rem[idx + keyword.len()..].trim_start();
                let digits: String = after_kw
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(p) = digits.parse::<u16>() {
                    if (1024..=65535).contains(&p) && !ports.contains(&p) {
                        ports.push(p);
                    }
                }
                kw_rem = &kw_rem[idx + keyword.len()..];
            }
        }
    }

    ports
}


#[cfg(windows)]
fn windows_netstat(pid: u32) -> Vec<u16> {
    let output = std::process::Command::new("netstat").arg("-ano").output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            parse_netstat(stdout.as_ref(), pid)
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
    let mut ports = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 {
            continue;
        }
        if parts[3] != "LISTENING" {
            continue;
        }
        match parts[4].parse::<u32>() {
            Ok(p) if p == target_pid => {
                if let Some(port) = extract_port(parts[1]) {
                    if !ports.contains(&port) {
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
  TCP    0.0.0.0:6000    0.0.0.0:0    ESTABLISHED  12345
  TCP    [::]:4000       [::]:0        LISTENING    12345
";
        let ports = parse_netstat(sample, 12345);
        assert_eq!(ports, vec![4000]);
    }

    #[test]
    fn test_detect_ports_live() {
        use std::process::Command;
        let mut child = Command::new("python3")
            .args(["-m", "http.server", "9871"])
            .spawn()
            .expect("Failed to spawn python3");
        let pid = child.id();
        let mut ports = Vec::new();
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            ports = detect_ports_for_pids(&[pid]);
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
    fn test_detect_ports_for_project_portable() {
        let temp = std::env::temp_dir();
        let ports = detect_ports_for_project(&temp, &[]);
        // Must succeed without panicking
        let _ = ports;
    }
}


