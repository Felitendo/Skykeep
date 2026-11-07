/**
 * Whether an element is within some distance of its scroll area's view, told to a callback as
 * it changes: one IntersectionObserver per scroll area and distance, shared by every element
 * watched there, rather than one per element — hundreds of months, each with an observer of
 * its own, each its own work in every frame.
 *
 * `margin` is how far beyond the view counts as near, as a CSS margin (`"100% 0px"`: a whole
 * view above and below).
 */
type Seen = (near: boolean, entry: IntersectionObserverEntry) => void;

const watchers = new WeakMap<Element | Document, Map<string, { io: IntersectionObserver; seen: Map<Element, Seen> }>>();

export function watchNear(el: Element, root: Element | null, margin: string, seen: Seen): () => void {
  if (typeof IntersectionObserver === "undefined") {
    seen(true, {} as IntersectionObserverEntry);
    return () => {};
  }
  const key = root ?? document;
  let byMargin = watchers.get(key);
  if (!byMargin) watchers.set(key, (byMargin = new Map()));
  let w = byMargin.get(margin);
  if (!w) {
    const told = new Map<Element, Seen>();
    const io = new IntersectionObserver(
      (entries) => {
        // The latest for each element: a layout that moved it out and back within a frame
        // comes as two entries, and the earlier one alone is wrong.
        const last = new Map<Element, IntersectionObserverEntry>();
        for (const e of entries) last.set(e.target, e);
        for (const [target, e] of last) told.get(target)?.(e.isIntersecting, e);
      },
      { root, rootMargin: margin },
    );
    w = { io, seen: told };
    byMargin.set(margin, w);
  }
  const mine = w;
  mine.seen.set(el, seen);
  mine.io.observe(el);
  return () => {
    mine.io.unobserve(el);
    mine.seen.delete(el);
  };
}

/** Whether `el` is out of sight altogether: in a screen put away (`hidden`, its slot not drawn). */
export function putAway(el: Element): boolean {
  return (el as HTMLElement).offsetParent === null && getComputedStyle(el).position !== "fixed";
}
