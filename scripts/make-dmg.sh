#!/usr/bin/env bash
# Build a polished .dmg for VibeRunner with:
#   - A custom branded background image
#   - A proper Finder window layout (icon view, sized window, no toolbar)
#   - The .app on the left, an Applications alias on the right
#   - Read-only, compressed, with a clean name and a "VibeRunner" volume label
#
# Called from package.json's `build:mac` script after Tauri's own
# bundle step produces the bare .app. The default `bundle_dmg.sh`
# that Tauri invokes is replaced by this one.
#
# Usage: scripts/make-dmg.sh <app-bundle-path> <out-dmg-path>
#   e.g. scripts/make-dmg.sh \
#          src-tauri/target/release/bundle/macos/VibeRunner.app \
#          src-tauri/target/release/bundle/dmg/VibeRunner_0.1.0_aarch64.dmg

set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <app-bundle.app> <out.dmg>" >&2
  exit 2
fi

APP_BUNDLE="$1"
OUT_DMG="$2"

if [[ ! -d "$APP_BUNDLE" ]]; then
  echo "app bundle not found: $APP_BUNDLE" >&2
  exit 1
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
BG_SRC="$ROOT/assets/dmg-background.png"

if [[ ! -f "$BG_SRC" ]]; then
  echo "background image not found at $BG_SRC" >&2
  echo "run: python3 scripts/generate-dmg-background.py" >&2
  exit 1
fi

# Pick a fresh temporary build directory under the output folder.
# RAW_DMG is placed in BUILD_DIR outside STAGE_DIR so it is never
# included in the disk image contents.
BUILD_DIR="$(mktemp -d "$(dirname "$OUT_DMG")/.dmg-build.XXXXXX")"
STAGE_DIR="$BUILD_DIR/stage"
mkdir -p "$STAGE_DIR"
RAW_DMG="$BUILD_DIR/VibeRunner-raw.dmg"
DEVICE=""

# Clean up mounts and temp files on exit.
cleanup() {
  if [[ -n "$DEVICE" ]]; then
    hdiutil detach "$DEVICE" >/dev/null 2>&1 || true
  fi
  if [[ "${DRY_RUN:-0}" == "1" ]]; then
    echo "(DRY_RUN=1) leaving $BUILD_DIR for inspection" >&2
  else
    rm -rf "$BUILD_DIR"
  fi
}
trap cleanup EXIT

# Copy the .app into the stage and add the Applications alias.
cp -R "$APP_BUNDLE" "$STAGE_DIR/VibeRunner.app"
ln -s /Applications "$STAGE_DIR/Applications"

# Place the background image in a hidden folder so it does NOT
# appear as a loose icon in the Finder window.
mkdir -p "$STAGE_DIR/.background"
cp "$BG_SRC" "$STAGE_DIR/.background/background.png"
if command -v SetFile >/dev/null 2>&1; then
  SetFile -a V "$STAGE_DIR/.background" 2>/dev/null || true
fi

# Build a read-write UDIF (UDRW) dmg from the clean stage directory.
hdiutil create \
  -volname "VibeRunner" \
  -srcfolder "$STAGE_DIR" \
  -ov \
  -format UDRW \
  -fs HFS+ \
  "$RAW_DMG" >/dev/null

# Mount the read-write image, lay out the Finder window, then
# convert to read-only compressed (UDZO).
MOUNT_DIR="/Volumes/VibeRunner"
if [[ -d "$MOUNT_DIR" ]]; then
  hdiutil detach "$MOUNT_DIR" >/dev/null 2>&1 || true
fi

# The image auto-mounts when we open it. Capture the device so we
# can detach it later.
DEVICE=$(hdiutil attach -readwrite -noverify -noautoopen "$RAW_DMG" \
  | awk '/\/Volumes\/VibeRunner/ { print $1; exit }')
if [[ -z "$DEVICE" ]]; then
  echo "failed to mount staging dmg" >&2
  exit 1
fi

# Configure the Finder window: icon view, hide chrome, size to
# 660×420 matching the background image, and place icons cleanly.
osascript <<'OSA' || echo "Note: Finder GUI layout skipped or partially applied" >&2
tell application "Finder"
  tell disk "VibeRunner"
    open
    delay 1
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    set the bounds of container window to {100, 100, 760, 520}
    set opts to the icon view options of container window
    tell opts
      set icon size to 128
      set text size to 13
      set arrangement to not arranged
      try
        set background picture to file "background.png" of folder ".background"
      end try
    end tell
    set position of item "VibeRunner" of container window to {160, 190}
    set position of item "Applications" of container window to {500, 190}
    try
      set position of item ".background" of container window to {2000, 2000}
    end try
    try
      set position of item ".fseventsd" of container window to {2000, 2000}
    end try
    update without registering applications
    delay 1
    close
  end tell
end tell
OSA

# Clean up any transient metadata before detaching.
rm -rf "$MOUNT_DIR/.fseventsd" 2>/dev/null || true
rm -rf "$MOUNT_DIR/.Trashes" 2>/dev/null || true

# Make sure the layout is written back to the image before we detach.
sync
hdiutil detach "$DEVICE" >/dev/null
DEVICE=""

mkdir -p "$(dirname "$OUT_DMG")"
# Re-zip as read-only compressed (UDZO). This is the format Apple
# ships its installers as.
hdiutil convert "$RAW_DMG" \
  -format UDZO \
  -imagekey zlib-level=9 \
  -o "$OUT_DMG" >/dev/null

echo "wrote $OUT_DMG"

