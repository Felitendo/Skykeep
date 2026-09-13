import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, messageOf, type Calendar, type CalendarEvent, type Reminder } from "../lib/ipc";
import { useSync } from "../lib/sync";

const ZONE = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";

/** Local midnight for a date, as unix seconds. */
const startOfDay = (d: Date) =>
  Math.floor(new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() / 1000);

const isoDate = (seconds: number) => {
  const d = new Date(seconds * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
};

const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() &&
  a.getMonth() === b.getMonth() &&
  a.getDate() === b.getDate();

const weekdayNames = () => {
  // Locale-correct short weekday names, starting Sunday as Apple's calendar does.
  const base = new Date(2024, 8, 1); // a Sunday
  return Array.from({ length: 7 }, (_, i) => {
    const d = new Date(base);
    d.setDate(base.getDate() + i);
    return d.toLocaleDateString(undefined, { weekday: "short" });
  });
};

const clockTime = (seconds: number) =>
  new Date(seconds * 1000)
    .toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
    .replace(":00", "");

/** The 42 days a month grid shows: the weeks containing the first and last of the month. */
function gridDays(anchor: Date): Date[] {
  const first = new Date(anchor.getFullYear(), anchor.getMonth(), 1);
  const start = new Date(first);
  start.setDate(first.getDate() - first.getDay());
  return Array.from({ length: 42 }, (_, i) => {
    const d = new Date(start);
    d.setDate(start.getDate() + i);
    return d;
  });
}

export function CalendarView() {
  const today = useMemo(() => new Date(), []);
  const [anchor, setAnchor] = useState(() => new Date(today.getFullYear(), today.getMonth(), 1));
  const [events, setEvents] = useState<CalendarEvent[]>([]);
  const [dueReminders, setDueReminders] = useState<Reminder[]>([]);
  const [calendars, setCalendars] = useState<Calendar[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** A month outside the window the shared refresh keeps warm is being fetched. */
  const [fetchingMonth, setFetchingMonth] = useState(false);
  const { revision } = useSync();

  const days = useMemo(() => gridDays(anchor), [anchor]);
  const range = useMemo(
    () => ({
      from: startOfDay(days[0]),
      to: startOfDay(days[41]) + 86_400,
    }),
    [days]
  );

  const read = useCallback(async () => {
    // Apple's Calendar shows reminders beside events, which for most people is most of
    // what fills a month.
    const [found, cals, due] = await Promise.all([
      api.calendarEvents(range.from, range.to),
      api.calendarList(),
      api.remindersDueBetween(range.from, range.to),
    ]);
    setEvents(Array.isArray(found) ? found : []);
    setDueReminders(Array.isArray(due) ? due : []);
    setCalendars(Array.isArray(cals) ? cals : []);
    setLoaded(true);
  }, [range]);

  // The grid always paints from the library: instant, and correct offline. `revision` ticks
  // when the shared refresh finishes, which is the only thing that can change what is there.
  useEffect(() => {
    let cancelled = false;
    read().catch((e) => {
      if (!cancelled) {
        setError(messageOf(e));
        setLoaded(true);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [read, revision]);

  // The shared refresh keeps a few months around today warm. Scroll past that and this
  // fetches the month being looked at, once — clicking through a year must not mean a
  // dozen round trips for months already fetched.
  const fetched = useRef(new Set<string>());

  useEffect(() => {
    const from = isoDate(range.from);
    const to = isoDate(range.to - 1);
    const key = `${from}:${to}`;
    if (fetched.current.has(key)) return undefined;

    let cancelled = false;
    // A short wait so holding down the arrow scrolls through months without fetching each.
    const timer = window.setTimeout(() => {
      fetched.current.add(key);
      setFetchingMonth(true);
      api
        .calendarSync(from, to, ZONE)
        .then(() => {
          if (!cancelled) return read();
          return undefined;
        })
        .catch((e) => {
          // The library still holds the last good data, so the grid stays usable.
          fetched.current.delete(key);
          if (!cancelled) setError(messageOf(e));
        })
        .finally(() => {
          if (!cancelled) setFetchingMonth(false);
        });
    }, 350);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [range, read]);

  const colours = useMemo(
    () => new Map(calendars.map((c) => [c.guid, c.colour])),
    [calendars]
  );

  /** What a day cell shows: an event, or a reminder due that day. */
  type Entry =
    | { kind: "event"; event: CalendarEvent }
    | { kind: "reminder"; reminder: Reminder };

  /** Entries keyed by local day, so each cell is a lookup rather than a scan. */
  const byDay = useMemo(() => {
    const map = new Map<string, Entry[]>();
    const push = (key: string, entry: Entry) => {
      const bucket = map.get(key);
      if (bucket) bucket.push(entry);
      else map.set(key, [entry]);
    };
    for (const event of events) {
      // An event spanning days belongs in every day it touches.
      let cursor = new Date(event.startsAt * 1000);
      cursor = new Date(cursor.getFullYear(), cursor.getMonth(), cursor.getDate());
      const last = new Date(Math.max(event.endsAt - 1, event.startsAt) * 1000);
      let guard = 0;
      while (cursor <= last && guard++ < 60) {
        push(cursor.toDateString(), { kind: "event", event });
        cursor = new Date(cursor.getFullYear(), cursor.getMonth(), cursor.getDate() + 1);
      }
    }
    for (const reminder of dueReminders) {
      if (reminder.dueAt === null) continue;
      push(new Date(reminder.dueAt * 1000).toDateString(), { kind: "reminder", reminder });
    }
    // Within a day, order by time so the cell reads like a schedule.
    for (const bucket of map.values()) {
      bucket.sort((a, b) => {
        const at = a.kind === "event" ? a.event.startsAt : (a.reminder.dueAt ?? 0);
        const bt = b.kind === "event" ? b.event.startsAt : (b.reminder.dueAt ?? 0);
        return at - bt;
      });
    }
    return map;
  }, [events, dueReminders]);

  const shift = (months: number) =>
    setAnchor((a) => new Date(a.getFullYear(), a.getMonth() + months, 1));

  const monthLabel = anchor.toLocaleDateString(undefined, { month: "long", year: "numeric" });

  return (
    <div className="cal">
      <header className="cal-bar">
        <div className="cal-nav">
          <h1>{monthLabel}</h1>
          <button className="chev" onClick={() => shift(-1)} aria-label="Previous month">‹</button>
          <button
            className="today-btn"
            onClick={() => setAnchor(new Date(today.getFullYear(), today.getMonth(), 1))}
          >
            Today
          </button>
          <button className="chev" onClick={() => shift(1)} aria-label="Next month">›</button>
        </div>
        <div className="cal-actions">
          {fetchingMonth && <span className="badge">Fetching this month…</span>}
        </div>
      </header>

      {error && <p className="error" role="alert">{error}</p>}

      <div className="cal-weekdays" aria-hidden="true">
        {weekdayNames().map((name) => (
          <span key={name}>{name}</span>
        ))}
      </div>

      <div className="cal-grid" role="grid" aria-label={`${monthLabel} calendar`}>
        {/* A grid's cells must sit inside rows, or screen readers cannot navigate it by
            week. The rows use display:contents so the visual layout is unchanged. */}
        {[0, 1, 2, 3, 4, 5].map((week) => (
          <div className="cal-week" role="row" key={week}>
            {days.slice(week * 7, week * 7 + 7).map((day) => {
              const outside = day.getMonth() !== anchor.getMonth();
              const isToday = sameDay(day, today);
              const items = byDay.get(day.toDateString()) ?? [];
              const label = day.toLocaleDateString(undefined, {
                weekday: "long",
                day: "numeric",
                month: "long",
              });
              return (
                <div
                  key={day.toISOString()}
                  className={`cal-day${outside ? " outside" : ""}`}
                  role="gridcell"
                  aria-label={
                    items.length ? `${label}, ${items.length} event(s)` : label
                  }
                  aria-current={isToday ? "date" : undefined}
                >
                  <span className={`daynum${isToday ? " is-today" : ""}`}>
                    {day.getDate()}
                  </span>
                  <div className="chips">
                    {items.slice(0, 3).map((entry) =>
                      entry.kind === "event" ? (
                        <span
                          key={entry.event.guid + day.getDate()}
                          className={`chip${entry.event.allDay ? " all-day" : ""}`}
                          style={
                            entry.event.allDay
                              ? {
                                  background:
                                    colours.get(entry.event.calendarGuid) || "var(--gold)",
                                }
                              : undefined
                          }
                          title={entry.event.title}
                        >
                          {!entry.event.allDay && (
                            <span
                              className="chip-dot"
                              style={{
                                background:
                                  colours.get(entry.event.calendarGuid) || "var(--gold)",
                              }}
                            />
                          )}
                          {!entry.event.allDay && (
                            <span className="chip-time">
                              {clockTime(entry.event.startsAt)}
                            </span>
                          )}
                          <span className="chip-title">{entry.event.title}</span>
                        </span>
                      ) : (
                        <span
                          key={entry.reminder.guid + day.getDate()}
                          className={`chip reminder${entry.reminder.completedAt ? " is-done" : ""}`}
                          title={entry.reminder.title}
                        >
                          {/* A ring rather than a dot, so a reminder reads as a task. */}
                          <span className="chip-ring" />
                          <span className="chip-time">
                            {clockTime(entry.reminder.dueAt ?? 0)}
                          </span>
                          <span className="chip-title">{entry.reminder.title}</span>
                        </span>
                      )
                    )}
                    {items.length > 3 && (
                      <span className="chip more">{items.length - 3} more</span>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        ))}
      </div>

      {loaded && events.length === 0 && dueReminders.length === 0 && !error && (
        <p className="cal-empty">
          No events in this month. Quince keeps them on this computer once synced, so they
          open instantly and work offline.
        </p>
      )}
    </div>
  );
}
