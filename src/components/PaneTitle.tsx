import { useLayoutEffect, useRef } from "react";

/**
 * A pane's title, with how much is in it on its own small line beneath — as Notes and Mail put
 * "1 note" under a folder's name. The title has its column's whole width: a long one wraps to
 * two lines, and if two lines at the full size are still not enough it steps down one size;
 * only past that is it clamped. A title is never cut off to make room for a count or a button.
 */
export function PaneTitle({
  children,
  count,
  countLabel,
  countClassName,
  className,
  before,
}: {
  children: React.ReactNode;
  /** The small line under it: "3 notes". */
  count?: React.ReactNode;
  countLabel?: string;
  /** A screen's own hook for the count line. */
  countClassName?: string;
  className?: string;
  /** Above the title: a path back, for a pane opened inside another. */
  before?: React.ReactNode;
}) {
  const title = useRef<HTMLHeadingElement>(null);
  useLayoutEffect(() => {
    const el = title.current;
    if (!el) return undefined;
    // Measured, not guessed from its length: the column's width is what decides it. Only a
    // width decides it, and only a cut-off line counts: a "y" or a tall Windows font pokes a
    // few pixels past one line of "Today", which read as overflow and shrank it a frame after
    // it drew — and the head's own height changing then measured it again, flipping it back
    // and forth, so every app's content shook up and down. A hidden screen, measured at no
    // width, waits until it has one.
    const column = el.parentElement ?? el;
    let width = -1;
    const fit = () => {
      const now = column.clientWidth;
      if (now === 0 || now === width) return;
      width = now;
      el.classList.remove("is-small");
      const line = parseFloat(getComputedStyle(el).lineHeight) || 24;
      if (el.scrollHeight - el.clientHeight > line / 2) el.classList.add("is-small");
    };
    fit();
    const watch = new ResizeObserver(fit);
    watch.observe(column);
    return () => watch.disconnect();
  }, [children]);
  return (
    <div className={`pane-title${className ? ` ${className}` : ""}`}>
      {before}
      <h1 ref={title}>{children}</h1>
      {count !== undefined && count !== null && count !== false && (
        <p className={`pane-count${countClassName ? ` ${countClassName}` : ""}`} aria-label={countLabel}>
          {count}
        </p>
      )}
    </div>
  );
}
