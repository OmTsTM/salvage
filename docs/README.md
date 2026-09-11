# docs/

Images referenced by the repository's README. Each one is also described, at the
place it belongs, in an HTML comment inside `README.md` — search for `BANNER`,
`SCREENSHOT` or `DIAGRAM` there.

| File | Size | How to produce it |
|---|---|---|
| `banner.png` | 1280×420 | `python tools/gen_banner.py` |
| `screenshot-scan.png` | 1256×939 | **done** |
| `screenshot-verdict.png` | 348×610 | **done** |
| `screenshot-consent.png` | 600×496 | **done** |
| `screenshot-layouts.png` | 331×680 | **done** — rendered, see below |
| `write-order.svg` | 880×300 | `python tools/gen_diagrams.py` |
| `layouts.svg` | 880×260 | `python tools/gen_diagrams.py` |

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

## screenshot-layouts.png — the chooser, 331×680

The only image here not captured from a device, and the reason is worth stating
rather than hiding.

The chooser appears when a card has defects *and* enough clean area between them
to build something. Two real cards were tried and neither produces it:

- A card that is nearly all bad — 99.9% condemned, largest clean run 17 MB —
  refuses every layout, because fencing needs 25 MB contiguous.
- The same card after fencing, re-inspected, comes back with no defect at all,
  and a chooser has nothing to choose between.

So this frame was produced by loading `ui/` in a browser and handing
`buildPlans()` the response shape the backend returns, from
`tools/harness/layouts.html`. **Every pixel is drawn by the shipped interface**
— the same stylesheet, the same `renderPlanList`, the same wording tables. What
is synthetic is the measurement: a 32 GB card with seven defect regions, which
is a plausible card rather than one that was inspected.

That distinction is the line this repository draws. Rendering the real interface
with illustrative numbers shows how the program behaves. Generating a picture of
an interface would show how it does not.

Replace it with a capture the moment a card in that condition is to hand.

---

# Diagrams — vector, generated from the numbers

Produced by [`tools/gen_diagrams.py`](../tools/gen_diagrams.py):

```powershell
python tools/gen_diagrams.py
```

SVG, and written by a script rather than drawn by hand or by a model. These
carry labels and exact proportions, and an image model gets both wrong subtly
rather than obviously — a diagram that states the wrong thing confidently is
worse than no diagram.

The geometry comes from the figures it depicts. In `write-order.svg` the
boundary sits at exactly half the card, because that is the case being
described: one advertising twice the memory it has. In `layouts.svg` both strips
read their defect positions from the same list, so the picture cannot end up
claiming the two mechanisms were handed different cards.

To change either, edit the script and re-run.

## write-order.svg

Two strips, one card, one variable: the direction the write pass travels. Front
to back, high addresses overwrite the low ones they collide with, and on
read-back the memory that genuinely exists is the half that fails — the
diagnosis inverts. Back to front, it comes out the right way round.

## layouts.svg

The same defects under both mechanisms. Fencing keeps the largest clean run and
quarantines everything else; splicing spans the whole card and withholds the
defective clusters in the allocation table. The second recovers more, and the
caption under it is where the cost is stated.
