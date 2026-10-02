//! Auto-discover installable / portable build artifacts in a project
//! folder.
//!
//! The frontend surfaces "Install" and "Run Portable" buttons when
//! artifacts are found, so the user can launch the just-built
//! `.app` / `.exe` / `.AppImage` without opening a file manager.
//!
//! ## OS filter
//!
//! Only artifacts relevant to the running OS are considered:
//! - macOS:   install = `.dmg` / `.pkg`,  portable = `.app` bundle
//! - Windows: install = `.msi`, NSIS `*-setup.exe`,  portable = `.exe`
//! - Linux:   install = `.deb` / `.rpm`,  portable = `.AppImage`
//!
//! ## Ranking
//!
//! A Rust `target/` holds hundreds of `.exe`s that are not the app —
//! build scripts, test binaries (`name-<16 hex>.exe`), examples. Those
//! directories are skipped and those names excluded. Remaining
//! candidates are scored:
//!
//! - file name starts with the project's product name (from
//!   `tauri.conf.json`, `Cargo.toml`, `package.json`, or the folder name)
//! - inside a `bundle/` folder (Tauri) or a `dist/` / `release/` /
//!   `out/` output folder (electron-builder & co.)
//! - inside a `release` (beats `debug`) path component
//!
//! Ties go to the newest file. The best Install and Portable are
//! returned, plus a few runner-up `others` for the UI's overflow menu.

use std::path::Path;
use std::time::SystemTime;

use serde::Serialize;

/// What kind of button the artifact powers.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    /// Distributable installer.
    Install,
    /// Already-built, ready-to-run app.
    Portable,
}

/// A single discovered artifact.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectArtifact {
    pub kind: ArtifactKind,
    pub path: String,
    pub display_name: String,
    /// Parent directory (for UI hints like "in target/release/bundle/msi").
    pub parent: String,
    pub size_bytes: u64,
    pub modified_ms: u128,
    #[serde(skip)]
    score: i32,
}

/// Top-ranked Install + Portable for a project, plus runners-up.
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactsScan {
    pub install: Option<ProjectArtifact>,
    pub portable: Option<ProjectArtifact>,
    pub others: Vec<ProjectArtifact>,
}

/// Cap recursion depth so a wildly deep repo can't stall the scan.
const MAX_DEPTH: usize = 8;
const MAX_OTHERS: usize = 6;

/// Directory names never worth descending into.
const SKIP_DIRS: &[&str] = &[
    "node_modules", ".git", ".hg", ".svn", ".cache", ".npm", ".pnpm-store", ".yarn",
    ".next", ".nuxt", ".svelte-kit", ".parcel-cache", ".turbo", ".vite", ".venv", "venv",
    "__pycache__", ".gradle", ".idea", ".vscode", "Pods", "vendor", "coverage",
    ".fingerprint",
];

/// Cargo profile-dir children that hold intermediates, not deliverables
/// (`target/release/deps`, `target/debug/build`, Tauri's `nsis`/`wix`
/// working dirs — the finished installers live under `bundle/`).
const SKIP_UNDER_PROFILE: &[&str] = &["deps", "build", "incremental", "examples", "nsis", "wix"];

struct OsRules {
    install: &'static [&'static str],
    portable: &'static [&'static str],
}

fn os_rules() -> Option<OsRules> {
    match std::env::consts::OS {
        "macos" => Some(OsRules { install: &["dmg", "pkg"], portable: &["app"] }),
        "windows" => Some(OsRules { install: &["msi"], portable: &["exe"] }),
        "linux" => Some(OsRules { install: &["deb", "rpm"], portable: &["appimage"] }),
        _ => None,
    }
}

/// Scan `project_path` for build artifacts.
pub fn scan_project_artifacts(project_path: &Path) -> ArtifactsScan {
    let Some(rules) = os_rules() else {
        return ArtifactsScan::default();
    };
    let names = product_names(project_path);

    let mut found: Vec<ProjectArtifact> = Vec::new();
    walk(project_path, 0, &mut |entry: &Path, is_dir: bool| {
        if let Some(a) = classify_entry(project_path, entry, is_dir, &rules, &names) {
            found.push(a);
        }
    });

    found.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| b.modified_ms.cmp(&a.modified_ms)));

    let install_idx = found.iter().position(|a| a.kind == ArtifactKind::Install);
    let portable_idx = found.iter().position(|a| a.kind == ArtifactKind::Portable);
    let install = install_idx.map(|i| found[i].clone());
    let portable = portable_idx.map(|i| found[i].clone());
    let others = found
        .into_iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != install_idx && Some(*i) != portable_idx)
        .map(|(_, a)| a)
        .take(MAX_OTHERS)
        .collect();

    ArtifactsScan { install, portable, others }
}

/// Lowercase alphanumerics only: "VibeRunner_0.1.0" → "viberunner010".
fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

/// Names the project's deliverable is likely called, normalized.
fn product_names(project_path: &Path) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let n = normalize(s);
        if n.len() >= 3 && !names.contains(&n) {
            names.push(n);
        }
    };

    for dir in [project_path.to_path_buf(), project_path.join("src-tauri")] {
        if let Some(v) = read_json(&dir.join("tauri.conf.json")) {
            for key in ["productName", "mainBinaryName"] {
                if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
                    push(s);
                }
            }
        }
        if let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) {
            if let Ok(v) = text.parse::<toml::Table>() {
                if let Some(s) = v.get("package").and_then(|p| p.get("name")).and_then(|x| x.as_str()) {
                    push(s);
                }
            }
        }
    }
    if let Some(v) = read_json(&project_path.join("package.json")) {
        for s in [
            v.get("productName"),
            v.get("build").and_then(|b| b.get("productName")),
            v.get("name"),
        ]
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
        {
            // "@scope/app" → "app"
            push(s.rsplit('/').next().unwrap_or(s));
        }
    }
    if let Some(folder) = project_path.file_name().and_then(|f| f.to_str()) {
        push(folder);
    }
    names
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// `name-0123456789abcdef` — a cargo test / dep binary.
fn has_cargo_hash_suffix(stem: &str) -> bool {
    stem.rsplit_once('-')
        .map(|(_, h)| h.len() == 16 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .unwrap_or(false)
}

fn classify_entry(
    root: &Path,
    entry: &Path,
    is_dir: bool,
    rules: &OsRules,
    names: &[String],
) -> Option<ProjectArtifact> {
    let file_name = entry.file_name()?.to_str()?;
    let ext = entry.extension().and_then(|e| e.to_str())?.to_ascii_lowercase();
    let stem = entry.file_stem().and_then(|s| s.to_str()).unwrap_or(file_name);
    let stem_lower = stem.to_ascii_lowercase();

    // `.app` is a directory bundle; everything else must be a file.
    if (ext == "app") != is_dir {
        return None;
    }

    let parent_name = entry
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let kind = if rules.install.contains(&ext.as_str()) {
        ArtifactKind::Install
    } else if rules.portable.contains(&ext.as_str()) {
        if ext == "exe" {
            if stem_lower.starts_with("build-script")
                || stem_lower.starts_with("unins")
                || stem_lower.contains("uninstall")
                || has_cargo_hash_suffix(&stem_lower)
            {
                return None;
            }
            if parent_name == "nsis" || stem_lower.contains("setup") || stem_lower.contains("installer") {
                ArtifactKind::Install
            } else {
                ArtifactKind::Portable
            }
        } else {
            ArtifactKind::Portable
        }
    } else {
        return None;
    };

    // ---- score ---------------------------------------------------------
    let rel = entry.strip_prefix(root).unwrap_or(entry);
    let components: Vec<String> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let dirs = &components[..components.len().saturating_sub(1)];
    let has = |n: &str| dirs.iter().any(|d| d == n);

    let norm_stem = normalize(stem);
    let name_match = names.iter().any(|n| norm_stem.starts_with(n.as_str()));

    let mut score = 0;
    if name_match {
        score += 100;
    }
    if has("bundle") {
        score += 40;
    } else if dirs.first().is_some_and(|d| matches!(d.as_str(), "dist" | "release" | "out" | "build" | "bin")) {
        score += 20;
    }
    if has("release") || has("prod") || has("production") {
        score += 30;
    } else if has("debug") || has("dev") {
        score += 10;
    }
    // A loose binary with no name match outside any output folder is
    // probably a vendored tool (e.g. `tools/nuget.exe`), not the app.
    if kind == ArtifactKind::Portable && score == 0 {
        return None;
    }

    let meta = entry.metadata().ok()?;
    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);

    Some(ProjectArtifact {
        kind,
        path: entry.to_string_lossy().to_string(),
        display_name: file_name.to_string(),
        parent: entry
            .parent()
            .map(|p| p.strip_prefix(root).unwrap_or(p).to_string_lossy().to_string())
            .unwrap_or_default(),
        size_bytes: if is_dir { dir_size(entry) } else { meta.len() },
        modified_ms,
        score,
    })
}

/// Size of an `.app` bundle (shallow-capped walk).
fn dir_size(dir: &Path) -> u64 {
    fn go(d: &Path, depth: usize) -> u64 {
        if depth > 12 {
            return 0;
        }
        let Ok(entries) = std::fs::read_dir(d) else { return 0 };
        entries
            .flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => go(&e.path(), depth + 1),
                Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => 0,
            })
            .sum()
    }
    go(dir, 0)
}

/// Recursive walker. Skips symlinks, irrelevant dirs, and cargo
/// intermediates. `.app` bundles are reported, not descended into.
fn walk<F: FnMut(&Path, bool)>(dir: &Path, depth: usize, visit: &mut F) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let dir_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_profile_dir = dir_name == "release" || dir_name == "debug";

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_string) else { continue };
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if SKIP_DIRS.contains(&name.as_str())
                || (is_profile_dir && SKIP_UNDER_PROFILE.contains(&name.to_ascii_lowercase().as_str()))
            {
                continue;
            }
            if name.to_ascii_lowercase().ends_with(".app") {
                visit(&path, true);
                continue;
            }
            walk(&path, depth + 1, visit);
        } else if file_type.is_file() {
            visit(&path, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tempdir(label: &str) -> std::path::PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir()
            .join(format!("vr-artifacts-{label}-{}-{n}", std::process::id()))
            .join("MyApp");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if path.extension().and_then(|e| e.to_str()) == Some("app") {
            fs::create_dir_all(path.join("Contents")).unwrap();
        } else {
            fs::write(path, b"x").unwrap();
        }
    }

    fn cleanup(root: &Path) {
        fs::remove_dir_all(root.parent().unwrap()).ok();
    }

    /// (install ext, portable ext) for the current OS.
    fn exts() -> (&'static str, &'static str) {
        match std::env::consts::OS {
            "macos" => ("dmg", "app"),
            "windows" => ("msi", "exe"),
            _ => ("deb", "AppImage"),
        }
    }

    #[test]
    fn picks_bundle_outputs_over_cargo_intermediates() {
        let root = tempdir("cargo");
        let (inst, port) = exts();
        let rel = root.join("src-tauri").join("target").join("release");
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"my-app\"\n").unwrap();

        // Noise a real target/ is full of.
        touch(&rel.join("deps").join(format!("my_app-0123456789abcdef.{port}")));
        touch(&rel.join("build").join("x-1").join(format!("build-script-build.{port}")));
        touch(&rel.join(format!("helper-0123456789abcdef.{port}")));
        touch(&root.join("tools").join(format!("vendored.{port}")));
        // Real deliverables.
        touch(&rel.join("bundle").join(inst).join(format!("MyApp_0.1.0_x64.{inst}")));
        if port == "exe" {
            touch(&rel.join("my-app.exe"));
        } else {
            touch(&rel.join("bundle").join("x").join(format!("MyApp.{port}")));
        }

        let scan = scan_project_artifacts(&root);
        let install = scan.install.expect("install");
        let portable = scan.portable.expect("portable");
        assert!(install.path.contains("bundle"), "{}", install.path);
        assert!(portable.display_name.to_lowercase().starts_with("my"), "{}", portable.path);
        assert!(scan
            .others
            .iter()
            .chain([&install, &portable])
            .all(|a| !a.path.contains("deps") && !a.path.contains("build-script") && !a.path.contains("vendored")));
        cleanup(&root);
    }

    #[test]
    fn release_outranks_debug() {
        let root = tempdir("rank");
        let (inst, _) = exts();
        touch(&root.join("target").join("release").join("bundle").join(format!("MyApp.{inst}")));
        std::thread::sleep(std::time::Duration::from_millis(20));
        touch(&root.join("target").join("debug").join("bundle").join(format!("MyApp.{inst}")));
        let chosen = scan_project_artifacts(&root).install.expect("install");
        assert!(chosen.path.contains("release"), "{}", chosen.path);
        cleanup(&root);
    }

    #[test]
    fn same_rank_picks_newer() {
        let root = tempdir("mtime");
        let (inst, _) = exts();
        touch(&root.join("dist").join(format!("MyApp-1.{inst}")));
        std::thread::sleep(std::time::Duration::from_millis(50));
        touch(&root.join("dist").join(format!("MyApp-2.{inst}")));
        let chosen = scan_project_artifacts(&root).install.expect("install");
        assert!(chosen.display_name.starts_with("MyApp-2"), "{}", chosen.path);
        cleanup(&root);
    }

    #[test]
    fn skips_node_modules_and_git() {
        let root = tempdir("skip");
        let (inst, _) = exts();
        touch(&root.join("node_modules").join(format!("MyApp.{inst}")));
        touch(&root.join(".git").join(format!("MyApp.{inst}")));
        assert!(scan_project_artifacts(&root).install.is_none());
        cleanup(&root);
    }

    #[cfg(windows)]
    #[test]
    fn nsis_setup_exe_is_an_installer() {
        let root = tempdir("nsis");
        touch(&root.join("target/release/bundle/nsis/MyApp_0.1.0_x64-setup.exe"));
        let scan = scan_project_artifacts(&root);
        assert!(scan.install.expect("install").display_name.ends_with("-setup.exe"));
        assert!(scan.portable.is_none());
        cleanup(&root);
    }

    /// `cargo test --lib artifacts::tests::scan_this_repo -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn scan_this_repo() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let t = std::time::Instant::now();
        let scan = scan_project_artifacts(root);
        println!("scanned in {:?}", t.elapsed());
        println!("install:  {:?}", scan.install.map(|a| a.path));
        println!("portable: {:?}", scan.portable.map(|a| a.path));
        for o in scan.others {
            println!("other:    {} ({})", o.path, o.score);
        }
    }

    #[test]
    fn hash_suffix_detection() {
        assert!(has_cargo_hash_suffix("viberunner-0123456789abcdef"));
        assert!(!has_cargo_hash_suffix("viberunner-setup"));
        assert!(!has_cargo_hash_suffix("viberunner"));
    }
}
