"""Generates the two README diagrams as SVG.

Vector, and written by a script rather than drawn by hand or by a model, for the
same reason the rest of this repository is the way it is: these carry labels and
exact proportions. An image model gets both wrong subtly rather than obviously,
and a diagram that states the wrong thing confidently is worse than no diagram.

Here the geometry comes from the numbers. In `write_order`, the boundary sits at
exactly half because that is the case being described — a card advertising twice
the memory it has. In `layouts`, the defect positions are a list, and both strips
are drawn from the same list, so the two cannot disagree about where the damage
is.

Usage:
    python tools/gen_diagrams.py [--dir docs]
"""

import argparse
import os

# The interface's palette, so the diagrams sit with the screenshots beside them.
BG = "#0b1018"
PANEL = "#141b26"
GOOD = "#34d399"
BAD = "#f43f5e"
ACCENT = "#38bdf8"
TEXT = "#f0f5fa"
DIM = "#a7b5c6"
FAINT = "#6b7c91"

FONT = (
    "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, "
    "'Helvetica Neue', Arial, sans-serif"
)


def head(w, h):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" '
        f'width="{w}" height="{h}" role="img">'
        f'<rect width="{w}" height="{h}" fill="{BG}"/>'
        # One arrowhead definition, reused in both directions by rotating the
        # line rather than defining a mirrored marker.
        f'<defs><marker id="a" viewBox="0 0 10 10" refX="9" refY="5" '
        f'markerWidth="6" markerHeight="6" orient="auto-start-reverse">'
        f'<path d="M0 0 L10 5 L0 10 z" fill="{ACCENT}"/></marker>'
        f'<pattern id="hatch" width="7" height="7" patternUnits="userSpaceOnUse" '
        f'patternTransform="rotate(45)">'
        f'<rect width="7" height="7" fill="{PANEL}"/>'
        f'<line x1="0" y1="0" x2="0" y2="7" stroke="{FAINT}" stroke-width="2" '
        f'opacity="0.5"/></pattern></defs>'
    )


def text(x, y, s, size=13, fill=TEXT, weight="400", anchor="start"):
    return (
        f'<text x="{x}" y="{y}" font-family="{FONT}" font-size="{size}" '
        f'fill="{fill}" font-weight="{weight}" text-anchor="{anchor}">{s}</text>'
    )


def write_order():
    """Why the write pass runs back to front.

    The two strips are identical except for the arrow direction, which is the
    whole point: one variable changes and the verdict inverts.
    """
    w, h = 880, 300
    x0, x1 = 210, 850
    mid = (x0 + x1) // 2
    sh = 46  # strip height
    out = [head(w, h)]

    for top, label, correct in ((62, "front to back", False), (186, "back to front", True)):
        left_fill = GOOD if correct else BAD
        right_fill = BAD if correct else GOOD

        # Direction of travel, above the strip.
        ay = top - 16
        if correct:
            out.append(
                f'<line x1="{x1}" y1="{ay}" x2="{x0}" y2="{ay}" stroke="{ACCENT}" '
                f'stroke-width="1.6" marker-end="url(#a)"/>'
            )
        else:
            out.append(
                f'<line x1="{x0}" y1="{ay}" x2="{x1}" y2="{ay}" stroke="{ACCENT}" '
                f'stroke-width="1.6" marker-end="url(#a)"/>'
            )

        out.append(
            f'<rect x="{x0}" y="{top}" width="{mid - x0}" height="{sh}" '
            f'fill="{left_fill}" rx="3"/>'
        )
        out.append(
            f'<rect x="{mid}" y="{top}" width="{x1 - mid}" height="{sh}" '
            f'fill="{right_fill}" rx="3"/>'
        )
        out.append(
            f'<line x1="{mid}" y1="{top - 6}" x2="{mid}" y2="{top + sh + 6}" '
            f'stroke="{TEXT}" stroke-width="1.6" stroke-dasharray="4 3"/>'
        )

        out.append(text(20, top + 20, label, 14, TEXT, "600"))
        out.append(
            text(
                20,
                top + 38,
                "correct" if correct else "wrong",
                12,
                GOOD if correct else BAD,
                "600",
            )
        )

        out.append(text(x0 + 14, top + 28, "memory that exists", 12, "#06231a" if correct else "#2a0510", "600"))
        out.append(text(mid + 14, top + 28, "memory that does not", 12, "#2a0510" if correct else "#06231a", "600"))

        caption = (
            "the real half verifies, the imaginary half is condemned"
            if correct
            else "high addresses overwrite the low ones they collide with, so the real half fails"
        )
        out.append(text(x0, top + sh + 22, caption, 12, DIM))

    out.append(
        text(mid, 24, "real capacity ends here", 11, DIM, "400", "middle")
    )
    out.append("</svg>")
    return w, h, "".join(out)


def layouts():
    """Fencing against splicing, on one card.

    Both strips take their defect positions from the same list, so the picture
    cannot claim the two mechanisms were given different cards.
    """
    w, h = 880, 260
    x0, x1 = 150, 850
    span = x1 - x0
    sh = 42
    # Defects as fractions of the card, and the largest clean run between them.
    defects = [(0.18, 0.021), (0.33, 0.014), (0.37, 0.026), (0.62, 0.018), (0.79, 0.030)]
    out = [head(w, h)]

    for top, title, spliced in ((58, "Fenced", False), (170, "Spliced", True)):
        out.append(text(20, top + 18, title, 14, TEXT, "600"))

        if spliced:
            # One volume over the whole card; the defects stay visible inside it
            # because they are still there, simply never handed out.
            out.append(
                f'<rect x="{x0}" y="{top}" width="{span}" height="{sh}" fill="{GOOD}" rx="3"/>'
            )
            out.append(text(20, top + 36, "1 drive", 11, FAINT))
        else:
            # Quarantine everywhere except the largest clean run, which here is
            # the stretch between the third and fourth defect.
            run_a = x0 + int(span * (defects[2][0] + defects[2][1]))
            run_b = x0 + int(span * defects[3][0])
            out.append(
                f'<rect x="{x0}" y="{top}" width="{span}" height="{sh}" '
                f'fill="url(#hatch)" rx="3"/>'
            )
            out.append(
                f'<rect x="{run_a}" y="{top}" width="{run_b - run_a}" height="{sh}" fill="{GOOD}"/>'
            )
            out.append(text(20, top + 36, "1 drive", 11, FAINT))

        for frac, width in defects:
            dx = x0 + int(span * frac)
            dw = max(4, int(span * width))
            out.append(f'<rect x="{dx}" y="{top}" width="{dw}" height="{sh}" fill="{BAD}"/>')

        if spliced:
            out.append(text(x1, top - 8, "22.45 GB usable", 13, GOOD, "600", "end"))
            out.append(
                text(
                    x0,
                    top + sh + 21,
                    "one volume across the whole card — every cluster touching a defect is marked in the FAT",
                    12,
                    DIM,
                )
            )
            out.append(
                text(x0, top + sh + 38, "and never allocated. No guard band; files capped at 4 GiB.", 12, DIM)
            )
        else:
            out.append(text(x1, top - 8, "13.51 GB usable", 13, GOOD, "600", "end"))
            out.append(
                text(
                    x0,
                    top + sh + 21,
                    "one volume over the largest clean run; the rest becomes partitions Windows will not mount,",
                    12,
                    DIM,
                )
            )
            out.append(text(x0, top + sh + 38, "each defect kept behind a guard band.", 12, DIM))

    out.append(text(x0, 30, "the same card, with the same defects", 12, FAINT))
    out.append(
        f'<rect x="{x1 - 128}" y="20" width="10" height="10" fill="{BAD}" rx="2"/>'
    )
    out.append(text(x1 - 112, 29, "defective", 11, DIM))
    out.append("</svg>")
    return w, h, "".join(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--dir", default="docs")
    args = ap.parse_args()
    os.makedirs(args.dir, exist_ok=True)

    for name, build in (("write-order", write_order), ("layouts", layouts)):
        w, h, svg = build()
        path = os.path.join(args.dir, f"{name}.svg")
        with open(path, "w", encoding="utf-8") as f:
            f.write(svg)
        print(f"wrote {path} ({w}x{h})")


if __name__ == "__main__":
    main()
