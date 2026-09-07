#!/usr/bin/env python3
"""
Generate the background image used by the VibeRunner .dmg installer
window. The image is placed at the root of the DMG volume and
referenced by the Finder window via `set background picture`.

The layout has:
  - A vertical gradient using the VibeRunner primary → secondary
    palette.
  - A faint play-triangle mark in the top-left (matches the app
    icon).
  - A subtle text label "VibeRunner" along the top.
  - Centred "Drag VibeRunner to Applications ↗" instruction in the
    middle (so when the user opens the DMG they immediately know
    what to do).

You only need to re-run this when you change the brand colors or
the copy. Otherwise `pnpm build:mac` picks up the existing PNG.

Usage: python3 scripts/generate-dmg-background.py
Writes: assets/dmg-background.png
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFont
except ImportError:
    print(
        "This script needs Pillow. Install with:\n"
        "  pip3 install --user pillow",
        file=sys.stderr,
    )
    sys.exit(1)


# ---- Brand palette (mirrors src/styles.css) ----
PRIMARY = (38, 70, 83)        # #264653
SECONDARY = (42, 157, 143)    # #2a9d8f
ACCENT = (233, 196, 106)      # #e9c46a
WHITE = (255, 255, 255)
TEXT_MUTED = (200, 220, 220)
ARROW_TINT = (240, 240, 240)

# Output dimensions. The Finder window is sized to 660×420 in the
# layout, so the background matches exactly (avoids scaling
# artefacts on Retina).
WIDTH, HEIGHT = 660, 420
APP_X, APP_Y = 160, 190
DEST_X, DEST_Y = 500, 190


def lerp_color(c1, c2, t):
    return tuple(int(c1[i] + (c2[i] - c1[i]) * t) for i in range(3))


def make_gradient(width: int, height: int) -> Image.Image:
    """Vertical gradient from PRIMARY (top) to SECONDARY (bottom)."""
    img = Image.new("RGB", (width, height), PRIMARY)
    px = img.load()
    for y in range(height):
        t = y / max(height - 1, 1)
        c = lerp_color(PRIMARY, SECONDARY, t)
        for x in range(width):
            px[x, y] = c
    return img


def find_font(size: int) -> ImageFont.FreeTypeFont:
    """Try a few macOS system fonts in order; fall back to default."""
    candidates = [
        "/System/Library/Fonts/SFNS.ttf",
        "/System/Library/Fonts/SFNSDisplay.ttf",
        "/Library/Fonts/SF Pro Display.ttc",
        "/System/Library/Fonts/Helvetica.ttc",
        "/System/Library/Fonts/HelveticaNeue.ttc",
    ]
    for path in candidates:
        if os.path.exists(path):
            try:
                return ImageFont.truetype(path, size)
            except OSError:
                continue
    return ImageFont.load_default()


def draw_brand_mark(draw: ImageDraw.ImageDraw, x: int, y: int) -> None:
    """A small play-triangle mark — the VibeRunner logo glyph."""
    pad = 6
    w, h = 28, 28
    shadow = [
        (x + pad + 2, y + pad + 3),
        (x + pad + 2, y + pad + h + 3),
        (x + pad + w + 3, y + pad + h // 2 + 3),
    ]
    draw.polygon(shadow, fill=(0, 0, 0, 80))
    tri = [
        (x + pad, y + pad),
        (x + pad, y + pad + h),
        (x + pad + w, y + pad + h // 2),
    ]
    draw.polygon(tri, fill=WHITE)


def draw_arrow(draw: ImageDraw.ImageDraw, start_x: int, end_x: int, y: int) -> None:
    """Draw a smooth directional arrow from start_x to end_x centered at y."""
    shaft_height = 4
    head_len = 22
    head_wing = 16

    # Shadow
    s_y = y + 2
    draw.rounded_rectangle(
        [(start_x, s_y - shaft_height // 2), (end_x - head_len + 4, s_y + shaft_height // 2)],
        radius=2,
        fill=(0, 0, 0, 70),
    )
    head_shadow = [
        (end_x, s_y),
        (end_x - head_len, s_y - head_wing),
        (end_x - head_len + 5, s_y),
        (end_x - head_len, s_y + head_wing),
    ]
    draw.polygon(head_shadow, fill=(0, 0, 0, 70))

    # Main arrow body
    draw.rounded_rectangle(
        [(start_x, y - shaft_height // 2), (end_x - head_len + 4, y + shaft_height // 2)],
        radius=2,
        fill=(255, 255, 255, 200),
    )
    head = [
        (end_x, y),
        (end_x - head_len, y - head_wing),
        (end_x - head_len + 5, y),
        (end_x - head_len, y + head_wing),
    ]
    draw.polygon(head, fill=(255, 255, 255, 240))


def main() -> None:
    here = Path(__file__).resolve().parent
    out = here.parent / "assets" / "dmg-background.png"
    out.parent.mkdir(parents=True, exist_ok=True)

    img = make_gradient(WIDTH, HEIGHT)
    draw = ImageDraw.Draw(img, "RGBA")

    # Top-left: small VibeRunner brand mark + wordmark.
    draw_brand_mark(draw, 32, 28)
    title_font = find_font(22)
    draw.text((80, 32), "VibeRunner", fill=WHITE, font=title_font)
    subtitle_font = find_font(12)
    draw.text((80, 58), "Dev-App Runner Manager", fill=TEXT_MUTED, font=subtitle_font)

    # Centre directional indicator
    centre_x = (APP_X + DEST_X) // 2  # 330
    arrow_y = 190
    draw_arrow(draw, centre_x - 55, centre_x + 55, arrow_y)

    # Instruction above the arrow
    instruction_font = find_font(15)
    hint_font = find_font(12)

    def measure(text: str, font) -> tuple[int, int]:
        if hasattr(font, "getbbox"):
            l, t, r, b = font.getbbox(text)
            return r - l, b - t
        return font.getsize(text)

    inst_text = "Drag to Applications"
    iw, ih = measure(inst_text, instruction_font)
    draw.text(
        (centre_x - iw // 2, arrow_y - 36),
        inst_text,
        fill=WHITE,
        font=instruction_font,
    )

    hint_text = "to install"
    hw, hh = measure(hint_text, hint_font)
    draw.text(
        (centre_x - hw // 2, arrow_y + 20),
        hint_text,
        fill=TEXT_MUTED,
        font=hint_font,
    )

    # Bottom-left: subtle build / version stamp.
    stamp_font = find_font(11)
    draw.text(
        (32, HEIGHT - 28),
        "VibeRunner · Install Disk",
        fill=TEXT_MUTED,
        font=stamp_font,
    )

    # Bottom-right: ejection hint
    eject_text = "Eject disk after copying"
    ew, eh = measure(eject_text, stamp_font)
    draw.text(
        (WIDTH - ew - 32, HEIGHT - 28),
        eject_text,
        fill=TEXT_MUTED,
        font=stamp_font,
    )

    img.save(out, "PNG", dpi=(72, 72), optimize=True)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()

