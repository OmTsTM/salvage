/* The splash's one line of script.
 *
 * A separate file because the content security policy forbids inline script,
 * and `i18n.js` needs nothing from Tauri — it reads localStorage, navigator
 * and Intl, all of which a window with no permissions already has. So the
 * splash shares the very dictionary the program uses, rather than carrying a
 * second copy of three sentences that would drift from it.
 *
 * On a first run there is nothing stored yet and the language comes from the
 * system, which is the right guess to make before anyone has had a chance to
 * choose. */
window.I18N.apply();

/* The running version, beside the wordmark, exactly as the program shows it.
 *
 * Commands declared with `generate_handler!` are not gated by the capability
 * file the way plugin commands are, so this needs no permission of its own.
 * Every failure path is silent on purpose: the splash exists to say the program
 * is starting, and it must never be the thing that stops it from starting. */
(async () => {
  try {
    const version = await window.__TAURI__.core.invoke("app_version");
    if (!version) return;
    const badge = document.getElementById("app-version");
    badge.textContent = `v${version}`;
    badge.hidden = false;
  } catch (_) {
    /* No badge, no consequence. */
  }
})();
