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
  /** Refresh one service alone — one app's "update now". Never starts a second pass of it. */
  refreshService: (service: string) => Promise<void>;
  /** The services being refreshed on their own right now. */
  updating: readonly string[];
  /** Why a service's own refresh failed outright, by service. */
  serviceErrors: Readonly<Record<string, string>>;
  /** When the whole-account pass in flight began (unix seconds), or null. */
  passStartedAt: number | null;
};

const Context = createContext<Sync | null>(null);

/** The least time between refreshes brought on by coming back to the window. */
export const RETURN_THROTTLE_MS = 30_000;

export function SyncProvider({ children }: { children: ReactNode }) {
  const [report, setReport] = useState<Refresh | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [revision, setRevision] = useState(0);
  const [error, setError] = useState<string | null>(null);
  // A second click while a pass is running would mean two walks of the same zones.
  const inFlight = useRef(false);
  const [updating, setUpdating] = useState<string[]>([]);
  const [serviceErrors, setServiceErrors] = useState<Record<string, string>>({});
  const [passStartedAt, setPassStartedAt] = useState<number | null>(null);
  const serviceInFlight = useRef(new Set<string>());

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setSyncing(true);
    setPassStartedAt(Math.floor(Date.now() / 1000));
    setError(null);
    try {
      setReport(await api.syncAll());
    } catch (e) {
      setError(messageOf(e));
    } finally {
      inFlight.current = false;
      setSyncing(false);
      setPassStartedAt(null);
      setRevision((n) => n + 1);
    }
  }, []);

  const refreshService = useCallback(async (service: string) => {
    if (serviceInFlight.current.has(service)) return;
    serviceInFlight.current.add(service);
    setUpdating([...serviceInFlight.current]);
    setServiceErrors(({ [service]: _gone, ...rest }) => rest);
    try {
      const part = await api.syncSome([service]);
      // The backend's event brings the whole picture; this keeps the answer on screen even
      // where no event arrives, by folding this service's own report into the last one.
      setReport((was) =>
        was
          ? {
              ...was,
              services: [
                ...was.services.filter((s) => !part.services.some((p) => p.service === s.service)),
                ...part.services,
              ],
            }
          : part,
      );
    } catch (e) {
      const reason = messageOf(e);
      setServiceErrors((was) => ({ ...was, [service]: reason }));
    } finally {
      serviceInFlight.current.delete(service);
      setUpdating([...serviceInFlight.current]);
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

    /*
     * Coming back to the window is when somebody looks for a change they just made on their
     * phone. Between the five-minute background passes it could be up to five minutes stale
     * at exactly that moment, which reads as sync not working at all. So returning to the
     * window refreshes — but not more than once in half a minute, because switching back and
     * forth between two windows is not a reason to ask Apple the same question again.
     */
    let lastFocusPass = Date.now();
    const onReturn = () => {
      if (document.visibilityState === "hidden") return;
      if (Date.now() - lastFocusPass < RETURN_THROTTLE_MS) return;
      lastFocusPass = Date.now();
      void refresh();
    };
    window.addEventListener("focus", onReturn);
    document.addEventListener("visibilitychange", onReturn);

    return () => {
      cancelled = true;
      void stop.then((off) => off());
      window.removeEventListener("focus", onReturn);
      document.removeEventListener("visibilitychange", onReturn);
    };
  }, [refresh]);

  const value = useMemo(
    () => ({ report, syncing, revision, refresh, error, refreshService, updating, serviceErrors, passStartedAt }),
    [report, syncing, revision, refresh, error, refreshService, updating, serviceErrors, passStartedAt]
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

/**
 * Whether a service's first copy is still on its way: never synced here, and a pass running
 * (or not yet reported). An empty screen then means "coming", not "you have none" — saying the
 * second while the first is true told a person with years of reminders that they had none.
 */
export function useArriving(service: string): boolean {
  const { report, syncing, error } = useSync();
  if (error) return false;
  const mine = report?.services.find((s) => s.service === service);
  if (mine?.outcome === "failed" || mine?.outcome === "unavailable") return false;
  if (mine?.lastSyncedAt) return false;
  return syncing || report === null;
}

/** One service as an app shows it: whether it is being fetched, and how it last went. */
export type ServiceState = {
  /** Being fetched now — on its own, or as part of a whole-account pass it has not landed from. */
  working: boolean;
  /** Why it did not update, when it did not. */
  failure: string | null;
  lastSyncedAt: number | null;
  unavailable: boolean;
};

/**
 * One service's own state, for the app showing it. During a whole-account pass a service
 * counts as working only until its own part has landed — the passes run side by side, and
 * Reminders is done long before Photos is.
 */
export function useServiceState(service: string): ServiceState {
  const { report, syncing, passStartedAt, updating, serviceErrors } = useSync();
  const mine = report?.services.find((s) => s.service === service);
  const lastSyncedAt = mine?.lastSyncedAt ?? null;
  const landed = passStartedAt !== null && lastSyncedAt !== null && lastSyncedAt >= passStartedAt;
  const working = updating.includes(service) || (syncing && !landed && mine?.outcome !== "unavailable");
  const failure = serviceErrors[service] ?? (mine?.outcome === "failed" ? (mine.detail ?? "It didn’t update") : null);
  return { working, failure, lastSyncedAt, unavailable: mine?.outcome === "unavailable" };
}
