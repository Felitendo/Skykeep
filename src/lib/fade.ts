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
    // A frame's time can be from just before the fade began (it was started mid-frame): never
    // below where it starts.
    const t = Math.max(0, Math.min(1, (now - start) / duration));
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

/** The same fade the other way, then `done` — for something small leaving, as a toast. */
export function fadeOut(el: HTMLElement | null, done: () => void, duration = 120) {
  if (!el || (typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches)) {
    done();
    return;
  }
  cancelAnimationFrame(running.get(el) ?? 0);
  // Gone on time even where frames are not drawn (a window out of sight draws none).
  let finished = false;
  const finish = () => {
    if (finished) return;
    finished = true;
    running.delete(el);
    done();
  };
  window.setTimeout(finish, duration + 250);
  const start = performance.now();
  const step = (now: number) => {
    const t = Math.min(1, (now - start) / duration);
    // Ease in: gathering pace as it goes, so the end is not watched.
    el.style.opacity = String(1 - t * t);
    if (t >= 1) {
      finish();
      return;
    }
    running.set(el, requestAnimationFrame(step));
  };
  running.set(el, requestAnimationFrame(step));
}

/**
 * Every sheet, as it is added, fades in this way — set before the first frame is painted (a
 * mutation observer runs before painting), so none shows whole for a frame first. The
 * full-window dims behind them simply appear: a large surface is never animated.
 *
 * Every sheet is drawn at the top of the page (components/Portal), so only what is added to
 * the page's body is looked at. Watching the whole page looked through every row, tile and line
 * of text added anywhere — a photo grid scrolling, a note typed in — for a sheet. A sheet that
 * takes another's place on a dim already there fades itself in (`fadeIn`, ImportSheet).
 */
export function fadeSheetsIn() {
  const sheets = ".modal-sheet";
  new MutationObserver((changes) => {
    for (const change of changes) {
      for (const node of change.addedNodes) {
        if (!(node instanceof HTMLElement)) continue;
        if (node.matches(sheets)) fadeIn(node, { duration: 140 });
        node.querySelectorAll<HTMLElement>(sheets).forEach((el) => fadeIn(el, { duration: 140 }));
      }
    }
  }).observe(document.body, { childList: true });
}

/**
 * A row folding away, drawn by the page the same way: each frame its height, padding and
 * opacity set from here, so the rows below glide up. Then `done`.
 */
export function foldAway(el: HTMLElement, done: () => void, duration = 240) {
  if (typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    done();
    return;
  }
  const cs = getComputedStyle(el);
  const h = el.getBoundingClientRect().height;
  const [pt, pb] = [parseFloat(cs.paddingTop), parseFloat(cs.paddingBottom)];
  el.style.overflow = "hidden";
  const start = performance.now();
  const step = (now: number) => {
    const t = Math.min(1, (now - start) / duration);
    // Quick at first, settling: Apple's own curve for things leaving.
    const k = 1 - (1 - t) ** 3;
    el.style.height = `${h * (1 - k)}px`;
    el.style.paddingTop = `${pt * (1 - k)}px`;
    el.style.paddingBottom = `${pb * (1 - k)}px`;
    el.style.opacity = String(1 - k);
    if (t < 1) requestAnimationFrame(step);
    else done();
  };
  requestAnimationFrame(step);
}
