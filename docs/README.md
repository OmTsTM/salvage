# docs/

Images referenced by the repository's README. Each one is also described, at the
place it belongs, in an HTML comment inside `README.md` — search for `BANNER`,
`SCREENSHOT` or `DIAGRAM` there.

| File | Size | How to produce it |
|---|---|---|
| `banner.png` | 1280×420 | Image generator — prompt below |
| `screenshot-scan.png` | ~1240×900 | **Screen capture**, not generated |
| `write-order.png` | ~880×300 | **Vector diagram**, not generated |
| `layouts.png` | ~880×260 | **Vector diagram**, not generated |

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

## banner.png — generated

Image models render text badly and hex codes approximately. Both problems are
avoided by asking for **no text at all** and compositing the wordmark afterwards
in any editor, over the empty middle the prompt reserves for it.

**Prompt**

> Wide brand banner for a developer tool, 32:10 aspect ratio, very dark navy
> background, near black, with a soft vignette.
>
> On the left third: a stylised lion's head facing forward, thick golden amber
> mane in flat cel-shaded vector style, confident geometric shapes, no outline
> strokes. The lion is biting down on a small black microSD memory card held
> crosswise in its jaws, angled slightly downward, with a row of gold contact
> pins clearly visible on the card. Warm rim light on the mane from above.
>
> On the right third: an abstract data grid, hundreds of small rounded squares
> in a dense uniform matrix. Most squares are deep crimson red; a scattered
> handful glow emerald green. One thin cyan horizontal line sweeps across the
> grid with a soft glow, like a scanner. The grid fades out toward the right
> edge.
>
> The middle third is empty dark background.
>
> Style: modern software branding, flat vector illustration, high contrast, dark
> UI aesthetic, clean and uncluttered.

**Negative prompt**

> text, letters, words, typography, watermark, signature, photorealistic, 3d
> render, glossy reflections, busy background, drop shadows, cluttered

**Notes**

- Midjourney: append `--ar 32:10 --style raw`.
- The empty middle is deliberate. Place `Salvage` there in a semibold sans
  (Inter, Segoe UI Variable) in near-white, with the line *"Diagnose a failing
  microSD and reclaim the space that still works"* beneath it in `#a7b5c6`.
- If the lion comes out inconsistent with the app icon, generate the grid half
  alone and composite `assets/icon-source.png` on the left instead. That keeps
  the mark identical to the one on the taskbar, which matters more than the
  banner being a single generation.
- The red-to-green ratio is the whole point: the product is a card that is
  mostly dead and the little that survived. If a result comes back balanced or
  mostly green, it is wrong, however pretty.

---

## screenshot-scan.png — captured, not generated

Do not generate this. A generated interface is a fabricated one: it would show
a program that does not exist, in a repository whose entire argument is that it
does not overstate what it found.

Capture the real window during the **verify** phase, around 60–70%, so the frame
carries the card diagram half painted, the address ruler beside it, the
throughput and remaining time under the progress bar, and the verdict panel on
the right.

- Windows: `Win+Shift+S`, window mode, or `Alt+PrtScn`.
- Crop to the window, no desktop around it.
- A card that is genuinely failing makes the better image, and any card being
  inspected is being erased — so use one you have already written off.

---

## write-order.png, layouts.png — vector, not generated

Do not generate these either, for a different reason: they carry labels and
exact proportions, and image models get both wrong in ways that are subtly
misleading rather than obviously broken. A diagram that says the wrong thing
confidently is worse than no diagram.

Build them as SVG, or in any vector editor, from these specifications.

### write-order.png (~880×300)

Two stacked strips, each representing the same counterfeit card whose real
capacity is half what it advertises. Split each strip in half with a dashed
vertical line labelled *real capacity ends here*.

**Top strip — "front to back", marked wrong.** Arrows along the top running
left to right. The left half (the memory that exists) is red; the right half
(which does not exist) is green. Caption: *the working half is condemned, the
imaginary half approved*.

**Bottom strip — "back to front", marked correct.** Arrows running right to
left. The left half is green, the right half red. Caption: *the diagnosis comes
out the right way round*.

### layouts.png (~880×260)

Two stacked strips representing one card, defects as narrow red bands scattered
across an otherwise green length.

**Top — "Fenced".** One green segment, the largest clean run, labelled `D:`
with a size. Everything else hatched grey, labelled *quarantine — Windows does
not mount it*. Sub-caption: *keeps a guard band around every defect*.

**Bottom — "Spliced".** One continuous green segment spanning the whole card,
labelled `D:` with a visibly larger size, with the red defect bands still
visible inside it. Sub-caption: *marked in the FAT, never allocated — no guard
band*.

The point the reader should take: the second recovers more space, and the
caption is where the cost is stated.
