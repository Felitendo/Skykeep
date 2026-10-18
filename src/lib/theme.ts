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
