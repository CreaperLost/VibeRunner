# AGENTS.md

Guidance for AI agents and new developers working on VibeRunner.
Read this first if you're going to change code.

## What this is

Cross-platform desktop app (Tauri 2 + React 19 + TypeScript + Rust) for
managing dev-app runners from one window. Read `README.md` for the
product story; this file is about how to work on the code.

### The model (read this first)

Each **project** is a folder on disk. VibeRunner can either:

- **Auto-discover** the project's actions from
  `.codex/environments/environment.toml` inside the folder (the
  same convention used by [codex](https://github.com/openai/codex)).
  This file has:
  ```toml
  [setup]
  script = "bash scripts/boot.sh"   # optional

  [[actions]]
  name = "Run"
  icon = "run"
  command = "bash scripts/start.sh"

  [[actions]]
  name = "Stop"
  icon = "stop"
  command = "bash scripts/stop.sh"
  ```
  VibeRunner turns each `[[actions]]` entry into a button.

- **Run manual** commands defined inline in `vibe.config.json` —
  for repos that don't follow the TOML convention.

Either way, every project gets a built-in **Restart** button (Stop →
Setup → the action named "Run" or icon="run", in that order).

`.env` files inside a project are auto-loaded into the spawned PTY's
environment (lower priority than `vibe.config.json` env, higher than
the parent shell's env).

## Project layout

```
VibeRunner/
├── vibe.config.json            # list of projects (each = a folder)
├── assets/
│   └── icon-source.png         # 1024×1024 mark; `pnpm icon` regenerates all sizes
├── scripts/
│   └── generate-icon.py        # PIL script that builds icon-source.png
├── src/                        # React + TS frontend
│   ├── App.tsx                 # shell, state, event subscriptions
│   ├── types.ts                # wire types (mirrors Rust events/config)
│   ├── styles.css              # design system + all component styles
│   ├── hooks/
│   │   └── useRunnerEvents.ts  # status / output / restarting subscriptions
│   └── components/
│       ├── Sidebar.tsx         # project list + + New button
│       ├── ProjectCard.tsx     # one project (name, status, action summary, trash)
│       ├── ProjectDetail.tsx   # right panel: action buttons, config, log, ports
│       ├── ProjectForm.tsx     # modal form for adding a project (auto or manual)
│       ├── StatusPill.tsx      # status badge
│       ├── LogViewer.tsx       # xterm.js wrapper for live PTY output
│       └── PortList.tsx        # clickable localhost:PORT list
├── src-tauri/                  # Rust backend
│   ├── src/
│   │   ├── main.rs             # bin entry
│   │   ├── lib.rs              # Tauri builder, command registration
│   │   ├── config.rs           # VibeConfig / ProjectConfig / ResolvedProject,
│   │   │                       # JSONC parse, TOML auto-discovery, atomic write
│   │   ├── state.rs            # AppState (config + per-project handles)
│   │   ├── events.rs           # typed Tauri events (project:* + config:reloaded)
│   │   ├── commands.rs         # #[tauri::command] handlers
│   │   ├── pty.rs              # portable-pty wrapper, .env auto-load
│   │   ├── process.rs          # keystroke parser + process-tree kill
│   │   ├── ports.rs            # lsof / netstat parsing
│   │   ├── runner.rs           # RunnerHandle (status, I/O, restart count)
│   │   └── watcher.rs          # file-system watcher with debounce
│   ├── capabilities/default.json
│   ├── icons/                  # generated from assets/icon-source.png
│   ├── tauri.conf.json
│   ├── Cargo.toml
│   ├── .tauri-updater-key      # gitignored; private signing key
│   └── .gitignore
├── package.json                # all build commands live here
├── AGENTS.md                   # this file
├── README.md                   # user-facing docs
└── vibe.config.json            # sample projects
```

## Commands (the only ones that matter)

All commands are in `package.json`. Run with `pnpm <name>`.

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
pnpm tauri dev          # vite + tauri dev, hot reload both sides
```

The first dev run downloads + compiles all Rust deps (~3-5 min).
Subsequent runs are fast.

### Build for distribution

`tauri build` does whatever the current platform produces:

```bash
pnpm build:mac          # on macOS → src-tauri/target/release/bundle/dmg/*.dmg
pnpm build:mac:universal  # universal: x86_64 + arm64
pnpm build:win          # on Windows → src-tauri/target/release/bundle/msi/*.msi
pnpm build:linux        # on Linux → .deb / .AppImage / .rpm
pnpm build:debug        # unoptimized, faster, useful for testing the pipeline
```

Note: **cross-compilation is limited.** Building Windows `.msi` from
macOS works for simple projects; Tauri uses platform-specific tools
that don't always play nice. Easiest: build each platform on its own
machine (or a CI runner with that OS).

### Tooling

```bash
pnpm icon               # regenerate all icon sizes from assets/icon-source.png
pnpm icon:regen         # regenerate the source PNG, then re-derive all sizes
```

## Conventions

### Adding a new Tauri command

1. Add the handler to `src-tauri/src/commands.rs` with `#[tauri::command]`
2. Add its name to the `tauri::generate_handler![...]` macro in `lib.rs`
3. Add the wire type to `src/types.ts` (if it has a payload)
4. Call from the frontend: `invoke<T>("your_command", { ... })`

### Adding a new event

1. Add a constant `pub const EVT_FOO: &str = "foo"` and a payload struct
   to `src-tauri/src/events.rs`
2. Emit it: `app.emit(EVT_FOO, payload)?;` (needs `use tauri::Emitter;`)
3. Mirror the payload type in `src/types.ts`
4. Subscribe in the frontend: `listen<FooPayload>(EVT_FOO, ...)`
   (usually in `useRunnerEvents.ts` for project lifecycle events, or
   directly in `App.tsx` for one-off events like `config:reloaded`)

### Adding a new config field

1. Add the field to the relevant struct in `src-tauri/src/config.rs`
   (use `#[serde(rename = "camelCase")]` for JSON naming) and update
   the resolver if needed.
2. Update `src/types.ts` to match.
3. Update the form in `src/components/ProjectForm.tsx` if it's a
   user-facing field.
4. Update the detail view in `src/components/ProjectDetail.tsx` to
   display it.
5. If there's validation, update `config::validate` in `config.rs`.
6. If the field needs a default, add it to the relevant `Default` impl.

### Adding a new action type / button

Most projects will use the standard "Run" / "Stop" actions from
their TOML. If you need a different shape:

1. Update `ResolvedAction` in `config.rs` if the wire shape changes.
2. Update `ProjectDetail.tsx`'s action button row to render any new
   fields (e.g. an icon mapping).
3. The backend doesn't care what an action is named — any
   `[[actions]]` entry becomes a button. The naming only affects
   "Restart" (which picks the first "Run"-shaped action).

### Code style

- **Frontend:** plain React + TypeScript, no router, no state library.
  One `useState` per concern. KISS.
- **Backend:** one `RwLock<AppState>` plus a small set of
  `#[tauri::command]` functions. New state → `state.rs`. New commands
  → `commands.rs`. Cross-cutting logic → its own module.
- **Wire types** in `src/types.ts` and `src-tauri/src/{config,events,runner}.rs`
  are the contract. Keep them in sync; serde derives are the safety net.

## Common pitfalls

- **`pnpm tauri build` runs from CWD `src-tauri/`, not project root.**
  That's why the config loader walks up from CWD looking for
  `vibe.config.json`. Don't break that.
- **Edit a Rust file → cargo rebuilds on save in `pnpm tauri dev`.** First
  build is slow; subsequent are fast.
- **The `project:output` event payload is a JSON array of byte numbers.**
  Convert to `Uint8Array` on the frontend before writing to xterm.js.
  See `LogViewer.tsx`.
- **Watch out for `RunnerHandle` field access.** `status`, `pid`, `writer`,
  etc. are private. Use the provided accessors (`set_status`, `send_input`,
  `take_writer`, etc.) or add a new one.
- **The file watcher ignores self-writes** via a 1-second skip flag. If
  you add a new write path, set `watcher::skip_next_change()` before
  the write.
- **Detached / backgrounded processes (e.g. `nohup ... &`) exit the
  PTY quickly.** If the action is marked `detached: true`,
  VibeRunner tracks the whole process tree rooted at the PTY's
  PID and keeps the project "Running" as long as any descendant
  is alive. Without that flag, the project goes "stopped" the
  moment the launcher returns even if the app is still alive.
- **`.env` is loaded from the project root automatically.** The values
  go into the spawned PTY's environment. If your script also loads
  `.env` (e.g. via `dotenv` or by sourcing it), the values are
  loaded twice — fine, just be aware.
- **The Rust log to stderr is visible in `pnpm tauri dev` output** but
  not in the running app. Use `eprintln!` from the backend if you need
  to debug.

## Where things live (search hints)

| If you want to change… | Look in… |
|---|---|
| The main window layout | `src/App.tsx`, `src/styles.css` |
| How a project displays in the sidebar | `src/components/ProjectCard.tsx` |
| The "add project" form (folder picker) | `src/components/ProjectForm.tsx` |
| The action buttons / log / ports | `src/components/ProjectDetail.tsx` |
| What the log viewer shows | `src/components/LogViewer.tsx` |
| How ports are detected | `src-tauri/src/ports.rs`, `src/components/PortList.tsx` |
| How processes are killed | `src-tauri/src/process.rs` (`kill_tree`) |
| How `.env` is auto-loaded | `src-tauri/src/pty.rs` (`spawn`) |
| How TOML is read from each project | `src-tauri/src/config.rs` (`read_toml_environment`, `resolve_project`) |
| What events exist | `src-tauri/src/events.rs` (Rust), `src/types.ts` (TS) |
| The `vibe.config.json` format | `src-tauri/src/config.rs` |
| File-watching behavior | `src-tauri/src/watcher.rs` |
| The app icon | `scripts/generate-icon.py`, `assets/icon-source.png` |
| Build commands | `package.json` |
| App metadata (version, publisher) | `src-tauri/tauri.conf.json` |
| Permissions / capabilities | `src-tauri/capabilities/default.json` |

## How to verify before committing

```bash
pnpm typecheck && pnpm test:rust
```

This is the minimum bar. For anything touching the build:

```bash
pnpm verify
```

(This runs the test suite then does `tauri build --debug`, which is
slower but catches issues that test:rust alone might miss.)

## Release setup (deferred)

The updater plugin is wired but no CI workflow. To actually ship:

1. Get the Apple Developer Program ($99/yr) and/or a Windows EV cert
   (free for OSS via SignPath.io).
2. Add a GitHub Actions workflow at `.github/workflows/release.yml` that
   builds for each target OS, signs the bundles, and publishes to a
   GitHub Release with `latest.json` next to it.
3. Bump the version in `tauri.conf.json` per release.

The private signing key lives at `src-tauri/.tauri-updater-key` (gitignored)
or in `TAURI_SIGNING_PRIVATE_KEY` env var.
