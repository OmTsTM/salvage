/* Salvage — the window's own logic.
 *
 * What is left after the card drawing, the dialog and the fencing panel moved
 * to files of their own: choosing a device, running an inspection, wiring the
 * controls, and the launch that puts the window on screen. Loaded last, so
 * everything it reaches for is already defined.
 */

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
    // The banner alone. Both at once put the same sentence in two boxes that
    // overlap: the toast sits at 22px from the bottom and the banner at 46px,
    // and the banner is the right one here — a device list that failed to load
    // is standing context, not a passing notice.
    showFailure(t(String(e)));
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
      `<button id="btn-remembered" class="btn btn-ghost">${escapeHtml(t("remembered.use"))}</button>` +
      // Offered only where there is a pattern to compare against. A record from
      // before that was stored has nothing to check the card's memory with.
      (d.remembered.can_recheck
        ? `<p class="remembered-caveat">${escapeHtml(t("recheck.offer"))}</p>` +
          `<button id="btn-recheck" class="btn btn-ghost">${escapeHtml(t("recheck.button"))}</button>`
        : "");
    remembered.classList.remove("hidden");
    $("btn-remembered").addEventListener("click", useRemembered);
    if (d.remembered.can_recheck) {
      $("btn-recheck").addEventListener("click", recheckRetention);
    }
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
  $("dead-run").classList.add("hidden");
  renderLegend(null, 512);
}

/* ───────────────────────────────────────────────────────────── inspection */

function applySnapshot(snap) {
  state.lastSnapshot = snap;
  $("stage-idle").classList.add("hidden");
  renderDeadRun(snap);

  if (snap.scanning) {
    armWatchdog();
    // Progress is the answer to the only question the watchdog asks. Left
    // standing, its banner sat beside a live progress bar for the rest of the
    // scan, the two saying opposite things about the same run.
    if (state.watchdogAlarmed) {
      state.watchdogAlarmed = false;
      clearFailure();
    }
  }

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
  renderRetention(r.retention);

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

/* How much has to fail in a row before the offer to stop appears.
 *
 * The note costs attention, so it should only arrive where the time it could
 * save is worth the interruption. A gigabyte of unbroken damage is minutes of
 * reading on a healthy card and considerably more on a sick one, because a
 * sector that fails takes far longer to fail than a good one takes to pass:
 * this card read its damaged half at 5 MB/s against 23 MB/s writing. */
const DEAD_RUN_THRESHOLD_BYTES = 1024 * 1024 * 1024;

/* Offers the choice to stop, without making the prediction.
 *
 * The temptation here is to say the rest of the card is bad. The program must
 * not: it has not looked, and its whole claim rests on never reporting what it
 * did not measure. So the note states three measured things — where the damage
 * began, what stopping preserves, and what stopping gives up — and leaves the
 * inference to the person, who is entitled to make it.
 *
 * Stopping costs no usable space. Unexamined area is withheld from data exactly
 * as failed area is, so the approved total is the same either way. What it does
 * cost is the diagnosis: a card that lies about its capacity gives itself away
 * in the tail, and area that survived past the damage would be found there too. */
function renderDeadRun(snap) {
  const box = $("dead-run");
  const run = snap.dead_run;
  if (!run || run.bytes < DEAD_RUN_THRESHOLD_BYTES) {
    box.classList.add("hidden");
    return;
  }

  $("dead-run-lead").textContent = t("dead.lead", {
    from: humanBytes(run.start_bytes),
    size: humanBytes(run.bytes),
  });
  $("dead-run-keeps").textContent = t("dead.keeps", {
    approved: humanBytes(Number(snap.counts.good || 0) * snap.sector_size),
  });
  $("dead-run-costs").textContent = t("dead.costs");
  box.classList.remove("hidden");
}

/* The scan runs on a backend thread. If it dies, stalls, or its events stop
 * arriving, the window would sit at "waiting" forever — exactly the symptom
 * that motivated this watchdog. Progress arrives every 120 ms, so twenty
 * seconds of silence is abnormal even on the slowest card. */
const WATCHDOG_SECONDS = 20;

/* How late a timer may fire and still be believed.
 *
 * A timer that fires two seconds late lost a race with a busy main thread. One
 * that fires twenty minutes late did not measure twenty minutes of silence —
 * it was not running, and observed nothing at all. That happens whenever the
 * machine suspends, and a scan of a large card is exactly the thing left
 * running overnight: the timer is deferred until the machine wakes, then fires
 * at once, and the window would accuse a healthy scan of having stopped.
 *
 * This is what a 0.5.6 log recorded. A 33 GB card stalled for 1,243 s between
 * two defect lines, the alarm fired at the end of that gap rather than twenty
 * seconds into it, and the scan then resumed on the very next block at its
 * normal pace. Nothing had gone wrong; the window had simply been asleep. */
const WATCHDOG_TOLERANCE_MS = 2000;

function armWatchdog() {
  clearTimeout(state.watchdog);
  const deadline = Date.now() + WATCHDOG_SECONDS * 1000;
  state.watchdog = setTimeout(() => {
    if (!state.scanning) return;

    // The clock is the witness, not the timer. If the callback arrives far
    // past its own deadline, the interval it was meant to measure went
    // unobserved, so the scan gets a fresh window to prove it is alive.
    if (Date.now() - deadline > WATCHDOG_TOLERANCE_MS) {
      report("watchdog fired long after its deadline; the window was not running");
      armWatchdog();
      return;
    }

    state.watchdogAlarmed = true;
    report(`watchdog: no progress event in ${WATCHDOG_SECONDS}s`);
    showFailure(t("scan.watchdog", { n: WATCHDOG_SECONDS }));
  }, WATCHDOG_SECONDS * 1000);
}

function setScanning(on) {
  state.scanning = on;
  state.rate = null;
  if (on) { armWatchdog(); } else { clearTimeout(state.watchdog); }
  $("btn-scan").classList.toggle("hidden", on);
  $("btn-cancel").classList.toggle("hidden", !on);
  if (!on) $("dead-run").classList.add("hidden");
  $("device-select").disabled = on;
  $("btn-refresh").disabled = on;
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
$("btn-stop-early").addEventListener("click", () => invoke("cancel_scan").catch(() => {}));
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

/* Re-reads the approved area and compares it against what was written there.
 *
 * The one question an inspection cannot answer about itself. It writes nothing
 * — the backend opens the device for reading, so the guarantee is the operating
 * system's — which is why this asks for no typed name while every other button
 * on this screen does. */
async function recheckRetention() {
  try {
    await invoke("recheck_retention");
    setScanning(true);
    clearFailure();
    toast(t("recheck.started"), "");
  } catch (e) {
    showFailure(t(String(e)));
  }
}

listen("scan:progress", (e) => applySnapshot(e.payload));

/* What the card still held. The interval is the measurement: "nothing lost"
 * means nothing over that span, in that time, and says nothing about longer. */
listen("recheck:done", (e) => {
  const r = e.payload;
  setScanning(false);
  const when = r.age_seconds == null ? t("remembered.unknownAge") : humanAge(r.age_seconds);
  if (r.held) {
    clearFailure();
    toast(t("recheck.held", { size: humanBytes(r.examined_bytes), when }), "ok");
  } else {
    showFailure(t("recheck.lost", {
      lost: humanBytes(r.lost_bytes),
      size: humanBytes(r.examined_bytes),
      when,
    }));
  }
});

/* The comparison was against a pattern that is no longer on the card. Reporting
 * that as damage would condemn a card that may be perfectly well; from here the
 * two are indistinguishable, so neither is claimed. */
listen("recheck:reference-gone", () => {
  setScanning(false);
  showFailure(t("recheck.referenceGone"));
});

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
