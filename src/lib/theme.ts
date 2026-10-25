/**
 * The window's appearance: the system's, or one chosen in Settings. Remembered on this
 * computer; applied before the first paint so the window never opens in the wrong one.
 */
export type Theme = "auto" | "light" | "dark";
const KEY = "quince.theme";

export function storedTheme(): Theme {
  try {
    const t = window.localStorage.getItem(KEY);
    return t === "light" || t === "dark" ? t : "auto";
  } catch {
    return "auto";
  }
}

export function applyTheme(theme: Theme) {
  if (theme === "auto") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme;
}

export function setTheme(theme: Theme) {
  try {
    window.localStorage.setItem(KEY, theme);
  } catch {
    // Applies for now; the system's appearance on the next launch.
  }
  applyTheme(theme);
}

/**
 * The appearance chosen in another window: Settings lives in the library window, and each
 * app's own window (and one put away, hidden, to be shown again) kept the old appearance until
 * it was opened afresh. They share this computer's storage, which says when it changes.
 */
export function followTheme() {
  window.addEventListener("storage", (e) => {
    if (e.key === KEY || e.key === null) applyTheme(storedTheme());
  });
}

/**
 * The desktop's Increase Contrast, where the engine does not say it itself: on Linux the app
 * reads the Settings portal (appearance.rs) and the root is marked `data-contrast="more"`,
 * which a11y.css treats as `prefers-contrast: more`. Followed as it changes.
 */
export function followContrast() {
  const mark = (more: unknown) => {
    if (more === true) document.documentElement.dataset.contrast = "more";
    else delete document.documentElement.dataset.contrast;
  };
  void import("@tauri-apps/api/core")
    .then(({ invoke }) => invoke<boolean>("system_contrast"))
    .then(mark, () => {});
  void import("@tauri-apps/api/event")
    .then(({ listen }) => listen<boolean>("quince://contrast", (e) => mark(e.payload)))
    .catch(() => {});
}
