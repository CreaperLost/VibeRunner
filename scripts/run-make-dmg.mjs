#!/usr/bin/env node
/**
 * Wrapper around scripts/make-dmg.sh that finds the just-built
 * .app, calls the shell script to produce a polished .dmg, and
 * removes Tauri's default (unpolished) .dmg first.
 *
 * Run automatically by `pnpm build:mac`, or directly with
 * `pnpm dmg:custom`.
 *
 * Exits non-zero if the .app can't be found, so CI / release
 * scripts fail loud rather than silently shipping Tauri's default
 * .dmg.
 */

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, rmSync, statSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

const here = fileURLToPath(new URL(".", import.meta.url));
const root = resolve(here, "..");

// Detect whether we just did a universal, release, or debug build by looking
// for the .app in all target directories, preferring the newest (just built).
const candidates = [
  "src-tauri/target/universal-apple-darwin/release/bundle/macos/VibeRunner.app",
  "src-tauri/target/universal-apple-darwin/debug/bundle/macos/VibeRunner.app",
  "src-tauri/target/release/bundle/macos/VibeRunner.app",
  "src-tauri/target/debug/bundle/macos/VibeRunner.app",
];
const existingCandidates = candidates
  .map((p) => resolve(root, p))
  .filter((p) => existsSync(p));

if (existingCandidates.length === 0) {
  console.error(
    "Could not find VibeRunner.app in target/{universal-apple-darwin,release,debug}/bundle/macos/.\n" +
      "Did you run `pnpm build:mac`, `pnpm build:mac:universal`, or `pnpm build:debug` first?",
  );
  process.exit(1);
}

// Sort by modification time descending to select the one that was just built.
existingCandidates.sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs);
const appBundle = existingCandidates[0];

// Locate the .dmg that tauri would have produced (sibling of the
// .app) so we can delete it before making our own.
const isUniversal = appBundle.includes("universal-apple-darwin");
const arch = isUniversal
  ? "universal"
  : process.arch === "arm64"
    ? "aarch64"
    : "x64";

const dmgDir = resolve(appBundle, "../../dmg");
let defaultDmg = null;
if (existsSync(dmgDir)) {
  const dmgFiles = readdirSync(dmgDir).filter(
    (f) => f.endsWith(".dmg") && f.startsWith("VibeRunner_"),
  );
  if (dmgFiles.length > 0) {
    const matching = dmgFiles.find((f) => f.includes(arch));
    defaultDmg = resolve(dmgDir, matching ?? dmgFiles[0]);
  }
}
if (!defaultDmg) {
  defaultDmg = resolve(dmgDir, `VibeRunner_0.1.0_${arch}.dmg`);
}

if (existsSync(defaultDmg)) {
  rmSync(defaultDmg);
}

const outDmg = defaultDmg; // same path, custom contents

try {
  execFileSync("bash", [resolve(here, "make-dmg.sh"), appBundle, outDmg], {
    stdio: "inherit",
  });
} catch (err) {
  console.error("make-dmg.sh failed:", err.message);
  process.exit(1);
}

console.log(`\nDMG ready: ${outDmg}`);
