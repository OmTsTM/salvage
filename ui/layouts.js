/* Salvage — zone three, where the card is written to.
 *
 * Loaded as a plain script, in the order index.html lists: this file may use
 * anything the ones before it defined, and defines what the ones after need.
 */
/*
 * The fencing panel and everything reachable from it: the layout chooser, the
 * refusal that replaces it when no layout is possible, the offer to format a
 * card nothing was condemned on, and the three operations that write a
 * partition table.
 *
 * Every one of those ends in the backend, which refuses on its own terms. What
 * is decided here is only what to show and what to ask.
 */

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

  // Whether the card carries a volume right now, from the source that is
  // trustworthy in each case. After an inspection run in this session the
  // enumerated list is stale by construction — the scan wrote its pattern over
  // the filesystem it lists — so a fresh map settles it: the card is erased.
  // Otherwise the enumeration is current and it is the only thing that knows.
  const letters = fresh ? [] : (state.selected?.volumes ?? []).filter((v) => v.includes(":"));
  const carriesVolume = letters.length > 0;

  const box = $("plan-prepare");
  box.innerHTML =
    `<strong>${escapeHtml(t("prepare.title"))}</strong>` +
    `<p>${escapeHtml(t(carriesVolume ? "prepare.bodyKept" : "prepare.body"))}</p>` +
    `<p class="prepare-erased">${escapeHtml(
      fresh
        ? t("prepare.erased")
        : carriesVolume
          ? t("prepare.hasVolume", { letter: letters.join(", ") })
          : t("prepare.needsFresh"),
    )}</p>`;
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
    showFailure(t(String(e)));
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
    // Persisted rather than toasted: see showFailure. This is the path that
    // refuses a layout built from a record adopted in an earlier session, and
    // the remedy is to run the inspection — which clears it.
    showFailure(t(String(e)));
  }
}

