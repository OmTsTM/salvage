"""Renders the repository banner.

Composited rather than generated. An image model would redraw the lion into
something close to but not the same as the one on the taskbar, approximate the
palette, and misspell the wordmark — three problems that do not exist when the
mark is the actual icon file, the colours are the interface's own constants, and
the text is drawn by a font.

The sector grid is the argument, not decoration: it is rendered at the real
proportion measured on the card this program was built against — 99.9% lost,
and the little that survived. Getting that from a parameter rather than from
taste is why it stays honest if it is ever regenerated.

Usage:
    python tools/gen_banner.py [--out docs/banner.png]
"""

import argparse
import os
import random

from PIL import Image, ImageDraw, ImageFilter, ImageFont

SS = 2  # supersampling, downsampled at the end
W, H = 1280, 420

# The interface's own palette, so the banner and the window are the same object.
BG = (0x0B, 0x10, 0x18)
GOOD = (0x34, 0xD3, 0x99)
BAD = (0xF4, 0x3F, 0x5E)
ACCENT = (0x38, 0xBD, 0xF8)
TEXT = (0xF0, 0xF5, 0xFA)
DIM = (0xA7, 0xB5, 0xC6)

# Survivors, as a fraction. The card this was built against came back with
# 0.075% intact; a little above that keeps the green findable at banner size
# without misrepresenting the result.
SURVIVOR_RATE = 0.018

# The grid, in cells.
COLS, ROWS = 32, 19


def font(names, size):
    """First available of `names`, or the bundled default.

    Banner text is not worth failing a build over: a fallback face is a
    cosmetic loss, a crash is not.
    """
    for name in names:
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    return ImageFont.load_default(size)


def backdrop(img):
    """A pool of light behind the mark, so the left third is not flat black."""
    d = ImageDraw.Draw(img)
    cx, cy = 250 * SS, H * SS // 2
    for i in range(140, 0, -1):
        t = i / 140
        r = int(430 * SS * t)
        v = 1 - t
        d.ellipse(
            [cx - r, cy - r, cx + r, cy + r],
            fill=(
                int(BG[0] + 10 * v),
                int(BG[1] + 14 * v),
                int(BG[2] + 20 * v),
            ),
        )


def grid(img, seed=20260910):
    """The sector map: mostly lost, a scattering intact.

    Drawn into its own layer and faded toward the right edge, so the banner ends
    in background rather than at a hard border.
    """
    rng = random.Random(seed)
    layer = Image.new("RGB", img.size, (0, 0, 0))
    d = ImageDraw.Draw(layer)

    left, top = 846 * SS, 118 * SS
    cell, gap = 11 * SS, 4 * SS

    for r in range(ROWS):
        for c in range(COLS):
            x = left + c * (cell + gap)
            y = top + r * (cell + gap)
            alive = rng.random() < SURVIVOR_RATE
            colour = GOOD if alive else BAD
            # Slight per-cell variation, or the field reads as a printed
            # texture rather than as thousands of individual readings.
            k = 0.74 + rng.random() * 0.26
            fill = tuple(int(v * k) for v in colour)
            d.rounded_rectangle([x, y, x + cell, y + cell], radius=2 * SS, fill=fill)

    # Horizontal fade so the grid dissolves into the background.
    mask = Image.new("L", img.size, 0)
    md = ImageDraw.Draw(mask)
    x0 = 812 * SS
    for x in range(x0, W * SS):
        t = (x - x0) / (W * SS - x0)
        # Eases in over the first third and away over the last quarter, so the
        # field has no edge at either end.
        rise = min(1.0, t / 0.34) ** 1.6
        fall = 1.0 - max(0.0, (t - 0.74) / 0.26) ** 1.4
        md.line([(x, 0), (x, H * SS)], fill=int(255 * max(0.0, rise * fall)))
    img.paste(layer, (0, 0), mask)


def scanline(img):
    """The one cyan element: the sweep the interface draws while it works."""
    glow = Image.new("RGB", img.size, (0, 0, 0))
    d = ImageDraw.Draw(glow)
    y = 214 * SS
    d.rectangle([836 * SS, y - 2 * SS, W * SS, y + 2 * SS], fill=ACCENT)
    glow = glow.filter(ImageFilter.GaussianBlur(7 * SS))

    out = Image.blend(img, Image.new("RGB", img.size, (0, 0, 0)), 0)
    out = Image.eval(out, lambda v: v)
    img.paste(
        Image.blend(img.crop((0, 0, W * SS, H * SS)), glow, 0.0),
        (0, 0),
    )
    # Additive: a blend would darken what it crosses.
    base = img.load()
    gl = glow.load()
    for yy in range(max(0, y - 30 * SS), min(H * SS, y + 30 * SS)):
        for xx in range(820 * SS, W * SS):
            br, bg, bb = base[xx, yy]
            gr, gg, gb = gl[xx, yy]
            base[xx, yy] = (min(255, br + gr), min(255, bg + gg), min(255, bb + gb))

    d2 = ImageDraw.Draw(img)
    d2.rectangle([846 * SS, y, W * SS, y + 1 * SS], fill=ACCENT)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", default="docs/banner.png")
    ap.add_argument("--icon", default="assets/icon-source.png")
    args = ap.parse_args()

    img = Image.new("RGB", (W * SS, H * SS), BG)
    backdrop(img)
    grid(img)
    scanline(img)

    # The mark itself, never a redrawing of it.
    mark_px = 268 * SS
    mark = Image.open(args.icon).convert("RGBA").resize((mark_px, mark_px), Image.LANCZOS)
    mx, my = 78 * SS, (H * SS - mark_px) // 2

    halo = Image.new("RGB", img.size, (0, 0, 0))
    ImageDraw.Draw(halo).rounded_rectangle(
        [mx, my, mx + mark_px, my + mark_px], radius=60 * SS, fill=(0x14, 0x3A, 0x6B)
    )
    halo = halo.filter(ImageFilter.GaussianBlur(34 * SS))
    base, gl = img.load(), halo.load()
    for yy in range(max(0, my - 70 * SS), min(H * SS, my + mark_px + 70 * SS)):
        for xx in range(max(0, mx - 70 * SS), min(W * SS, mx + mark_px + 70 * SS)):
            br, bg, bb = base[xx, yy]
            gr, gg, gb = gl[xx, yy]
            base[xx, yy] = (min(255, br + gr), min(255, bg + gg), min(255, bb + gb))

    img.paste(mark, (mx, my), mark)

    d = ImageDraw.Draw(img)
    title = font(["segoeuisb.ttf", "seguisb.ttf", "arialbd.ttf", "DejaVuSans-Bold.ttf"], 76 * SS)
    sub = font(["segoeui.ttf", "arial.ttf", "DejaVuSans.ttf"], 25 * SS)

    tx = 392 * SS
    title_text = "Salvage"
    tb = d.textbbox((0, 0), title_text, font=title)
    title_h = tb[3] - tb[1]

    # Vertically centred as a block, with the subtitle placed from the title's
    # measured box instead of a guessed offset.
    line_gap = 36 * SS
    block_top = (H * SS - (title_h + line_gap + 72 * SS)) // 2
    d.text((tx, block_top - tb[1]), title_text, font=title, fill=TEXT)

    sy = block_top + title_h + line_gap
    d.text((tx + 3 * SS, sy), "Diagnose a failing microSD and reclaim", font=sub, fill=DIM)
    d.text((tx + 3 * SS, sy + 34 * SS), "the space that still works", font=sub, fill=DIM)

    os.makedirs(os.path.dirname(args.out) or ".", exist_ok=True)
    img.resize((W, H), Image.LANCZOS).save(args.out)
    print(f"wrote {args.out} ({W}x{H})")


if __name__ == "__main__":
    main()
