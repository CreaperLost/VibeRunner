#!/usr/bin/env bash
# Build the polished .dmg for VibeRunner on macOS.
#
# Pipeline:
#   1. Locates the just-built VibeRunner.app
#      (scripts/build-app.sh produces it; release/universal targets are
#       searched, newest wins)
#   2. Removes Tauri's default .dmg (the unpolished one)
#   3. Runs scripts/make-dmg.sh to lay out a branded Finder window
#      with a custom background, .app on the left, Applications alias
#      on the right, and a compressed read-only image.
#
# Produces:
#   - src-tauri/target/release/bundle/dmg/VibeRunner_0.1.0_<arch>.dmg
#
# Usage:
#   scripts/build-dmg.sh
#
# Equivalent to: `pnpm dmg:custom`
# For the full pipeline (.app + polished .dmg in one go), use
# `pnpm build:mac` or scripts/build-mac.sh.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
cd "$ROOT"

# Fail fast on non-macOS — .dmg is a macOS-only format.
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: build-dmg.sh must be run on macOS (got $(uname -s))" >&2
  exit 1
fi

command -v pnpm >/dev/null 2>&1 || { echo "error: pnpm not found on PATH" >&2; exit 1; }

# Background image is required by make-dmg.sh; bail early with a
# helpful pointer if it's missing.
if [[ ! -f assets/dmg-background.png ]]; then
  echo "error: assets/dmg-background.png not found" >&2
  echo "       run: pnpm dmg:background:regen" >&2
  exit 1
fi

echo "▶ Packaging polished .dmg from existing VibeRunner.app..."
pnpm dmg:custom

DMG_PATH="src-tauri/target/release/bundle/dmg"
DMG_FILE="$(ls -1t "$DMG_PATH"/VibeRunner_*.dmg 2>/dev/null | head -1 || true)"
if [[ -z "$DMG_FILE" ]]; then
  echo "error: no .dmg found under $DMG_PATH after build" >&2
  exit 1
fi

echo
echo "✓ Polished .dmg ready:"
echo "    $ROOT/$DMG_FILE"