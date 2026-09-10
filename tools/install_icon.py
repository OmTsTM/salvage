"""Installs an externally supplied image as the application icon.

The icon used to be drawn by `gen_icon.py`. When the mark comes from outside
instead — an illustrator, an image generator — this script is what turns that
one file into every size Windows, the bundler and the interface ask for, so the
tab icon, the installer, the taskbar and the header inside the window can never
drift apart.

It performs no artistic work. What it does do is the part that is easy to get
wrong by hand:

- squares the source without distorting it, padding with the corner colour
  rather than stretching, since a non-square mark otherwise arrives ovalised;
- writes a multi-resolution `.ico` where every size is resampled from the
  full-resolution original, because Windows picks the nearest size rather than
  scaling, and a 16px icon downsampled from an already-downsampled 32px one
  loses the little structure it had;
- emits the small PNG the window header uses, so the interface shows the same
  mark as the taskbar.

Usage:
    python tools/install_icon.py <image> [--dir src-tauri/icons] [--ui ui/brand.png]

The Tauri build embeds the icon through its build script and does not notice the
file changing, so `cargo clean -p salvage-gui` is needed before rebuilding.
"""

import argparse
import os
import sys

from PIL import Image

# Windows picks the closest available size rather than scaling one, so every
# size the shell can ask for is rendered from the original.
ICO_SIZES = (256, 128, 64, 48, 32, 24, 16)

# Sizes the Tauri bundler references by name.
PNG_SIZES = {"32x32.png": 32, "128x128.png": 128, "128x128@2x.png": 256}

# The header mark, at twice its rendered size so it stays sharp on a 200% display.
UI_MARK_PX = 96


def squared(img):
    """Pads the image to a square without distorting it.

    Stretching a non-square mark to fit is the obvious shortcut and the one that
    makes a lion look like it was sat on. The padding takes the corner colour so
    a full-bleed source gains no visible border.
    """
    if img.width == img.height:
        return img
    side = max(img.width, img.height)
    corner = img.convert("RGBA").getpixel((0, 0))
    canvas = Image.new("RGBA", (side, side), corner)
    canvas.paste(img, ((side - img.width) // 2, (side - img.height) // 2), img)
    return canvas


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("image", help="source image, ideally 512px square or larger")
    ap.add_argument("--dir", default="src-tauri/icons", help="where the app icons go")
    ap.add_argument(
        "--ui",
        default="ui/brand.png",
        help="the mark shown in the window header and on the splash",
    )
    args = ap.parse_args()

    if not os.path.isfile(args.image):
        sys.exit(f"não encontrei o arquivo: {args.image}")

    src = squared(Image.open(args.image).convert("RGBA"))
    if min(src.size) < 256:
        print(f"aviso: origem tem {src.width}px; 512px ou mais renderiza melhor a 256px")

    os.makedirs(args.dir, exist_ok=True)
    master = src.resize((512, 512), Image.LANCZOS)

    png = os.path.join(args.dir, "icon.png")
    master.save(png)

    ico = os.path.join(args.dir, "icon.ico")
    master.save(ico, sizes=[(s, s) for s in ICO_SIZES])

    for name, size in PNG_SIZES.items():
        src.resize((size, size), Image.LANCZOS).save(os.path.join(args.dir, name))

    os.makedirs(os.path.dirname(args.ui) or ".", exist_ok=True)
    src.resize((UI_MARK_PX, UI_MARK_PX), Image.LANCZOS).save(args.ui)

    print(f"instalado a partir de {args.image} ({src.width}x{src.height})")
    print(f"  {png}")
    print(f"  {ico}  ({', '.join(str(s) for s in ICO_SIZES)})")
    for name in PNG_SIZES:
        print(f"  {os.path.join(args.dir, name)}")
    print(f"  {args.ui}  (marca do cabeçalho)")
    print("\nrode `cargo clean -p salvage-gui` antes de reconstruir.")


if __name__ == "__main__":
    main()
