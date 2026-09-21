# VibeRunner

> A cross-platform runner manager for your dev apps — start, stop, restart, and watch logs from one window.

VibeRunner reads a `vibe.config.json` (a list of project folders) and
turns each project into a set of buttons: **Setup / Start / Stop /
Restart** plus whatever custom actions the project declares. If a
project ships a `.codex/environments/environment.toml` (the same
convention used by [codex](https://github.com/openai/codex)), the
actions are auto-discovered. Otherwise you can write the commands
manually.

Targets **Linux**, **macOS**, and **Windows** with a single codebase
(Tauri 2 + React + TypeScript + Rust).

```
┌──────────────────────────────────────────────────────────────┐
│ ▶ VibeRunner    [▶ Run all] [■ Stop all]  ./vibe.config.json  ↻ ↗ ↓│
├──────────────┬───────────────────────────────────────────────┤
│ AssetFlow AI │ AssetFlow AI                                  │
│ ● Stopped    │ id: assetflow · auto (TOML)                   │
│ Run · Stop   │                                               │
│ ~/AssetFlow… │ [🔧 Setup] [▶ Run] [■ Stop] [↻ Restart]       │
│              │                                               │
│ X-Automation │ Path    /Users/.../AssetFlow-AI               │
│ ● Stopped    │                                               │
│ Run · Stop   │ Actions                                       │
│ ~/AXM…       │   Run   ./script/build_and_run.sh             │
│              │   Stop  ./script/build_and_run.sh stop        │
│              │                                               │
│ Static (M)   │ ┌─ Logs ─────────────────────────────────────┐│
│ ● Stopped    │ │ live PTY output streams here               ││
│ Run · Stop   │ └─────────────────────────────────────────────┘│
│              │                                               │
│              │ ┌─ Ports ────────────────────────────────────┐│
│              │ │ open ports listed here, click to open      ││
│              │ └─────────────────────────────────────────────┘│
└──────────────┴───────────────────────────────────────────────┘
```

## Status

This is a working, end-to-end build. What's in:

- ✅ **`vibe.config.json` v2** — list of project folders; JSONC
  accepted on read
- ✅ **Auto-discover from `.codex/environments/environment.toml`** —
  any project that follows the codex convention gets its actions
  picked up automatically (Run / Stop / Setup / etc.)
- ✅ **Manual mode** — projects without a TOML can have their
  setup / run / stop commands declared inline in `vibe.config.json`
- ✅ **`.env` auto-load** — each project's `.env` is injected into
  the spawned PTY's environment automatically
- ✅ **Detached-process tracking** — mark an action as
  `detached: true` and VibeRunner tracks the whole process tree
  (e.g. for `nohup start.sh &` patterns), so the project stays
  "Running" as long as any descendant is alive
- ✅ **Native crash notifications** via `tauri-plugin-notification`
- ✅ **Sidebar search** — ⌘F focuses a filter that searches by
  name, id, path, or action
- ✅ **Open in Finder / Open config** quick actions on each
  project card
- ✅ Sidebar of projects with status pill, action summary, source
  badge (TOML / manual), warnings, and trash button
- ✅ Detail panel: action buttons (one per TOML action + Stop +
  built-in Restart), live PTY log (xterm.js), detected ports
- ✅ **Restart** button = Stop → Setup → primary action, run
  sequentially
- ✅ Start / stop with real PTY; **process-tree kill** (npm-style
  chains die together)
- ✅ Stop escalation: keystroke → SIGTERM → SIGKILL; **Stopping**
  status for instant feedback
- ✅ Port detection via `lsof` (Unix) / `netstat` (Windows),
  polled every 2s, clickable to open (walks the tree for
  detached actions)
- ✅ **Auto-restart on crash** with retry counter, delay, and a
  transient banner
- ✅ **In-app add/remove** projects (modal form with folder picker)
- ✅ **Group actions** — Run all / Stop all in the header
- ✅ **Open config in editor** — one click in the header
- ✅ **JSONC support** — comments and trailing commas accepted on read
- ✅ App icon (1024×1024 source + all platform sizes generated)
- ✅ Bundle metadata (publisher, category, descriptions)

What's deferred (intentionally — the app is fully usable without them):

- **Signed/notarized releases** — costs $99/yr (Apple Developer
  Program) + $0-400/yr (Windows cert). The structure is in place;
  only the money-and-time part is missing. See
  [Release setup](#release-setup) below.
- **CI workflow** — same reason. The local `pnpm tauri build`
  command produces a `.dmg` or `.msi` you can share directly; CI
  is for publishing repeatable builds without you sitting at a Mac.

## Quick start

If you are setting up a development machine or building a release,
see the platform-specific [build guide](BUILDING.md) first.

```bash
pnpm install
pnpm tauri dev
```

The app launches and reads `vibe.config.json` from the current
working directory. A starter config is included — point each
project at a folder on your disk, save the file, and hit
**↻ Reload** (or let the file watcher pick it up automatically).

If the folder you point at has a
`.codex/environments/environment.toml` in it, VibeRunner reads the
`[[actions]]` and `[setup]` from there. Otherwise you can declare
the actions inline in `vibe.config.json` (the "static server"
project in the sample demonstrates this).

## Configuration

`vibe.config.json` (v2 schema, JSONC accepted):

```jsonc
{
  // VibeRunner v2: list of project folders.
  "version": 2,
  "projects": [
    // Auto-discover: VibeRunner reads
    //   <path>/.codex/environments/environment.toml
    // and turns its `[[actions]]` + `[setup].script` into buttons.
    {
      "id": "assetflow",
      "path": "/Users/you/projects/AssetFlow-AI"
    },

    // Manual: no TOML, you write the actions here.
    {
      "id": "static",
      "name": "Static File Server",
      "path": "/Users/you/projects/static",
      "manual": true,
      "setup": { "command": "" },
      "actions": [
        { "name": "Run",  "icon": "run",  "command": "python3 -m http.server 8080" },
        { "name": "Stop", "icon": "stop", "command": "Ctrl+C" }
      ]
    }
  ]
}
```

### The TOML convention (auto-discover)

If a project has `.codex/environments/environment.toml`, VibeRunner
reads it and turns each `[[actions]]` entry into a button. Example
(`AssetFlow-AI/.codex/environments/environment.toml`):

```toml
# THIS IS AUTOGENERATED. DO NOT EDIT MANUALLY
version = 1
name = "AssetFlow AI"

[setup]
script = ""

[[actions]]
name = "Run"
icon = "run"
command = "./script/build_and_run.sh"

[[actions]]
name = "Stop"
icon = "stop"
command = "./script/build_and_run.sh stop"
```

The `Setup` button is automatically prepended when `[setup].script`
is set and non-empty.

### Fields

| Field | Type | Required | Notes |
|---|---|---|---|
| `version` | number | yes | Must be `2` |
| `projects` | array | no | Empty array is fine |
| `project.id` | string | yes | Unique within the file |
| `project.path` | string | yes | Absolute path to the project folder |
| `project.name` | string | no | Display name; defaults to TOML's `name` or `id` |
| `project.primaryAction` | string | no | Action the built-in **Restart** runs. Defaults to first action with `name="Run"` or `icon="run"`. |
| `project.manual` | boolean | no | Skip TOML discovery; use inline `actions` + `setup` |
| `project.setup` | object | no | Inline `{ "command": "..." }` for manual projects |
| `project.actions` | array | no | Inline `[{ name, icon?, command }]` for manual projects |
| `project.env` | object | no | Extra env vars (manual projects) — overrides `.env` |
| `project.autoRestart` | object | no | `{ enabled, maxRetries, delayMs }` (manual projects) |

### TOML fields (auto-discovered)

| Field | Type | Notes |
|---|---|---|
| `name` | string | Used as the project's display name |
| `[setup].script` | string | The setup command; becomes a "Setup" button |
| `[[actions]]` | array | Each entry becomes a button (Run / Stop / custom) |
| `actions[].name` | string | Button label |
| `actions[].icon` | string | Optional; one of `run`, `stop`, `tool`, `build`, `test`, `migrate` |
| `actions[].command` | string | The shell command (runs through `sh -c` / `cmd /C`) |
| `actions[].detached` | bool | If `true`, VibeRunner tracks the whole process tree (for `nohup start.sh &` patterns) |

### How `.env` is loaded

When a project's PTY is spawned, VibeRunner reads
`<project>/.env` (if present) and injects its `KEY=VALUE` pairs
into the process's environment. The order of precedence is:

1. Parent process env (lowest)
2. `<project>/.env`
3. `vibe.config.json` `project.env` (highest — overrides the above)

This means your `.env.example` ships with the project and works
out of the box, but you can override any key in `vibe.config.json`
if you need to.

### Resolution rules

- **Where is the config loaded from?**
  1. `$VIBE_CONFIG` env var (if set, points directly to a file)
  2. Walk up from the current working directory looking for `vibe.config.json`
  3. Walk up from the executable's directory
- **Duplicate `id`** values are rejected at load time.
- **Empty `path`** is rejected at load time.

## Commands

All commands live in `package.json`. Run with `pnpm <name>`.

### Verify a clean checkout

```bash
pnpm install            # JS deps
pnpm tauri:info         # toolchain + package versions
pnpm typecheck          # tsc --noEmit
pnpm test:rust          # cargo test --lib (14 tests)
pnpm test               # typecheck + test:rust
pnpm clean              # remove frontend dist/ and temp build files
pnpm clean:all          # remove dist/, src-tauri/target/, and generated schemas
pnpm verify             # test + tauri build --debug (slow, full pipeline)
```

### Develop

```bash
pnpm tauri dev          # vite + tauri dev, hot reload on both sides
```

### Build for distribution

For native prerequisites, platform-specific packaging notes, and
troubleshooting, see [BUILDING.md](BUILDING.md).

`tauri build` does whatever the current platform produces:

```bash
pnpm build:mac              # on macOS  → src-tauri/target/release/bundle/dmg/*.dmg
pnpm build:mac:universal    # macOS    → universal .dmg (x86_64 + arm64)
pnpm build:win              # on Win    → NSIS setup .exe + .msi installer
pnpm build:linux            # on Linux  → .deb / .AppImage / .rpm
pnpm build:debug            # unoptimized, faster, useful for testing the pipeline
```

The produced bundle lives under `src-tauri/target/release/bundle/`. On
Windows, use the NSIS file in `bundle/nsis/` as the normal distributable
installer; an MSI is also written to `bundle/msi/`. See the
[step-by-step Windows instructions](BUILDING.md#windows) for prerequisites,
exact output paths, testing, and signing notes.

**Cross-compilation note:** Building Windows `.msi` from macOS works
for simple projects but Tauri uses platform-specific tools that
don't always cooperate. Easiest path: build each platform on its
own machine, or on a CI runner with that OS.

### Tooling

```bash
pnpm icon                  # regenerate all icon sizes from assets/icon-source.png
pnpm icon:regen            # regenerate the source PNG, then re-derive all sizes
```

## Architecture

```
VibeRunner/
├── vibe.config.json            # user-editable source of truth
├── assets/icon-source.png      # 1024×1024 mark; `pnpm icon` produces all sizes
├── scripts/generate-icon.py    # PIL script that builds icon-source.png
├── package.json                # all build commands
├── src/                        # React + TS frontend
│   ├── App.tsx                 # shell, state, event subscriptions
│   ├── types.ts                # wire types (mirror Rust events/config)
│   ├── hooks/
│   │   └── useRunnerEvents.ts  # status / output / restarting subscriptions
│   └── components/
│       ├── Sidebar.tsx
│       ├── ProjectCard.tsx
│       ├── ProjectDetail.tsx
│       ├── ProjectForm.tsx     # add-project modal (with folder picker)
│       ├── StatusPill.tsx
│       ├── LogViewer.tsx       # xterm.js wrapper
│       └── PortList.tsx
└── src-tauri/                  # Rust backend
    └── src/
        ├── main.rs             # bin entry
        ├── lib.rs              # Tauri builder, command registration
        ├── config.rs           # VibeConfig / ProjectConfig / ResolvedProject,
        │                       # JSONC parse, TOML auto-discovery, atomic write
        ├── state.rs            # AppState (config + per-project handles)
        ├── events.rs           # typed Tauri events (project:* + config:reloaded)
        ├── commands.rs         # #[tauri::command] handlers
        ├── pty.rs              # portable-pty wrapper, .env auto-load
        ├── process.rs          # keystroke parser + process-tree kill
        ├── ports.rs            # lsof / netstat parsing
        ├── runner.rs           # RunnerHandle
        └── watcher.rs          # file-system watcher with debounce
```

### Events (Rust → React)

| Event | Payload | When |
|---|---|---|
| `project:status` | `{ id, status, action?, reason? }` | Any status transition; `action` names the action whose PTY is alive |
| `project:output` | `{ id, chunk: number[] }` | Streamed PTY bytes (≤4 KiB chunks) |
| `project:ports` | `{ id, ports: number[] }` | Snapshot every 2s while active |
| `project:restarting` | `{ id, attempt, max, delayMs }` | Auto-restart timer fired |
| `config:reloaded` | `{ config, projects, path }` | File watcher or Add/Remove |

### Commands (React → Rust)

| Command | Purpose |
|---|---|
| `get_config` / `get_config_path` | Inspect the loaded config |
| `list_projects` | Get the resolved projects (TOML + manual merged) |
| `reload_config` | Re-read `vibe.config.json` from disk |
| `add_project` / `remove_project` | Mutate the file (atomic write) |
| `run_action` | Run a named action in a fresh PTY |
| `setup_project` | Run the project's implicit "Setup" action |
| `stop_project` | Kill the current PTY (keystroke → SIGTERM → SIGKILL) |
| `restart_project` | Stop → setup → primary action (sequential) |
| `write_to_pty` | Pipe user keystrokes into the active PTY |

## Caveats

- **For detached processes, set `detached: true` on the action.**
  When the launcher script detaches (e.g. `nohup start.sh &`
  returning after a health check passes), VibeRunner tracks the
  *whole process tree* and keeps the project "Running" as long as
  any descendant is alive. The "Run" action in your project's
  TOML/inline config should be marked `detached: true` if the
  command returns before the app exits.
- **`.env` is loaded automatically** into the spawned PTY's env.
  If your start script also loads it (e.g. via `dotenv` or by
  sourcing it), the values are loaded twice — fine, just be aware.
- **Windows port detection** (`netstat` parsing) is untested —
  developed primarily on macOS.

## Release setup

The app builds an unsigned `.dmg` (mac) or `.msi` (Windows) locally
via `pnpm build:mac` / `pnpm build:win`. You can share these
directly, but:

- **macOS** users will see a "cannot be opened because the
  developer cannot be verified" warning. They can right-click →
  Open to bypass. Smooth first-run requires the **Apple Developer
  Program** ($99/yr), which includes the signing certificate and
  notarization.
- **Windows** users will see a SmartScreen "Unknown publisher"
  warning. Smooth first-run requires a Windows code signing cert
  (~$0 for OSS via SignPath.io, ~$60-400/yr otherwise).

To actually publish signed releases, the missing piece is a CI
workflow that:

1. Builds for each target OS
2. Signs with the appropriate cert (from CI secrets)
3. Publishes to a GitHub Release with `latest.json` next to it

If you want help with the CI workflow when you're ready, ask.

## License

TBD
