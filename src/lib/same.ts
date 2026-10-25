/**
 * A list read again, drawn only where it changed.
 *
 * Every read of the library answers with new objects, the same rows as before included, and a
 * row drawn from a new object draws again: a sync that brought one reminder had every row of
 * the list drawn again, and one that brought nothing had the whole screen drawn again for it.
 * Here each row equal to the one it had keeps the object it had, so the rows around a change
 * (memo'd by their row) stay as they are — and a list equal throughout is the list it was, so
 * nothing is drawn at all.
 */
export function keepSame<T>(was: readonly T[] | null | undefined, next: T[], keyOf: (row: T) => string): T[] {
  if (!was || was.length === 0) return next;
  const before = new Map<string, T>();
  for (const row of was) before.set(keyOf(row), row);
  let changed = was.length !== next.length;
  const out = next.map((row, i) => {
    const had = before.get(keyOf(row));
    if (had !== undefined && same(had, row)) {
      if (was[i] !== had) changed = true;
      return had;
    }
    changed = true;
    return row;
  });
  return changed ? out : (was as T[]);
}

/** Whether two values read from the library are the same throughout. */
export function same(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a)) {
    if (!Array.isArray(b) || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!same(a[i], b[i])) return false;
    return true;
  }
  if (Array.isArray(b)) return false;
  const x = a as Record<string, unknown>;
  const y = b as Record<string, unknown>;
  const keys = Object.keys(x);
  if (keys.length !== Object.keys(y).length) return false;
  for (const k of keys) if (!(k in y) || !same(x[k], y[k])) return false;
  return true;
}
