/* Checks that every filled-in sentence comes out with its blanks filled.
 *
 * `check_strings.py` proves a key exists in all four languages. It cannot prove
 * the key's placeholders match what the window passes: a template asking for
 * `{approved}` and a caller supplying `aproved` produce no error anywhere — the
 * sentence simply reaches the user with a brace in it, in one language, months
 * later.
 *
 * So this loads the shipped dictionary, renders each parameterised sentence with
 * the arguments its caller actually passes, and fails if a brace survives.
 *
 * Usage:
 *     node tools/check_placeholders.mjs
 */

import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../ui/i18n.js", import.meta.url), "utf8");

// Enough of a browser for the module to load: it reads the preferred languages
// to pick a default and hangs itself off `window` at the end.
const sandbox = {
  window: {},
  navigator: { languages: ["en"], language: "en" },
  document: { documentElement: {}, querySelectorAll: () => [] },
  Intl,
  console,
};
vm.createContext(sandbox);
vm.runInContext(source, sandbox);
const I18N = sandbox.window.I18N;

/* Every key the window renders with arguments, and the arguments it passes.
 *
 * Kept beside the call sites in spirit, not in code — a mismatch is exactly what
 * this file exists to catch, so reading the names from app.js would defeat it. */
const CALLS = [
  ["recheck.held", { size: "15.67 GB", when: "3 days ago" }],
  ["recheck.lost", { lost: "4.2 MB", size: "15.67 GB", when: "3 days ago" }],
  ["retention.window", { min: "under a minute", max: "2.7 h" }],
  ["duration.minutes", { n: 32 }],
  ["duration.hours", { n: "2.7" }],
  ["dead.lead", { from: "15.71 GB", size: "13.26 GB" }],
  ["dead.keeps", { approved: "15.67 GB" }],
  ["scan.watchdog", { n: 20 }],
  ["prepare.hasVolume", { letter: "F:" }],
  ["prepare.warning", { name: "Generic MassStorageClass" }],
  ["prepare.willWrite", { fs: "FAT32", label: "SALVAGE" }],
  ["prepare.where", { letter: "F" }],
  ["release.warning", { name: "Generic MassStorageClass" }],
  ["apply.erases", { name: "Generic MassStorageClass" }],
  ["apply.layoutIs", { name: "Single contiguous run" }],
  ["apply.dataParts", { n: "1", size: "15.69 GB" }],
  ["apply.hiddenParts", { n: "1" }],
  ["apply.where", { letter: "F" }],
  ["remembered.body", { when: "3 days ago", approved: "305.41 MB", defective: "197.91 MB" }],
  ["age.hours", { n: 4 }],
  ["age.days", { n: 3 }],
  ["age.months", { n: 2 }],
  ["plan.fencedToast", { size: "53.75 MB" }],
  ["progress.defects", { n: "4,608,201" }],
  ["refusal.inPieces", { n: 7 }],
  ["ui.evidenceCount", { n: 6 }],
  ["step.volume_mounted", { letter: "F" }],
  ["step.volume_warning", { detail: "in use" }],
];

let failures = 0;
for (const code of I18N.order) {
  I18N.set(code);
  for (const [key, params] of CALLS) {
    const filled = I18N.t(key, params);
    if (filled === key) {
      console.log(`FAIL  ${code}  ${key} has no entry`);
      failures++;
      continue;
    }
    const leftover = filled.match(/\{(\w+)\}/g);
    if (leftover) {
      console.log(`FAIL  ${code}  ${key} still asks for ${leftover.join(", ")}`);
      console.log(`        ${filled}`);
      failures++;
    }
  }
}

if (failures) {
  console.error(`${failures} sentence(s) reach the user with a blank in them`);
  process.exit(1);
}
console.log(
  `${CALLS.length} parameterised sentences fill correctly in all ${I18N.order.length} languages`,
);
