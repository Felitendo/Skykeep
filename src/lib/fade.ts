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

/* ---- arrival: content that came in from another device ------------------------------------
 *
 * It settles in without being pointed at — no highlight, no wash, as the best apps do it —
 * drawn by the page frame by frame (as above), on opacity and a 2px transform only, so nothing
 * is laid out and no row beside it moves; only ever on a small thing, a row or a tile, never a
 * pane or a list. Under Reduce Motion each is instant. Timings are tokens (tokens.css "Arrival").
 *
 *   arrive(row)          a new row: fades in over 120ms, rising 2px
 *   leave(row, remove)   a row deleted elsewhere: out by opacity over 140ms, lifting 2px, then
 *                        `remove` takes it away (the rows below close up at once)
 *   <ReceivingLine>      the calm line while many things arrive at once (components/ReceivingLine)
 */

const reduced = () =>
  typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** A timing token in ms (or px, for a distance), as tokens.css sets it — 0 under Reduce
    Motion — or `fallback`. */
function token(name: string, fallback: number): number {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const n = parseFloat(v);
  if (!Number.isFinite(n)) return fallback;
  return v.endsWith("ms") ? n : v.endsWith("s") ? n * 1000 : n;
}

/** A new row arriving: a quick fade in, rising 2px. No highlight — it simply settles in. */
export function arrive(el: HTMLElement | null) {
  if (!el) return;
  const duration = token("--arrive-in-d", 120);
  if (reduced() || duration <= 0) return;
  const rise = token("--arrive-rise", 2);
  cancelAnimationFrame(running.get(el) ?? 0);
  const start = performance.now();
  el.style.opacity = "0";
  el.style.transform = `translateY(${rise}px)`;
  const finish = () => {
    el.style.opacity = "";
    el.style.transform = "";
    running.delete(el);
  };
  const step = (now: number) => {
    const t = Math.max(0, Math.min(1, (now - start) / duration));
    if (t >= 1) return finish();
    const k = 1 - (1 - t) * (1 - t);
    el.style.opacity = String(k);
    el.style.transform = `translateY(${(rise * (1 - k)).toFixed(2)}px)`;
    running.set(el, requestAnimationFrame(step));
  };
  running.set(el, requestAnimationFrame(step));
  window.setTimeout(() => { if (running.has(el)) { cancelAnimationFrame(running.get(el)!); finish(); } }, duration + 250);
}

/**
 * A row deleted on another device leaving: out by opacity, lifting 2px, then `done` — which
 * removes it. Its height is never animated (a layout animation, and the flicker this file
 * exists to avoid); the rows below close up the moment it is gone.
 */
export function leave(el: HTMLElement | null, done: () => void) {
  const duration = el ? token("--leave-d", 140) : 0;
  if (!el || reduced() || duration <= 0) {
    done();
    return;
  }
  cancelAnimationFrame(running.get(el) ?? 0);
  el.style.pointerEvents = "none";
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
    const k = t * t;
    el.style.opacity = String(1 - k);
    el.style.transform = `translateY(${(-2 * k).toFixed(2)}px)`;
    if (t >= 1) return finish();
    running.set(el, requestAnimationFrame(step));
  };
  running.set(el, requestAnimationFrame(step));
}
