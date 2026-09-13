import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { api, messageOf, SYNCED, type Refresh } from "./ipc";

/**
 * One refresh for the whole account, shared by every screen.
 *
 * Screens read the local library and never the network, so they do not need to know a
 * refresh is happening — only that one finished. `revision` is that signal: it ticks once
 * per completed pass, from the button or from the background timer alike, and a screen
 * re-reads by listing it as a dependency.
 */
type Sync = {
  /** What the last pass did, per service. */
  report: Refresh | null;
  /** A pass is in flight. */
  syncing: boolean;
  /** Ticks once per completed pass. Screens re-read when it changes. */
  revision: number;
  /** Ask for a pass now. Safe to call while one is running; it will not start a second. */
  refresh: () => Promise<void>;
  /** The last refresh that failed outright, rather than a single service reporting trouble. */
  error: string | null;
};

const Context = createContext<Sync | null>(null);

export function SyncProvider({ children }: { children: ReactNode }) {
  const [report, setReport] = useState<Refresh | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [revision, setRevision] = useState(0);
  const [error, setError] = useState<string | null>(null);
  // A second click while a pass is running would mean two walks of the same zones.
  const inFlight = useRef(false);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setSyncing(true);
    setError(null);
    try {
      setReport(await api.syncAll());
    } catch (e) {
      setError(messageOf(e));
    } finally {
      inFlight.current = false;
      setSyncing(false);
      setRevision((n) => n + 1);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;

    // Show the freshness the library already knows before asking Apple anything, so the
    // window is never blank while the first pass runs.
    api
      .syncStatus()
      .then((known) => {
        if (!cancelled) setReport(known);
      })
      .catch(() => {
        /* the refresh below will say what is wrong */
      })
      // Opening the window is itself a request for current data, so a pass always runs.
      // Showing the stored freshness first means the wait happens behind real content.
      .finally(() => {
        if (!cancelled) void refresh();
      });

    // Background passes finish without anyone asking, and every open screen has to notice.
    const stop = listen<Refresh>(SYNCED, (event) => {
      if (cancelled) return;
      setReport(event.payload);
      setRevision((n) => n + 1);
    });

    return () => {
      cancelled = true;
      void stop.then((off) => off());
    };
  }, [refresh]);

  const value = useMemo(
    () => ({ report, syncing, revision, refresh, error }),
    [report, syncing, revision, refresh, error]
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useSync(): Sync {
  const found = useContext(Context);
  if (!found) throw new Error("useSync must be used inside a SyncProvider");
  return found;
}

/** "just now", "4 min ago", "yesterday" — how old the library is, in words. */
export function freshness(asOf: number | null): string {
  if (asOf === null) return "Not synced yet";
  const seconds = Math.max(0, Math.floor(Date.now() / 1000) - asOf);
  if (seconds < 60) return "Updated just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `Updated ${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `Updated ${hours} hr ago`;
  const days = Math.floor(hours / 24);
  return days === 1 ? "Updated yesterday" : `Updated ${days} days ago`;
}
