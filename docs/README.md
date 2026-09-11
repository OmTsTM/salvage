# docs/

Images referenced by the repository's README. Each one is also described, at the
place it belongs, in an HTML comment inside `README.md` — search for `BANNER`,
`SCREENSHOT` or `DIAGRAM` there.

| File | Size | How to produce it |
|---|---|---|
| `banner.png` | 1280×420 | `python tools/gen_banner.py` |
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

# banner.png — composited, not generated

Produced by [`tools/gen_banner.py`](../tools/gen_banner.py):

```powershell
python tools/gen_banner.py
```

Composited rather than generated, for three reasons an image model cannot
address. The lion is `assets/icon-source.png` itself, so the banner and the
taskbar show the same mark rather than two similar ones. The colours are the
interface's own constants. The wordmark is drawn by a font, so it is spelled
correctly.

The sector field is the argument rather than decoration: `SURVIVOR_RATE` in the
script sets the proportion of intact cells, at a little above the 0.075% the
card this program was built against actually returned. Regenerating it cannot
quietly turn into a prettier, more balanced picture than the truth.

To change it, edit the script and re-run. To replace it with something generated
instead, the composition to match is: mark on the left, wordmark and tagline in
the middle, the red-and-green field bleeding off the right edge, one cyan sweep
across it.

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
