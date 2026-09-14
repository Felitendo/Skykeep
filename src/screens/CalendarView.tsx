import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  api,
  defaultCalendar,
  messageOf,
  type Calendar,
  type CalendarEvent,
  type Reminder,
} from "../lib/ipc";
import { useSync } from "../lib/sync";
import { Picker } from "../components/Picker";

const ZONE = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";

/** Local midnight for a date, as unix seconds. */
const startOfDay = (d: Date) =>
  Math.floor(new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() / 1000);

/**
 * Local midnight the morning after, as unix seconds.
 *
 * Not midnight plus 86,400. On the two days a year a clock moves, a day is twenty-three hours
 * or twenty-five, so adding a fixed day lands an hour either side of midnight and takes an
 * extra evening's events into the view or leaves one out. Asking the calendar for the next
 * date is right wherever the viewer is.
 */
const startOfNextDay = (d: Date) =>
  Math.floor(new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1).getTime() / 1000);

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

/** `YYYY-MM-DDTHH:mm` for a datetime-local input, in local time. */
const toInputValue = (seconds: number) => {
  const d = new Date(seconds * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
};

/** An event being written. A guid means it already exists at Apple. */
type EventDraft = {
  guid: string | null;
  calendarGuid: string;
  title: string;
  location: string;
  startsAt: number;
  endsAt: number;
  allDay: boolean;
  notes: string;
};

/** A new event on `day`, at the next whole hour, running an hour — Apple's own default. */
function blankEvent(day: Date, calendarGuid: string): EventDraft {
  const now = new Date();
  const at = new Date(day.getFullYear(), day.getMonth(), day.getDate(), now.getHours() + 1, 0, 0);
  const startsAt = Math.floor(at.getTime() / 1000);
  return {
    guid: null,
    calendarGuid,
    title: "",
    location: "",
    startsAt,
    endsAt: startsAt + 3600,
    allDay: false,
    notes: "",
  };
}

function draftFrom(event: CalendarEvent): EventDraft {
  return {
    guid: event.guid,
    calendarGuid: event.calendarGuid,
    title: event.title,
    location: event.location ?? "",
    startsAt: event.startsAt,
    endsAt: event.endsAt,
    allDay: event.allDay,
    notes: event.notes ?? "",
  };
}

/** The rows of a day or week grid. Apple shows the full twenty-four. */
const HOURS = Array.from({ length: 24 }, (_, i) => i);

/** How tall one hour is in the day and week grids. */
const HOUR_HEIGHT = 44;

/**
 * Where a timed event sits on the hour grid.
 *
 * The grid has a header row, an all-day row and then one row per hour, so an event is
 * placed against the grid's own tracks rather than by measuring pixels — that way it stays
 * put when the window resizes.
 */
function placement(
  event: CalendarEvent,
  day: Date,
  column: number
): React.CSSProperties {
  const dayStart = startOfDay(day);
  const dayEnd = startOfNextDay(day);
  const from = Math.max(event.startsAt, dayStart);
  const to = Math.min(event.endsAt, dayEnd);
  const minutesIn = (from - dayStart) / 60;
  // A zero-length event still has to be readable, so nothing is thinner than a quarter hour.
  const minutesLong = Math.max((to - from) / 60, 15);
  return {
    gridColumn: column + 2,
    // Rows: 1 is the column head, 2 is all-day, so hour zero starts at 3.
    gridRow: "3 / -1",
    marginTop: `${(minutesIn / 60) * HOUR_HEIGHT}px`,
    height: `${(minutesLong / 60) * HOUR_HEIGHT}px`,
  };
}

/** Apple's four scopes, in Apple's order. */
const VIEWS = [
  { key: "day", label: "Day" },
  { key: "week", label: "Week" },
  { key: "month", label: "Month" },
  { key: "year", label: "Year" },
] as const;

type View = (typeof VIEWS)[number]["key"];

const midnight = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());

/** The Sunday on or before `d`, which is where Apple starts a week. */
const startOfWeek = (d: Date) => {
  const start = midnight(d);
  start.setDate(start.getDate() - start.getDay());
  return start;
};

/** Move a date by one unit of `view`. */
function step(from: Date, view: View, by: number): Date {
  const d = new Date(from);
  if (view === "day") d.setDate(d.getDate() + by);
  else if (view === "week") d.setDate(d.getDate() + by * 7);
  else if (view === "month") d.setMonth(d.getMonth() + by);
  else d.setFullYear(d.getFullYear() + by);
  return d;
}

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
  /** The day whose contents the inspector shows. Defaults to today.  */
  const [selected, setSelected] = useState<Date>(() => new Date(today));
  /** Day, week, month or year — the scope everything else follows. */
  const [view, setView] = useState<View>("month");
  /** The event being written, or null when the inspector is just listing. */
  const [draft, setDraft] = useState<EventDraft | null>(null);
  const [busy, setBusy] = useState(false);

  const days = useMemo(() => gridDays(anchor), [anchor]);

  /** The seven days of the week `anchor` falls in. */
  const weekDays = useMemo(() => {
    const start = startOfWeek(anchor);
    return Array.from({ length: 7 }, (_, i) => {
      const d = new Date(start);
      d.setDate(start.getDate() + i);
      return d;
    });
  }, [anchor]);

  // Each view asks Apple for exactly what it shows, so a day view does not fetch a month
  // and a year view does not fetch it twelve times over.
  const range = useMemo(() => {
    if (view === "day") {
      return { from: startOfDay(anchor), to: startOfNextDay(anchor) };
    }
    if (view === "week") {
      return { from: startOfDay(weekDays[0]), to: startOfNextDay(weekDays[6]) };
    }
    if (view === "year") {
      return {
        from: startOfDay(new Date(anchor.getFullYear(), 0, 1)),
        to: startOfNextDay(new Date(anchor.getFullYear(), 11, 31)),
      };
    }
    return { from: startOfDay(days[0]), to: startOfNextDay(days[41]) };
  }, [view, anchor, days, weekDays]);

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

  const dayEntries = byDay.get(selected.toDateString()) ?? [];

  // A month grid moves with the arrow keys, a week at a time vertically — the same shape
  // as a spreadsheet, and what a screen reader announces a grid as being.
  const selectedCell = useRef<HTMLButtonElement | null>(null);

  const onGridKey = (e: React.KeyboardEvent, day: Date) => {
    const step = (days: number) => {
      e.preventDefault();
      const to = new Date(day.getFullYear(), day.getMonth(), day.getDate() + days);
      setSelected(to);
      setDraft(null);
      // Stepping off the month follows the selection there rather than stranding it.
      if (to.getMonth() !== anchor.getMonth() || to.getFullYear() !== anchor.getFullYear()) {
        setAnchor(new Date(to.getFullYear(), to.getMonth(), 1));
      }
    };
    if (e.key === "ArrowRight") step(1);
    else if (e.key === "ArrowLeft") step(-1);
    else if (e.key === "ArrowDown") step(7);
    else if (e.key === "ArrowUp") step(-7);
    else if (e.key === "Home") step(-day.getDay());
    else if (e.key === "End") step(6 - day.getDay());
  };

  // Keep focus with the selection when the arrows move it, but never steal it otherwise.
  const hadFocus = useRef(false);
  useEffect(() => {
    if (hadFocus.current) selectedCell.current?.focus();
  }, [selected]);

  /** Every write goes to Apple first, then the library is re-read. */
  const write = useCallback(
    async (run: () => Promise<void>) => {
      setBusy(true);
      setError(null);
      try {
        await run();
        await read();
        return true;
      } catch (e) {
        setError(messageOf(e));
        return false;
      } finally {
        setBusy(false);
      }
    },
    [read]
  );

  const saveDraft = useCallback(async () => {
    if (!draft) return;
    const ok = await write(async () => {
      await api.eventSave({
        guid: draft.guid,
        calendarGuid: draft.calendarGuid,
        title: draft.title,
        location: draft.location || null,
        startsAt: draft.startsAt,
        endsAt: draft.endsAt,
        allDay: draft.allDay,
        timezone: ZONE,
        notes: draft.notes || null,
        url: null,
      });
    });
    if (ok) setDraft(null);
  }, [draft, write]);

  const removeEvent = useCallback(
    async (guid: string) => {
      if (await write(() => api.eventDelete(guid))) setDraft(null);
    },
    [write]
  );

  // The arrows move by whatever is on screen: a day in Day, a week in Week, and so on.
  const shift = (by: number) =>
    setAnchor((a) => {
      const to = step(a, view, by);
      // In Day and Week the thing being looked at is also the thing selected.
      if (view === "day" || view === "week") setSelected(to);
      return to;
    });

  /** What the title says, which is different in every view — as it is in Calendar. */
  const title = useMemo(() => {
    if (view === "day") {
      return anchor.toLocaleDateString(undefined, {
        weekday: "long",
        day: "numeric",
        month: "long",
        year: "numeric",
      });
    }
    if (view === "week") {
      const [first, last] = [weekDays[0], weekDays[6]];
      const sameMonth = first.getMonth() === last.getMonth();
      const left = first.toLocaleDateString(undefined, { day: "numeric", month: "short" });
      const right = last.toLocaleDateString(
        undefined,
        sameMonth ? { day: "numeric" } : { day: "numeric", month: "short" }
      );
      return `${left} – ${right} ${last.getFullYear()}`;
    }
    if (view === "year") return String(anchor.getFullYear());
    return anchor.toLocaleDateString(undefined, { month: "long", year: "numeric" });
  }, [view, anchor, weekDays]);

  const monthLabel = title;

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
          {fetchingMonth && <span className="badge">Fetching…</span>}
          {/* Calendar's own scope control, in Calendar's own order. */}
          <div className="segmented" role="group" aria-label="Calendar view">
            {VIEWS.map((v) => (
              <button
                key={v.key}
                type="button"
                className="segment"
                aria-pressed={view === v.key}
                onClick={() => {
                  setView(v.key);
                  setDraft(null);
                  // Switching scope keeps you where you were looking.
                  if (v.key === "day" || v.key === "week") setAnchor(new Date(selected));
                }}
              >
                {v.label}
              </button>
            ))}
          </div>
        </div>
      </header>

      {error && <p className="error" role="alert">{error}</p>}

      {view === "month" && (
        <>
      <div className="cal-weekdays" aria-hidden="true">
        {weekdayNames().map((name) => (
          <span key={name}>{name}</span>
        ))}
      </div>

      <div
        className="cal-grid"
        role="grid"
        aria-label={`${monthLabel} calendar`}
        onFocus={() => {
          hadFocus.current = true;
        }}
        onBlur={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node)) hadFocus.current = false;
        }}
      >
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
              const isSelected = sameDay(day, selected);
              return (
                <div
                  key={day.toISOString()}
                  className={`cal-day${outside ? " outside" : ""}${isSelected ? " is-selected" : ""}`}
                  role="gridcell"
                  aria-current={isToday ? "date" : undefined}
                >
                  <button
                    type="button"
                    className="cal-day-hit"
                    ref={(el) => {
                      if (isSelected) selectedCell.current = el;
                    }}
                    // One tab stop for the whole month, as a grid should have; the arrows
                    // move within it.
                    tabIndex={isSelected ? 0 : -1}
                    aria-label={items.length ? `${label}, ${items.length} item(s)` : label}
                    aria-pressed={isSelected}
                    onClick={() => {
                      setSelected(day);
                      setDraft(null);
                    }}
                    onKeyDown={(e) => onGridKey(e, day)}
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
                  </button>
                </div>
              );
            })}
          </div>
        ))}
      </div>

        </>
      )}

      {view === "year" && (
        <div className="cal-year" role="grid" aria-label={`${anchor.getFullYear()} calendar`}>
          {Array.from({ length: 12 }, (_, month) => {
            const first = new Date(anchor.getFullYear(), month, 1);
            const cells = gridDays(first);
            return (
              <button
                key={month}
                type="button"
                className="cal-mini"
                aria-label={first.toLocaleDateString(undefined, { month: "long", year: "numeric" })}
                onClick={() => {
                  setAnchor(first);
                  setView("month");
                }}
              >
                <span className="cal-mini-name">
                  {first.toLocaleDateString(undefined, { month: "long" })}
                </span>
                <span className="cal-mini-grid" aria-hidden="true">
                  {weekdayNames().map((name) => (
                    <span className="cal-mini-weekday" key={name}>
                      {name.slice(0, 1)}
                    </span>
                  ))}
                  {cells.map((day) => {
                    const outside = day.getMonth() !== month;
                    const busyDay = (byDay.get(day.toDateString()) ?? []).length > 0;
                    return (
                      <span
                        key={day.toISOString()}
                        className={`cal-mini-day${outside ? " outside" : ""}${
                          sameDay(day, today) ? " is-today" : ""
                        }${busyDay && !outside ? " has-items" : ""}`}
                      >
                        {day.getDate()}
                      </span>
                    );
                  })}
                </span>
              </button>
            );
          })}
        </div>
      )}

      {(view === "day" || view === "week") && (
        <div className="cal-hours-wrap">
          <div
            className={`cal-hours${view === "day" ? " is-day" : ""}`}
            role="grid"
            aria-label={`${title} schedule`}
          >
            <span className="cal-hours-corner" aria-hidden="true" />
            {(view === "day" ? [anchor] : weekDays).map((day) => (
              <button
                key={`head-${day.toISOString()}`}
                type="button"
                className={`cal-col-head${sameDay(day, today) ? " is-today" : ""}${
                  sameDay(day, selected) ? " is-selected" : ""
                }`}
                onClick={() => {
                  setSelected(day);
                  setDraft(null);
                }}
              >
                <span className="cal-col-weekday">
                  {day.toLocaleDateString(undefined, { weekday: "short" })}
                </span>
                <span className="cal-col-daynum">{day.getDate()}</span>
              </button>
            ))}

            {/* All-day events sit above the hours, as they do in Calendar. */}
            <span className="cal-allday-label" aria-hidden="true">
              all-day
            </span>
            {(view === "day" ? [anchor] : weekDays).map((day) => (
              <div className="cal-allday" key={`allday-${day.toISOString()}`}>
                {(byDay.get(day.toDateString()) ?? [])
                  .filter((e) => e.kind === "event" && e.event.allDay)
                  .map((entry) =>
                    entry.kind === "event" ? (
                      <button
                        key={entry.event.guid}
                        type="button"
                        className="chip all-day"
                        style={{
                          background: colours.get(entry.event.calendarGuid) || "var(--gold)",
                        }}
                        onClick={() => {
                          setSelected(day);
                          setDraft(draftFrom(entry.event));
                        }}
                      >
                        <span className="chip-title">{entry.event.title}</span>
                      </button>
                    ) : null
                  )}
              </div>
            ))}

            {HOURS.map((hour) => (
              <Fragment key={hour}>
                <span className="cal-hour-label">
                  {hour === 0
                    ? ""
                    : new Date(2024, 0, 1, hour).toLocaleTimeString(undefined, {
                        hour: "numeric",
                      })}
                </span>
                {(view === "day" ? [anchor] : weekDays).map((day) => (
                  <button
                    key={`${day.toISOString()}-${hour}`}
                    type="button"
                    className="cal-hour-cell"
                    aria-label={`${day.toLocaleDateString(undefined, {
                      weekday: "long",
                      day: "numeric",
                      month: "long",
                    })} at ${hour}:00`}
                    onClick={() => {
                      setSelected(day);
                      const at = new Date(
                        day.getFullYear(),
                        day.getMonth(),
                        day.getDate(),
                        hour
                      );
                      // An account with no calendars has nowhere to put an event. The New
                      // event button already knows that; clicking an empty hour has to know
                      // it too, or it opens a form that cannot be saved.
                      const target = defaultCalendar(calendars)?.guid;
                      if (!target) return;
                      const startsAt = Math.floor(at.getTime() / 1000);
                      setDraft({
                        ...blankEvent(day, target),
                        startsAt,
                        endsAt: startsAt + 3600,
                      });
                    }}
                  />
                ))}
              </Fragment>
            ))}

            {/* Timed events, laid over the hour grid where they actually fall. */}
            {(view === "day" ? [anchor] : weekDays).map((day, column) =>
              (byDay.get(day.toDateString()) ?? [])
                .filter((e) => e.kind === "event" && !e.event.allDay)
                .map((entry) =>
                  entry.kind === "event" ? (
                    <button
                      key={`timed-${entry.event.guid}-${column}`}
                      type="button"
                      className="cal-timed"
                      style={placement(entry.event, day, column)}
                      onClick={() => {
                        setSelected(day);
                        setDraft(draftFrom(entry.event));
                      }}
                    >
                      <span
                        className="cal-timed-bar"
                        style={{
                          background: colours.get(entry.event.calendarGuid) || "var(--gold)",
                        }}
                      />
                      <span className="cal-timed-text">
                        <span className="cal-timed-title">{entry.event.title}</span>
                        <span className="cal-timed-when">{clockTime(entry.event.startsAt)}</span>
                      </span>
                    </button>
                  ) : null
                )
            )}
          </div>
        </div>
      )}

      <section
        className="cal-day-panel"
        aria-label={`${selected.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" })}`}
      >
        <header className="cal-day-head">
          <h2>
            {selected.toLocaleDateString(undefined, {
              weekday: "long",
              day: "numeric",
              month: "long",
            })}
          </h2>
          <button
            className="quiet"
            disabled={busy || defaultCalendar(calendars) === null}
            onClick={() => {
              const target = defaultCalendar(calendars)?.guid;
              if (target) setDraft(blankEvent(selected, target));
            }}
          >
            New event
          </button>
        </header>

        {draft ? (
          <form
            className="cal-form"
            onSubmit={(e) => {
              e.preventDefault();
              void saveDraft();
            }}
          >
            <label>
              <span>Title</span>
              <input
                value={draft.title}
                autoFocus
                onChange={(e) => setDraft({ ...draft, title: e.target.value })}
                placeholder="New event"
              />
            </label>
            <div className="cal-field">
              <span>Calendar</span>
              <Picker
                label="Calendar"
                value={draft.calendarGuid}
                onChange={(guid) => setDraft({ ...draft, calendarGuid: guid })}
                /* A subscribed feed or a calendar shared for reading only cannot take an
                   event, so it is not offered as somewhere to put one. The one the event is
                   already in stays listed either way, or moving it would look impossible. */
                options={calendars
                  .filter((c) => c.writable || c.guid === draft.calendarGuid)
                  .map((c) => ({ value: c.guid, label: c.title }))}
              />
            </div>
            <label>
              <span>Starts</span>
              <input
                type="datetime-local"
                value={toInputValue(draft.startsAt)}
                onChange={(e) => {
                  const startsAt = Math.floor(new Date(e.target.value).getTime() / 1000);
                  if (Number.isNaN(startsAt)) return;
                  // Dragging the start carries the end with it, keeping the length, which
                  // is what every calendar does and what nobody wants to redo by hand.
                  const length = Math.max(draft.endsAt - draft.startsAt, 0);
                  setDraft({ ...draft, startsAt, endsAt: startsAt + length });
                }}
              />
            </label>
            <label>
              <span>Ends</span>
              <input
                type="datetime-local"
                value={toInputValue(draft.endsAt)}
                onChange={(e) => {
                  const endsAt = Math.floor(new Date(e.target.value).getTime() / 1000);
                  if (!Number.isNaN(endsAt)) setDraft({ ...draft, endsAt });
                }}
              />
            </label>
            <label className="cal-check">
              <input
                type="checkbox"
                checked={draft.allDay}
                onChange={(e) => setDraft({ ...draft, allDay: e.target.checked })}
              />
              <span>All day</span>
            </label>
            <label>
              <span>Location</span>
              <input
                value={draft.location}
                onChange={(e) => setDraft({ ...draft, location: e.target.value })}
              />
            </label>
            <label>
              <span>Notes</span>
              <textarea
                rows={2}
                value={draft.notes}
                onChange={(e) => setDraft({ ...draft, notes: e.target.value })}
              />
            </label>

            <div className="cal-form-actions">
              <button type="submit" className="primary" disabled={busy || !draft.title.trim()}>
                {busy ? "Saving…" : draft.guid ? "Save" : "Add event"}
              </button>
              <button type="button" className="quiet" onClick={() => setDraft(null)}>
                Cancel
              </button>
              {draft.guid && (
                <button
                  type="button"
                  className="quiet danger"
                  disabled={busy}
                  onClick={() => void removeEvent(draft.guid as string)}
                >
                  Delete
                </button>
              )}
            </div>
          </form>
        ) : dayEntries.length === 0 ? (
          <p className="cal-empty">Nothing on this day.</p>
        ) : (
          <ul className="cal-day-list">
            {dayEntries.map((entry) =>
              entry.kind === "event" ? (
                <li key={`e${entry.event.guid}`}>
                  <button className="cal-day-item" onClick={() => setDraft(draftFrom(entry.event))}>
                    <span
                      className="chip-dot"
                      style={{ background: colours.get(entry.event.calendarGuid) || "var(--gold)" }}
                    />
                    <span className="cal-day-when">
                      {entry.event.allDay ? "All day" : clockTime(entry.event.startsAt)}
                    </span>
                    <span className="cal-day-title">{entry.event.title}</span>
                  </button>
                </li>
              ) : (
                <li key={`r${entry.reminder.guid}`}>
                  {/* A reminder belongs to Reminders; the calendar shows it, and sends you
                      there to change it. */}
                  <span className="cal-day-item is-reminder">
                    <span className="chip-ring" />
                    <span className="cal-day-when">
                      {clockTime(entry.reminder.dueAt ?? 0)}
                    </span>
                    <span className="cal-day-title">{entry.reminder.title}</span>
                    <span className="badge">Reminder</span>
                  </span>
                </li>
              )
            )}
          </ul>
        )}
      </section>

      {loaded && events.length === 0 && dueReminders.length === 0 && !error && (
        <p className="cal-empty">
          No events in this month. Quince keeps them on this computer once synced, so they
          open instantly and work offline.
        </p>
      )}
    </div>
  );
}
