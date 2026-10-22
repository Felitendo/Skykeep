/**
 * A modal sheet keeps the keyboard inside it while it is open — Tab and Shift-Tab wrap at its
 * ends rather than wander into the dimmed window behind — and when it closes the focus goes
 * back to what had it before, as a Mac sheet does. Without the second half, closing a sheet
 * left the focus on the page itself and a keyboard user started again from the top.
 */
import { useEffect, useState, type RefObject } from "react";

const TABBABLE = [
  "a[href]",
  "button:not([disabled])",
  'input:not([disabled]):not([type="hidden"])',
  "select:not([disabled])",
  "textarea:not([disabled])",
  '[tabindex]:not([tabindex="-1"])',
  '[contenteditable="true"]',
].join(",");

// What had the focus before the latest move: a sheet's own autofocus has already taken it by
// the time the sheet's effect runs, so the one before is remembered here.
let current: Element | null = null;
let previous: Element | null = null;
// And the few before it, newest last: a sheet opened from a menu finds the menu gone when it
// closes, and goes back to the ⋯ that opened the menu instead.
const recent: Element[] = [];
if (typeof document !== "undefined") {
  document.addEventListener(
    "focusin",
    (e) => {
      if (e.target === current) return;
      previous = current;
      current = e.target as Element;
      recent.push(current);
      if (recent.length > 8) recent.shift();
    },
    true,
  );
}

const shown = (el: HTMLElement) => el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden";

export function useModalFocus(ref: RefObject<HTMLElement | null>, active = true) {
  useEffect(() => {
    const sheet = ref.current;
    if (!active || !sheet) return undefined;
    const now = document.activeElement;
    const before = (sheet.contains(now) ? previous : now) as HTMLElement | null;
    const earlier = recent.filter((el) => !sheet.contains(el)).reverse() as HTMLElement[];
    // Nothing inside asked for the focus: the sheet takes it, so the keys and a screen reader
    // start there.
    if (!sheet.contains(document.activeElement)) {
      if (!sheet.hasAttribute("tabindex")) sheet.tabIndex = -1;
      sheet.focus({ preventScroll: true });
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Tab" || e.defaultPrevented) return;
      // A sheet opened over this one keeps its own keys.
      const open = document.querySelectorAll('[aria-modal="true"]');
      const top = open[open.length - 1];
      if (top && top !== sheet && !sheet.contains(top)) return;
      const items = [...sheet.querySelectorAll<HTMLElement>(TABBABLE)].filter(shown);
      if (!items.length) {
        e.preventDefault();
        return;
      }
      const at = document.activeElement;
      const first = items[0];
      const last = items[items.length - 1];
      if (!sheet.contains(at)) {
        e.preventDefault();
        (e.shiftKey ? last : first).focus();
      } else if (e.shiftKey && at === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && at === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      // Back where it was — unless an answer sent it somewhere on purpose.
      const after = document.activeElement;
      if (after && after !== document.body && !sheet.contains(after) && after.isConnected) return;
      const back = [before, ...earlier].find((el) => el && el !== document.body && el.isConnected && !sheet.contains(el));
      back?.focus({ preventScroll: true });
    };
  }, [ref, active]);
}

/**
 * A layer that is not modal — a menu, a popover — gives the focus back when it goes, as a Mac
 * menu or popover does: to what had it when the layer was drawn (its ⋯, the event it is open
 * for). Only when the focus would otherwise be lost — on the page, or in the layer that went;
 * a choice that sent it somewhere on purpose (a sheet, a field) keeps it there. `fallback`
 * finds the opener again when it was drawn anew while the layer was open.
 */
// Whether the latest press was a key: a layer put away by the pointer leaves the focus where
// the pointer put it, and moves nothing under it.
let byKeys = false;
if (typeof document !== "undefined") {
  document.addEventListener("keydown", () => (byKeys = true), true);
  document.addEventListener("pointerdown", () => (byKeys = false), true);
}

export function useReturnFocus(fallback?: () => HTMLElement | null) {
  // Read while drawing, before the layer's own effects move the focus into it.
  const [opener] = useState(() => (typeof document === "undefined" ? null : (document.activeElement as HTMLElement | null)));
  useEffect(
    () => () => {
      const now = document.activeElement;
      const lost = !now || now === document.body || !now.isConnected;
      if (!lost || !byKeys) return;
      const back = opener && opener !== document.body && opener.isConnected ? opener : fallback?.() ?? null;
      back?.focus({ preventScroll: true });
    },
    [opener], // eslint-disable-line react-hooks/exhaustive-deps
  );
}
