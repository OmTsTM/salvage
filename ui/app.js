/* Salvage — window logic.
 *
 * This layer decides nothing about safety: it presents what the core computed
 * and collects the user's confirmation. Every refusal comes from the backend,
 * which is where the guards are tested.
 *
 * All user-facing strings live in the lookup tables below. The core returns
 * classifications and numbers, never sentences, so translating this interface
 * means replacing those tables and nothing else. */

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

/* Every sentence on this screen comes from i18n.js, and so does every number:
 * a thousands separator is a period in one language and a comma in the next,
 * which is why the backend stopped sending strings it had already formatted. */
const { t, apply: applyI18n } = window.I18N;

/* Mirrors the domain's SectorState. Order matches `state_code` in the backend.
 * Built on demand rather than held in a constant: the names change when the
 * language does, and a frozen array would keep whichever one was chosen first. */
function stateNames() {
  return [0, 1, 2, 3, 4, 5, 6].map((code) => t(`state.${code}`));
}

/* Green is reserved for the one state that was written and read back identical.
 * Nothing else may borrow it: certifying area that was never verified is the
 * costliest mistake this tool can make. */
/* Area the write pass has already covered.
 *
 * Not a sector state and never stored as one: a write that succeeded proves
 * nothing about whether the data will still be there, so the map keeps those
 * sectors uninspected until the verify pass reads them back. This is only what
 * the picture shows meanwhile, so that hours of writing are not spent staring
 * at a map where nothing moves. */
const WRITTEN_COLOR = "#33465f";

const STATE_COLORS = [
  "#232b38", "#34d399", "#fbbf24", "#fb923c", "#f43f5e", "#c084fc", "#64748b",
];

/* Per-state texture.
 *
 * Colour alone cannot carry meaning: someone with red-green colour blindness —
 * roughly 8% of men — cannot tell "approved" from "corrupt" on a purely
 * coloured map. Each state gets its own weave, and the legend shows the same
 * weave beside the name, so the map reads in greyscale. */
const STATE_TEXTURES = [
  "dots",       // não conferido — poeira esparsa, "sem informação"
  "solid",      // íntegro — cheio, sem ruído
  "diag-back",  // erro de leitura
  "diag-fwd",   // erro de gravação
  "cross",      // conteúdo alterado — trama densa
  "checker",    // dado de outro endereço
  "horiz",      // isolado: margem de segurança ou layout anterior
];

/* Pattern cache: rebuilding these per frame would be costly in the draw loop. */
const patternCache = new Map();

function statePattern(ctx, code) {
  const key = `${code}`;
  if (patternCache.has(key)) return patternCache.get(key);

  const color = STATE_COLORS[code];
  const kind = STATE_TEXTURES[code];
  if (kind === "solid") {
    patternCache.set(key, color);
    return color;
  }

  const S = 8;
  const tile = document.createElement("canvas");
  tile.width = tile.height = S;
  const g = tile.getContext("2d");
  g.fillStyle = color;
  g.fillRect(0, 0, S, S);

  // The weave is always darker than the base colour, so it neither shifts the
  // region's perceived brightness nor competes with reading the colour.
  g.strokeStyle = "rgba(4, 8, 14, 0.55)";
  g.fillStyle = "rgba(4, 8, 14, 0.55)";
  g.lineWidth = 1.6;
  g.beginPath();

  switch (kind) {
    case "diag-fwd":
      g.moveTo(-2, 6); g.lineTo(6, -2);
      g.moveTo(2, 10); g.lineTo(10, 2);
      break;
    case "diag-back":
      g.moveTo(-2, 2); g.lineTo(2, -2);
      g.moveTo(-2, 10); g.lineTo(10, -2);
      break;
    case "cross":
      g.moveTo(-2, 2); g.lineTo(10, 14);
      g.moveTo(-2, 6); g.lineTo(6, -2);
      g.moveTo(2, 10); g.lineTo(10, 2);
      break;
    case "horiz":
      g.moveTo(0, 2.5); g.lineTo(S, 2.5);
      g.moveTo(0, 6.5); g.lineTo(S, 6.5);
      break;
    case "vert":
      g.moveTo(2.5, 0); g.lineTo(2.5, S);
      g.moveTo(6.5, 0); g.lineTo(6.5, S);
      break;
    case "checker":
      g.closePath();
      g.fillRect(0, 0, S / 2, S / 2);
      g.fillRect(S / 2, S / 2, S / 2, S / 2);
      break;
    case "dots":
      g.closePath();
      g.beginPath();
      g.arc(2, 2, 1.1, 0, Math.PI * 2);
      g.arc(6, 6, 1.1, 0, Math.PI * 2);
      g.fill();
      break;
  }
  if (kind !== "checker" && kind !== "dots") g.stroke();

  const pattern = ctx.createPattern(tile, "repeat");
  patternCache.set(key, pattern);
  return pattern;
}

/* SectorCounts keys, in the order above. */
const COUNT_KEYS = [
  "untested", "good", "bad_read", "bad_write", "corrupt", "aliased", "fenced",
];

/* Wording for safety blocks and warnings.
 *
 * The core returns the classification; word choice belongs to this layer. */
function blockMessage(b) {
  switch (b.kind) {
    case "hosts_operating_system":
      return t("block.hosts_operating_system", {
        volumes: b.volumes.length
          ? b.volumes.map(t).join(", ")
          : t("block.hosts_operating_system.system"),
      });
    case "internal_bus":
      return t("block.internal_bus", { bus: b.bus });
    case "exceeds_addressable_capacity":
      return t("block.exceeds_addressable_capacity", {
        capacity: humanBytes(b.capacity_bytes),
        limit: humanBytes(b.limit_bytes),
      });
    case "zero_capacity":
      return t("block.zero_capacity");
    default:
      return b.kind;
  }
}

function warningMessage(w) {
  switch (w.kind) {
    case "not_declared_removable":
      return t("warn.not_declared_removable");
    case "has_mounted_volumes":
      return t("warn.has_mounted_volumes", { volumes: w.volumes.join(", ") });
    case "unusually_large":
      return t("warn.unusually_large", { capacity: humanBytes(w.capacity_bytes) });
    case "unknown_bus":
      return t("warn.unknown_bus");
    default:
      return w.kind;
  }
}

/* Filesystems as they are written on the box, not as they are spelled in the
 * enum. "ex_fat" reached the user once, in a list of things that had just been
 * done to their card, which is a poor place to leak an identifier. */
/* The display name of a filesystem, from either spelling of it.
 *
 * `ex_fat` is how the backend enum serialises; `exfat` is the value the
 * selector carries. Both name the same thing, and a confirmation dialog that
 * said "exfat" would be the only place on screen spelling it that way. */
function fsName(kind) {
  return { ex_fat: "exFAT", exfat: "exFAT", fat32: "FAT32" }[kind] || kind;
}

/* Description of each step performed while applying a layout. */
function stepMessage(s) {
  switch (s.step) {
    case "volumes_dismounted":
      return t("step.volumes_dismounted");
    case "volume_warning":
      return t("step.volume_warning", { detail: s[0] ?? "" });
    case "table_changed":
      return t("step.table_changed", {
        slot: s.slot,
        action: t(s.added ? "step.table_changed.added" : "step.table_changed.removed"),
        type: `0x${s.partition_type.toString(16).padStart(2, "0")}`,
        from: formatInt(s.start_lba),
        to: formatInt(s.start_lba + s.sectors),
        n: formatInt(s.sectors),
      });
    case "partition_head_wiped":
      return t("step.partition_head_wiped", { label: s.label });
    case "table_restored":
      return t(s.from_card ? "step.table_restored_card" : "step.table_restored_full");
    case "table_written":
      return t("step.table_written");
    case "system_notified":
      return t("step.system_notified");
    case "volume_mounted":
      return t("step.volume_mounted", { letter: s.letter });
    case "formatted":
      return t("step.formatted", { letter: s.letter, filesystem: fsName(s.filesystem) });
    case "cluster_map_written":
      return t("step.cluster_map_written", { n: formatInt(s.withheld_clusters) });
    case "mount_timed_out":
      return t("step.mount_timed_out");
    default:
      return s.step;
  }
}

/* One measured fact behind the verdict. The backend sends a kind and numbers;
 * the sentence and the number formatting are both chosen here. */
function detailMessage(d, sectorSize) {
  switch (d.kind) {
    case "announced_capacity":
    case "real_capacity":
    case "newly_failed_sectors":
    case "unverified_area":
      return t(`detail.${d.kind}`, { size: humanBytes(d.sectors * sectorSize) });
    case "alias_evidence":
    case "defect_regions":
    case "new_regions":
      return t(`detail.${d.kind}`, { n: formatInt(d.count) });
    default:
      return t(`detail.${d.kind}`);
  }
}


/* A JavaScript error would leave the window frozen with no explanation.
 * Forwarding it to the backend's diagnostic file is what makes that class of
 * failure investigable after the fact. */
function report(message) {
  try { invoke("log_front", { message: String(message) }); } catch (_) { /* ignora */ }
}
window.addEventListener("error", (e) => {
  report(`erro: ${e.message} (${e.filename}:${e.lineno})`);
  showFailure(t("ui.failure", { msg: e.message }));
});
window.addEventListener("unhandledrejection", (e) => {
  report(`promessa rejeitada: ${e.reason}`);
  showFailure(t("ui.failure", { msg: e.reason }));
});

const state = {
  rate: null,        // { phase, t0, sectors0 }, to estimate the time left
  watchdog: null,    // catches a scan that starts and reports no progress
  devices: [],
  selected: null,
  lastSnapshot: null,
  plans: [],
  selectedPlan: null,
  scanning: false,
};

const $ = (id) => document.getElementById(id);

/* ───────────────────────────────────────────────────────────── helpers */

/* Serious errors must not disappear on their own: anyone away from the screen
 * loses the only clue about what happened. They persist until the next action. */
function showFailure(message) {
  const el = $("failure");
  if (!el) return;
  el.textContent = message;
  el.classList.remove("hidden");
}

function clearFailure() {
  const el = $("failure");
  if (el) el.classList.add("hidden");
}

function toast(message, kind = "") {
  const el = $("toast");
  el.textContent = message;
  el.className = `toast ${kind}`;
  clearTimeout(toast._timer);
  toast._timer = setTimeout(() => el.classList.add("hidden"), 6000);
}

function formatInt(n) {
  return window.I18N.nf(n);
}

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

/* ─────────────────────────────────────────────────────────── devices */

async function refreshDevices() {
  try {
    state.devices = await invoke("list_devices");
  } catch (e) {
    /* The list starts out reading "searching". Leaving that in place turns a
     * failed enumeration into a search that never ends, which is the one thing
     * it must not look like. The toast is gone in six seconds; the list is what
     * the user goes on staring at. */
    const select = $("device-select");
    select.innerHTML = `<option value="">${escapeHtml(t("ui.listFailed"))}</option>`;
    showFailure(t(String(e)));
    toast(t(String(e)), "error");
    return;
  }

  clearFailure();
  populateDeviceSelect();

  // With exactly one cleared candidate, select it straight away.
  const usable = state.devices.filter((d) => d.verdict !== "blocked");
  if (usable.length === 1) {
    $("device-select").value = usable[0].path;
    await selectDevice(usable[0].path);
  }
}

/* Fills the picker from what is already in hand. Split out from the fetch so
 * that changing the language can redraw it without going back to the disks. */
function populateDeviceSelect() {
  const select = $("device-select");
  const keep = select.value;
  select.innerHTML = "";

  if (state.devices.length === 0) {
    select.innerHTML = `<option value="">${escapeHtml(t("ui.noDevice"))}</option>`;
    $("no-device").classList.remove("hidden");
    $("device-details").classList.add("hidden");
    $("btn-scan").disabled = true;
    return;
  }

  $("no-device").classList.add("hidden");
  select.appendChild(new Option(t("ui.choose"), ""));
  for (const d of state.devices) {
    /* The physical capacity, even on a card this program already fenced. This
     * is where a card gets *identified*, and somebody hunting for their 64 GB
     * card has to find a 64 GB card. The panel below then leads with what the
     * earlier layout actually left, which is the figure that governs from
     * there on. */
    const suffix = d.verdict === "blocked" ? t("ui.blockedSuffix") : "";
    select.appendChild(
      new Option(`${d.name} · ${humanBytes(d.capacity_bytes)}${suffix}`, d.path));
  }
  select.value = keep;
}

async function selectDevice(path) {
  if (!path) {
    state.selected = null;
    $("device-details").classList.add("hidden");
    $("btn-scan").disabled = true;
    return;
  }

  try {
    state.selected = await invoke("select_device", { path });
  } catch (e) {
    toast(t(String(e)), "error");
    return;
  }

  renderDeviceDetails(state.selected);

  // Switching cards invalidates the previous diagnosis and plans.
  resetResults();
}

/* Everything the panel says about the selected card. Split out because
 * applying a layout changes the answers — the card gains a fenced area at that
 * moment — and the panel has to be able to say so without going back through
 * a selection, which would discard the very measurement the layout was built
 * from. */
function renderDeviceDetails(d) {
  /* On a card an earlier layout fenced, every figure in the window is about
   * what that layout left — the inspection covers only that, so measuring
   * anything else here would label the picture beside it wrongly. The card's
   * own capacity is not hidden, only demoted: it is what is printed on the
   * shell, and a smaller number needs it standing beside it to make sense. */
  const usableBytes = d.prior ? d.prior.inspect_sectors * d.sector_size : d.capacity_bytes;
  const usableSectors = d.prior ? d.prior.inspect_sectors : d.total_sectors;

  $("spec-capacity").innerHTML = d.prior
    ? `${escapeHtml(humanBytes(usableBytes))}<span class="spec-was">` +
      `${escapeHtml(t("spec.of", { total: humanBytes(d.capacity_bytes) }))}</span>`
    : escapeHtml(humanBytes(d.capacity_bytes));
  $("spec-bus").textContent =
    `${d.bus} · ${d.removable ? t("spec.removable") : t("spec.fixed")}`;
  $("spec-sectors").textContent = `${formatInt(usableSectors)} × ${d.sector_size} B`;
  // Drive letters pass through `t` untouched — it returns any key it does not
  // know — which is what lets the one entry that is a key be worded.
  $("spec-volumes").textContent =
    d.volumes.length ? d.volumes.map(t).join(" ") : t("spec.none");

  const box = $("safety-box");
  box.className = `safety ${d.verdict}`;
  const notes = [
    ...(d.blocks || []).map(blockMessage),
    ...(d.warnings || []).map(warningMessage),
  ];
  box.innerHTML = `<strong>${escapeHtml(t(`verdict.${d.verdict}`))}</strong>` +
    (notes.length
      ? `<ul>${notes.map((m) => `<li>${escapeHtml(m)}</li>`).join("")}</ul>`
      : escapeHtml(t("verdict.noNotes")));

  /* A card this program already fenced carries the record in its own partition
   * table, and that changes what the next inspection covers. Saying so here is
   * what keeps the figure in the confirmation from looking like a mistake. */
  const prior = $("prior-note");
  if (d.prior) {
    const scattered = d.prior.data_partitions > 1
      ? `<p>${t("prior.scattered", { count: formatInt(d.prior.data_partitions) })}</p>`
      : "";
    prior.innerHTML =
      `<strong>${escapeHtml(t("prior.title"))}</strong>` +
      `<p>${t("prior.body", {
        remaining: humanBytes(usableBytes),
        fenced: humanBytes(d.prior.fenced_sectors * d.sector_size),
      })}</p>` +
      scattered;
    prior.classList.remove("hidden");
  } else {
    prior.classList.add("hidden");
  }

  /* What an earlier session measured.
   *
   * Stated with its age, because that is what the user has to weigh, and
   * offered as a shortcut rather than as an answer: adopting it restores the
   * picture and the layouts, and approves nothing. */
  const remembered = $("remembered-note");
  if (d.remembered) {
    const age = d.remembered.age_seconds;
    remembered.innerHTML =
      `<strong>${escapeHtml(t("remembered.title"))}</strong>` +
      `<p>${escapeHtml(t("remembered.body", {
        when: age == null ? t("remembered.unknownAge") : humanAge(age),
        approved: humanBytes(d.remembered.approved_bytes),
        defective: humanBytes(d.remembered.defective_bytes),
      }))}</p>` +
      `<p class="remembered-caveat">${escapeHtml(t("remembered.caveat"))}</p>` +
      `<button id="btn-remembered" class="btn btn-ghost">${escapeHtml(t("remembered.use"))}</button>`;
    remembered.classList.remove("hidden");
    $("btn-remembered").addEventListener("click", useRemembered);
  } else {
    remembered.classList.add("hidden");
  }

  // Releasing only means something on a card this program has fenced.
  $("release-box").classList.toggle("hidden", !d.prior);

  $("device-details").classList.remove("hidden");
  $("btn-scan").disabled = d.verdict === "blocked" || state.scanning;
  // The scale appears on selection: the card gains dimension before the scan
  // even starts. It measures the same interval the picture will cover.
  renderScale(usableBytes);

  $("stage-idle").querySelector("p").textContent = d.verdict === "blocked"
    ? t("stage.blocked")
    : t("stage.ready", { name: d.name, size: humanBytes(usableBytes) });
}

function escapeHtml(s) {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}

function resetResults() {
  state.lastSnapshot = null;
  state.plans = [];
  state.selectedPlan = null;
  $("diagnosis").classList.add("hidden");
  $("diagnosis-empty").classList.remove("hidden");
  $("plan-area").classList.add("hidden");
  $("plan-empty").classList.remove("hidden");
  $("plan-list").innerHTML = "";
  $("plan-prepare").classList.add("hidden");
  $("plan-refusal").classList.add("hidden");
  $("plan-actions").classList.add("hidden");
  $("stage-idle").classList.remove("hidden");
  $("progress-bar").style.width = "0";
  $("progress-pct").textContent = "—";
  $("phase-label").textContent = t("phase.waiting");
  $("progress-detail").innerHTML = "&nbsp;";
  renderLegend(null, 512);
}

/* ───────────────────────────────────────────────────────────── inspection */

function applySnapshot(snap) {
  state.lastSnapshot = snap;
  $("stage-idle").classList.add("hidden");
  if (snap.scanning) armWatchdog();

  drawBuckets(snap.buckets, snap.fraction, snap.approved_marks, snap.phase);
  renderLegend(snap.counts, snap.sector_size, snap.phase);
  renderScale(snap.capacity_bytes);

  $("phase-label").textContent = t(`phase.${snap.phase}`);
  $("progress-pct").textContent = `${(snap.fraction * 100).toFixed(1)}%`;
  $("progress-bar").style.width = `${snap.fraction * 100}%`;

  if (snap.scanning) {
    const defects = snap.defects_found > 0
      ? t("progress.defects", { n: formatInt(snap.defects_found) })
      : "";
    $("progress-detail").textContent =
      t("progress.of", {
        done: humanBytes(snap.sectors_done * snap.sector_size),
        total: humanBytes(snap.sectors_total * snap.sector_size),
      }) + defects + estimateRemaining(snap);
  }

  if (snap.report) renderReport(snap.report);
}

/* Estimates time remaining from the rate observed in the current phase.
 *
 * Measurement restarts per phase because writing and reading run at very
 * different speeds on a memory card: reusing the write rate to predict the read
 * would give a consistently wrong number. */
function estimateRemaining(snap) {
  const now = performance.now();

  if (!state.rate || state.rate.phase !== snap.phase) {
    state.rate = { phase: snap.phase, t0: now, sectors0: snap.sectors_done };
    return "";
  }

  const elapsed = (now - state.rate.t0) / 1000;
  const advanced = snap.sectors_done - state.rate.sectors0;
  // Too short a sample does not yet yield a trustworthy rate.
  if (elapsed < 4 || advanced <= 0) return "";

  const perSecond = advanced / elapsed;
  const remaining = Math.max(0, snap.sectors_total - snap.sectors_done);
  const seconds = remaining / perSecond;
  const speed = humanBytes(perSecond * snap.sector_size);

  return t("progress.rate", { speed, time: formatDuration(seconds) });
}

function formatDuration(seconds) {
  if (!isFinite(seconds)) return "—";
  if (seconds < 90) return t("dur.seconds", { n: Math.round(seconds) });
  const m = Math.round(seconds / 60);
  if (m < 60) return t("dur.minutes", { n: m });
  return t("dur.hours", { h: Math.floor(m / 60), m: String(m % 60).padStart(2, "0") });
}

/* Numeric summary of what is condemned. This is what turns a coloured picture
 * into an argument: without the totals, "there is a lot of red" is an
 * impression. */
function renderReport(r) {
  $("diagnosis-empty").classList.add("hidden");
  $("diagnosis").classList.remove("hidden");

  const snap = state.lastSnapshot;
  const ss = snap.sector_size;
  const bytes = (n) => humanBytes(n * ss);

  const total = COUNT_KEYS.reduce((a, k) => a + Number(snap.counts[k] || 0), 0);
  const good = Number(snap.counts.good || 0);
  const bad = ["bad_read", "bad_write", "corrupt", "aliased"]
    .reduce((a, k) => a + Number(snap.counts[k] || 0), 0);
  const rest = Math.max(total - good - bad, 0);

  // The headline is what the user can use, not what was lost. Both numbers are
  // shown, but the one that decides the next action leads.
  $("verdict-figure").textContent = bytes(good);

  const badge = $("assurance-badge");
  badge.className = `assurance ${r.assurance}`;
  badge.textContent = t(`assurance.${r.assurance}`);

  const pct = (n) => (total ? (100 * n / total) : 0);
  $("verdict-bar-good").style.width = `${pct(good).toFixed(2)}%`;
  $("verdict-bar-bad").style.width = `${pct(bad).toFixed(2)}%`;

  let split = t("verdict.split", {
    good: window.I18N.pct(pct(good)),
    bad: window.I18N.pct(pct(bad)),
  });
  if (rest > 0) split += t("verdict.splitUnverified", { rest: window.I18N.pct(pct(rest)) });
  $("verdict-split-text").textContent =
    split + t("verdict.splitTotal", { total: bytes(total) });

  $("scenario-label").textContent = t(`scenario.${r.scenario_kind}`);
  $("scenario-mechanism").textContent = t(`mechanism.${r.scenario_kind}`);
  $("assurance-statement").textContent = t(`statement.${r.assurance}`);

  const details = [
    t("detail.largestRun", { size: escapeHtml(humanBytes(r.largest_usable_bytes)) }),
    ...r.details.map((d) => escapeHtml(detailMessage(d, ss))),
  ];
  $("scenario-details").innerHTML = details.map((d) => `<li>${d}</li>`).join("");

  const rows = stateNames().map((name, code) => {
    const sectors = Number(snap.counts[COUNT_KEYS[code]] || 0);
    // Approved always shows, even at zero: its absence is the whole answer.
    if (sectors === 0 && code !== 1) return "";
    const sw = `<span class="swatch" style="background:${STATE_COLORS[code]}"></span>`;
    return `<tr><td>${sw}${name}</td>` +
           `<td>${bytes(sectors)}</td>` +
           `<td class="num">${formatInt(sectors)}</td></tr>`;
  }).filter(Boolean);
  $("counts-body").innerHTML = rows.join("");
  $("evidence-count").textContent = t("ui.evidenceCount", { n: formatInt(details.length + rows.length) });

  // What zone 3 is for depends on what was found, and there are three
  // answers rather than two. Fencing is one of them.
  if (r.isolation_worthwhile) {
    $("plan-empty").classList.add("hidden");
    $("plan-area").classList.remove("hidden");
    // Computed straight away: the inspection already established what is
    // approved, and making the user press a button to find out what can be
    // built with it only postpones the answer.
    buildPlans();
  } else if (r.scenario_kind === "pristine") {
    // A card with no defects has nothing to fence, which is not the same as
    // having nothing to do. Until this branch existed it fell through to the
    // sentence below, and the best possible result was the only one that ended
    // with no action at all — the card left erased and unpartitioned.
    $("plan-empty").classList.add("hidden");
    $("plan-area").classList.remove("hidden");
    showPrepare();
  } else {
    $("plan-area").classList.add("hidden");
    $("plan-actions").classList.add("hidden");
    $("plan-empty").classList.remove("hidden");
    $("plan-empty").textContent = r.scenario_kind === "not_proven"
      ? t("plan.none.not_proven")
      : t("plan.none.degrading");
  }
}

/* The scan runs on a backend thread. If it dies, stalls, or its events stop
 * arriving, the window would sit at "waiting" forever — exactly the symptom
 * that motivated this watchdog. Twenty seconds is generous even for the first
 * block of a slow card. */
const WATCHDOG_SECONDS = 20;

function armWatchdog() {
  clearTimeout(state.watchdog);
  state.watchdog = setTimeout(() => {
    if (!state.scanning) return;
    report("vigia: nenhum evento de andamento apos o inicio da varredura");
    showFailure(t("scan.watchdog", { n: WATCHDOG_SECONDS }));
  }, WATCHDOG_SECONDS * 1000);
}

function setScanning(on) {
  state.scanning = on;
  state.rate = null;
  if (on) { armWatchdog(); } else { clearTimeout(state.watchdog); }
  $("btn-scan").classList.toggle("hidden", on);
  $("btn-cancel").classList.toggle("hidden", !on);
  $("device-select").disabled = on;
  $("btn-refresh").disabled = on;
}

/* ───────────────────────────────────────────────────────────────── modal */

let modalResolve = null;

/* The typed-name field, the button labels and their styling are the modal's
 * state, not the page's, so every opener sets all of them. Leaving one behind
 * is how applying a layout used to hide the confirmation field for the rest of
 * the session: nothing ever put it back. */
function resetModalChrome() {
  $("modal-typed").classList.remove("hidden");
  $("modal-confirm").classList.remove("hidden");
  $("modal-confirm").className = "btn btn-danger";
  $("modal-confirm").textContent = t("modal.confirm");
  $("modal-cancel").textContent = t("modal.cancel");
}

function openConfirm({ title, bodyHtml, expected }) {
  return new Promise((resolve) => {
    modalResolve = resolve;
    resetModalChrome();
    $("modal-title").textContent = title;
    $("modal-body").innerHTML = bodyHtml;
    $("modal-expected").textContent = expected;
    const input = $("modal-input");
    input.value = "";
    $("modal-error").textContent = "";
    $("modal-confirm").disabled = true;
    $("modal").classList.remove("hidden");
    setTimeout(() => input.focus(), 40);
  });
}

/* A plain two-way question. No name to type: this asks which of two things to
 * do, and neither of them destroys anything that was not already at stake. */
function openChoice({ title, bodyHtml, confirmLabel, cancelLabel }) {
  return new Promise((resolve) => {
    modalResolve = resolve;
    resetModalChrome();
    $("modal-title").textContent = title;
    $("modal-body").innerHTML = bodyHtml;
    $("modal-typed").classList.add("hidden");
    $("modal-confirm").textContent = confirmLabel;
    $("modal-cancel").textContent = cancelLabel;
    $("modal-confirm").disabled = false;
    $("modal").classList.remove("hidden");
    setTimeout(() => $("modal-confirm").focus(), 40);
  });
}

function closeModal(value) {
  $("modal").classList.add("hidden");
  if (modalResolve) {
    modalResolve(value);
    modalResolve = null;
  }
}

$("modal-input").addEventListener("input", (e) => {
  const expected = $("modal-expected").textContent.trim().toLowerCase();
  $("modal-confirm").disabled = e.target.value.trim().toLowerCase() !== expected;
});

$("modal-input").addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !$("modal-confirm").disabled) closeModal($("modal-input").value);
  if (e.key === "Escape") closeModal(null);
});

$("modal-cancel").addEventListener("click", () => closeModal(null));

/* The typed name when one was asked for, plain assent when it was not. An
 * empty string is what the hidden field holds, and it would read as a refusal. */
$("modal-confirm").addEventListener("click", () => {
  const asksForName = !$("modal-typed").classList.contains("hidden");
  closeModal(asksForName ? $("modal-input").value : true);
});

/* Escape has to work with the field hidden too, and it is the field that used
 * to be the only thing listening for it. */
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !$("modal").classList.contains("hidden")) closeModal(null);
});

/* An age in words. Coarse on purpose: the difference between 91 and 94 days is
 * not what the user is weighing, and a precise figure would invite reading it
 * as precision about the card. */
function humanAge(seconds) {
  const day = 86400;
  if (seconds < 3600) return t("age.underHour");
  if (seconds < day) return t("age.hours", { n: Math.floor(seconds / 3600) });
  if (seconds < 60 * day) return t("age.days", { n: Math.floor(seconds / day) });
  return t("age.months", { n: Math.max(2, Math.round(seconds / (30 * day))) });
}

/* Adopts the remembered map, so the diagnosis and the layouts appear without a
 * second inspection. It approves nothing: applying a layout from it still
 * demands an inspection run now, and the backend refuses otherwise. */
async function useRemembered() {
  try {
    await invoke("use_remembered");
    toast(t("remembered.adopted"), "ok");
  } catch (e) {
    toast(t(String(e)), "error");
  }
}

/* Gives the card its capacity back.
 *
 * The wording carries the weight here. This does not undo the inspection — the
 * pattern went over every sector long before any layout existed — and it
 * removes the protection rather than the damage: a card fenced because most of
 * it is dead comes back as one full-size volume that will accept files and lose
 * them. */
async function releaseCard() {
  const d = state.selected;
  if (!d) return;

  const typed = await openConfirm({
    title: t("release.confirmTitle"),
    bodyHtml:
      `<p class="destructive">${escapeHtml(t("release.warning", { name: d.name }))}</p>` +
      `<p>${escapeHtml(t("release.explain"))}</p>` +
      `<p>${escapeHtml(
        d.remembered && d.remembered.can_restore_table
          ? t("release.restores")
          : t("release.invents"))}</p>`,
    expected: d.name,
  });
  if (typed === null) return;

  try {
    const result = await invoke("release_card", {
      typedName: typed,
      filesystem: $("fs-select") ? $("fs-select").value : "exfat",
    });
    const steps = result.steps.map((x) => `<li>${escapeHtml(stepMessage(x))}</li>`).join("");
    resetModalChrome();
    $("modal-title").textContent = t("release.doneTitle");
    $("modal-body").innerHTML = `<ul class="steps">${steps}</ul>`;
    $("modal-typed").classList.add("hidden");
    $("modal-confirm").classList.add("hidden");
    $("modal-cancel").textContent = t("modal.close");
    $("modal").classList.remove("hidden");
    resetResults();
    await refreshDevices();
  } catch (e) {
    toast(t(String(e)), "error");
  }
}

/* ─────────────────────────────────────────────────────────────── layouts */

async function buildPlans() {
  const controls = $("plan-controls");
  const refusal = $("plan-refusal");

  let response;
  try {
    response = await invoke("build_plans", { filesystem: $("fs-select").value });
  } catch (e) {
    showRefusal(`<p>${escapeHtml(t(String(e)))}</p>`);
    return;
  }

  // Failing to produce a layout is a result, not a setback: it answers "can
  // this card be salvaged?". It replaces the chooser rather than appearing
  // under it — a filesystem selector is meaningless once the answer is no.
  if (response.refusal) {
    showRefusal(refusalHtml(response.refusal));
    return;
  }

  controls.classList.remove("hidden");
  refusal.classList.add("hidden");
  state.plans = response.plans;

  // Repaint the card highlighting the good space the guard band consumed: one
  // bad sector condemns a whole erase block, and without this the cost would be
  // invisible.
  if (state.lastSnapshot) {
    state.lastSnapshot = { ...state.lastSnapshot, buckets: response.buckets, counts: response.counts };
    drawBuckets(response.buckets, 1, response.approved_marks);
    renderLegend(response.counts, state.lastSnapshot.sector_size);
  }
  if (Number(response.counts.fenced || 0) > 0) {
    toast(t("plan.fencedToast", { size: humanBytes(response.fenced_bytes) }));
  }

  renderPlanList();
}

/* Puts the reason in place of the chooser, and takes the action away with it. */
function showRefusal(bodyHtml) {
  $("plan-controls").classList.add("hidden");
  $("plan-actions").classList.add("hidden");
  $("plan-prepare").classList.add("hidden");
  $("plan-list").innerHTML = "";
  const box = $("plan-refusal");
  box.innerHTML = `<strong>${escapeHtml(t("plan.cannot"))}</strong>${bodyHtml}`;
  box.classList.remove("hidden");
}

/* Explains a refusal with the measurements behind it.
 *
 * "No usable area" next to a figure of 94 MB reads as a contradiction. What is
 * true is that the approved space is in pieces, and every mechanism has a
 * minimum contiguous size that none of the pieces reaches. */
function refusalHtml(r) {
  const rows = [
    [t("refusal.approved"), `${humanBytes(r.approved_bytes)} — ${t("refusal.inPieces", { n: r.approved_runs })}`],
    [t("refusal.largest"), humanBytes(r.largest_run_bytes)],
    [t("refusal.fencedNeeds"), humanBytes(r.fenced_needs_bytes)],
  ];
  if (r.spliced_needs_bytes != null) {
    rows.push([t("refusal.splicedNeeds"), humanBytes(r.spliced_needs_bytes)]);
  }
  return `<p>${escapeHtml(t("refusal.lead"))}</p>
     <dl class="refusal-figures">${
       rows.map(([k, v]) =>
         `<dt>${escapeHtml(k)}</dt><dd>${escapeHtml(v)}</dd>`).join("")
     }</dl>
     <p class="refusal-why">${escapeHtml(t("refusal.why"))}</p>`;
}

/* Offers the one step a clean card still needs.
 *
 * The inspection writes over every sector, so a card leaves it erased whatever
 * the verdict. That was the whole result for an intact card: nothing to fence,
 * therefore nothing offered, and a working card handed back unusable. The
 * filesystem selector and the volume name stay — they are exactly the two
 * choices left — and the layout list is not drawn, because there is no layout. */
function showPrepare() {
  $("plan-controls").classList.remove("hidden");
  $("plan-refusal").classList.add("hidden");
  $("plan-list").innerHTML = "";

  // A map adopted from a stored record says the card was intact months ago.
  // The backend refuses to write on one, and saying so here is the difference
  // between reading why and finding out after typing the device name into a
  // confirmation dialog.
  const fresh = !state.lastSnapshot || state.lastSnapshot.verified_now;

  const box = $("plan-prepare");
  box.innerHTML =
    `<strong>${escapeHtml(t("prepare.title"))}</strong>` +
    `<p>${escapeHtml(t("prepare.body"))}</p>` +
    (fresh
      ? `<p class="prepare-erased">${escapeHtml(t("prepare.erased"))}</p>`
      : `<p class="prepare-erased">${escapeHtml(t("prepare.needsFresh"))}</p>`);
  box.classList.remove("hidden");

  $("btn-apply").classList.add("hidden");
  const button = $("btn-prepare");
  button.disabled = !fresh;
  button.classList.remove("hidden");
  $("plan-actions").classList.remove("hidden");
}

/* Writes one volume across a card the inspection found intact.
 *
 * Confirmed by the typed device name like every other write to a partition
 * table. The card is already erased by this point, so what the confirmation
 * guards is the disk it lands on, not the data on it — and getting that wrong
 * would format a different card. */
async function prepareCard() {
  const d = state.selected;
  if (!d) return;

  const label = volumeLabel();
  const typed = await openConfirm({
    title: t("prepare.confirmTitle"),
    bodyHtml:
      `<p class="destructive">${escapeHtml(t("prepare.warning", { name: d.name }))}</p>` +
      `<p>${escapeHtml(t("prepare.willWrite", {
        fs: fsName($("fs-select").value),
        label,
      }))}</p>`,
    expected: d.name,
  });
  if (typed === null) return;

  toast(t("prepare.working"), "");
  try {
    const result = await invoke("prepare_card", {
      typedName: typed,
      filesystem: $("fs-select").value,
      label,
    });

    const steps = result.steps.map((x) => `<li>${escapeHtml(stepMessage(x))}</li>`).join("");
    // Its own wording, not the one fencing uses. "The reliable area is at F:"
    // describes a volume carved out of a card; here the volume is the card.
    const where = result.drive_letter
      ? `<p>${t("prepare.where", { letter: escapeHtml(result.drive_letter) })}</p>`
      : "";
    resetModalChrome();
    $("modal-title").textContent = t("prepare.doneTitle");
    $("modal-body").innerHTML = `${where}<ul class="steps">${steps}</ul>`;
    $("modal-typed").classList.add("hidden");
    $("modal-confirm").classList.add("hidden");
    $("modal-cancel").textContent = t("modal.close");
    $("modal").classList.remove("hidden");
    toast(t("prepare.done"), "ok");
    await refreshDevices();
  } catch (e) {
    toast(t(String(e)), "error");
  }
}

/* The name the volume will carry, or a usable one when the field is empty.
 *
 * Trimmed here and sanitised again in the backend, which owns the rules: both
 * filesystems accept eleven characters from a restricted set, and a label the
 * user typed is not obliged to know that. */
function volumeLabel() {
  const field = $("label-input");
  const typed = field ? field.value.trim() : "";
  return typed || "SALVAGE";
}

/* Draws the layout chooser.
 *
 * Kept apart from fetching them: `buildPlans` talks to the backend and repaints
 * the card diagram, and mixing that with the drawing made the list impossible
 * to exercise on its own. */
function renderPlanList() {
  $("plan-prepare").classList.add("hidden");
  $("btn-prepare").classList.add("hidden");
  $("btn-apply").classList.remove("hidden");
  const list = $("plan-list");
  list.innerHTML = "";

  // A single-select list, not a stack of open cards. Every layout shows its
  // name, its size and its cost in one row; only the chosen one opens. The
  // decision is which row to pick, so that is what stays on screen.
  state.plans.forEach((p, i) => {
    const el = document.createElement("div");
    el.className = "plan";
    el.dataset.index = p.index;
    el.setAttribute("role", "radio");
    el.setAttribute("aria-checked", "false");
    el.tabIndex = i === 0 ? 0 : -1;

    const segments = p.partitions
      .map((part) =>
        `<div class="plan-seg ${part.role}" style="left:${part.offset_percent}%;` +
        `width:${Math.max(part.length_percent, 0.6)}%"></div>`)
      .join("");

    const parts = p.partitions
      .map((part) => {
        const kind = t(part.role === "data" ? "plan.visible" : "plan.hidden");
        const label = t("plan.part", {
          label: escapeHtml(part.label),
          kind,
          type: part.mbr_type,
        });
        return `<div><span>${label}</span>` +
               `<span>${escapeHtml(humanBytes(part.size_bytes))}</span></div>`;
      })
      .join("");

    el.innerHTML =
      `<div class="plan-head">
         <span class="plan-name">${escapeHtml(t(`strategy.${p.strategy}`))}</span>
         <span class="plan-size">${escapeHtml(humanBytes(p.usable_bytes))}</span>
       </div>
       <div class="plan-cost">${escapeHtml(t(`cost.${p.strategy}`))}</div>
       <div class="plan-body">
         <p class="plan-note">${escapeHtml(t(`note.${p.strategy}`))}</p>
         <div class="plan-bar">${segments}</div>
         <div class="plan-parts">${parts}</div>
         ${Number(p.sacrificed_bytes) > 0
           ? `<p class="plan-note plan-sacrificed">${escapeHtml(
                t("plan.sacrificed", { size: humanBytes(p.sacrificed_bytes) }))}</p>`
           : ""}
       </div>`;

    const choose = () => {
      list.querySelectorAll(".plan").forEach((n) => {
        n.classList.remove("selected");
        n.setAttribute("aria-checked", "false");
        n.tabIndex = -1;
      });
      el.classList.add("selected");
      el.setAttribute("aria-checked", "true");
      el.tabIndex = 0;
      state.selectedPlan = p;
      $("plan-actions").classList.remove("hidden");
    };

    el.addEventListener("click", choose);
    el.addEventListener("keydown", (e) => {
      if (e.key === " " || e.key === "Enter") {
        e.preventDefault();
        choose();
        return;
      }
      // Arrow keys move within a radio group, which is what this behaves like.
      const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
      if (!step) return;
      e.preventDefault();
      const all = [...list.querySelectorAll(".plan")];
      const next = all[(all.indexOf(el) + step + all.length) % all.length];
      next.click();
      next.focus();
    });

    list.appendChild(el);
  });

  // Preselect the first, which is the layout that keeps the widest margin.
  if (list.firstChild) list.firstChild.click();
}

async function applyPlan() {
  const plan = state.selectedPlan;
  const d = state.selected;
  if (!plan || !d) return;

  const dataParts = plan.partitions.filter((p) => p.role === "data");
  const hiddenParts = plan.partitions.filter((p) => p.role === "quarantine");

  const body =
    `<p class="destructive">${escapeHtml(t("apply.erases", { name: d.name }))}</p>` +
    `<p>${t("apply.layoutIs", { name: escapeHtml(t(`strategy.${plan.strategy}`)) })}</p>` +
    `<ul><li>${t("apply.dataParts", {
      n: formatInt(dataParts.length),
      size: escapeHtml(humanBytes(plan.usable_bytes)),
    })}</li>` +
    `<li>${t("apply.hiddenParts", { n: formatInt(hiddenParts.length) })}</li></ul>` +
    `<div class="warn-list">${escapeHtml(t("apply.caveat"))}</div>`;

  const typed = await openConfirm({
    title: t("apply.confirmTitle"),
    bodyHtml: body,
    expected: d.name,
  });
  if (typed === null) return;

  toast(t("apply.applying"), "");
  try {
    const result = await invoke("apply", {
      planIndex: plan.index,
      typedName: typed,
      label: volumeLabel(),
      filesystem: $("fs-select").value,
    });

    // The card is fenced now, and every figure in the panel is about what the
    // layout left. Redrawn from the table the backend read back, not from the
    // plan — and without going through a selection, which would throw away the
    // measurement this layout was built from.
    if (state.selected) {
      state.selected.prior = result.prior;
      renderDeviceDetails(state.selected);
      // And the picture has to be redrawn over the same interval the ruler now
      // measures. The backend restricts both to the layout's data area; a
      // canvas still spanning the whole card under a ruler measuring part of it
      // would put every position beside the wrong address.
      const fresh = await invoke("snapshot").catch(() => null);
      if (fresh) applySnapshot(fresh);
    }

    const steps = result.steps.map((s) => `<li>${escapeHtml(stepMessage(s))}</li>`).join("");
    const where = result.drive_letter
      ? `<p>${t("apply.where", { letter: escapeHtml(result.drive_letter) })}</p>`
      : "";
    resetModalChrome();
    $("modal-title").textContent = t("apply.doneTitle");
    $("modal-body").innerHTML = `${where}<ul class="steps">${steps}</ul>`;
    $("modal-typed").classList.add("hidden");
    $("modal-confirm").classList.add("hidden");
    $("modal-cancel").textContent = t("modal.close");
    $("modal").classList.remove("hidden");
    toast(t("apply.done"), "ok");
  } catch (e) {
    toast(t(String(e)), "error");
  }
}

/* ────────────────────────────────────────────────────────────── events */

/* Fills the language picker and redraws everything the language touches.
 *
 * Markup-borne text is handled by `applyI18n`; everything else on this screen
 * was built from data by a render function, and those have to run again. The
 * order matters: the panel writes the idle caption, and a snapshot in hand
 * replaces it. */
function relocalize() {
  applyI18n();

  const select = $("lang-select");
  select.innerHTML = "";
  for (const code of window.I18N.order) {
    select.appendChild(new Option(window.I18N.name(code), code));
  }
  select.value = window.I18N.current();

  const snap = state.lastSnapshot;
  renderLegend(snap ? snap.counts : null, snap ? snap.sector_size : 512,
               snap ? snap.phase : null);
  if (state.devices.length) populateDeviceSelect();
  if (state.selected) renderDeviceDetails(state.selected);
  if (snap) applySnapshot(snap);
  if (state.plans.length) renderPlanList();
}

$("lang-select").addEventListener("change", (e) => {
  if (window.I18N.set(e.target.value)) relocalize();
});

/* Opened by the backend in the user's own browser. An anchor here would
 * navigate this window to the page and take the program off the screen. */
$("author-link").addEventListener("click", () => {
  invoke("open_author_page").catch((e) => toast(t(String(e)), "error"));
});

/* The backend opens its own log by its own path — the click carries nothing.
 * Which is also why this still works when the path never arrived and the
 * footer is showing a dash. */
$("log-path").addEventListener("click", () => {
  invoke("open_diagnostics").catch((e) => toast(t(String(e)), "error"));
});

$("btn-refresh").addEventListener("click", refreshDevices);
$("device-select").addEventListener("change", (e) => selectDevice(e.target.value));

async function startScan() {
  const d = state.selected;
  if (!d) return;

  const notes = [
    ...(d.blocks || []).map(blockMessage),
    ...(d.warnings || []).map(warningMessage),
  ];
  const warnings = notes.length
    ? `<div class="warn-list"><strong>${escapeHtml(t("scan.warningTitle"))}</strong><ul>` +
      notes.map((m) => `<li>${escapeHtml(m)}</li>`).join("") + `</ul></div>`
    : "";

  const scope = d.prior
    ? t("scan.scopePrior", {
        size: humanBytes(d.prior.inspect_sectors * d.sector_size),
        name: escapeHtml(d.name),
      })
    : t("scan.scopeAll", {
        name: escapeHtml(d.name),
        size: humanBytes(d.capacity_bytes),
      });

  const typed = await openConfirm({
    title: t("scan.confirmTitle"),
    bodyHtml: t("scan.confirmBody", { scope }) + warnings,
    expected: d.name,
  });
  if (typed === null) return;

  clearFailure();
  resetResults();
  setScanning(true);
  try {
    await invoke("start_scan", { typedName: typed });
  } catch (e) {
    setScanning(false);
    toast(t(String(e)), "error");
  }
}

$("btn-scan").addEventListener("click", startScan);

$("btn-cancel").addEventListener("click", () => invoke("cancel_scan").catch(() => {}));
// The layouts depend only on the filesystem, so they follow it automatically.
/* Stamps the running version beside the wordmark.
 *
 * Failing quietly is deliberate: a missing badge is a cosmetic loss, while a
 * badge showing the wrong number would be quoted back in a bug report and send
 * somebody looking at the wrong build. */
(async () => {
  try {
    const version = await invoke("app_version");
    if (!version) return;
    const el = $("app-version");
    el.textContent = `v${version}`;
    el.hidden = false;
  } catch (e) {
    report(`could not read the version: ${e}`);
  }
})();

// The filesystem choice feeds two different panels. On a clean card there are
// no layouts to recompute — asking for them would fetch the refusal a card with
// no defects gets, and put "this card cannot be partitioned" over the offer to
// format it.
$("fs-select").addEventListener("change", () => {
  if ($("plan-prepare").classList.contains("hidden")) buildPlans();
});
$("btn-apply").addEventListener("click", applyPlan);
$("btn-release").addEventListener("click", releaseCard);
$("btn-prepare").addEventListener("click", prepareCard);

listen("scan:progress", (e) => applySnapshot(e.payload));

listen("scan:done", (e) => {
  setScanning(false);
  applySnapshot(e.payload);
  $("phase-label").textContent = t("phase.done");
  $("progress-pct").textContent = "100%";
  $("progress-bar").style.width = "100%";
  $("progress-detail").textContent =
    t("progress.inspected", { size: humanBytes(e.payload.capacity_bytes) });
  toast(t("scan.done"), "ok");
});

listen("scan:error", (e) => {
  setScanning(false);
  showFailure(t(String(e.payload)));
  toast(t(String(e.payload)), "error");
});

/* Cancelling is not a failure, so it gets neither the persistent banner nor an
 * error-coloured toast — and it carries no text from the backend, which writes
 * its error strings for the log and the command-line tools, in English. One
 * line, here, in the language of the window. */
listen("scan:cancelled", () => {
  setScanning(false);
  clearFailure();
  toast(t("scan.cancelled"), "");
});

listen("scan:warning", (e) => toast(t(String(e.payload)), ""));

/* Closing mid-inspection is the one moment the program has to insist on being
 * asked. The card is dismounted and half overwritten, and the backend holds
 * the window open until this is answered. */
let closePromptOpen = false;
listen("app:close-requested", async () => {
  // A second click on the X while the question is already up must not stack
  // another copy of it behind the first.
  if (closePromptOpen) return;
  closePromptOpen = true;

  const name = state.selected ? escapeHtml(state.selected.name) : t("ui.thecard");
  const stop = await openChoice({
    title: t("close.title"),
    bodyHtml: t("close.body", { name }),
    confirmLabel: t("close.stop"),
    cancelLabel: t("close.keep"),
  });
  closePromptOpen = false;

  if (stop) {
    toast(t("close.leaving"), "");
    invoke("stop_and_close").catch((e) => showFailure(t(String(e))));
  }
});

window.addEventListener("resize", () => {
  if (state.lastSnapshot) {
    drawBuckets(state.lastSnapshot.buckets, state.lastSnapshot.fraction,
                state.lastSnapshot.approved_marks, state.lastSnapshot.phase);
  }
});

/* ────────────────────────────────────────────────────────────── startup */

/* The splash holds the screen until this one has something worth looking at,
 * and the device list is the first thing anyone reads here.
 *
 * The `finally` is the point of the block, not an afterthought: whatever
 * happens on the way, the window is what has to end up on screen. A failure
 * cannot be reported onto a splash — there is nothing on it to report into,
 * and the backend's deadline would be the only thing left to save the launch. */
(async () => {
  relocalize();

  invoke("diagnostics_path")
    .then((p) => { $("log-path").textContent = p; })
    .catch(() => {});

  try {
    await refreshDevices();
  } finally {
    invoke("finish_launch").catch((e) => report(`finish_launch: ${e}`));
  }
})();
