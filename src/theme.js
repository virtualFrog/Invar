// Appearance: system, light or dark.
//
// A classic script loaded from <head>, not a module, so the stored choice is
// applied before the first paint. A module would run after it, and a light
// theme user would see a dark frame on every launch.
//
// The choice is a per-machine display preference, so it lives in localStorage
// rather than config.json, which the headless exporter shares.

(function () {
  const KEY = "invar.theme";
  const CHOICES = ["system", "light", "dark"];
  const root = document.documentElement;

  function stored() {
    try {
      const value = localStorage.getItem(KEY);
      return CHOICES.includes(value) ? value : "system";
    } catch {
      return "system";
    }
  }

  function apply(choice) {
    // "system" removes the attribute and leaves prefers-color-scheme in charge.
    if (choice === "system") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", choice);

    // Match the native title bar. Best effort: the webview is already right,
    // and a missing permission must not break the page.
    const win = window.__TAURI__?.window?.getCurrentWindow?.();
    win?.setTheme(choice === "system" ? null : choice).catch(() => {});
  }

  apply(stored());

  document.addEventListener("DOMContentLoaded", () => {
    const buttons = document.querySelectorAll("#theme-switch button");
    const mark = (choice) =>
      buttons.forEach((b) => b.setAttribute("aria-pressed", String(b.value === choice)));
    mark(stored());

    buttons.forEach((button) =>
      button.addEventListener("click", () => {
        const choice = button.value;
        try {
          localStorage.setItem(KEY, choice);
        } catch {
          // Unavailable storage only costs persistence; still switch now.
        }
        apply(choice);
        mark(choice);
      }),
    );
  });
})();
