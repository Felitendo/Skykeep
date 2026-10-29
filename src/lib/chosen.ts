/**
 * Which rows of a long list are selected, told to each row on its own.
 *
 * Handed down as a prop, the selection changed the props of the list itself at every arrow
 * press, so the list was drawn again — every one of thousands of rows made anew and compared,
 * to find the two that changed. Here the list is handed one object that never changes, each
 * row asks it whether it is selected, and a change wakes only the rows it moved between.
 */
import { useCallback, useLayoutEffect, useState, useSyncExternalStore } from "react";

export class Chosen {
  private keys = new Set<string>();
  /** The keys rows were last told: a change is told against these. */
  private told = new Set<string>();
  private listeners = new Map<string, Set<() => void>>();

  has(key: string): boolean {
    return this.keys.has(key);
  }

  /** The selection as of the render in progress, read by rows drawn in it. */
  hold(next: Iterable<string>) {
    this.keys = new Set(next);
  }

  /** Wake the rows whose part in the selection changed since they were last told. */
  tell() {
    const changed: string[] = [];
    for (const k of this.keys) if (!this.told.has(k)) changed.push(k);
    for (const k of this.told) if (!this.keys.has(k)) changed.push(k);
    this.told = new Set(this.keys);
    for (const k of changed) this.listeners.get(k)?.forEach((wake) => wake());
  }

  subscribe(key: string, wake: () => void): () => void {
    let set = this.listeners.get(key);
    if (!set) this.listeners.set(key, (set = new Set()));
    set.add(wake);
    return () => {
      set.delete(wake);
      if (set.size === 0) this.listeners.delete(key);
    };
  }
}

/**
 * One `Chosen` for the life of the list, holding `keys` — the selected rows, as this render
 * has them. Rows drawn in this render read it at once; the rows not drawn are woken once it
 * commits, before the frame is painted.
 */
export function useChosen(keys: Iterable<string>): Chosen {
  const [chosen] = useState(() => new Chosen());
  chosen.hold(keys);
  useLayoutEffect(() => chosen.tell());
  return chosen;
}

/** Whether the row `key` is selected, drawn again only when that changes. */
export function useIsChosen(chosen: Chosen, key: string): boolean {
  const subscribe = useCallback((wake: () => void) => chosen.subscribe(key, wake), [chosen, key]);
  return useSyncExternalStore(subscribe, () => chosen.has(key));
}
