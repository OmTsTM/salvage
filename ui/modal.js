/* Salvage — the confirmation dialog.
 *
 * Loaded as a plain script, in the order index.html lists: this file may use
 * anything the ones before it defined, and defines what the ones after need.
 */
/*
 * One dialog, reused for every irreversible operation. It owns the typed-name
 * field that gates them: the button stays disabled until the device name is
 * typed in full, and it is the interface's last stop before a partition table
 * is written.
 */

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
/* Puts a deadline on the claim the verdict makes.
 *
 * "Every sector was written and read back identical" is true and, without an
 * interval attached, misleading: it reads as durability. What an inspection
 * measures is that the cell *took* the data and gave it back — and on the card
 * that prompted this, the sector at address zero was read back a quarter of a
 * second after it was written.
 *
 * A worn cell answers that question correctly and loses the data overnight,
 * which is how a card passes an inspection and stutters the next day. So the
 * interval is stated, and its limit with it. */
function renderRetention(window_) {
  const el = $("retention-statement");
  if (!window_) {
    // A map adopted from a stored record: the interval belongs to a session
    // this one knows nothing about, and quoting it would be inventing it.
    el.textContent = "";
    el.classList.add("hidden");
    return;
  }
  el.textContent = t("retention.window", {
    min: humanDuration(window_.shortest_secs),
    max: humanDuration(window_.longest_secs),
  });
  el.classList.remove("hidden");
}

/* A duration in the coarsest unit that still says something true.
 *
 * The fraction is formatted for the language, not for whoever wrote this:
 * "2.6 h" and "2,6 h" are the same measurement, and only one of them is right
 * in front of a given reader. */
function humanDuration(seconds) {
  if (seconds < 60) return t("duration.underMinute");
  if (seconds < 3600) return t("duration.minutes", { n: Math.floor(seconds / 60) });
  return t("duration.hours", {
    n: (seconds / 3600).toLocaleString(window.I18N.locale(), {
      minimumFractionDigits: 1,
      maximumFractionDigits: 1,
    }),
  });
}

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
    showFailure(t(String(e)));
  }
}

