import { useEffect, useLayoutEffect, useRef, useSyncExternalStore } from "react";
import { CircleCheck, TriangleAlert, X } from "lucide-react";
import { fadeIn, fadeOut } from "../lib/fade";
import { useScreenActive } from "../lib/screen";
import { api, messageOf } from "../lib/ipc";

/**
 * Everything the window says in passing — a file saved, a change undone, a change Apple turned
 * down — said one way, in one place: a small card at the foot of the content, centred over it,
 * above anything that sits at the bottom (Photos' filmstrip). At most two at once, the newest
 * nearest the edge. Each goes by itself after a moment, an error only when put away, since a
 * reason must be read; any one can carry a single action — Undo, Show in Folder, Try Again.
 *
 * Every screen used to say these its own way, in its own corner: a red bar floating over a
 * list, a line of blue under a toolbar, a pill at the bottom of the window, a box at the left.
 */
export type ToastAction = { label: string; run: () => void };
export type ToastInput = {
  message: string;
  /** "done" carries a check; "error" a warning, and stays until put away. */
  tone?: "info" | "done" | "error";
  action?: ToastAction;
  /** How long it stays, in ms; null for until put away. */
  duration?: number | null;
};
type Toast = ToastInput & { id: number };

const MAX = 2;
let shown: Toast[] = [];
let next = 1;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());
const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};

/** Say something in passing. Returns its id, to take it back early with `dismissToast`. */
export function toast(input: string | ToastInput): number {
  const t: Toast = { tone: "info", ...(typeof input === "string" ? { message: input } : input), id: next++ };
  // The same words twice are one toast, said again: not a stack of copies.
  shown = [...shown.filter((s) => s.message !== t.message), t].slice(-MAX);
  emit();
  return t.id;
}

export function dismissToast(id: number) {
  if (!shown.some((t) => t.id === id)) return;
  shown = shown.filter((t) => t.id !== id);
  emit();
}

/**
 * Room kept clear at the foot of the content while something is drawn there — Photos'
 * filmstrip — so a toast never lands on it. The largest asked for wins.
 */
const clearances = new Map<symbol, number>();
const clearance = () => Math.max(0, ...clearances.values());
export function useToastClearance(px: number) {
  const key = useRef(Symbol("clearance")).current;
  useEffect(() => {
    if (px > 0) clearances.set(key, px);
    else clearances.delete(key);
    emit();
    return () => {
      clearances.delete(key);
      emit();
    };
  }, [key, px]);
}

/**
 * A screen's error, said as a toast while that screen is in front — as its notice was, drawn
 * in the screen — and cleared from the screen's state once put away. Out of sight, it waits.
 */
export function useErrorToast(error: string | null, clear: () => void) {
  const active = useScreenActive();
  const clearRef = useRef(clear);
  clearRef.current = clear;
  const id = useRef<number | null>(null);
  useEffect(() => {
    if (!error || !active) return undefined;
    const mine = toast({ message: error, tone: "error", duration: null });
    id.current = mine;
    // Put away in the toast: the screen forgets it too.
    const stop = subscribe(() => {
      if (id.current === mine && !shown.some((t) => t.id === mine)) {
        id.current = null;
        clearRef.current();
      }
    });
    return () => {
      stop();
      // Gone from the screen's state or out of sight: withdrawn without clearing it.
      if (id.current === mine) {
        id.current = null;
        dismissToast(mine);
      }
    };
  }, [error, active]);
}

/** Drawn once, in the library, over the foot of the content. */
export function ToastHost() {
  const list = useSyncExternalStore(subscribe, () => shown);
  const lift = useSyncExternalStore(subscribe, clearance);
  return (
    <div className="toast-host" style={lift ? ({ "--toast-clear": `${lift}px` } as React.CSSProperties) : undefined}>
      {list.map((t) => (
        <ToastCard key={t.id} toast={t} />
      ))}
    </div>
  );
}

function ToastCard({ toast: t }: { toast: Toast }) {
  const card = useRef<HTMLDivElement>(null);
  const leaving = useRef(false);
  useLayoutEffect(() => fadeIn(card.current, { duration: 140 }), []);
  const dismiss = () => {
    if (leaving.current) return;
    leaving.current = true;
    // The keys go back to the window, not to a card on its way out.
    if (card.current?.contains(document.activeElement)) (document.activeElement as HTMLElement).blur();
    fadeOut(card.current, () => dismissToast(t.id), 120);
  };
  // Gone by itself after a moment — longer when there is something to do — but never while
  // the pointer or the keys are on it.
  const held = useRef(false);
  const duration = t.duration === undefined ? (t.tone === "error" ? null : t.action ? 6000 : 4000) : t.duration;
  useEffect(() => {
    if (duration === null) return undefined;
    const timer = window.setInterval(() => {
      if (!held.current) dismiss();
    }, duration);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [duration]);
  const hold = (on: boolean) => () => {
    held.current = on;
  };
  return (
    <div
      ref={card}
      className={`toast is-${t.tone ?? "info"}`}
      role={t.tone === "error" ? "alert" : "status"}
      aria-live={t.tone === "error" ? "assertive" : "polite"}
      onPointerEnter={hold(true)}
      onPointerLeave={hold(false)}
      onFocus={hold(true)}
      onBlur={hold(false)}
      onKeyDown={(e) => {
        if (e.key === "Escape" && !leaving.current) {
          e.stopPropagation();
          dismiss();
        }
      }}
    >
      {t.tone === "error" ? (
        <TriangleAlert className="toast-glyph" size={15} strokeWidth={2} aria-hidden="true" />
      ) : t.tone === "done" ? (
        <CircleCheck className="toast-glyph" size={15} strokeWidth={2} aria-hidden="true" />
      ) : null}
      <p>{t.message}</p>
      {t.action && (
        <button
          className="toast-action"
          onClick={() => {
            t.action!.run();
            dismiss();
          }}
        >
          {t.action.label}
        </button>
      )}
      <button className="icon-button small toast-close" aria-label="Dismiss" title="Dismiss" onClick={dismiss}>
        <X size={13} strokeWidth={2.25} aria-hidden="true" />
      </button>
    </div>
  );
}

/** Where a file went, as a person names it: the folder, never the whole path from the root. */
export function folderOf(path: string): string {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return parts.length > 1 ? parts[parts.length - 2] : "your computer";
}

/**
 * A file saved to this computer, said the one way: the folder's name — never the whole path —
 * and Show in Folder, which opens the file manager on the file itself.
 */
export function toastSaved(path: string, count = 1) {
  return toast({
    message: count === 1 ? `Saved to ${folderOf(path)}` : `${count} saved to ${folderOf(path)}`,
    tone: "done",
    action: {
      label: "Show in Folder",
      run: () => void api.downloadReveal(path).catch((e) => toast({ message: messageOf(e), tone: "error" })),
    },
  });
}
