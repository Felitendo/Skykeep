import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, messageOf, type Reminder, type ReminderList } from "../lib/ipc";

const dueLabel = (seconds: number) => {
  const due = new Date(seconds * 1000);
  const today = new Date();
  const sameDay =
    due.getFullYear() === today.getFullYear() &&
    due.getMonth() === today.getMonth() &&
    due.getDate() === today.getDate();
  if (sameDay) return `Today ${due.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}`;
  return due.toLocaleDateString(undefined, { day: "numeric", month: "short" });
};

function relative(seconds: number | null): string {
  if (seconds === null) return "never";
  const ago = Math.max(0, Math.floor(Date.now() / 1000) - seconds);
  if (ago < 60) return "just now";
  if (ago < 3600) return `${Math.floor(ago / 60)} min ago`;
  if (ago < 86_400) return `${Math.floor(ago / 3600)} h ago`;
  return `${Math.floor(ago / 86_400)} d ago`;
}

export function RemindersView() {
  const [lists, setLists] = useState<ReminderList[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [items, setItems] = useState<Reminder[]>([]);
  const [showDone, setShowDone] = useState(false);
  const [lastSynced, setLastSynced] = useState<number | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [busyGuid, setBusyGuid] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const readLists = useCallback(async () => {
    const [found, synced] = await Promise.all([
      api.reminderLists(),
      api.remindersLastSynced(),
    ]);
    const safe = Array.isArray(found) ? found : [];
    setLists(safe);
    setLastSynced(typeof synced === "number" ? synced : null);
    // Prefer a list with something outstanding: opening on an empty one looks broken.
    if (safe.length > 0) {
      const counts = await Promise.all(
        safe.map(async (l) => [l.guid, (await api.remindersIn(l.guid, false)).length] as const)
      );
      setSelected((current) => {
        if (current) return current;
        const busiest = counts.reduce((a, b) => (b[1] > a[1] ? b : a));
        return busiest[1] > 0 ? busiest[0] : safe[0].guid;
      });
    }
    setLoaded(true);
  }, []);

  const readItems = useCallback(async () => {
    if (!selected) return setItems([]);
    const found = await api.remindersIn(selected, showDone);
    setItems(Array.isArray(found) ? found : []);
  }, [selected, showDone]);

  const refreshRef = useRef<null | (() => Promise<void>)>(null);

  const refresh = useCallback(async () => {
    setSyncing(true);
    setError(null);
    try {
      await api.remindersSync();
      await readLists();
      await readItems();
    } catch (e) {
      // The library still holds the last good data, so the list stays usable.
      setError(messageOf(e));
    } finally {
      setSyncing(false);
    }
  }, [readLists, readItems]);
  refreshRef.current = refresh;

  // Render from the library first, then fetch if nothing is stored yet.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        await readLists();
        if (cancelled) return;
        const synced = await api.remindersLastSynced();
        if (!cancelled && synced === null) void refreshRef.current?.();
      } catch (e) {
        if (!cancelled) {
          setError(messageOf(e));
          setLoaded(true);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [readLists]);

  useEffect(() => {
    readItems().catch((e) => setError(messageOf(e)));
  }, [readItems]);

  async function toggle(reminder: Reminder) {
    setBusyGuid(reminder.guid);
    setError(null);
    try {
      // Writes to Apple, then re-reads — so what's on screen is what Apple stored.
      await api.reminderSetDone(reminder.guid, reminder.listGuid, !reminder.completedAt);
      await readItems();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusyGuid(null);
    }
  }

  const current = useMemo(
    () => lists.find((l) => l.guid === selected) ?? null,
    [lists, selected]
  );
  const outstanding = items.filter((r) => !r.completedAt).length;

  return (
    <div className="rem">
      <aside className="rem-lists" aria-label="Reminder lists">
        {lists.map((list) => (
          <button
            key={list.guid}
            className="rem-list"
            aria-current={selected === list.guid}
            onClick={() => setSelected(list.guid)}
          >
            <span className="dot" style={{ background: list.colour || "var(--gold)" }} />
            <span className="rem-list-name">{list.title}</span>
          </button>
        ))}
        {loaded && lists.length === 0 && <p className="badge">No lists yet</p>}
      </aside>

      <section className="rem-items">
        <header className="cal-bar">
          <div className="cal-nav">
            <h1>{current?.title ?? "Reminders"}</h1>
            {outstanding > 0 && <span className="badge">{outstanding} left</span>}
          </div>
          <div className="cal-actions">
            <button className="quiet" onClick={() => setShowDone((v) => !v)}>
              {showDone ? "Hide completed" : "Show completed"}
            </button>
            <span className="badge">Updated {relative(lastSynced)}</span>
            <button className="quiet" onClick={refresh} disabled={syncing}>
              {syncing ? "Refreshing…" : "Refresh"}
            </button>
          </div>
        </header>

        {error && <p className="error" role="alert">{error}</p>}

        <ul className="rem-list-items">
          {items.map((reminder) => (
            <li key={reminder.guid} className={reminder.completedAt ? "is-done" : undefined}>
              <button
                className="tick"
                role="checkbox"
                aria-checked={Boolean(reminder.completedAt)}
                aria-label={reminder.title}
                disabled={busyGuid === reminder.guid}
                onClick={() => toggle(reminder)}
                style={{ borderColor: current?.colour || "var(--gold)" }}
              >
                {reminder.completedAt && (
                  <span
                    className="tick-fill"
                    style={{ background: current?.colour || "var(--gold)" }}
                  />
                )}
              </button>
              <div className="rem-text">
                <span className="rem-title">{reminder.title}</span>
                {reminder.notes && <span className="rem-notes">{reminder.notes}</span>}
              </div>
              {reminder.dueAt && <span className="rem-due">{dueLabel(reminder.dueAt)}</span>}
            </li>
          ))}
        </ul>

        {loaded && items.length === 0 && !error && (
          <p className="cal-empty">
            {showDone ? "Nothing in this list." : "All done here."}
          </p>
        )}
      </section>
    </div>
  );
}
