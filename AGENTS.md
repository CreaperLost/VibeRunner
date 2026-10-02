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

  # Optional: scope an action to one OS. Absent = everywhere.
  [[actions]]
  name = "Start (Windows)"
  icon = "run"
  platform = "windows"
  command = 'powershell.exe -File ".\scripts\start.ps1"'
  ```
  VibeRunner turns each `[[actions]]` entry into a button.

- **Run manual** commands defined inline in `vibe.config.json` —
  for repos that don't follow the TOML convention.

Either way, every project gets a built-in **Restart** button (Stop →
Setup → the action named "Run" or icon="run", in that order).

`platform` matters for that rule. Restart and Stop both resolve their
action by **first icon match**, so a cross-platform repo that declares
a bash action and a PowerShell action sharing an `icon` and no
`platform` will silently pick the bash one on Windows. Scope both sets
(`platform = "unix"` / `platform = "windows"`) and the resolver filters
to the running OS *before* picking, so both buttons land correctly.
Actions dropped this way are reported in `ResolvedProject.warnings`.

`.env` files inside a project are auto-loaded into the spawned PTY's
environment (lower priority than `vibe.config.json` env, higher than
the parent shell's env).

## Project layout

```
VibeRunner/
├── vibe.config.json            # list of projects (each = a folder)
├── .codex/
│   └── environments/
│       └── environment.toml    # VibeRunner's OWN Start/Stop/Build actions
│                               # (per-OS; see "VibeRunner as its own project")
├── assets/
│   └── icon-source.png         # 1024×1024 mark; `pnpm icon` regenerates all sizes
├── scripts/
│   ├── dev.sh / dev.ps1        # per-OS dev launcher: start | stop
│   ├── build.sh / build.ps1    # per-OS validation + frontend build
│   └── generate-icon.py        # PIL script that builds icon-source.png
├── src/                        # React + TS frontend
│   ├── App.tsx                 # shell, state, event subscriptions
│   ├── types.ts                # wire types (mirrors Rust events/config)
│   ├── styles.css              # design system + all component styles
│   ├── hooks/
│   │   └── useRunnerEvents.ts  # runtime (status+ports, rehydrated via get_statuses), restarting
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
│   │   ├── commands.rs         # #[tauri::command] handlers (thin)
│   │   ├── lifecycle.rs        # spawn / monitor / stop supervisor / ports / restart
│   │   ├── pty.rs              # portable-pty wrapper, .env auto-load
│   │   ├── process.rs          # keystrokes + identity-checked ProcessTree
│   │   ├── ports.rs            # lsof / netstat parsing, URL hints from output
│   │   ├── artifacts.rs        # .msi/.exe/.dmg/.app/.AppImage discovery + ranking
│   │   ├── runner.rs           # RunnerHandle (status, I/O, tree, restart count)
│   │   └── watcher.rs          # file-system watcher with debounce
│   ├── capabilities/default.json
│   ├── icons/                  # generated from assets/icon-source.png
│   ├── tauri.conf.json
│   ├── Cargo.toml
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
pnpm test:rust          # cargo test --lib (incl. end-to-end lifecycle tests)
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
   "Restart" (which picks the first "Run"-shaped action). Icons are
   load-bearing for both Restart and Stop; use `platform` to keep
   per-OS actions from colliding on the same icon.

### Code style

- **Frontend:** plain React + TypeScript, no router, no state library.
  One `useState` per concern. KISS.
- **Backend:** one `RwLock<AppState>` plus a small set of
  `#[tauri::command]` functions. New state → `state.rs`. New commands
  → `commands.rs`. Cross-cutting logic → its own module.
- **Wire types** in `src/types.ts` and `src-tauri/src/{config,events,runner}.rs`
  are the contract. Keep them in sync; serde derives are the safety net.

## Common pitfalls

- **Never spawn a helper process without `hidden_command()`.** The
  release build is a GUI-subsystem app with no console, so Windows
  allocates a *brand-new console window* for every console child
  (`taskkill`, `netstat`, `cmd`). That's the "terminal keeps popping
  up" bug. Use `process::hidden_command()` (it applies
  `CREATE_NO_WINDOW`) for anything VibeRunner spawns on the user's
  behalf. (Killing no longer spawns anything: `ProcessTree::kill`
  signals processes directly through `sysinfo`.)
- **Never match processes by path, and never trust a bare PID.** A
  project's processes are exactly its `ProcessTree`: the PTY child plus
  descendants adopted by (pid, start time), with dead entries pruned.
  Matching "cwd or command line contains the project path" used to make
  Stop kill VS Code / terminals and showed their ports as the
  project's; reusing stale PIDs made runs hang in Running/Stopping and
  made Stop `taskkill /T /F` recycled PIDs. PID files are only adopted
  if written after the run started *and* the process started after it.
- **Status has two owners.** The run's monitor thread finalizes natural
  exits (exit 0 → Stopped, else Crashed); once the user clicks Stop,
  the stop supervisor owns the run and always reaches Stopped (after
  terminate → kill → a hard deadline). Both go through
  `RunnerHandle::finish`, which rejects stale generations — use it,
  don't `set_status` a final state directly.
- **Ports come from the tree, plus URL hints.** `project:ports` is
  emitted only by the port poller: listeners owned by the tree, plus
  `http://localhost:N` / "on port N" from the run's output while
  something accepts connections there. Auto-open fires once per run,
  only for the primary action.
- **Windows test binaries need the app manifest.** `build.rs` embeds
  `windows-app-manifest.xml` via the linker for every target (instead of
  tauri-build's resource) so Tauri's mock runtime loads in
  `cargo test`; without it tests die with `STATUS_ENTRYPOINT_NOT_FOUND`.
- **`powershell.exe -File` rejects forward slashes.** A command like
  `-File "./scripts/run.ps1"` dies with *"Illegal characters in
  path"* before the script opens. Use `-File ".\scripts\run.ps1"`.
  This is a PowerShell CLI quirk, not a path-resolution problem, so
  VibeRunner can't paper over it — the project config must use
  backslashes.
- **Icon collisions decide Restart and Stop.** Both pick the *first*
  action with a matching `icon`. See "Cross-platform action sets" in
  the model section above.
- **`pnpm tauri build` runs from CWD `src-tauri/`, not project root.**
  That's why the config loader walks up from CWD looking for
  `vibe.config.json`. Don't break that.
- **Edit a Rust file → cargo rebuilds on save in `pnpm tauri dev`.** First
  build is slow; subsequent are fast.
- **The `project:output` event payload is a JSON array of byte numbers.**
  Convert to `Uint8Array` on the frontend before writing to xterm.js.
  See `LogViewer.tsx`.
- **Watch out for `RunnerHandle` field access.** `status`, `tree`, `writer`,
  etc. are private. Use the provided accessors (`try_begin`, `finish`,
  `with_tree`, `send_input`, `write_raw`, etc.) or add a new one.
- **The file watcher ignores self-writes** via a 1-second skip flag. If
  you add a new write path, set `watcher::skip_next_change()` before
  the write.
- **Detached / backgrounded processes (e.g. `nohup ... &`) exit the
  PTY quickly.** If the action is marked `detached: true`,
  VibeRunner keeps the project "Running" as long as anything in its
  process tree is alive. Without the flag, a launcher that exits 0 is
  still treated as having started a daemon if a surviving process
  listens on a TCP port or wrote a fresh PID file; otherwise the run
  is Stopped (leftover toolchain helpers like `mspdbsrv.exe` don't
  keep a finished Build "running").
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
| How processes are tracked / killed | `src-tauri/src/process.rs` (`ProcessTree`), `src-tauri/src/lifecycle.rs` (`supervise_stop`) |
| Run status transitions | `src-tauri/src/lifecycle.rs`, `src-tauri/src/runner.rs` (`finish`) |
| Install / Run Portable discovery | `src-tauri/src/artifacts.rs` |
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

## VibeRunner as its own project

VibeRunner consumes its own `environment.toml`, so the config is a
live test of the feature it implements. `.codex/environments/environment.toml`
declares Start / Stop / Build, each in a `unix` and a `windows` variant
sharing the same `icon`, plus a three-way `Package` split (macOS /
Linux / Windows) replacing the old macOS-only "Build .app" that failed
elsewhere.

`scripts/dev.sh` and `scripts/dev.ps1` are the launchers. The contract
they exist to guarantee:

- **Stop touches only what Start launched.** Start records the root
  PID *and its start time* in `.codex/viberunner-dev.state`; Stop
  re-checks both before signalling. The start-time check is not
  decoration — the OS recycles PIDs, and without it a stale state file
  could eventually kill an unrelated process.
- **Start → Stop → Start is deterministic.** Stop waits for the tree
  to actually exit and always removes the state file, so the next
  Start never races a dying process or a stale port.
- **Both are idempotent.** Stop with nothing running exits 0 and says
  so; Start with a live process refuses instead of stacking a second
  dev server.

`.codex/*` is gitignored except the TOML, so the state file is
runtime-only. `config::tests::repo_environment_toml_resolves_to_one_action_per_icon`
guards the invariant that after filtering, exactly one action survives
per icon — two survivors on one icon is the ambiguity that silently
made Restart pick the bash script on Windows.

## Release setup (deferred)
No CI workflow yet. To actually ship:

1. Get the Apple Developer Program ($99/yr) and/or a Windows EV cert
   (free for OSS via SignPath.io).
2. Add a GitHub Actions workflow at `.github/workflows/release.yml` that
   builds for each target OS, signs the bundles, and publishes to a
   GitHub Release with `latest.json` next to it.
3. Bump the version in `tauri.conf.json` per release.
