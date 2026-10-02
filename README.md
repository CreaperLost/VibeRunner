<p align="center">
  <img src="assets/icon-source.png" width="96" alt="VibeRunner icon" />
</p>

<h1 align="center">VibeRunner</h1>

<p align="center">
  Start, stop and watch all your dev apps from one window.<br/>
  Windows · macOS · Linux
</p>

---

You have a handful of projects, and each one starts differently:
`npm run dev`, `docker compose up`, a PowerShell script, a Python
server. VibeRunner turns every project folder into a row of buttons
(**Run**, **Stop**, **Restart**, **Build**, plus anything else you
define) and gives each one a live terminal, its open ports, and a
one-click "open in browser".

- **One-click run / stop / restart** for every project, with a real
  terminal you can type into
- **Stop that actually stops**: the whole process tree is shut down,
  so no orphaned `node` holding port 5173
- **Detected ports** with clickable `localhost` links, and optional
  auto-open in your browser when the app comes up
- **Install / Run buttons** for your latest build outputs (`.exe`,
  setup installer, `.msi`, `.dmg`, `.app`, `.AppImage`, `.deb`)
- **Actions live in the repo** (`.codex/environments/environment.toml`),
  so they're versioned with the code and an AI assistant can write
  them for you
- `.env` files are loaded automatically; crash notifications; optional
  auto-restart; light and dark themes

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Give a repo its buttons](#give-a-repo-its-buttons)
  - [Let your AI assistant write it](#let-your-ai-assistant-write-it)
  - [Example configs](#example-configs)
  - [Reference](#reference)
- [Using VibeRunner](#using-viberunner)
- [Troubleshooting](#troubleshooting)
- [Build from source](#build-from-source)

## Install

Download the latest version from the
[**Releases page**](https://github.com/CreaperLost/VibeRunner/releases/latest).

| OS | File | Notes |
|---|---|---|
| **Windows 10/11** | `VibeRunner_x.y.z_x64-setup.exe` | Recommended. An `.msi` is also provided for managed installs. |
| **macOS** | `VibeRunner_x.y.z_<arch>.dmg` | Open it and drag VibeRunner into Applications. |
| **Linux** | `.AppImage`, `.deb` or `.rpm` | AppImage: `chmod +x VibeRunner*.AppImage` and run it. |

The builds are **not code-signed yet**, so your OS will warn you the
first time:

- **Windows** (SmartScreen, "Windows protected your PC"): click
  **More info → Run anyway**.
- **macOS** ("cannot be opened because the developer cannot be
  verified"): right-click the app in Applications → **Open** → **Open**.
  If macOS says the app is damaged, run
  `xattr -dr com.apple.quarantine /Applications/VibeRunner.app` once.

Windows needs the Microsoft Edge **WebView2** runtime. It ships with
Windows 10/11; the setup installer downloads it if it's missing.

## Quick start

1. Open VibeRunner and click **New** in the sidebar.
2. Pick your project folder.
   - If the folder has a `.codex/environments/environment.toml`, keep
     **Auto-discover (TOML)** and its buttons appear immediately.
   - If it doesn't, either choose **Manual (inline commands)** and type the commands in
     the form, or [have your AI assistant create the file](#let-your-ai-assistant-write-it)
     (recommended: it lives in the repo and works for everyone on the
     team).
3. Click the primary button (usually **Run**). Output streams into the
   **Logs** tab, and once the app listens on a port, a
   `localhost:PORT` button appears.

## Give a repo its buttons

VibeRunner reads one file inside each project:

```
your-project/
└── .codex/
    └── environments/
        └── environment.toml
```

It's the same convention [Codex](https://github.com/openai/codex) uses,
so if your repo already has one, it just works. Every `[[actions]]`
entry becomes a button:

```toml
version = 1
name = "My App"

[setup]                      # optional "Setup" button, also run by Restart
script = "npm install"

[[actions]]
name = "Run"
icon = "run"                 # "run" marks the primary action (Restart runs it)
command = "npm run dev"

[[actions]]
name = "Test"
icon = "test"
command = "npm test"
```

Save the file and VibeRunner picks it up (or click the reload button in
the header).

### Let your AI assistant write it

The fastest way to configure a repo is to ask the AI coding assistant
you already use (Claude Code, Codex, Cursor, Copilot, …) to inspect it
and write the file. Open the project in your assistant and paste this
prompt:

````text
Create `.codex/environments/environment.toml` for this repository so the
VibeRunner app can run it. First inspect the repo (package.json scripts,
lockfiles, Makefile, docker-compose, pyproject/requirements, Cargo.toml,
existing scripts/ folder, README) to learn how it is installed, run,
built and tested. Don't guess commands that don't exist.

Format:

    version = 1
    name = "<Project name>"

    [setup]                  # optional: dependency install, run before Run on Restart
    script = "<command>"

    [[actions]]              # one block per button
    name = "<Button label>"
    icon = "run"             # run | stop | build | test | tool | migrate
    command = "<shell command, run from the repo root>"
    platform = "windows"     # optional: windows | unix | macos | linux
    detached = false         # optional, see rule 4

Rules:
1. The main "start the app" action uses icon = "run". There must be
   exactly one "run" action per OS. Add Build (icon "build") and Test
   (icon "test") if the repo supports them; use "tool" or "migrate" for
   other useful tasks (lint, db migrate, seed, package).
2. Commands run through `cmd /C` on Windows and `sh -c` on macOS/Linux.
   If a command differs per OS, write two actions with the same icon and
   set platform = "windows" on one and platform = "unix" on the other.
   Never leave two actions sharing an icon without platform set.
3. For PowerShell scripts, use:
   powershell.exe -NoProfile -ExecutionPolicy Bypass -File ".\scripts\x.ps1"
   with BACKSLASHES. `-File` rejects forward slashes.
4. Run actions must stay in the foreground until the app exits (e.g.
   `npm run dev`, `docker compose up`, not `docker compose up -d`), so
   logs stream and Stop works. Only if a command must launch something
   in the background and return immediately, set detached = true.
5. Only add an icon = "stop" action if the app needs a special shutdown
   (e.g. `docker compose down`). Without one, VibeRunner sends Ctrl+C
   and then ends the whole process tree it started, which is right for
   most dev servers.
6. If the app serves a web UI, make sure it prints its URL
   (http://localhost:PORT) on startup so VibeRunner can detect it.
7. Use the repo's package manager (pnpm/yarn/bun/npm, based on the
   lockfile) and existing scripts rather than inventing new ones.
   If several services must run together (e.g. frontend + API), prefer
   one Run action that starts them all (an existing "dev" script, or
   docker compose); otherwise give each its own action.

When done, show me the file and briefly explain each action.
````

Commit the file so everyone on the team (and every machine) gets the
same buttons.

### Example configs

**Node / Vite app** (same commands on every OS):

```toml
version = 1
name = "Storefront"

[setup]
script = "pnpm install"

[[actions]]
name = "Run"
icon = "run"
command = "pnpm dev"

[[actions]]
name = "Build"
icon = "build"
command = "pnpm build"

[[actions]]
name = "Test"
icon = "test"
command = "pnpm test"
```

**Python API + frontend with Docker** (needs a real shutdown, so it has
a Stop action):

```toml
version = 1
name = "Thumbnail Studio"

[[actions]]
name = "Run"
icon = "run"
command = "docker compose up --build"

[[actions]]
name = "Stop"
icon = "stop"
command = "docker compose down"

[[actions]]
name = "Migrate DB"
icon = "migrate"
command = "docker compose run --rm api alembic upgrade head"
```

**Cross-platform repo with its own scripts** (bash on macOS/Linux,
PowerShell on Windows, so every action is scoped with `platform`):

```toml
version = 1
name = "Asset Flow"

[setup]
script = "npm ci"

[[actions]]
name = "Start"
icon = "run"
platform = "unix"
command = "./scripts/dev.sh"

[[actions]]
name = "Start (Windows)"
icon = "run"
platform = "windows"
command = 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File ".\scripts\dev.ps1"'

[[actions]]
name = "Package"
icon = "build"
platform = "unix"
command = "./scripts/package.sh"

[[actions]]
name = "Package (Windows)"
icon = "build"
platform = "windows"
command = 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File ".\scripts\package.ps1"'
```

VibeRunner's own repo uses one as well: see
[`.codex/environments/environment.toml`](.codex/environments/environment.toml).

### Reference

**File:** `<project>/.codex/environments/environment.toml`

| Key | Required | Meaning |
|---|---|---|
| `version` | no | Always `1`. |
| `name` | no | Display name in the sidebar (defaults to the project's id). |
| `[setup] script` | no | Adds a **Setup** button. **Restart** runs it before the primary action and only continues if it succeeds. |
| `[build] script` | no | Adds a **Build** button (same as an action with `icon = "build"`). |
| `[[actions]]` | no | One button per entry, in file order. |
| `actions.name` | yes | Button label. |
| `actions.command` | yes | Shell command, run from the project folder (`cmd /C` on Windows, `sh -c` elsewhere). |
| `actions.icon` | no | `run`, `stop`, `build`, `test`, `tool`, `migrate`. Decides the button's icon **and role** (below). |
| `actions.platform` | no | `windows`, `unix` (macOS + Linux), `macos`, `linux`. Omit for "every OS". Actions for other OSes are hidden. |
| `actions.detached` | no | `true` if the command starts something in the background and returns at once (`nohup … &`). The project then stays **Running** while anything it started is alive. |

**Roles.** A few buttons have special meaning; VibeRunner finds them by
`icon` first, then by name:

| Role | Found by | What it does |
|---|---|---|
| Primary | `icon = "run"` or name `Run` (else the first action) | Highlighted button, what **Restart** and **Run all** start, the only action that triggers browser auto-open. |
| Stop | `icon = "stop"` or name `Stop` | Runs when you click **Stop**, alongside Ctrl+C; may also be a keystroke like `Ctrl+C`. Optional. |
| Build | `icon = "build"` or name `Build` | Shown as its own button. If you declare none, VibeRunner offers one when it recognizes your build system (`package.json` build script, `Cargo.toml`, `Makefile` build target, `go.mod`, `scripts/build.sh`). |

**Environment.** A `.env` file in the project folder is loaded into
every command's environment automatically.

## Using VibeRunner

**Sidebar.** Every project with its status: Stopped, Starting,
Running (with uptime), Stopping, or Crashed (with the exit code).
`Ctrl F` / `⌘F` searches. Hover a card to open its folder, open its
TOML, or remove it from the list (this never deletes files).

**Toolbar.**

- **Run** (primary action), and every other action, start in a fresh
  terminal. One action runs per project at a time.
- **Stop** runs your Stop action (if any) and sends Ctrl+C. Whatever is
  still running after 3 seconds is terminated, then force-killed, so
  Stop always finishes, usually within a second or two. Only processes
  this project started are touched.
- **Restart** = Stop → Setup (if any) → primary action.
- **Clean up** appears after a crash: it stops anything the crashed run
  left behind.
- **localhost:PORT** opens the app. **Auto-open** opens it once per run,
  when Run first reports its URL.

**Install / Run bar.** When a project contains build outputs (an
installer and/or an app binary), VibeRunner shows the newest of each.
The arrow lists other matches, and the list refreshes after every run.

**Tabs.**

- **Logs**: live, colored terminal output. Click into it to type
  (answer prompts, press keys). Copy, clear, and maximize from the
  tab bar.
- **Ports**: every port the project's processes listen on.
- **Details**: the resolved actions, commands, and any warnings.

**Header.** Run all / Stop all, reload config, theme toggle (system /
light / dark), and the path of your project list. Click the path to
open it.

**Where the project list lives.** The installed app keeps it at:

| OS | Path |
|---|---|
| Windows | `%APPDATA%\com.viberunner.app\vibe.config.json` |
| macOS | `~/Library/Application Support/com.viberunner.app/vibe.config.json` |
| Linux | `~/.local/share/com.viberunner.app/vibe.config.json` |

You rarely need to edit it by hand (use **New** and the trash button),
but it's plain JSON with comments allowed, and VibeRunner reloads it
when it changes. Set the `VIBE_CONFIG` environment variable to use a
different file.

<details>
<summary>Manual projects (commands stored in VibeRunner instead of the repo)</summary>

Choosing **Manual** in the New dialog stores the commands in your
project list instead of the repo. Use it for folders you can't or don't
want to change. The same fields are available, plus per-project
environment variables and an auto-restart policy:

```jsonc
{
  "version": 2,
  "projects": [
    {
      "id": "static-site",
      "path": "C:\\dev\\static-site",
      "manual": true,
      "setup": { "command": "npm install" },
      "actions": [
        { "name": "Run", "icon": "run", "command": "npx serve -l 8080" }
      ],
      "env": { "NODE_ENV": "development" },
      "autoRestart": { "enabled": true, "maxRetries": 3, "delayMs": 2000 }
    }
  ]
}
```

`env` values override the project's `.env`. With `autoRestart`, a crash
re-runs the action up to `maxRetries` times in a row.
</details>

## Troubleshooting

| Problem | Fix |
|---|---|
| A button is missing | Check **Details**: actions for another OS are hidden on purpose, and TOML errors are shown as warnings. |
| PowerShell action fails with *"Illegal characters in path"* | Use backslashes: `-File ".\scripts\run.ps1"`. |
| PowerShell says scripts are disabled | Add `-ExecutionPolicy Bypass` to the command (see the examples). |
| Project shows **Stopped** right after Run, but the app is running | The command started the app in the background and exited. Run it in the foreground, or set `detached = true`. |
| No port shows up | The port must be opened by a process the project started, or printed as `http://localhost:PORT` in the logs. Apps inside Docker count if they print their URL. |
| `command not found` on macOS (`pnpm`, `node`, …) | VibeRunner loads your login shell's `PATH`. Make sure the tool is on `PATH` in `~/.zprofile` or `~/.zshrc`, then restart VibeRunner. |
| Stop finishes but something is still running | The leftover wasn't started by this project (e.g. a container started with `-d`). Add a Stop action such as `docker compose down`. |

Found a bug? [Open an issue](https://github.com/CreaperLost/VibeRunner/issues)
with your OS, the action's command, and the log output.

## Build from source

You need [Node.js](https://nodejs.org/) (LTS), [pnpm](https://pnpm.io/),
and [Rust](https://rustup.rs/) (stable), plus the platform tools Tauri
needs:

- **Windows:** [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
  with "Desktop development with C++".
- **macOS:** `xcode-select --install`.
- **Linux (Debian/Ubuntu):**
  `sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf`

See [Tauri's prerequisites](https://tauri.app/start/prerequisites/)
for other distributions. Then:

```bash
git clone https://github.com/CreaperLost/VibeRunner.git
cd VibeRunner
pnpm install
pnpm tauri dev        # run in development mode
```

Build installers for your current OS:

```bash
pnpm build:win        # Windows → src-tauri/target/release/bundle/nsis/*-setup.exe (+ .msi)
pnpm build:mac        # macOS   → src-tauri/target/release/bundle/dmg/*.dmg
pnpm build:linux      # Linux   → src-tauri/target/release/bundle/{appimage,deb,rpm}/
```

Contributing? [AGENTS.md](AGENTS.md) explains the architecture,
conventions, and how to test (`pnpm test`).

## License

[MIT](LICENSE) © George Paterakis
