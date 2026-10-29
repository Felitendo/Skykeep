/**
 * A segmented control's thumb slides from the segment that was chosen to the one chosen now,
 * as macOS 26 draws it — so the eye follows the choice instead of watching one cap vanish
 * and another appear.
 *
 * Every segmented control in the app (kit.css .segmented) is plain buttons with aria-pressed;
 * nothing in them changes for this. One observer, on that attribute alone, sees a choice
 * change: before the frame is painted it hides the new segment's own cap and draws the thumb
 * (the group's ::before) where the old one was, then moves it, frame by frame from the page
 * (lib/fade.ts says why never the compositor), to the new one: 160ms, decelerating, by
 * `translate` and its width only. The segments' words change weight at once; only the thumb
 * moves. A choice made mid-slide sets off from where the thumb is. Instant under Reduce
 * Motion, and when the control was not on screen.
 */
import { reduced, token } from "./fade";

type Box = { x: number; w: number };
const sliding = new WeakMap<HTMLElement, { frame: number; at: Box; timer: number }>();

function boxOf(group: HTMLElement, seg: HTMLElement): Box {
  const g = group.getBoundingClientRect();
  const r = seg.getBoundingClientRect();
  return { x: r.left - g.left - group.clientLeft, w: r.width };
}

function slide(group: HTMLElement, from: HTMLElement | null, to: HTMLElement) {
  const duration = token("--motion-thumb", 160);
  const running = sliding.get(group);
  const start = running?.at ?? (from ? boxOf(group, from) : null);
  if (running) {
    cancelAnimationFrame(running.frame);
    clearTimeout(running.timer);
  }
  const end = boxOf(group, to);
  if (!start || reduced() || duration <= 0 || !group.isConnected || end.w === 0 || (start.x === end.x && start.w === end.w)) {
    stop(group);
    return;
  }
  const r = to.getBoundingClientRect();
  const g = group.getBoundingClientRect();
  group.style.setProperty("--thumb-y", `${(r.top - g.top - group.clientTop).toFixed(2)}px`);
  group.style.setProperty("--thumb-h", `${r.height.toFixed(2)}px`);
  const place = (b: Box) => {
    group.style.setProperty("--thumb-x", `${b.x.toFixed(2)}px`);
    group.style.setProperty("--thumb-w", `${b.w.toFixed(2)}px`);
  };
  place(start);
  group.classList.add("is-sliding");
  const t0 = performance.now();
  const state = { frame: 0, at: start, timer: 0 };
  const step = (now: number) => {
    const t = Math.max(0, Math.min(1, (now - t0) / duration));
    if (t >= 1) return stop(group);
    const k = 1 - (1 - t) ** 3;
    state.at = { x: start.x + (end.x - start.x) * k, w: start.w + (end.w - start.w) * k };
    place(state.at);
    state.frame = requestAnimationFrame(step);
  };
  state.frame = requestAnimationFrame(step);
  // Settled on time even where no frames are drawn (a window out of sight draws none).
  state.timer = window.setTimeout(() => stop(group), duration + 250);
  sliding.set(group, state);
}

function stop(group: HTMLElement) {
  const s = sliding.get(group);
  if (s) {
    cancelAnimationFrame(s.frame);
    clearTimeout(s.timer);
  }
  sliding.delete(group);
  group.classList.remove("is-sliding");
  for (const p of ["--thumb-x", "--thumb-w", "--thumb-y", "--thumb-h"]) group.style.removeProperty(p);
}

export function slideThumbs() {
  new MutationObserver((records) => {
    const moves = new Map<HTMLElement, { from: HTMLElement | null; to: HTMLElement | null }>();
    for (const rec of records) {
      const seg = rec.target as HTMLElement;
      if (!seg.classList?.contains("segment")) continue;
      const group = seg.parentElement;
      if (!group?.classList.contains("segmented")) continue;
      const m = moves.get(group) ?? { from: null, to: null };
      const on = seg.getAttribute("aria-pressed") === "true";
      if (on && rec.oldValue !== "true") m.to = seg;
      if (!on && rec.oldValue === "true") m.from = seg;
      moves.set(group, m);
    }
    for (const [group, { from, to }] of moves) if (to && (from || sliding.has(group))) slide(group, from, to);
  }).observe(document.body, { subtree: true, attributes: true, attributeFilter: ["aria-pressed"], attributeOldValue: true });
}
