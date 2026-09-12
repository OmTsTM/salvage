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
  "dots",       // untested — sparse dust, "no information"
  "solid",      // intact — filled, no noise
  "diag-back",  // read error
  "diag-fwd",   // write error
  "cross",      // altered content — a dense weave
  "checker",    // another address's data
  "horiz",      // fenced: guard band, or an earlier layout
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
  watchdog: null,    // catches a scan whose progress events stop arriving
  devices: [],
  selected: null,
  lastSnapshot: null,
  plans: [],
  selectedPlan: null,
  scanning: false,
  fsTouched: false,  // the filesystem menu stops guessing once it is touched
  watchdogAlarmed: false,   // so the alarm can be withdrawn, and only that alarm
};

const $ = (id) => document.getElementById(id);

/* Where the filesystem default flips.
 *
 * Exactly Windows' own FAT32 ceiling — `FORMAT_COM_FAT32_LIMIT` in
 * salvage-win32/src/apply.rs. Below it a card is usually going somewhere that
 * wants maximum compatibility, and every tool can make the volume. Above it
 * exFAT is the norm, and FAT32 becomes a deliberate choice this program is
 * unusually able to carry out.
 *
 * A default, not a rule. The menu keeps both options and the labels say what
 * each costs, so a drift from the backend constant would cost a preselection
 * rather than a guarantee. */
const FAT32_DEFAULT_CEILING_BYTES = 32 * 1024 * 1024 * 1024;

/* Preselects the filesystem the card is most likely for.
 *
 * Left alone once the user has touched the menu: a guess may open the question
 * but must not answer it twice, and re-selecting the same card should not undo
 * a choice already made about it. */
function preselectFilesystem(device) {
  if (state.fsTouched || !device) return;
  const select = $("fs-select");
  if (!select) return;
  select.value =
    device.capacity_bytes < FAT32_DEFAULT_CEILING_BYTES ? "fat32" : "exfat";
}

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


