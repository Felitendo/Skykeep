/**
 * The window's own colour, kept the colour of what fills its edges. A window being resized is
 * drawn larger by the system a few frames before the page is drawn at its new size, and what
 * shows in between is the window's colour: left to the toolkit's grey, that was a light band
 * flashing beside the near-black photo viewer — the flicker on every resize.
 */
import { invoke } from "@tauri-apps/api/core";

let last = "";

/** Tint the window the background colour `el` is painted. */
export function tintFrom(el: Element | null) {
  if (!el) return;
  const m = /rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(getComputedStyle(el).backgroundColor);
  if (!m) return;
  const key = `${m[1]},${m[2]},${m[3]}`;
  if (key === last) return;
  last = key;
  invoke("window_tint", { rgb: [Number(m[1]), Number(m[2]), Number(m[3])] }).catch(() => {
    last = "";
  });
}

/** The app's own background, at start and whenever the appearance changes. */
export function keepTinted() {
  const now = () => tintFrom(document.body);
  now();
  window.matchMedia?.("(prefers-color-scheme: dark)").addEventListener?.("change", now);
  new MutationObserver(now).observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "class", "style"] });
}
