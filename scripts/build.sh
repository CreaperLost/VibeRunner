#!/usr/bin/env bash
# VibeRunner validation + frontend build — Linux and macOS.
#
# This is the "Build" button: the fast, every-commit check. It runs the
# same gate AGENTS.md documents as the minimum bar (typecheck + Rust
# tests) and then produces the frontend bundle.
#
# It deliberately does NOT run `pnpm tauri build` — that is a 3-5 minute
# Rust release compile plus bundling. Use the "Package" action for a
# distributable.
#
# Usage:
#   scripts/build.sh

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
cd "$ROOT"

command -v pnpm >/dev/null 2>&1 || {
  echo "error: pnpm not found on PATH" >&2
  exit 1
}

echo "==> typecheck + Rust tests"
pnpm test

echo
echo "==> frontend bundle"
pnpm build

echo
echo "ok: validation passed and dist/ is up to date"
