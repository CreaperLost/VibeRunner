//! Auto-discover installable / portable build artifacts in a project
//! folder.
//!
//! The frontend surfaces "Install" and "Run Portable" buttons when
//! artifacts are found, so the user can launch the just-built
//! `.app` / `.exe` / `.AppImage` without opening Finder.
//!
//! ## OS filter
//!
//! Only artifacts relevant to the running OS are considered:
//! - macOS:   install = `.dmg`,                portable = `.app`
//! - Windows: install = `.msi`,                portable = `.exe`
//! - Linux:   install = `.deb` / `.rpm`,       portable = `.appimage`
//!
//! An `.exe` on macOS or a `.dmg` on Windows is irrelevant and ignored.
//!
//! ## Ranking (highest wins, ties broken by mtime desc)
//!
//! 1. Inside a path component named `release` / `prod` / `production`
//! 2. Inside a path component named `debug` / `dev`
//! 3. Anything else
//!
//! Only one `Install` and one `Run Portable` are returned — the
//! top-ranked candidate for each kind.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

/// What kind of button the artifact powers.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    /// Distributable installer (.dmg / .msi / .deb / .rpm).
    Install,
    /// Already-built, ready-to-run binary (.app / .exe / .AppImage).
    Portable,
}

/// A single discovered artifact.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectArtifact {
    pub kind: ArtifactKind,
    pub path: String,
    pub display_name: String,
    /// Parent directory (for UI hints like "in target/release/bundle/macos").
    pub parent: String,
    pub size_bytes: u64,
    pub modified_ms: u128,
}

/// Top-ranked Install + Portable for a project (or neither).
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactsScan {
    pub install: Option<ProjectArtifact>,
    pub portable: Option<ProjectArtifact>,
}

/// Folder classification used for ranking. `Release > Debug > Other`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Priority {
    Other = 0,
    Debug = 1,
    Release = 2,
}

/// Cap recursion depth so a wildly deep repo can't stall the UI.
const MAX_DEPTH: usize = 8;

/// Directory names we never recurse into. None of these can contain a
/// `.dmg` / `.app` / `.exe` we'd care about; skipping them keeps the
/// scan fast on real-world repos.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".cache",
    ".npm",
    ".next",
    ".parcel-cache",
    ".turbo",
    ".vite",
    ".fingerprint", // cargo metadata
];

/// Scan `project_path` for build artifacts. Returns at most one Install
/// and one Portable, the highest-ranked match for each kind.
///
/// Safe to call from the Tauri command thread — does no I/O outside
/// the project tree, swallows per-entry errors, and never panics on
/// permission failures / / broken symlinks.
pub fn scan_project_artifacts(project_path: &Path) -> ArtifactsScan {
    let (install_exts, portable_exts): (&[&str], &[&str]) = match std::env::consts::OS {
        "macos" => (&["dmg"], &["app"]),
        "windows" => (&["msi"], &["exe"]),
        "linux" => (&["deb", "rpm"], &["appimage"]),
        _ => (&[], &[]),
    };

    if install_exts.is_empty() && portable_exts.is_empty() {
        return ArtifactsScan::default();
    }

    let mut installs: Vec<ProjectArtifact> = Vec::new();
    let mut portables: Vec<ProjectArtifact> = Vec::new();

    walk(
        project_path,
        0,
        &mut |entry: &Path| {
            let Some(artifact) = classify_entry(entry, install_exts, portable_exts) else {
                return;
            };
            match artifact.kind {
                ArtifactKind::Install => installs.push(artifact),
                ArtifactKind::Portable => portables.push(artifact),
            }
        },
    );

    // Sort: higher-priority folder first; within the same priority,
    // newer mtime first. We pass paths in directly to avoid storing
    // the priority on the wire.
    let sort_key = |a: &ProjectArtifact| -> (Priority, u128) {
        (
            classify(&PathBuf::from(&a.path)),
            // Negate the sort direction: newer → larger value, so
            // standard `Ord` puts it first. Done in the comparator
            // below for clarity.
            a.modified_ms,
        )
    };
    let cmp = |a: &ProjectArtifact, b: &ProjectArtifact| {
        let (pa, ma) = sort_key(a);
        let (pb, mb) = sort_key(b);
        pb.cmp(&pa).then_with(|| mb.cmp(&ma))
    };
    installs.sort_by(cmp);
    portables.sort_by(cmp);

    ArtifactsScan {
        install: installs.into_iter().next(),
        portable: portables.into_iter().next(),
    }
}

/// Inspect one path and turn it into an artifact if its extension
/// matches a tracked kind. Handles `.app` bundles (directories on
/// macOS) the same as plain files.
fn classify_entry(
    entry: &Path,
    install_exts: &[&str],
    portable_exts: &[&str],
) -> Option<ProjectArtifact> {
    let file_name = entry.file_name()?.to_str()?;
    let lower = file_name.to_lowercase();

    // Match either a real file with the right extension, OR an .app
    // bundle (which is a directory on macOS).
    let kind = if lower.ends_with(".app") {
        ArtifactKind::Portable
    } else {
        let ext = entry.extension().and_then(|e| e.to_str())?.to_lowercase();
        if install_exts.contains(&ext.as_str()) {
            ArtifactKind::Install
        } else if portable_exts.contains(&ext.as_str()) {
            ArtifactKind::Portable
        } else {
            return None;
        }
    };

    let meta = entry.metadata().ok()?;
    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);

    let parent = entry
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    Some(ProjectArtifact {
        kind,
        path: entry.to_string_lossy().to_string(),
        display_name: file_name.to_string(),
        parent,
        size_bytes: meta.len(),
        modified_ms,
    })
}

/// Return the highest-priority folder component in `path`. Order
/// matters: if both "release" and "debug" appear in the same path
/// (unusual but possible), the first match wins, which is fine for
/// ranking — release still beats debug.
fn classify(path: &Path) -> Priority {
    let mut best = Priority::Other;
    for component in path.components() {
        if let Some(name) = component.as_os_str().to_str() {
            match name.to_lowercase().as_str() {
                "release" | "prod" | "production" => return Priority::Release,
                "debug" | "dev" => {
                    if best == Priority::Other {
                        best = Priority::Debug;
                    }
                }
                _ => {}
            }
        }
    }
    best
}

/// Recursive directory walker. Skips symlinks (no loops / / no escapes
/// out of the project tree) and a curated set of irrelevant dirs.
fn walk<F: FnMut(&Path)>(dir: &Path, depth: usize, visit: &mut F) {
    if depth > MAX_DEPTH {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return, // unreadable dir — give up silently
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };

        if file_type.is_symlink() {
            continue;
        }

        if file_type.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            // Treat `.app` bundles as opaque artifacts — don't recurse
            // into their Contents/MacOS subtree. The walker will pick
            // up the bundle itself via `classify_entry`.
            if name.to_lowercase().ends_with(".app") {
                visit(&path);
                continue;
            }
            walk(&path, depth + 1, visit);
        } else if file_type.is_file() {
            visit(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Make a unique tempdir under the system temp.
    fn tempdir(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("vr-artifacts-{label}-{pid}-{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn macos_picks_dmg_and_app() {
        if std::env::consts::OS != "macos" {
            return; // behavior is OS-conditional; verified manually on mac
        }
        let root = tempdir("macos");
        let rel = root.join("target").join("release").join("bundle");
        fs::create_dir_all(&rel).unwrap();
        fs::write(rel.join("VibeRunner_0.1.0_aarch64.dmg"), b"dmg").unwrap();
        fs::create_dir_all(rel.join("macos")).unwrap();
        // .app is a directory bundle
        fs::create_dir_all(rel.join("macos").join("VibeRunner.app")).unwrap();

        let scan = scan_project_artifacts(&root);
        assert!(scan.install.is_some(), "expected dmg install");
        assert!(scan.portable.is_some(), "expected app portable");
        assert!(scan.install.unwrap().path.ends_with(".dmg"));
        assert!(scan.portable.unwrap().path.ends_with(".app"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn release_outranks_debug() {
        if std::env::consts::OS != "macos" {
            return;
        }
        let root = tempdir("rank");
        let dbg = root.join("target").join("debug").join("bundle").join("macos");
        let rel = root.join("target").join("release").join("bundle").join("macos");
        fs::create_dir_all(&dbg).unwrap();
        fs::create_dir_all(&rel).unwrap();
        fs::write(dbg.join("Older.dmg"), b"d").unwrap();
        fs::write(rel.join("Newer.dmg"), b"d").unwrap();

        let scan = scan_project_artifacts(&root);
        let chosen = scan.install.expect("install present");
        assert!(
            chosen.path.contains("release"),
            "release folder should outrank debug, got: {}",
            chosen.path
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn same_folder_picks_newer_mtime() {
        if std::env::consts::OS != "macos" {
            return;
        }
        let root = tempdir("mtime");
        let dir = root.join("bundle");
        fs::create_dir_all(&dir).unwrap();
        let older = dir.join("Older.dmg");
        let newer = dir.join("Newer.dmg");
        fs::write(&older, b"x").unwrap();
        // Ensure mtimes differ by a margin the FS can resolve.
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(&newer, b"x").unwrap();

        let scan = scan_project_artifacts(&root);
        let chosen = scan.install.expect("install present");
        assert!(
            chosen.path.ends_with("Newer.dmg"),
            "newer file should win same-folder tie, got: {}",
            chosen.path
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn skips_node_modules_and_git() {
        if std::env::consts::OS != "macos" {
            return;
        }
        let root = tempdir("skip");
        let nm = root.join("node_modules");
        let gt = root.join(".git");
        fs::create_dir_all(&nm).unwrap();
        fs::create_dir_all(&gt).unwrap();
        fs::write(nm.join("Sneaky.dmg"), b"x").unwrap();
        fs::write(gt.join("Sneaky2.dmg"), b"x").unwrap();

        let scan = scan_project_artifacts(&root);
        assert!(scan.install.is_none(), "should ignore skipped dirs");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn only_returns_one_per_kind() {
        if std::env::consts::OS != "macos" {
            return;
        }
        let root = tempdir("one");
        let dir = root.join("out");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("First.dmg"), b"x").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(dir.join("Second.dmg"), b"x").unwrap();

        let scan = scan_project_artifacts(&root);
        let chosen = scan.install.expect("install present");
        // Both are in the same folder → newer (Second) wins.
        assert!(chosen.path.ends_with("Second.dmg"));

        fs::remove_dir_all(&root).ok();
    }
}