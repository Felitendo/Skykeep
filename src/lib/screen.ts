/**
 * Whether the screen a component belongs to is the one showing. Screens are kept once opened
 * (coming back to one is instant, and finds it as it was left — its place, its selection), so
 * a screen out of sight is still mounted: its menus and sheets must not be drawn, and its keys
 * must not answer, while another screen is in front.
 */
import { createContext, useContext, useEffect, useRef } from "react";

export const ScreenActive = createContext(true);

export const useScreenActive = () => useContext(ScreenActive);

/** The same, as a ref, for listeners set up once and read when a key comes. */
export function useActiveRef() {
  const active = useScreenActive();
  const ref = useRef(active);
  ref.current = active;
  useEffect(() => {
    ref.current = active;
  }, [active]);
  return ref;
}

/** Whether a modal sheet is up (a confirmation, Keyboard Shortcuts, an import): the keys that
 * go anywhere — Ctrl+, and Ctrl+/ — wait until it is answered, rather than taking the focus out
 * of it or changing the screen under it. */
export function modalOpen(): boolean {
  for (const el of document.querySelectorAll('[aria-modal="true"]')) {
    if (!el.closest("[inert], [hidden]")) return true;
  }
  return false;
}

const OVERLAY = '[role="menu"], [role="listbox"], [role="dialog"], [role="alertdialog"], .confirm';

/**
 * Every element that may be an overlay, kept as the page changes rather than looked for each
 * time. Looked for on every key, a search of the whole page cost 4–5 ms an arrow press with
 * an address book or a folder of 3,000 rows on screen — most of each step. Here the page is
 * searched once; after that only what is added, or given a role or class, is looked at, and
 * what has gone is dropped as it is met.
 */
let candidates: { found: Set<Element>; watch: MutationObserver } | null = null;
/** What was added, or given a role or class, that may be an overlay, put with the rest. */
function takeChanges(found: Set<Element>, records: MutationRecord[]) {
  for (const r of records) {
    if (r.type === "attributes") {
      if ((r.target as Element).matches(OVERLAY)) found.add(r.target as Element);
      continue;
    }
    r.addedNodes.forEach((node) => {
      if (node.nodeType !== Node.ELEMENT_NODE) return;
      const el = node as Element;
      if (el.matches(OVERLAY)) found.add(el);
      if (el.firstElementChild) el.querySelectorAll(OVERLAY).forEach((x) => found.add(x));
    });
  }
}
function overlayCandidates(): Set<Element> {
  if (!candidates) {
    const found = new Set<Element>(document.querySelectorAll(OVERLAY));
    const watch = new MutationObserver((records) => takeChanges(found, records));
    watch.observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ["role", "class"] });
    candidates = { found, watch };
  }
  // What changed since the observer last ran, taken now: an overlay drawn a moment ago, in the
  // same task, counts as surely as one drawn long before.
  takeChanges(candidates.found, candidates.watch.takeRecords());
  return candidates.found;
}

/**
 * Whether a menu, a picker's list or a sheet is open in front, which answers its own keys. A
 * screen's own list of rows (a list box marked `data-rows`, as Notes and Contacts draw theirs)
 * is not one, and nothing in a screen kept out of sight counts.
 */
export function overlayOpen(): boolean {
  const found = overlayCandidates();
  for (const el of found) {
    // Gone from the page, or no longer one (its role or class changed): let go of it.
    if (!el.isConnected || !el.matches(OVERLAY)) {
      found.delete(el);
      continue;
    }
    if (el.matches("[data-rows]") || el.closest("[inert], [hidden]")) continue;
    return true;
  }
  return false;
}
