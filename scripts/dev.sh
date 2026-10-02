#!/usr/bin/env bash
# VibeRunner dev launcher — Linux and macOS.
#
#   scripts/dev.sh start   # run `pnpm tauri dev` in the foreground
#   scripts/dev.sh stop    # stop only what a previous `start` launched
#
# Why this exists instead of putting `pnpm tauri dev` straight into
# environment.toml:
#
#   Stop has to be reliable and must not touch processes this launcher
#   did not start. `pnpm tauri dev` spawns vite, cargo, and the app
#   itself; Ctrl+C alone leaves cargo's children alive and the dev port
#   bound, so the next Start fails with "port already in use".
#
#   So `start` records the root PID *and its start time* in
#   .codex/viberunner-dev.state, and `stop` verifies both before
#   signalling anything. The start time is the part that matters: a bare
#   PID is not proof of identity, because the OS recycles PIDs, and
#   without the check a stale state file could eventually kill an
#   unrelated process.
#
# `.codex/*` is gitignored (see .gitignore), so the state file is
# runtime-only.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
STATE="$ROOT/.codex/viberunner-dev.state"

# Read pid/started out of the state file. Returns non-zero if the file
# is missing or incomplete.
read_state() {
  STATE_PID=""
  STATE_STARTED=""
  [ -f "$STATE" ] || return 1
  STATE_PID="$(sed -n 's/^pid=//p' "$STATE" | head -n1)"
  STATE_STARTED="$(sed -n 's/^started=//p' "$STATE" | head -n1)"
  [ -n "$STATE_PID" ] && [ -n "$STATE_STARTED" ]
}

# Echo the recorded PID, but only if it is still the exact process we
# started. Returns non-zero when nothing of ours is left running.
live_pid() {
  read_state || return 1
  kill -0 "$STATE_PID" 2>/dev/null || return 1
  local now
  now="$(ps -o lstart= -p "$STATE_PID" 2>/dev/null | sed 's/^[[:space:]]*//')"
  [ -n "$now" ] || return 1
  if [ "$now" != "$STATE_STARTED" ]; then
    echo "note: pid $STATE_PID now belongs to a different process; ignoring stale state" >&2
    return 1
  fi
  printf '%s' "$STATE_PID"
}

# Signal a process and everything beneath it, deepest first, so a
# parent cannot exit out from under its children mid-walk.
signal_tree() {
  local sig="$1" pid="$2" child
  for child in $(pgrep -P "$pid" 2>/dev/null || true); do
    signal_tree "$sig" "$child"
  done
  kill "-$sig" "$pid" 2>/dev/null || true
}

start_dev() {
  local existing
  if existing="$(live_pid)"; then
    echo "already running (pid $existing) — run 'stop' first" >&2
    return 1
  fi

  command -v pnpm >/dev/null 2>&1 || {
    echo "error: pnpm not found on PATH" >&2
    exit 1
  }

  # Clear any stale state so a later Stop can never target a recycled
  # PID from a previous run.
  rm -f "$STATE"
  mkdir -p "$(dirname "$STATE")"
  printf 'pid=%s\nstarted=%s\n' \
    "$$" "$(ps -o lstart= -p "$$" | sed 's/^[[:space:]]*//')" >"$STATE"

  cd "$ROOT"
  # exec keeps this PID, so the recorded root is the dev process itself.
  exec pnpm tauri dev
}

stop_dev() {
  local target
  if ! target="$(live_pid)"; then
    if [ -f "$STATE" ]; then
      echo "no live dev process recorded; clearing stale state"
    else
      echo "not running"
    fi
    rm -f "$STATE"
    return 0
  fi

  echo "stopping VibeRunner dev tree (root pid $target)..."
  signal_tree TERM "$target"

  # Wait for the tree to actually go away, so that
  # Start -> Stop -> Start is deterministic instead of racy.
  local i
  for i in $(seq 1 50); do
    kill -0 "$target" 2>/dev/null || break
    sleep 0.1
  done
  if kill -0 "$target" 2>/dev/null; then
    echo "still alive after TERM; sending KILL"
    signal_tree KILL "$target"
    sleep 0.3
  fi

  rm -f "$STATE"
  echo "stopped"
}

case "${1:-start}" in
  start) start_dev ;;
  stop) stop_dev ;;
  *)
    echo "usage: $(basename "$0") {start|stop}" >&2
    exit 2
    ;;
esac
