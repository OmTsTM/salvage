/* Salvage — the card, drawn.
 *
 * Loaded as a plain script, in the order index.html lists: this file may use
 * anything the ones before it defined, and defines what the ones after need.
 */
/*
 * Everything painted inside the microSD outline: the sector map, the address
 * ruler beside it and the legend under it. Nothing here reads state or talks
 * to the backend — it is handed numbers and draws them, which is what makes it
 * the one part of the window that can be exercised on its own.
 */

/* ─────────────────────────────────────────────────── card drawing */

const canvas = $("sector-canvas");
const ctx = canvas.getContext("2d", { alpha: false });

function drawBuckets(buckets, sweepFraction, approvedMarks, phase) {
  if (!buckets || buckets.length === 0) return;

  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  if (w === 0 || h === 0) return;

  if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

  const n = buckets.length;
  // Columns chosen to keep cells as close to square as possible.
  const cols = Math.max(1, Math.round(Math.sqrt((n * w) / h)));
  const rows = Math.ceil(n / cols);
  const cw = w / cols;
  const ch = h / rows;

  ctx.fillStyle = "#10151d";
  ctx.fillRect(0, 0, w, h);

  // Grouped by state to reduce context switches, filled with the matching
  // weave — colour and texture together, never colour alone.
  for (let code = 0; code < STATE_COLORS.length; code++) {
    let painted = false;
    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      if (buckets[i] !== code) continue;
      const c = i % cols;
      const r = (i - c) / cols;
      ctx.rect(c * cw, r * ch, cw + 0.6, ch + 0.6);
      painted = true;
    }
    if (painted) {
      ctx.fillStyle = statePattern(ctx, code);
      ctx.fill();
    }
  }

  // Area already written, while the writing is still going on.
  //
  // The pass runs back to front, so after a fraction f the covered region is
  // the last f of the address space. Derived from the progress rather than from
  // the map on purpose: the map has nothing to say about these sectors yet, and
  // that is exactly the point — they are done, not approved.
  if (phase === "writing" && sweepFraction > 0) {
    const from = Math.floor(n * (1 - sweepFraction));
    ctx.fillStyle = WRITTEN_COLOR;
    ctx.beginPath();
    let any = false;
    for (let i = from; i < n; i++) {
      if (buckets[i] !== 0) continue; // a verdict already exists here
      const c = i % cols;
      const r = (i - c) / cols;
      ctx.rect(c * cw, r * ch, cw + 0.6, ch + 0.6);
      any = true;
    }
    if (any) ctx.fill();
  }

  // Approved area that the block fill cannot show.
  //
  // A block takes the worst state inside it, so on a card where the survivors
  // are a fraction of a percent every block reads as damaged and the good area
  // vanishes. Marking it on top keeps both truths on screen: the fill still
  // says "there are defects here", the mark says "and something survived".
  const marks = approvedMarks || [];
  if (marks.length) {
    const r = Math.max(1.5, Math.min(cw, ch) * 0.22);
    ctx.save();
    // A dark rim so the mark reads against every weave underneath it.
    ctx.strokeStyle = "rgba(4,8,14,0.85)";
    ctx.lineWidth = 1.2;
    ctx.fillStyle = STATE_COLORS[1];
    for (const i of marks) {
      const c = i % cols;
      const row = (i - c) / cols;
      ctx.beginPath();
      ctx.arc(c * cw + cw / 2, row * ch + ch / 2, r, 0, Math.PI * 2);
      ctx.fill();
      ctx.stroke();
    }
    ctx.restore();
  }

  // Scan line: shows where the inspection currently is.
  if (state.scanning && sweepFraction > 0 && sweepFraction < 1) {
    const y = sweepFraction * h;
    const grad = ctx.createLinearGradient(0, y - 26, 0, y + 3);
    grad.addColorStop(0, "rgba(56,189,248,0)");
    grad.addColorStop(1, "rgba(56,189,248,0.30)");
    ctx.fillStyle = grad;
    ctx.fillRect(0, Math.max(0, y - 26), w, Math.min(29, h - y + 26));

    ctx.fillStyle = "rgba(125,211,252,0.85)";
    ctx.fillRect(0, y, w, 1.4);
  }
}

/* Graduated address scale, beside the map.
 *
 * Without it the heatmap is a picture: you can see there is red, you cannot say
 * where. With it the map becomes an instrument reading — "the damage starts at
 * 15.7 GB" is a sentence the user can form unaided, just by looking.
 *
 * The scale follows the canvas grid: each row of cells covers a contiguous
 * slice of addresses, so vertical position maps linearly to address. */
function renderScale(capacityBytes) {
  const el = $("address-scale");
  if (!el || !capacityBytes) return;

  const gb = capacityBytes / 1e9;
  // Nearest "round" step that yields between 4 and 8 ticks.
  const steps = [1, 2, 4, 5, 8, 10, 16, 20, 32, 50, 64, 100, 128, 256];
  const step = steps.find((s) => gb / s <= 8) || Math.ceil(gb / 8);

  // The cell grid spans 18.7% to 94.7% of the shell's height; the ruler must
  // land exactly on it, or it points at the wrong place.
  const TOP = 18.7;
  const SPAN = 76;
  const at = (fraction) => TOP + fraction * SPAN;

  let html = "";
  for (let v = 0; v <= gb + 1e-9; v += step) {
    const f = v / gb;
    if (f > 1.005) break;
    // The last regular tick yields to the exact end tick, to avoid collision.
    if (gb - v < step * 0.35) continue;
    html += `<div class="tick" style="top:${at(Math.min(f, 1)).toFixed(3)}%">
               <span class="tick-label">${v === 0 ? "0" : v.toFixed(0)}</span>
             </div>`;
  }
  html += `<div class="tick tick-end" style="top:${at(1)}%">
             <span class="tick-label">${gb.toFixed(1)} GB</span>
           </div>`;
  el.innerHTML = html;
}

function renderLegend(counts, sectorSize, phase) {
  const legend = $("legend");
  legend.innerHTML = "";

  if (phase === "writing") {
    const li = document.createElement("li");
    const sw = document.createElement("span");
    sw.className = "swatch";
    sw.style.background = WRITTEN_COLOR;
    li.append(sw, document.createTextNode(t("legend.written")));
    legend.appendChild(li);
  }
  stateNames().forEach((name, code) => {
    const sectors = counts ? Number(counts[COUNT_KEYS[code]] || 0) : 0;
    // States with no occurrences and no diagnostic relevance stay out of the
    // legend.
    // Uninspected and approved always show, even at zero: between them they
    // answer "how much of this card is actually known to work", and a missing
    // row reads as an answer of none.
    if (sectors === 0 && code !== 0 && code !== 1) return;

    const li = document.createElement("li");
    // The legend repeats the map's weave: without that, the texture would be
    // decoration rather than a reading key.
    const sw = document.createElement("canvas");
    sw.className = "swatch";
    sw.width = sw.height = 24;
    const sg = sw.getContext("2d");
    sg.fillStyle = statePattern(sg, code);
    sg.fillRect(0, 0, 24, 24);
    li.appendChild(sw);
    li.appendChild(document.createTextNode(name));

    if (counts) {
      const c = document.createElement("span");
      c.className = "count";
      c.textContent = humanBytes(sectors * sectorSize);
      li.appendChild(c);
    }
    legend.appendChild(li);
  });
}

function humanBytes(bytes) {
  return window.I18N.bytes(bytes);
}

