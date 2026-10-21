/**
 * Something under way, drawn one way everywhere. A spinner for a wait with no measure — a
 * folder being read, a first sync; a bar for one that has one — an upload, a download.
 */
export function Spinner({ small, label }: { small?: boolean; label?: string }) {
  return (
    <span
      className={`spinner${small ? " is-small" : ""}`}
      role={label ? "progressbar" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    />
  );
}

/**
 * How far along, 0 to 1; `null` while there is no measure yet (still counting what is to go),
 * which draws a segment gliding instead of a number that would only shrink.
 */
export function Progress({ value, label, instant, className }: { value: number | null; label?: string; instant?: boolean; className?: string }) {
  const known = value !== null && Number.isFinite(value);
  const share = known ? Math.max(0, Math.min(1, value)) : 0;
  return (
    <span
      className={`progress${known ? "" : " is-waiting"}${instant ? " is-instant" : ""}${className ? ` ${className}` : ""}`}
      role="progressbar"
      aria-label={label}
      aria-valuemin={known ? 0 : undefined}
      aria-valuemax={known ? 100 : undefined}
      aria-valuenow={known ? Math.round(share * 100) : undefined}
    >
      {/* The fill grows by scale, on the compositor: a width animated here laid out its whole
          panel on every step, and coming back to the window replayed every step at once. */}
      <span style={known ? { transform: `scaleX(${share})` } : undefined} />
    </span>
  );
}
