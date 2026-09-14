#!/usr/bin/env bash
# Build the VibeRunner .app bundle for macOS.
#
# Pipeline:
#   1. Runs `pnpm build`           — tsc + vite (frontend bundle into dist/)
#   2. Runs `pnpm tauri build`     — cargo release build + Tauri bundler
#
# Produces:
#   - src-tauri/target/release/bundle/macos/VibeRunner.app
#   - (by-product) src-tauri/target/release/bundle/dmg/VibeRunner_*.dmg
#     This is Tauri's default (unpolished) dmg. Run scripts/build-dmg.sh
#     afterwards to replace it with the polished, branded version.
#
# Usage:
#   scripts/build-app.sh
#
# Equivalent to: `pnpm tauri build`
# For the polished .dmg too, use `pnpm build:mac` (or scripts/build-mac.sh).

set -euo pipefail

# Resolve script location so this works regardless of CWD.
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
cd "$ROOT"

# Fail fast with a clear message on non-macOS — Tauri produces
# .app bundles only on macOS. Windows/Linux builds use a different
# target (run `pnpm build:win` / `pnpm build:linux` directly).
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: build-app.sh must be run on macOS (got $(uname -s))" >&2
  echo "       on Windows/Linux use 'pnpm build:win' or 'pnpm build:linux' instead" >&2
  exit 1
fi

# Sanity check: required toolchain.
command -v pnpm >/dev/null 2>&1 || { echo "error: pnpm not found on PATH" >&2; exit 1; }
command -v cargo >/dev/null 2>&1 || { echo "error: cargo not found on PATH" >&2; exit 1; }

echo "▶ Building VibeRunner.app (this runs vite + cargo + Tauri bundler)..."
pnpm tauri build

APP_PATH="src-tauri/target/release/bundle/macos/VibeRunner.app"
if [[ ! -d "$APP_PATH" ]]; then
  echo "error: expected bundle not found at $APP_PATH" >&2
  exit 1
fi

echo
echo "✓ VibeRunner.app ready:"
echo "    $ROOT/$APP_PATH"