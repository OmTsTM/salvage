"""Renders the window in a browser, for states a real card cannot produce.

The layout chooser only appears when a card has defects *and* enough clean area
between them to build something. A card that is nearly all bad refuses every
layout; one already fenced comes back with no defect to fence. Neither produces
the frame the README needs, and waiting for a card in between is not a plan.

So this loads `ui/` into a page, stubs the Tauri bridge, and hands `buildPlans()`
the response shape the backend returns. Every pixel is then drawn by the shipped
interface — same stylesheet, same render functions, same wording tables. Only the
measurement is synthetic.

That is the line, and it matters: rendering the real interface with illustrative
numbers shows how the program behaves. A generated picture of an interface would
show how it does not. Never use this for a state a device can actually reach —
capture those.

Usage:
    python tools/harness/build.py          # writes harness.html to TEMP
    # then open it, or screenshot it headless:
    #   msedge --headless=new --window-size=1256,1560 --screenshot=out.png file:///...
"""

import os, re
html = open('ui/index.html', encoding='utf-8').read()
css  = open('ui/style.css', encoding='utf-8').read()
i18n = open('ui/i18n.js', encoding='utf-8').read()
app  = open('ui/app.js', encoding='utf-8').read()

# Enough of a Tauri surface for app.js to initialise without throwing.
stub = """
window.__TAURI__ = {
  core: { invoke: (cmd) => {
    if (cmd === "list_devices") return Promise.resolve([]);
    if (cmd === "app_version") return Promise.resolve("0.5.8");
    if (cmd === "diagnostics_path") return Promise.resolve("");
    if (cmd === "build_plans") return Promise.resolve(window.__PLANS__);
    return Promise.resolve(null);
  } },
  event: { listen: () => Promise.resolve(() => {}) },
};
window.__ERRORS__ = [];
window.addEventListener("error", (e) => window.__ERRORS__.push(e.message));
window.addEventListener("unhandledrejection", (e) => window.__ERRORS__.push(String(e.reason)));
"""

drive = """
setTimeout(() => {
  try {
    I18N.set("en"); I18N.apply();
    const COUNTS = { untested: 0, good: 41943040, bad_read: 2048, bad_write: 0,
                     corrupt: 20480, aliased: 0, fenced: 196608 };
    const GB = 1073741824;
    const part = (label, role, type, bytes, off, len) => ({
      label, role, mbr_type: type, size_bytes: bytes,
      start_lba: 0, sectors: bytes / 512,
      offset_percent: off, length_percent: len });
    window.__PLANS__ = {
      buckets: [],
      approved_marks: [],
      counts: COUNTS,
      fenced_bytes: 0,
      refusal: null,
      plans: [
        { index: 0, strategy: "largest_contiguous",
          usable_bytes: 12.58 * GB, sacrificed_bytes: 1.07 * GB,
          partitions: [
            part("data", "data", "0x07", 12.58 * GB, 2, 39),
            part("quarantine-1", "quarantine", "0xda", 8.59 * GB, 41, 27),
            part("quarantine-2", "quarantine", "0xda", 9.66 * GB, 68, 30)] },
        { index: 1, strategy: "maximum_space",
          usable_bytes: 18.24 * GB, sacrificed_bytes: 0.41 * GB,
          partitions: [
            part("data-1", "data", "0x07", 12.58 * GB, 2, 39),
            part("data-2", "data", "0x07", 5.66 * GB, 44, 18),
            part("quarantine-1", "quarantine", "0xda", 12.9 * GB, 62, 36)] },
        { index: 2, strategy: "conservative",
          usable_bytes: 9.13 * GB, sacrificed_bytes: 4.52 * GB,
          partitions: [
            part("data", "data", "0x07", 9.13 * GB, 4, 28),
            part("quarantine-1", "quarantine", "0xda", 21.4 * GB, 33, 65)] },
        { index: 3, strategy: "spliced_fat32",
          usable_bytes: 20.91 * GB, sacrificed_bytes: 0.06 * GB,
          partitions: [
            part("spliced", "data", "0x0c", 31.46 * GB, 1, 98)] },
      ],
    };
    const ss = 512;
    state.lastSnapshot = { sector_size: ss, phase: "idle", approved_marks: [], counts: COUNTS };
    renderReport({
      scenario_kind: "exhausted_spare", assurance: "moderate", isolation_worthwhile: true,
      largest_usable_label: "12.58 GB",
      details: ["Distinct defective regions: 7",
                "Second pass: the same defects, in the same places"],
    });
    buildPlans().then(() => { document.title = "OK"; });
  } catch (e) {
    document.title = "ERRO: " + (e && e.message);
    window.__ERRORS__.push(String(e && e.stack));
  }
  const box = document.createElement("pre");
  box.id = "harness-errors";
  box.textContent = (window.__ERRORS__ || []).join(" | ");
  box.style.cssText = "position:fixed;left:0;bottom:0;z-index:9999;color:#f43f5e;font-size:11px;max-width:900px";
  document.body.appendChild(box);
}, 700);
"""

html = html.replace('<link rel="stylesheet" href="style.css" />', '<style>\n' + css + '\n</style>')
html = re.sub(r'<script src="[^"]*\.js"></script>', '', html)
html = html.replace('</body>',
                    '<script>' + stub + '</script>\n<script>' + i18n + '</script>\n'
                    '<script>' + app + '</script>\n<script>' + drive + '</script>\n</body>')
out = os.path.join(os.environ.get('TEMP', '/tmp'), 'harness.html')
open(out, 'w', encoding='utf-8').write(html)
print(out)
