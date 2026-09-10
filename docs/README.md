# docs/

Images referenced by the repository's README. Each one is also described, at the
place it belongs, in an HTML comment inside `README.md` — search for `BANNER`,
`SCREENSHOT` or `DIAGRAM` there.

| File | Size | How to produce it |
|---|---|---|
| `banner.png` | 1280×420 | Image generator, with the app icon as input |
| `screenshot-scan.png` | ~1240×900 | Screen capture — the hero image |
| `screenshot-verdict.png` | ~420×900 | Screen capture — the right panel alone |
| `screenshot-consent.png` | ~700×500 | Screen capture — the confirmation dialog |
| `screenshot-layouts.png` | ~420×700 | Screen capture — needs a card with usable area |
| `write-order.png` | ~880×300 | Vector diagram |
| `layouts.png` | ~880×260 | Vector diagram |

Only the banner and `screenshot-scan.png` are worth doing first. Everything else
deepens the page rather than carrying it.

## Palette

So the images sit with the interface rather than beside it.

| Role | Hex |
|---|---|
| Background | `#0b1018` |
| Panel | `#141b26` |
| Approved | `#34d399` |
| Defective | `#f43f5e` |
| Contacts / gold | `#d8b45e` |
| Accent | `#38bdf8` |
| Dim text | `#a7b5c6` |

---

# banner.png — generated, from the icon

`assets/icon-source.png` is the input. The lion must come out **identical** to
the one on the taskbar; only the space around it is generated. A model asked to
draw a lion from a description will draw a different lion, and two lions is
worse than none.

## The reliable way: outpainting

Generative expand preserves the pixels you give it and invents only the new
canvas, which is exactly the split needed here.

1. Make a 1280×420 canvas filled with `#0b1018`.
2. Place `assets/icon-source.png` on the left, scaled to about 300 px tall,
   vertically centred, with roughly 60 px of margin on the left.
3. Select the empty area to its right and generative-expand with the prompt
   below.

Photoshop's Generative Fill, Krea, Flux Fill and Firefly all do this. It is
slower than one generation and it is the only method that cannot redraw the
lion.

**Prompt for the expanded area**

> Dark navy background, near black, continuing seamlessly. On the right side, an
> abstract data grid: hundreds of small rounded squares in a dense uniform
> matrix, most of them deep crimson red, a scattered handful glowing emerald
> green. A single thin cyan horizontal line sweeps across the grid with a soft
> glow, like a scanner. The grid fades out toward the right edge. Flat vector
> style, high contrast, clean, uncluttered. The centre stays empty dark
> background.

## The one-shot way: reference image

Faster, and the lion drifts. Usable if you check the result against the icon
rather than against your memory of it.

**Prompt**

> Using the provided image as an exact, unmodified element: place this lion mark
> on the left third of a wide 32:10 banner, preserving its colours, shading and
> proportions exactly. Do not redraw, restyle or reinterpret it.
>
> Extend the canvas around it with a very dark navy background, near black, with
> a soft vignette. On the right third, add an abstract data grid: hundreds of
> small rounded squares in a dense uniform matrix, most deep crimson red, a
> scattered handful glowing emerald green, with one thin cyan horizontal scan
> line crossing it with a soft glow. The grid fades out toward the right edge.
>
> Leave the middle third as empty dark background.
>
> Flat vector style, modern software branding, high contrast, dark UI aesthetic.

**Negative prompt**

> text, letters, words, typography, watermark, signature, photorealistic, 3d
> render, glossy reflections, busy background, extra animals, second lion,
> redrawn logo, cluttered

**Parameters**

| Tool | Flags |
|---|---|
| Midjourney | image prompt + `--ar 32:10 --iw 2 --style raw` — `--iw 2` weights the reference heavily, which is the point |
| Gemini / Nano Banana | attach the icon and phrase it as an edit: "keep this lion exactly as it is and extend the image around it" |
| Flux Kontext | attach the icon; it is built for edit-style instructions and holds the input better than most |
| DALL·E | weakest at preserving an input; use outpainting instead |

## Whichever route

- The middle third is left empty on purpose. Put `Salvage` there afterwards in a
  semibold sans (Inter, Segoe UI Variable) in near-white, with *"Diagnose a
  failing microSD and reclaim the space that still works"* beneath it in
  `#a7b5c6`. Never ask the model for the text.
- **The red-to-green ratio is the argument**, not decoration: the product is a
  card that is mostly dead and the little that survived. A balanced or mostly
  green grid is wrong however well it renders.
- Compare the output against `assets/icon-source.png` side by side at 100%. Mane
  shape and the angle of the card in the jaws are where drift shows first.

---

# Screenshots — captured, never generated

A generated interface is a fabricated one. This repository's entire argument is
that the program does not overstate what it found; illustrating it with a
picture of software that does not exist contradicts that on the front page.

**Before capturing anything**

- Run the release build, not `cargo run` — the debug build renders identically
  but the window title says the version, and the version in the picture should
  be one somebody can download.
- Windows: `Win+Shift+S` → window mode, or `Alt+PrtScn`. Both capture the window
  alone, without desktop behind it.
- Keep the window at its default size. It was laid out for that, and a stretched
  window puts the card diagram in a proportion nobody will see.
- Language: Portuguese or English, but the **same one across every image**.
- Save as PNG. JPEG artefacts around small text look like a rendering fault.

## screenshot-scan.png — the hero, ~1240×900

The whole window during the **verify** phase, between 60% and 75%.

That range is deliberate: the card diagram is visibly half painted, so a reader
understands at a glance that this fills in as it works. Earlier and it looks
empty; later and it looks static.

Should be legible in the frame:

- the card diagram, partly painted, with the cyan scan line across it;
- the address ruler down its left side — this is what turns the picture into an
  instrument reading;
- under the progress bar: throughput in MB/s and the estimated time remaining;
- the right panel with the verdict forming.

Crop to the window. Full width, so the three columns read as one layout.

## screenshot-verdict.png — the result, ~420×900

The **right panel alone**, after an inspection completes. Crop tightly to the
panel, from the "Resultado" heading to the bottom of the window.

This is the image that shows judgement rather than activity: the usable figure
in green, the confidence chip beside it, the two-tone bar, and the scenario with
its one-line mechanism. Expand the evidence section so the figures behind the
verdict are visible — a claim with its measurements under it is the point.

## screenshot-consent.png — the safety guard, ~700×500

The confirmation dialog, with the device name **partly typed** into the field.

Capture it before the name is complete, so the disabled confirm button is
visible. That single frame says more about the engineering than any paragraph:
the operation cannot proceed until the operator has typed the name of the disk
they are about to erase.

Trigger it by selecting a device and pressing "Inspecionar cartão", then close
the dialog without confirming. Nothing is written until the button is pressed.

## screenshot-layouts.png — the chooser, ~420×700

The isolation section with layouts listed and one selected, so the expanded
detail is visible: the size, the cost line, the partition bar and the rows
beneath it.

**This one needs a card that has usable area left.** A card like the one used in
development — 99.9% condemned, largest clean run 10 MB — produces a refusal
instead, which is a truthful result but not the picture this slot wants. If no
such card is to hand, skip this image rather than staging it; the refusal panel
is not a substitute, because it illustrates the opposite point.

---

# Diagrams — vector, never generated

Not an aesthetic preference. These carry labels and exact proportions, and image
models get both wrong in ways that are subtly misleading rather than obviously
broken. A diagram that states the wrong thing confidently is worse than no
diagram.

Build them as SVG, or in any vector editor, from these specifications.

## write-order.png (~880×300)

Two stacked strips, each the same counterfeit card whose real capacity is half
what it advertises. A dashed vertical line splits each at the midpoint, labelled
*real capacity ends here*.

**Top strip — "front to back", marked wrong.** Arrows along the top running left
to right. The left half, the memory that exists, is red; the right half, which
does not exist, is green. Caption: *the working half is condemned, the imaginary
half approved*.

**Bottom strip — "back to front", marked correct.** Arrows running right to
left. The left half is green, the right half red. Caption: *the diagnosis comes
out the right way round*.

## layouts.png (~880×260)

Two stacked strips representing one card, defects as narrow red bands scattered
across an otherwise green length.

**Top — "Fenced".** One green segment, the largest clean run, labelled `D:` with
a size. Everything else hatched grey, labelled *quarantine — Windows does not
mount it*. Sub-caption: *keeps a guard band around every defect*.

**Bottom — "Spliced".** One continuous green segment spanning the whole card,
labelled `D:` with a visibly larger size, the red defect bands still visible
inside it. Sub-caption: *marked in the FAT, never allocated — no guard band*.

What the reader should take: the second recovers more space, and the sub-caption
is where the cost is stated.
