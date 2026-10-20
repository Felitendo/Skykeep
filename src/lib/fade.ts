/**
 * A fade drawn by the page itself, one frame at a time.
 *
 * Every animation WebKitGTK handed to its compositor (CSS transitions and animations, the Web
 * Animations API) glitched on his screen under Hyprland: frames where the element was drawn as
 * if the animation had not started or had already ended. This one is never handed over — each
 * frame the page sets the element's opacity itself and the engine simply paints it — so there
 * is no hand-over to go wrong. Kept short, and only ever on opacity.
 */
const running = new WeakMap<HTMLElement, number>();

export function fadeIn(el: HTMLElement | null, { from = 0, duration = 150 }: { from?: number; duration?: number } = {}) {
  if (!el) return;
  if (typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    el.style.opacity = "";
    return;
  }
  cancelAnimationFrame(running.get(el) ?? 0);
  const start = performance.now();
  el.style.opacity = String(from);
  const step = (now: number) => {
    const t = Math.min(1, (now - start) / duration);
    // Ease out: most of the change early, settling gently.
    const k = 1 - (1 - t) * (1 - t);
    if (t >= 1) {
      el.style.opacity = "";
      running.delete(el);
      return;
    }
    el.style.opacity = String(from + (1 - from) * k);
    running.set(el, requestAnimationFrame(step));
  };
  running.set(el, requestAnimationFrame(step));
}

/**
 * Every sheet, as it is added anywhere in the app, fades in this way — set before the first frame
 * is painted (a mutation observer runs before painting), so none shows whole for a frame first.
 * The full-window dims behind them simply appear: a large surface is never animated.
 */
export function fadeSheetsIn() {
  const sheets = ".modal-sheet, .px-sheet";
  new MutationObserver((changes) => {
    for (const change of changes) {
      for (const node of change.addedNodes) {
        if (!(node instanceof HTMLElement)) continue;
        if (node.matches(sheets)) fadeIn(node, { duration: 140 });
        node.querySelectorAll<HTMLElement>(sheets).forEach((el) => fadeIn(el, { duration: 140 }));
      }
    }
  }).observe(document.body, { childList: true, subtree: true });
}
