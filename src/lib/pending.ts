import { useCallback, useRef } from "react";

/**
 * Changes on their way to Apple, kept over every read of the library until they land.
 *
 * A change is drawn the moment it is made, then sent. While it is out, a refresh can finish
 * — Apple's push for that very change arrives within a second — and read the library from
 * before it was stored: a note moved to another folder came back in the old one for a
 * moment, then went again. Every read lays the changes still out over what it read, so what
 * is on screen only ever moves forward.
 */
export function usePending<T>() {
  const pending = useRef<((rows: T[]) => T[])[]>([]);
  /** Hold a change over reads; the returned function lets it go once it has landed. */
  const hold = useCallback((predict: (rows: T[]) => T[]) => {
    pending.current.push(predict);
    return () => {
      pending.current = pending.current.filter((p) => p !== predict);
    };
  }, []);
  /** What was read, with every change still on its way laid over it. */
  const over = useCallback((rows: T[]) => pending.current.reduce((acc, p) => p(acc), rows), []);
  return { hold, over };
}
