#!/usr/bin/env python3
"""Generate the VibeRunner 1024×1024 app icon.

Design:
- Rounded square in primary teal (#264653) with a subtle vertical gradient
  toward secondary teal (#2a9d8f) for depth
- White "play" triangle in the center, the universal "run" symbol
- Soft inner shadow for a tactile, modern feel

Output: assets/icon-source.png  (run `pnpm tauri icon` to generate all
the platform-specific sizes from this)
"""

from PIL import Image, ImageDraw, ImageFilter

SIZE = 1024
BG_TOP = (38, 70, 83)        # #264653 (primary)
BG_BOTTOM = (42, 157, 143)   # #2a9d8f (secondary)
FG = (255, 255, 255)         # white triangle

# 1. Background — vertical gradient inside a rounded square mask.
bg = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
draw = ImageDraw.Draw(bg)
for y in range(SIZE):
    t = y / (SIZE - 1)
    r = int(BG_TOP[0] + (BG_BOTTOM[0] - BG_TOP[0]) * t)
    g = int(BG_TOP[1] + (BG_BOTTOM[1] - BG_TOP[1]) * t)
    b = int(BG_TOP[2] + (BG_BOTTOM[2] - BG_TOP[2]) * t)
    draw.line([(0, y), (SIZE, y)], fill=(r, g, b, 255))

# Round the corners. macOS rounded square is ~22% corner radius.
radius = int(SIZE * 0.22)
mask = Image.new("L", (SIZE, SIZE), 0)
mask_draw = ImageDraw.Draw(mask)
mask_draw.rounded_rectangle((0, 0, SIZE - 1, SIZE - 1), radius=radius, fill=255)
rounded_bg = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
rounded_bg.paste(bg, (0, 0), mask)

# 2. Subtle inner shadow (top) for depth.
shadow = Image.new("L", (SIZE, SIZE), 0)
shadow_draw = ImageDraw.Draw(shadow)
shadow_draw.rounded_rectangle(
    (16, 16, SIZE - 17, SIZE - 17),
    radius=int(radius * 0.85),
    fill=180,
)
shadow = shadow.filter(ImageFilter.GaussianBlur(40))
# Use the shadow as a top darkening layer.
shadow_layer = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
shadow_draw_rgba = ImageDraw.Draw(shadow_layer)
shadow_draw_rgba.bitmap((0, 0), shadow, fill=(0, 0, 0, 70))
# Apply only at the top — invert and use as a gradient mask.
# (Simple: just use a soft black-to-transparent gradient at the top.)
top_shade = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
for y in range(SIZE // 2):
    a = int(70 * (1 - y / (SIZE / 2)))
    ImageDraw.Draw(top_shade).line(
        [(0, y), (SIZE, y)], fill=(0, 0, 0, a)
    )
rounded_bg = Image.alpha_composite(rounded_bg, top_shade)

# 3. The play triangle.
# An equilateral-ish triangle, centered, pointing right.
# Triangle vertices:
#   top-left, bottom-left (vertical edge), right (apex)
tri_w = int(SIZE * 0.42)        # triangle width
tri_h = int(SIZE * 0.46)        # triangle height
cx = SIZE // 2
cy = SIZE // 2
# Shift slightly right so the visual center balances the leftward mass.
tri_cx = cx + int(SIZE * 0.04)
tri_left = tri_cx - tri_w // 2
tri_right = tri_cx + tri_w // 2
tri_top = cy - tri_h // 2
tri_bottom = cy + tri_h // 2
triangle = [(tri_left, tri_top), (tri_left, tri_bottom), (tri_right, cy)]

triangle_layer = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
ImageDraw.Draw(triangle_layer).polygon(triangle, fill=FG + (255,))

# 4. Soft drop shadow for the triangle (very subtle).
shadow_tri = Image.new("L", (SIZE, SIZE), 0)
ImageDraw.Draw(shadow_tri).polygon(
    [
        (tri_left + 6, tri_top + 10),
        (tri_left + 6, tri_bottom + 10),
        (tri_right + 6, cy + 10),
    ],
    fill=180,
)
shadow_tri = shadow_tri.filter(ImageFilter.GaussianBlur(20))
shadow_layer_tri = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
ImageDraw.Draw(shadow_layer_tri).bitmap((0, 0), shadow_tri, fill=(0, 0, 0, 60))
rounded_bg = Image.alpha_composite(rounded_bg, shadow_layer_tri)
rounded_bg = Image.alpha_composite(rounded_bg, triangle_layer)

# Save
out = "assets/icon-source.png"
rounded_bg.save(out, "PNG")
print(f"Wrote {out} ({SIZE}x{SIZE})")
