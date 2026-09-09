#!/usr/bin/env node
/**
 * Clean up build files and artifacts for VibeRunner.
 *
 * Usage:
 *   node scripts/clean.mjs          # cleans frontend dist, temp dirs, tsbuildinfo
 *   node scripts/clean.mjs --all    # also cleans src-tauri/target, gen/schemas, .DS_Store
 */

import { existsSync, readdirSync, rmSync } from "node:fs";
import { resolve, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const root = resolve(here, "..");

const isAll = process.argv.includes("--all");

function remove(relPath, description) {
  const fullPath = resolve(root, relPath);
  if (existsSync(fullPath)) {
    rmSync(fullPath, { recursive: true, force: true });
    console.log(`✓ Removed ${description || relPath}`);
    return true;
  }
  return false;
}

function cleanTempBuildDirs(dir) {
  if (!existsSync(dir)) return;
  try {
    const entries = readdirSync(dir, { withFileTypes: true });
    for (const entry of entries) {
      const full = join(dir, entry.name);
      if (entry.isDirectory() && entry.name.startsWith(".dmg-build.")) {
        rmSync(full, { recursive: true, force: true });
        console.log(`✓ Removed temp build dir: ${entry.name}`);
      }
    }
  } catch {
    // Ignore errors scanning temp dirs
  }
}

console.log(`Cleaning build files (${isAll ? "full" : "standard"})...`);

// 1. Frontend dist directory
remove("dist", "dist/");

// 2. TypeScript build info if any
remove("tsconfig.tsbuildinfo", "tsconfig.tsbuildinfo");

// 3. Stale temp dmg build folders
const targetDmgDir = resolve(root, "src-tauri/target/release/bundle/dmg");
cleanTempBuildDirs(targetDmgDir);
const targetDebugDmgDir = resolve(root, "src-tauri/target/debug/bundle/dmg");
cleanTempBuildDirs(targetDebugDmgDir);

// 4. If --all, clean Rust target and generated schemas
if (isAll) {
  remove("src-tauri/target", "src-tauri/target/");
  remove("src-tauri/gen", "src-tauri/gen/");

  // Remove .DS_Store files in project
  function removeDsStore(dir) {
    if (!existsSync(dir)) return;
    try {
      const entries = readdirSync(dir, { withFileTypes: true });
      for (const entry of entries) {
        if (entry.name === "node_modules" || entry.name === ".git") continue;
        const full = join(dir, entry.name);
        if (entry.isDirectory()) {
          removeDsStore(full);
        } else if (entry.name === ".DS_Store") {
          rmSync(full, { force: true });
        }
      }
    } catch {
      // Ignore
    }
  }
  removeDsStore(root);
  console.log("✓ Removed .DS_Store files");
}

console.log("Cleanup complete.");
