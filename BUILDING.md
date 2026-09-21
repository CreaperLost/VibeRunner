# Building VibeRunner

This guide covers local development and distributable desktop builds for
Linux, macOS, and Windows. VibeRunner is a Tauri 2 application with a Vite /
React frontend and a Rust backend, so every build needs the JavaScript and
Rust toolchains plus the native dependencies for the host operating system.

For the most current Tauri dependency details, see the official
[Tauri prerequisites](https://tauri.app/start/prerequisites/) and
[distribution](https://tauri.app/distribute/) documentation.

## Build strategy

Build on the operating system you intend to ship for whenever possible:

| Target | Recommended build host | Main command | Typical output |
|---|---|---|---|
| Linux | Linux | `pnpm build:linux` | `.deb`, `.AppImage`, `.rpm` |
| macOS | macOS | `pnpm build:mac` | `.dmg` |
| macOS universal | macOS | `pnpm build:mac:universal` | universal `.dmg` |
| Windows | Windows | `pnpm build:win` | `.msi` and/or NSIS setup executable |

All artifacts are written below `src-tauri/target/`. The exact bundle
formats depend on the host platform and the Tauri configuration in
`src-tauri/tauri.conf.json`.

## Common setup

Install these on every platform:

1. Git
2. Node.js (an active LTS release)
3. pnpm compatible with the v9 lockfile in `pnpm-lock.yaml`
4. Rust stable through [rustup](https://rustup.rs/)

After cloning the repository:

```bash
pnpm install --frozen-lockfile
pnpm tauri:info
pnpm test
```

`pnpm tauri:info` is useful when reporting build failures because it records
the OS, Rust toolchain, Tauri CLI, and WebView details. `pnpm test` runs the
TypeScript check and Rust library tests.

To run the application during development:

```bash
pnpm tauri dev
```

To run the full debug build verification, including a Tauri bundle for the
current platform:

```bash
pnpm verify
```

## Linux

### Native dependencies

The following example is for Debian or Ubuntu. Install the equivalent
packages for another distribution using the
[Tauri Linux prerequisites](https://tauri.app/start/prerequisites/) as the
reference:

```bash
sudo apt update
sudo apt install \
  libwebkit2gtk-4.1-dev \
  build-essential \
  curl \
  wget \
  file \
  libxdo-dev \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  patchelf
```

Install Rust with rustup if it is not already available:

```bash
curl --proto '=https' --tlsv1.2 https://sh.rustup.rs -sSf | sh
rustup default stable
```

### Build

```bash
pnpm install --frozen-lockfile
pnpm build:linux
```

Look in these directories for the generated packages:

```text
src-tauri/target/release/bundle/deb/
src-tauri/target/release/bundle/appimage/
src-tauri/target/release/bundle/rpm/
```

An AppImage usually needs to be made executable before launching it:

```bash
chmod +x src-tauri/target/release/bundle/appimage/*.AppImage
```

Linux packages are distribution-sensitive. Test the generated package on the
oldest supported distribution you intend to advertise, especially when using
an AppImage or a package built on a newer distro.

## macOS

### Native dependencies

For desktop-only development, install Apple’s command-line developer tools:

```bash
xcode-select --install
```

Install the full Xcode application instead if you need Apple platform SDKs,
App Store tooling, or additional signing workflows. Launch Xcode once after
installing it so it can finish setup.

Confirm the selected developer directory when troubleshooting:

```bash
xcode-select -p
rustup show
```

### Apple Silicon or Intel build

Build for the architecture of the current Mac:

```bash
pnpm install --frozen-lockfile
pnpm build:mac
```

This command runs Tauri’s build and then the repository’s custom DMG wrapper.
The wrapper uses macOS’s built-in `hdiutil` and Finder automation to create a
branded disk image. The finished file is under:

```text
src-tauri/target/release/bundle/dmg/
```

### Universal build

To produce one DMG containing both Apple Silicon and Intel binaries, install
both Rust targets and run:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
pnpm build:mac:universal
```

The universal build is placed under:

```text
src-tauri/target/universal-apple-darwin/release/bundle/dmg/
```

The local DMG is unsigned by default. Users may see Gatekeeper warnings when
opening it. For public distribution, configure Apple code signing and
notarization; see the official
[Tauri macOS distribution guide](https://tauri.app/distribute/macos-application-bundle/)
and [DMG guide](https://tauri.app/distribute/dmg/).

## Windows

This section is the complete process for turning the repository into a
Windows executable. Run it on a 64-bit Windows machine; building on the target
operating system is the most reliable option.

### Native dependencies

Install the following before running a Tauri build:

1. [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
   with **Desktop development with C++** selected. Include the Windows SDK.
2. Microsoft Edge WebView2 Runtime. It is already present on most supported
   Windows installations, but install the Evergreen Runtime if it is missing.
3. Rust stable using rustup with the MSVC toolchain selected.

In PowerShell, verify the Rust host and toolchain:

```powershell
rustup show
rustc -Vv
```

If MSI bundling fails with an error mentioning `light.exe`, enable the
optional **VBScript** Windows feature. Tauri uses it for MSI packaging on
some Windows configurations.

### Build

Open PowerShell in the repository root (the folder containing `package.json`).
Install the dependencies and confirm the toolchains are visible:

```powershell
pnpm install --frozen-lockfile
pnpm tauri:info
```

Optionally run the checks before producing a release:

```powershell
pnpm test
```

Build the optimized application and its Windows installers:

```powershell
pnpm build:win
```

The first build downloads and compiles the Rust dependencies and can take
several minutes. Later builds reuse Cargo's cache and are normally much
faster. A successful build prints the paths to the generated bundles.

### Build outputs

The command produces three useful files:

| Artifact | Path | Use |
|---|---|---|
| NSIS installer executable | `src-tauri\target\release\bundle\nsis\VibeRunner_<version>_x64-setup.exe` | Recommended file to give to most Windows users |
| MSI installer | `src-tauri\target\release\bundle\msi\VibeRunner_<version>_x64_en-US.msi` | Useful for managed or enterprise installation |
| Raw application executable | `src-tauri\target\release\viberunner.exe` | Direct launch and local testing; not a complete installer |

For the current `0.1.0` version, the installer names are:

```text
src-tauri\target\release\bundle\nsis\VibeRunner_0.1.0_x64-setup.exe
src-tauri\target\release\bundle\msi\VibeRunner_0.1.0_x64_en-US.msi
```

The version comes from `src-tauri\tauri.conf.json`. Update it before making a
new release. The bundle target is configured as `"all"`, so Tauri creates both
the NSIS `.exe` and MSI installer.

Install and launch the NSIS or MSI package on a clean Windows account or test
machine before distributing it. The target machine needs the Microsoft Edge
WebView2 Runtime; current Windows releases generally already include it.

The Windows build is unsigned by default, so SmartScreen may show an
unknown-publisher warning. For public distribution, configure Windows code
signing; see the official
[Tauri Windows installer](https://tauri.app/distribute/windows-installer/)
and [code-signing](https://tauri.app/distribute/sign/windows/) guides.

### Rebuilding

For a normal rebuild, run `pnpm build:win` again. If stale generated files are
suspected, remove only the generated build output and rebuild:

```powershell
pnpm clean:all
pnpm build:win
```

`pnpm clean:all` removes compiled artifacts, so the following Rust build will
take longer. It does not remove application source files.

## Cross-compilation

Native builds are the supported path for release artifacts. Tauri documents
cross-compiling Windows NSIS installers from Linux or macOS, but it requires
additional tools and is less tested. MSI packages require Windows tooling.
Use a native machine, a VM, or a CI runner for the target OS when possible.
See Tauri’s [Windows cross-compilation notes](https://tauri.app/distribute/windows-installer/#build-windows-apps-on-linux-and-macos)
before adding a cross-build pipeline.

## Release and signing notes

Local builds are suitable for development and testing. Before publishing:

- bump the version in `src-tauri/tauri.conf.json`;
- build and test each target on its native OS;
- sign and notarize macOS artifacts;
- sign Windows installers;
- test Linux packages on the distributions you support; and
- keep any signing certificates out of Git.

## Troubleshooting checklist

When a build fails, collect the following before opening an issue:

```bash
pnpm tauri:info
rustup show
node --version
pnpm --version
```

Also include the host OS and architecture, the exact command, and the first
native dependency or linker error in the build log. Avoid posting secrets,
signing keys, or the contents of `.env` files.
