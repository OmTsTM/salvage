"""Renders the support card for the README's Ko-fi section.

Composited from the same pieces as the banner, for the same reason: the lion is
`assets/icon-source.png` itself rather than a redrawing of it, the colours are
the interface's own constants, and the nick is drawn by a font instead of being
approximated by an image model that would misspell it.

The one colour from outside the palette is Ko-fi's red, and only where it says
Ko-fi. A support card that repainted the project in a donation platform's brand
would read as an advertisement wearing the project's clothes.

Usage:
    python tools/gen_support.py [--out docs/support.png]
"""

import argparse
import os

from PIL import Image, ImageDraw, ImageFilter, ImageFont

SS = 2  # supersampling, downsampled at the end
W, H = 880, 260

# The interface's own palette, so this sits with the rest of the page.
BG = (0x0B, 0x10, 0x18)
PANEL = (0x14, 0x1B, 0x26)
GOOD = (0x34, 0xD3, 0x99)
GOLD = (0xD8, 0xB4, 0x5E)
ACCENT = (0x38, 0xBD, 0xF8)
TEXT = (0xF0, 0xF5, 0xFA)
DIM = (0xA7, 0xB5, 0xC6)

# Ko-fi's own red, used only on the words that are Ko-fi's.
KOFI = (0xFF, 0x5E, 0x5B)


def font(names, size):
    """First available of `names`, or the bundled default.

    A fallback face is a cosmetic loss; a crash over a decorative image is not
    worth failing a build for.
    """
    for name in names:
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    return ImageFont.load_default(size)


def glow(img, box, colour, radius, spread):
    """Adds a soft pool of light, so the panel is not flat black."""
    layer = Image.new("RGB", img.size, (0, 0, 0))
    ImageDraw.Draw(layer).rounded_rectangle(box, radius=radius, fill=colour)
    layer = layer.filter(ImageFilter.GaussianBlur(spread))

    base, gl = img.load(), layer.load()
    x0, y0, x1, y1 = box
    for yy in range(max(0, int(y0 - spread * 2)), min(img.height, int(y1 + spread * 2))):
        for xx in range(max(0, int(x0 - spread * 2)), min(img.width, int(x1 + spread * 2))):
            br, bg, bb = base[xx, yy]
            gr, gg, gb = gl[xx, yy]
            base[xx, yy] = (min(255, br + gr), min(255, bg + gg), min(255, bb + gb))


def recovered_bar(d, x, y, w, h):
    """The project's own argument, in one stripe.

    Green for what a card gave back and red for what it never will: the same
    pair the verdict panel draws, at the proportion of the card this program
    was built against. It is here because it is what the donation would be for.
    """
    kept = 0.47  # 15.69 GB of a 33.56 GB card, the one this was written against
    d.rounded_rectangle([x, y, x + w, y + h], radius=h // 2, fill=(0x26, 0x32, 0x44))
    d.rounded_rectangle([x, y, x + int(w * kept), y + h], radius=h // 2, fill=GOOD)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", default="docs/support.png")
    ap.add_argument("--icon", default="assets/icon-source.png")
    args = ap.parse_args()

    img = Image.new("RGB", (W * SS, H * SS), BG)
    d = ImageDraw.Draw(img)

    # The panel, inset, so the card reads as a piece of the interface.
    pad = 14 * SS
    d.rounded_rectangle(
        [pad, pad, W * SS - pad, H * SS - pad], radius=18 * SS, fill=PANEL
    )
    glow(img, (pad, pad, W * SS - pad, H * SS - pad), (0x0E, 0x22, 0x3A), 18 * SS, 26 * SS)

    # The mark itself, never a redrawing of it.
    mark_px = 150 * SS
    mark = Image.open(args.icon).convert("RGBA").resize((mark_px, mark_px), Image.LANCZOS)
    mx, my = 44 * SS, (H * SS - mark_px) // 2
    glow(img, (mx, my, mx + mark_px, my + mark_px), (0x14, 0x3A, 0x6B), 40 * SS, 26 * SS)
    img.paste(mark, (mx, my), mark)

    nick = font(["segoeuisb.ttf", "seguisb.ttf", "arialbd.ttf", "DejaVuSans-Bold.ttf"], 46 * SS)
    line = font(["segoeui.ttf", "arial.ttf", "DejaVuSans.ttf"], 21 * SS)
    small = font(["segoeui.ttf", "arial.ttf", "DejaVuSans.ttf"], 17 * SS)

    tx = 226 * SS
    top = 58 * SS

    # The nick, with the platform named in its own colour beside it.
    nb = d.textbbox((0, 0), "omtstm", font=nick)
    d.text((tx, top - nb[1]), "omtstm", font=nick, fill=TEXT)
    kb = d.textbbox((0, 0), "  ·  ko-fi", font=small)
    d.text(
        (tx + (nb[2] - nb[0]) + 10 * SS, top + (nb[3] - nb[1]) - (kb[3] - kb[1]) - 2 * SS),
        "  ·  ko-fi",
        font=small,
        fill=KOFI,
    )

    y = top + (nb[3] - nb[1]) + 26 * SS
    d.text((tx, y), "If Salvage got your card back, you can buy me a coffee.", font=line, fill=DIM)
    d.text((tx, y + 30 * SS), "Anything from $5, and thank you.", font=line, fill=DIM)

    # The stripe, and what it means, in the same terms the program uses.
    #
    # The caption is measured and the stripe takes what is left, rather than
    # both being guessed: a fallback font is wider than the intended one, and a
    # guessed split clips the last word on whichever machine lacks the face.
    bar_y = y + 76 * SS
    caption = "15.69 GB given back by a 33.56 GB card"
    cb = d.textbbox((0, 0), caption, font=small)
    caption_w = cb[2] - cb[0]

    right_edge = (W - 34) * SS
    gap = 16 * SS
    bar_w = max(120 * SS, right_edge - caption_w - gap - tx)

    recovered_bar(d, tx, bar_y, bar_w, 10 * SS)
    d.text((tx + bar_w + gap, bar_y - 5 * SS - cb[1]), caption, font=small, fill=GOLD)

    os.makedirs(os.path.dirname(args.out) or ".", exist_ok=True)
    img.resize((W, H), Image.LANCZOS).save(args.out)
    print(f"wrote {args.out} ({W}x{H})")


if __name__ == "__main__":
    main()
