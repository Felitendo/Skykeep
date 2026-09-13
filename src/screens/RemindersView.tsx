import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  api,
  messageOf,
  type Reminder,
  type ReminderList,
  type SmartList,
} from "../lib/ipc";

/** Apple's built-in views, which is how most people enter Reminders. */
const SMART: { kind: SmartList; title: string }[] = [
  { kind: "today", title: "Today" },
  { kind: "scheduled", title: "Scheduled" },
  { kind: "flagged", title: "Flagged" },
  { kind: "all", title: "All" },
];

/** End of today in the viewer's own timezone — the interface knows it, the backend doesn't. */
function endOfToday(): number {
  const d = new Date();
  return Math.floor(new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1).getTime() / 1000);
}

function dueLabel(seconds: number): { text: string; overdue: boolean } {
  const due = new Date(seconds * 1000);
  const now = new Date();
  const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const days = Math.floor((due.getTime() - midnight.getTime()) / 86_400_000);
  const time = due.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  if (days < 0) return { text: due.toLocaleDateString(undefined, { day: "numeric", month: "short" }), overdue: true };
  if (days === 0) return { text: `Today ${time}`, overdue: due.getTime() < now.getTime() };
  if (days === 1) return { text: `Tomorrow ${time}`, overdue: false };
  return {
    text: due.toLocaleDateString(undefined, { day: "numeric", month: "short" }),
    overdue: false,
  };
}

/** `YYYY-MM-DDTHH:mm` for a datetime-local input, in local time. */
function toInputValue(seconds: number | null): string {
  if (seconds === null) return "";
  const d = new Date(seconds * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function relative(seconds: number | null): string {
  if (seconds === null) return "never";
  const ago = Math.max(0, Math.floor(Date.now() / 1000) - seconds);
  if (ago < 60) return "just now";
  if (ago < 3600) return `${Math.floor(ago / 60)} min ago`;
  if (ago < 86_400) return `${Math.floor(ago / 3600)} h ago`;
  return `${Math.floor(ago / 86_400)} d ago`;
}

type Selection = { type: "smart"; kind: SmartList } | { type: "list"; guid: string };

export function RemindersView() {
  const [lists, setLists] = useState<ReminderList[]>([]);
  const [selection, setSelection] = useState<Selection>({ type: "smart", kind: "today" });
  const [items, setItems] = useState<Reminder[]>([]);
  const [showDone, setShowDone] = useState(false);
  const [lastSynced, setLastSynced] = useState<number | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [busyGuid, setBusyGuid] = useState<string | null>(null);
  const [openGuid, setOpenGuid] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const inList = selection.type === "list";
  const listGuid = inList ? selection.guid : null;

  const readLists = useCallback(async () => {
    const [found, synced] = await Promise.all([
      api.reminderLists(),
      api.remindersLastSynced(),
    ]);
    setLists(Array.isArray(found) ? found : []);
    setLastSynced(typeof synced === "number" ? synced : null);
    setLoaded(true);
  }, []);

  const readItems = useCallback(async () => {
    const found =
      selection.type === "smart"
        ? await api.remindersSmart(selection.kind, endOfToday())
        : await api.remindersIn(selection.guid, showDone);
    setItems(Array.isArray(found) ? found : []);
  }, [selection, showDone]);

  const refreshRef = useRef<null | (() => Promise<void>)>(null);

  const refresh = useCallback(async () => {
    setSyncing(true);
    setError(null);
    try {
      await api.remindersSync();
      await readLists();
      await readItems();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setSyncing(false);
    }
  }, [readLists, readItems]);
  refreshRef.current = refresh;

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        await readLists();
        if (cancelled) return;
        if ((await api.remindersLastSynced()) === null) void refreshRef.current?.();
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

  /** Every write goes through here: to Apple first, then re-read, so nothing on screen is a guess. */
  const write = useCallback(
    async (reminder: Reminder, run: () => Promise<void>) => {
      setBusyGuid(reminder.guid);
      setError(null);
      try {
        await run();
        await readItems();
      } catch (e) {
        setError(messageOf(e));
      } finally {
        setBusyGuid(null);
      }
    },
    [readItems]
  );

  const toggle = (r: Reminder) =>
    write(r, () => api.reminderSetDone(r.guid, r.listGuid, !r.completedAt));
  const flag = (r: Reminder) =>
    write(r, () => api.reminderEdit(r.guid, r.listGuid, { flagged: !r.flagged }));
  const remove = (r: Reminder) => write(r, () => api.reminderDelete(r.guid, r.listGuid));
  const rename = (r: Reminder, title: string) =>
    title.trim() && title.trim() !== r.title
      ? write(r, () => api.reminderEdit(r.guid, r.listGuid, { title: title.trim() }))
      : Promise.resolve();
  const setDue = (r: Reminder, value: string) =>
    write(r, () =>
      api.reminderEdit(r.guid, r.listGuid, {
        dueAt: value ? Math.floor(new Date(value).getTime() / 1000) : null,
      })
    );
  const setNotes = (r: Reminder, notes: string) =>
    write(r, () => api.reminderEdit(r.guid, r.listGuid, { notes: notes.trim() || null }));

  async function add(e: React.FormEvent) {
    e.preventDefault();
    const title = draft.trim();
    // Adding needs a real list: a smart list is a view, not somewhere to put things.
    if (!title || !listGuid) return;
    setSyncing(true);
    setError(null);
    try {
      await api.reminderCreate(listGuid, title);
      setDraft("");
      await readItems();
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setSyncing(false);
    }
  }

  const heading = inList
    ? lists.find((l) => l.guid === listGuid)?.title ?? "Reminders"
    : SMART.find((s) => s.kind === (selection as { kind: SmartList }).kind)?.title ?? "Reminders";
  const outstanding = items.filter((r) => !r.completedAt).length;
  const colour = useMemo(
    () => lists.find((l) => l.guid === listGuid)?.colour || "var(--gold)",
    [lists, listGuid]
  );

  return (
    <div className="rem">
      <aside className="rem-lists" aria-label="Reminder lists">
        {SMART.map((s) => (
          <button
            key={s.kind}
            className="rem-list"
            aria-current={selection.type === "smart" && selection.kind === s.kind}
            onClick={() => setSelection({ type: "smart", kind: s.kind })}
          >
            <span className="rem-list-name">{s.title}</span>
          </button>
        ))}

        <p className="rem-lists-heading">My Lists</p>
        {lists.map((list) => (
          <button
            key={list.guid}
            className="rem-list"
            aria-current={listGuid === list.guid}
            onClick={() => setSelection({ type: "list", guid: list.guid })}
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
            <h1>{heading}</h1>
            {outstanding > 0 && <span className="badge">{outstanding}</span>}
          </div>
          <div className="cal-actions">
            {inList && (
              <button className="quiet" onClick={() => setShowDone((v) => !v)}>
                {showDone ? "Hide completed" : "Show completed"}
              </button>
            )}
            <span className="badge">{relative(lastSynced)}</span>
            <button className="quiet" onClick={refresh} disabled={syncing}>
              {syncing ? "Refreshing…" : "Refresh"}
            </button>
          </div>
        </header>

        {error && <p className="error" role="alert">{error}</p>}

        {inList && (
          <form className="rem-add" onSubmit={add}>
            <span className="tick ghost" aria-hidden="true" />
            <input
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              placeholder="Add a reminder"
              aria-label={`Add a reminder to ${heading}`}
              disabled={syncing}
            />
          </form>
        )}

        <ul className="rem-list-items">
          {items.map((reminder) => {
            const due = reminder.dueAt === null ? null : dueLabel(reminder.dueAt);
            const isOpen = openGuid === reminder.guid;
            return (
              <li key={reminder.guid} className={reminder.completedAt ? "is-done" : undefined}>
                <button
                  className="tick"
                  role="checkbox"
                  aria-checked={Boolean(reminder.completedAt)}
                  aria-label={reminder.title}
                  disabled={busyGuid === reminder.guid}
                  onClick={() => toggle(reminder)}
                  style={{ borderColor: colour }}
                >
                  {reminder.completedAt && (
                    <span className="tick-fill" style={{ background: colour }} />
                  )}
                </button>

                <div className="rem-text">
                  <input
                    className="rem-title-edit"
                    defaultValue={reminder.title}
                    key={`${reminder.guid}-${reminder.title}`}
                    aria-label={`Rename ${reminder.title}`}
                    disabled={busyGuid === reminder.guid}
                    onBlur={(e) => void rename(reminder, e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") e.currentTarget.blur();
                      if (e.key === "Escape") {
                        e.currentTarget.value = reminder.title;
                        e.currentTarget.blur();
                      }
                    }}
                  />
                  {reminder.notes && !isOpen && (
                    <span className="rem-notes">{reminder.notes}</span>
                  )}

                  {isOpen && (
                    <div className="rem-detail">
                      <label>
                        <span>Due</span>
                        <input
                          type="datetime-local"
                          defaultValue={toInputValue(reminder.dueAt)}
                          onBlur={(e) => void setDue(reminder, e.target.value)}
                        />
                      </label>
                      <label>
                        <span>Notes</span>
                        <textarea
                          defaultValue={reminder.notes ?? ""}
                          rows={2}
                          onBlur={(e) => void setNotes(reminder, e.target.value)}
                        />
                      </label>
                    </div>
                  )}
                </div>

                <span className="rem-trailing">
                  {due && (
                    <span className={`rem-due${due.overdue ? " overdue" : ""}`}>{due.text}</span>
                  )}
                  <button
                    className={`rem-flag${reminder.flagged ? " on" : ""}`}
                    aria-label={reminder.flagged ? `Unflag ${reminder.title}` : `Flag ${reminder.title}`}
                    aria-pressed={reminder.flagged}
                    disabled={busyGuid === reminder.guid}
                    onClick={() => flag(reminder)}
                  >
                    ⚑
                  </button>
                  <button
                    className="rem-more"
                    aria-label={isOpen ? `Close details for ${reminder.title}` : `Details for ${reminder.title}`}
                    aria-expanded={isOpen}
                    onClick={() => setOpenGuid(isOpen ? null : reminder.guid)}
                  >
                    ⋯
                  </button>
                  <button
                    className="rem-delete"
                    aria-label={`Delete ${reminder.title}`}
                    disabled={busyGuid === reminder.guid}
                    onClick={() => remove(reminder)}
                  >
                    ×
                  </button>
                </span>
              </li>
            );
          })}
        </ul>

        {loaded && items.length === 0 && !error && (
          <p className="cal-empty">
            {inList ? (showDone ? "Nothing in this list." : "All done here.") : "Nothing here."}
          </p>
        )}
      </section>
    </div>
  );
}
