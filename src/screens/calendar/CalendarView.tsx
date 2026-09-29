import { Fragment, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  api,
  defaultCalendar,
  messageOf,
  type Calendar,
  type CalendarEvent,
  type Reminder,
} from "../../lib/ipc";
import { useSync } from "../../lib/sync";
import { Picker } from "../../components/Picker";
import {
  addDays,
  clock,
  dateOf,
  dayKey,
  hourLabel,
  midnight,
  monthDays,
  placeDay,
  sameDay,
  seconds,
  snap,
  spansDays,
  startOfWeek,
  weekSpans,
} from "./model";

const ZONE = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";

/** Apple's four scopes, in Apple's order. */
const VIEWS = [
  { key: "day", label: "Day" },
  { key: "week", label: "Week" },
  { key: "month", label: "Month" },
  { key: "year", label: "Year" },
] as const;
type View = (typeof VIEWS)[number]["key"];

/** One hour of the Day and Week grids. */
const HOUR = 48;
/** A bar in the all-day row, and the gap under it. */
const LANE = 20;
/** What a month cell shows before it says "N more". */
const MONTH_ROWS = 4;

/** Calendars the viewer has switched off here, remembered on this computer. */
const HIDDEN_KEY = "quince.calendar.hidden";
const VIEW_KEY = "quince.calendar.view";

const readSet = (key: string) => {
  try {
    return new Set<string>(JSON.parse(window.localStorage.getItem(key) ?? "[]"));
  } catch {
    return new Set<string>();
  }
};

/** An event being written. A guid means it already exists at Apple. */
type Draft = {
  guid: string | null;
  calendarGuid: string;
  title: string;
  location: string;
  startsAt: number;
  endsAt: number;
  allDay: boolean;
  notes: string;
};

const draftOf = (e: CalendarEvent): Draft => ({
  guid: e.guid,
  calendarGuid: e.calendarGuid,
  title: e.title,
  location: e.location ?? "",
  startsAt: e.startsAt,
  endsAt: e.endsAt,
  allDay: e.allDay,
  notes: e.notes ?? "",
});

/** `YYYY-MM-DDTHH:mm` and `YYYY-MM-DD` for the date inputs, in local time. */
const pad = (n: number) => String(n).padStart(2, "0");
const toLocalInput = (s: number, dateOnly: boolean) => {
  const d = dateOf(s);
  const day = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  return dateOnly ? day : `${day}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
};
const fromLocalInput = (value: string) => seconds(new Date(value.length === 10 ? `${value}T00:00` : value));

const weekdayShort = (d: Date) => d.toLocaleDateString(undefined, { weekday: "short" });

/** The colour a calendar is drawn in, or Apple's blue when it has none. */
const tint = (c: Calendar | undefined) =>
  c?.colour && /^#[0-9a-f]{6}$/i.test(c.colour) ? c.colour : "var(--sys-blue)";

type Anchor = { x: number; y: number };

export function CalendarView() {
  const today = useMemo(() => new Date(), []);
  const [now, setNow] = useState(() => new Date());
  const [view, setViewState] = useState<View>(() => {
    try {
      const saved = window.localStorage.getItem(VIEW_KEY) as View | null;
      return saved && VIEWS.some((v) => v.key === saved) ? saved : "month";
    } catch {
      return "month";
    }
  });
  const setView = (v: View) => {
    setViewState(v);
    try {
      window.localStorage.setItem(VIEW_KEY, v);
    } catch {
      // Remembering the view is a convenience.
    }
  };
  /** The day everything is anchored on: the one selected, and what the views are built around. */
  const [selected, setSelected] = useState<Date>(() => midnight(today));
  const [events, setEvents] = useState<CalendarEvent[]>([]);
  const [due, setDue] = useState<Reminder[]>([]);
  const [calendars, setCalendars] = useState<Calendar[]>([]);
  const [hidden, setHidden] = useState<Set<string>>(() => readSet(HIDDEN_KEY));
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [anchor, setAnchor] = useState<Anchor | null>(null);
  /** The last event deleted, held so the deletion can be taken back. */
  const [undo, setUndo] = useState<CalendarEvent | null>(null);
  const { revision } = useSync();

  // The current-time line moves once a minute, on the minute — not on a busy timer.
  useEffect(() => {
    let timer = 0;
    const tick = () => {
      setNow(new Date());
      timer = window.setTimeout(tick, 60_000 - (Date.now() % 60_000));
    };
    timer = window.setTimeout(tick, 60_000 - (Date.now() % 60_000));
    return () => window.clearTimeout(timer);
  }, []);

  const week = useMemo(() => {
    const start = startOfWeek(selected);
    return Array.from({ length: 7 }, (_, i) => addDays(start, i));
  }, [selected]);
  const month = useMemo(() => monthDays(selected), [selected]);

  // Each view asks for exactly what it shows.
  const range = useMemo(() => {
    if (view === "day") return { from: seconds(selected), to: seconds(addDays(selected, 1)) };
    if (view === "week") return { from: seconds(week[0]), to: seconds(addDays(week[6], 1)) };
    if (view === "year")
      return {
        from: seconds(new Date(selected.getFullYear(), 0, 1)),
        to: seconds(new Date(selected.getFullYear() + 1, 0, 1)),
      };
    return { from: seconds(month[0]), to: seconds(addDays(month[41], 1)) };
  }, [view, selected, week, month]);

  const read = useCallback(async () => {
    const [found, cals, reminders] = await Promise.all([
      api.calendarEvents(range.from, range.to),
      api.calendarList(),
      api.remindersDueBetween(range.from, range.to),
    ]);
    setEvents(Array.isArray(found) ? found : []);
    setCalendars(Array.isArray(cals) ? cals : []);
    setDue(Array.isArray(reminders) ? reminders : []);
    setLoaded(true);
  }, [range]);

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

  // Beyond the months the shared refresh keeps warm, fetch what is being looked at — once,
  // and not for every month flicked past on the way.
  const fetched = useRef(new Set<string>());
  useEffect(() => {
    const from = toLocalInput(range.from, true);
    const to = toLocalInput(range.to - 1, true);
    const key = `${from}:${to}`;
    if (fetched.current.has(key)) return undefined;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      fetched.current.add(key);
      api
        .calendarSync(from, to, ZONE)
        .then(() => (cancelled ? undefined : read()))
        .catch((e) => {
          fetched.current.delete(key);
          if (!cancelled) setError(messageOf(e));
        });
    }, 350);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [range, read]);

  const byGuid = useMemo(() => new Map(calendars.map((c) => [c.guid, c])), [calendars]);
  const shown = useMemo(
    () => events.filter((e) => !hidden.has(e.calendarGuid)),
    [events, hidden]
  );
  const writable = calendars.filter((c) => c.writable);
  const target = defaultCalendar(calendars);

  const toggleCalendar = (guid: string) =>
    setHidden((was) => {
      const next = new Set(was);
      if (next.has(guid)) next.delete(guid);
      else next.add(guid);
      try {
        window.localStorage.setItem(HIDDEN_KEY, JSON.stringify([...next]));
      } catch {
        // Still hidden for now; forgotten on restart.
      }
      return next;
    });

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

  const save = useCallback(
    async (d: Draft) =>
      write(async () => {
        await api.eventSave({
          guid: d.guid,
          calendarGuid: d.calendarGuid,
          title: d.title.trim() || "New Event",
          location: d.location.trim() || null,
          startsAt: d.startsAt,
          endsAt: Math.max(d.endsAt, d.startsAt + (d.allDay ? 86_400 : 900)),
          allDay: d.allDay,
          timezone: ZONE,
          notes: d.notes.trim() || null,
          url: null,
        });
      }),
    [write]
  );

  /**
   * iCloud keeps no bin for calendar events: deleting one is final at Apple. So the deletion
   * can be taken back here, the way Undo works in Calendar — the event is written again.
   */
  const remove = useCallback(
    async (guid: string) => {
      const gone = events.find((e) => e.guid === guid) ?? null;
      if (await write(() => api.eventDelete(guid))) {
        setDraft(null);
        setUndo(gone);
      }
    },
    [events, write]
  );
  const takeBack = useCallback(async () => {
    const gone = undo;
    setUndo(null);
    if (gone) await save({ ...draftOf(gone), guid: null });
  }, [undo, save]);
  useEffect(() => {
    if (!undo) return undefined;
    const timer = window.setTimeout(() => setUndo(null), 8000);
    return () => window.clearTimeout(timer);
  }, [undo]);

  const openDraft = (d: Draft, at: Anchor) => {
    setDraft(d);
    setAnchor(at);
  };
  const newAt = (startsAt: number, endsAt: number, allDay: boolean, at: Anchor) => {
    if (!target) return;
    openDraft(
      { guid: null, calendarGuid: target.guid, title: "", location: "", startsAt, endsAt, allDay, notes: "" },
      at
    );
  };
  const pointAt = (el: Element | null): Anchor => {
    const r = el?.getBoundingClientRect();
    return r ? { x: r.right, y: r.top } : { x: window.innerWidth / 2, y: 120 };
  };

  /** "New event": the next whole hour on the selected day, for an hour — Apple's default. */
  const newEvent = (e: React.MouseEvent) => {
    const at = new Date(selected);
    at.setHours(now.getHours() + 1, 0, 0, 0);
    newAt(seconds(at), seconds(at) + 3600, false, pointAt(e.currentTarget));
  };

  const shift = (by: number) =>
    setSelected((d) => {
      if (view === "day") return addDays(d, by);
      if (view === "week") return addDays(d, by * 7);
      if (view === "month") return new Date(d.getFullYear(), d.getMonth() + by, 1);
      return new Date(d.getFullYear() + by, d.getMonth(), 1);
    });

  // Keyboard, as in Calendar: ⌘/Ctrl-arrows page, T goes to today, N makes an event.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = (e.target as HTMLElement | null)?.closest("input, textarea, select, [contenteditable]");
      if (typing || draft) return;
      if ((e.metaKey || e.ctrlKey) && e.key === "ArrowLeft") shift(-1);
      else if ((e.metaKey || e.ctrlKey) && e.key === "ArrowRight") shift(1);
      else if (e.key === "t" && !e.metaKey && !e.ctrlKey) setSelected(midnight(new Date()));
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const title = useMemo(() => {
    if (view === "year") return { strong: String(selected.getFullYear()), rest: "" };
    if (view === "day")
      return {
        strong: selected.toLocaleDateString(undefined, { day: "numeric", month: "long" }),
        rest: `${selected.getFullYear()}`,
      };
    const base = view === "week" ? week[6] : selected;
    return {
      strong: base.toLocaleDateString(undefined, { month: "long" }),
      rest: String(base.getFullYear()),
    };
  }, [view, selected, week]);

  /** A day's reminders and timed events for a month cell, in time order. */
  const dayItems = useCallback(
    (day: Date) => {
      const start = seconds(day);
      const end = seconds(addDays(day, 1));
      const timed = shown
        .filter((e) => !spansDays(e) && e.startsAt < end && e.endsAt > start)
        .map((event) => ({ kind: "event" as const, at: Math.max(event.startsAt, start), event }));
      const reminders = due
        .filter((r) => r.dueAt !== null && r.dueAt >= start && r.dueAt < end)
        .map((reminder) => ({ kind: "reminder" as const, at: reminder.dueAt ?? 0, reminder }));
      return [...timed, ...reminders].sort((a, b) => a.at - b.at);
    },
    [shown, due]
  );

  return (
    <div className="cx">
      <aside className="cx-side" aria-label="Calendars">
        <div className="cx-cals">
          {[
            { heading: "iCloud", list: calendars.filter((c) => c.writable) },
            { heading: "Other", list: calendars.filter((c) => !c.writable) },
          ]
            .filter((g) => g.list.length > 0)
            .map((group) => (
              <Fragment key={group.heading}>
                <p className="cx-side-heading">{group.heading}</p>
                {group.list.map((c) => (
                  <label className="cx-cal" key={c.guid}>
                    <input
                      type="checkbox"
                      checked={!hidden.has(c.guid)}
                      onChange={() => toggleCalendar(c.guid)}
                      style={{ "--cal": tint(c) } as React.CSSProperties}
                    />
                    <span className="cx-cal-name">{c.title}</span>
                  </label>
                ))}
              </Fragment>
            ))}
          {loaded && calendars.length === 0 && (
            <p className="cal-empty">There are no calendars on this account yet.</p>
          )}
        </div>
        <MiniMonth
          selected={selected}
          today={now}
          busy={(d) => dayItems(d).length > 0}
          onPick={(d) => setSelected(d)}
        />
      </aside>

      <section className="cx-main">
        <header className="cx-bar">
          <h1 className="cx-title">
            <strong>{title.strong}</strong> {title.rest}
          </h1>
          <div className="segmented" role="group" aria-label="View">
            {VIEWS.map((v) => (
              <button
                key={v.key}
                className="segment"
                aria-pressed={view === v.key}
                onClick={() => setView(v.key)}
              >
                {v.label}
              </button>
            ))}
          </div>
          <div className="cx-tools">
            {writable.length > 0 && (
              <button className="cx-icon" aria-label="New event" title="New Event" onClick={newEvent}>
                +
              </button>
            )}
            <span className="cx-step">
              <button className="cx-icon" aria-label="Previous" onClick={() => shift(-1)}>
                ‹
              </button>
              <button className="cx-today" onClick={() => setSelected(midnight(new Date()))}>
                Today
              </button>
              <button className="cx-icon" aria-label="Next" onClick={() => shift(1)}>
                ›
              </button>
            </span>
          </div>
        </header>

        {error && (
          <p className="cx-error" role="alert">
            {error}
          </p>
        )}

        {view === "month" && (
          <MonthView
            days={month}
            selected={selected}
            today={now}
            events={shown}
            calendars={byGuid}
            items={dayItems}
            onSelect={setSelected}
            onOpen={(e, at) => openDraft(draftOf(e), at)}
            onCreate={(day, at) => {
              const start = new Date(day);
              start.setHours(9, 0, 0, 0);
              newAt(seconds(start), seconds(start) + 3600, false, at);
            }}
            onMore={(day) => {
              setSelected(day);
              setView("day");
            }}
          />
        )}

        {(view === "week" || view === "day") && (
          <TimeGrid
            days={view === "week" ? week : [selected]}
            today={now}
            events={shown}
            reminders={due}
            calendars={byGuid}
            canWrite={(guid) => byGuid.get(guid)?.writable ?? false}
            onOpen={(e, at) => openDraft(draftOf(e), at)}
            onCreate={(startsAt, endsAt, allDay, at) => newAt(startsAt, endsAt, allDay, at)}
            onMove={(e, startsAt, endsAt) => void save({ ...draftOf(e), startsAt, endsAt })}
            onPickDay={(d) => {
              setSelected(d);
              setView("day");
            }}
          />
        )}

        {view === "year" && (
          <YearView
            year={selected.getFullYear()}
            today={now}
            busy={(d) => dayItems(d).length > 0}
            onPick={(d) => {
              setSelected(d);
              setView("day");
            }}
          />
        )}
      </section>

      {draft && anchor && (
        <EventPopover
          draft={draft}
          at={anchor}
          calendars={calendars}
          busy={busy}
          onChange={setDraft}
          onClose={() => setDraft(null)}
          onSave={async () => {
            if (await save(draft)) setDraft(null);
          }}
          onDelete={draft.guid ? () => void remove(draft.guid as string) : undefined}
        />
      )}

      {undo && (
        <div className="cx-toast" role="status">
          Deleted “{undo.title}”
          <button className="cx-toast-undo" onClick={() => void takeBack()}>
            Undo
          </button>
        </div>
      )}
    </div>
  );
}

/* ---- the mini month in the sidebar ------------------------------------------------ */

function MiniMonth({
  selected,
  today,
  busy,
  onPick,
}: {
  selected: Date;
  today: Date;
  busy: (d: Date) => boolean;
  onPick: (d: Date) => void;
}) {
  const [shown, setShown] = useState(() => new Date(selected.getFullYear(), selected.getMonth(), 1));
  useEffect(() => {
    setShown(new Date(selected.getFullYear(), selected.getMonth(), 1));
  }, [selected]);
  const days = monthDays(shown);
  return (
    <div className="cx-minimonth">
      <div className="cx-minimonth-head">
        <span>{shown.toLocaleDateString(undefined, { month: "long", year: "numeric" })}</span>
        <span>
          <button
            className="cx-icon small"
            aria-label="Previous month"
            onClick={() => setShown((m) => new Date(m.getFullYear(), m.getMonth() - 1, 1))}
          >
            ‹
          </button>
          <button
            className="cx-icon small"
            aria-label="Next month"
            onClick={() => setShown((m) => new Date(m.getFullYear(), m.getMonth() + 1, 1))}
          >
            ›
          </button>
        </span>
      </div>
      <div className="cx-minimonth-grid">
        {days.slice(0, 7).map((d) => (
          <span className="cx-minimonth-wd" key={`wd${d.getDay()}`} aria-hidden="true">
            {weekdayShort(d).slice(0, 1)}
          </span>
        ))}
        {days.map((d) => (
          <button
            key={dayKey(d)}
            className={`cx-minimonth-day${d.getMonth() !== shown.getMonth() ? " is-out" : ""}${
              sameDay(d, today) ? " is-today" : ""
            }${sameDay(d, selected) ? " is-selected" : ""}${busy(d) ? " is-busy" : ""}`}
            aria-label={d.toLocaleDateString(undefined, { dateStyle: "full" })}
            onClick={() => onPick(d)}
          >
            {d.getDate()}
          </button>
        ))}
      </div>
    </div>
  );
}

/* ---- month ------------------------------------------------------------------------ */

type Item =
  | { kind: "event"; at: number; event: CalendarEvent }
  | { kind: "reminder"; at: number; reminder: Reminder };

function MonthView({
  days,
  selected,
  today,
  events,
  calendars,
  items,
  onSelect,
  onOpen,
  onCreate,
  onMore,
}: {
  days: Date[];
  selected: Date;
  today: Date;
  events: CalendarEvent[];
  calendars: Map<string, Calendar>;
  items: (d: Date) => Item[];
  onSelect: (d: Date) => void;
  onOpen: (e: CalendarEvent, at: Anchor) => void;
  onCreate: (d: Date, at: Anchor) => void;
  onMore: (d: Date) => void;
}) {
  const cell = useRef<HTMLButtonElement | null>(null);
  const focused = useRef(false);
  const inMonth = selected.getMonth();
  useEffect(() => {
    if (focused.current) cell.current?.focus();
  }, [selected]);

  const onKey = (e: React.KeyboardEvent, day: Date) => {
    const by = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: 7, ArrowUp: -7 }[e.key];
    if (by !== undefined) {
      e.preventDefault();
      focused.current = true;
      onSelect(addDays(day, by));
    } else if (e.key === "Home") {
      e.preventDefault();
      onSelect(addDays(day, -day.getDay()));
    } else if (e.key === "End") {
      e.preventDefault();
      onSelect(addDays(day, 6 - day.getDay()));
    }
  };

  // How many lines a cell has room for, measured rather than assumed: a small window has
  // room for two, a large one for eight, and a fixed guess spilt items out of the cell.
  const grid = useRef<HTMLDivElement | null>(null);
  const [rows, setRows] = useState(MONTH_ROWS);
  useLayoutEffect(() => {
    const el = grid.current;
    if (!el) return undefined;
    const measure = () => {
      const week = el.firstElementChild as HTMLElement | null;
      const height = week?.getBoundingClientRect().height ?? 0;
      setRows(Math.max(1, Math.floor((height - 28) / 18)));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const weeks = Array.from({ length: 6 }, (_, w) => days.slice(w * 7, w * 7 + 7));
  return (
    <div className="cx-month">
      <div className="cx-weekdays" aria-hidden="true">
        {days.slice(0, 7).map((d) => (
          <span key={d.getDay()}>{weekdayShort(d)}</span>
        ))}
      </div>
      <div className="cal-grid" role="grid" aria-label="Month" ref={grid}>
        {weeks.map((row) => {
          const spans = weekSpans(events, row[0]);
          const lanes = Math.max(0, ...spans.map((s) => s.lane + 1));
          return (
            <div className="cx-week" role="row" key={dayKey(row[0])}>
              {row.map((day, i) => {
                const list = items(day);
                const lanesHere = spans.filter((s) => s.from <= i && s.to >= i).length;
                const room = Math.max(0, rows - Math.max(lanes, lanesHere));
                // What fits, and when it does not, one line fewer so "N more" has a line too.
                const visible = list.length > room ? list.slice(0, Math.max(0, room - 1)) : list;
                const more = list.length - visible.length;
                const isSelected = sameDay(day, selected);
                return (
                  <div
                    className={`cx-day${day.getMonth() !== inMonth ? " is-out" : ""}${
                      day.getDay() === 0 || day.getDay() === 6 ? " is-weekend" : ""
                    }${isSelected ? " is-selected" : ""}`}
                    role="gridcell"
                    aria-selected={isSelected}
                    key={dayKey(day)}
                  >
                    <button
                      ref={isSelected ? cell : undefined}
                      className="cx-day-hit"
                      tabIndex={isSelected ? 0 : -1}
                      aria-label={`${day.toLocaleDateString(undefined, { dateStyle: "full" })}${
                        list.length ? `, ${list.length} item${list.length === 1 ? "" : "s"}` : ""
                      }`}
                      onClick={() => onSelect(day)}
                      onDoubleClick={(e) => onCreate(day, { x: e.clientX, y: e.clientY })}
                      onKeyDown={(e) => onKey(e, day)}
                      onBlur={() => (focused.current = false)}
                    />
                    <span className={`cx-daynum${sameDay(day, today) ? " is-today" : ""}`}>
                      {day.getDate() === 1 && day.getMonth() !== inMonth
                        ? day.toLocaleDateString(undefined, { month: "short", day: "numeric" })
                        : day.getDate()}
                    </span>
                    <ul className="cx-items" style={{ marginTop: `${lanes * LANE}px` }}>
                      {visible.map((item) =>
                        item.kind === "event" ? (
                          <li key={item.event.guid}>
                            <button
                              className="cx-item"
                              style={{ "--cal": tint(calendars.get(item.event.calendarGuid)) } as React.CSSProperties}
                              onClick={(e) => onOpen(item.event, { x: e.clientX, y: e.clientY })}
                            >
                              <span className="cx-dot" aria-hidden="true" />
                              <span className="cx-item-title">{item.event.title}</span>
                              <span className="cx-item-time">{clock(item.event.startsAt)}</span>
                            </button>
                          </li>
                        ) : (
                          <li key={item.reminder.guid}>
                            <span className={`cx-item is-reminder${item.reminder.completedAt ? " is-done" : ""}`}>
                              <span className="cx-ring" aria-hidden="true" />
                              <span className="cx-item-title">{item.reminder.title}</span>
                              <span className="cx-item-time">{clock(item.at)}</span>
                            </span>
                          </li>
                        )
                      )}
                      {more > 0 && (
                        <li>
                          <button className="cx-more" onClick={() => onMore(day)}>
                            {more} more
                          </button>
                        </li>
                      )}
                    </ul>
                  </div>
                );
              })}
              {spans.map((s) => (
                <button
                  key={`${s.event.guid}-${dayKey(row[0])}`}
                  className={`cx-bar-event${s.continuesLeft ? " from-before" : ""}${
                    s.continuesRight ? " to-after" : ""
                  }`}
                  style={
                    {
                      "--cal": tint(calendars.get(s.event.calendarGuid)),
                      left: `calc(${(s.from / 7) * 100}% + 3px)`,
                      width: `calc(${((s.to - s.from + 1) / 7) * 100}% - 6px)`,
                      top: `${26 + s.lane * LANE}px`,
                    } as React.CSSProperties
                  }
                  onClick={(e) => onOpen(s.event, { x: e.clientX, y: e.clientY })}
                >
                  {s.event.title}
                </button>
              ))}
            </div>
          );
        })}
      </div>
    </div>
  );
}

/* ---- week and day ------------------------------------------------------------------- */

type Drag =
  | { kind: "create"; day: Date; column: number; from: number; to: number }
  | { kind: "move"; event: CalendarEvent; startX: number; startY: number; dx: number; dMinutes: number };

function TimeGrid({
  days,
  today,
  events,
  reminders,
  calendars,
  canWrite,
  onOpen,
  onCreate,
  onMove,
  onPickDay,
}: {
  days: Date[];
  today: Date;
  events: CalendarEvent[];
  reminders: Reminder[];
  calendars: Map<string, Calendar>;
  canWrite: (calendarGuid: string) => boolean;
  onOpen: (e: CalendarEvent, at: Anchor) => void;
  onCreate: (startsAt: number, endsAt: number, allDay: boolean, at: Anchor) => void;
  onMove: (e: CalendarEvent, startsAt: number, endsAt: number) => void;
  onPickDay: (d: Date) => void;
}) {
  const scroller = useRef<HTMLDivElement | null>(null);
  const columnsEl = useRef<HTMLDivElement | null>(null);
  const [drag, setDrag] = useState<Drag | null>(null);
  const spans = weekSpans(events, days[0]).filter((s) => s.from < days.length);
  const lanes = Math.max(1, ...spans.map((s) => s.lane + 1));

  // Opened, it shows the working day rather than midnight — as Calendar does.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const showsToday = days.some((d) => sameDay(d, new Date()));
    const hour = showsToday ? Math.max(0, new Date().getHours() - 2) : 7;
    el.scrollTop = hour * HOUR;
    // Only when the days change, never under someone's scrolling.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [days.map((d) => dayKey(d)).join()]);

  /** Minutes since midnight under the pointer, in the column it is over. */
  const where = (clientX: number, clientY: number) => {
    const el = columnsEl.current;
    if (!el) return null;
    const r = el.getBoundingClientRect();
    const column = Math.min(days.length - 1, Math.max(0, Math.floor(((clientX - r.left) / r.width) * days.length)));
    const minutes = Math.min(24 * 60, Math.max(0, ((clientY - r.top) / HOUR) * 60));
    return { column, minutes };
  };

  const onColumnDown = (e: React.PointerEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest(".cx-block")) return;
    const at = where(e.clientX, e.clientY);
    if (!at) return;
    const from = snap(at.minutes) - (snap(at.minutes) === 24 * 60 ? 15 : 0);
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    setDrag({ kind: "create", day: days[at.column], column: at.column, from, to: from + 15 });
  };

  const onPointerMove = (e: React.PointerEvent) => {
    if (!drag) return;
    const at = where(e.clientX, e.clientY);
    if (!at) return;
    if (drag.kind === "create") {
      setDrag({ ...drag, to: Math.max(drag.from + 15, snap(at.minutes)) });
    } else {
      const el = columnsEl.current;
      const width = el ? el.getBoundingClientRect().width / days.length : 1;
      setDrag({
        ...drag,
        dx: Math.round((e.clientX - drag.startX) / width),
        dMinutes: snap(((e.clientY - drag.startY) / HOUR) * 60),
      });
    }
  };

  const onPointerUp = (e: React.PointerEvent) => {
    if (!drag) return;
    const done = drag;
    setDrag(null);
    if (done.kind === "create") {
      const base = seconds(done.day);
      // A click without a drag is an hour, like double-clicking in Calendar.
      const to = done.to - done.from <= 15 ? done.from + 60 : done.to;
      onCreate(base + done.from * 60, base + to * 60, false, { x: e.clientX, y: e.clientY });
    } else if (done.dx !== 0 || done.dMinutes !== 0) {
      const shift = done.dMinutes * 60;
      const moved = (s: number) => {
        const d = dateOf(s + shift);
        return seconds(new Date(d.getFullYear(), d.getMonth(), d.getDate() + done.dx, d.getHours(), d.getMinutes()));
      };
      onMove(done.event, moved(done.event.startsAt), moved(done.event.endsAt));
    } else {
      onOpen(done.event, { x: e.clientX, y: e.clientY });
    }
  };

  const nowMinutes = today.getHours() * 60 + today.getMinutes();
  const todayColumn = days.findIndex((d) => sameDay(d, today));

  return (
    <div className={`cal-hours${days.length === 1 ? " is-day" : ""}`}>
      <div className="cx-grid-head" style={{ "--cols": days.length } as React.CSSProperties}>
        <span className="cx-gutter" aria-hidden="true" />
        {days.map((d) => (
          <button
            className={`cal-col-head${sameDay(d, today) ? " is-today" : ""}`}
            key={dayKey(d)}
            onClick={() => onPickDay(d)}
            aria-label={d.toLocaleDateString(undefined, { dateStyle: "full" })}
          >
            <span className="cx-col-weekday">{weekdayShort(d)}</span>
            <span className="cx-col-num">{d.getDate()}</span>
          </button>
        ))}
      </div>

      <div className="cx-allday" style={{ "--cols": days.length, height: `${lanes * LANE + 8}px` } as React.CSSProperties}>
        <span className="cx-gutter cx-allday-label">all-day</span>
        <div className="cx-allday-lanes">
          {spans.map((s) => (
            <button
              key={s.event.guid}
              className={`cx-bar-event${s.continuesLeft ? " from-before" : ""}${s.continuesRight ? " to-after" : ""}`}
              style={
                {
                  "--cal": tint(calendars.get(s.event.calendarGuid)),
                  left: `calc(${(s.from / days.length) * 100}% + 2px)`,
                  width: `calc(${((Math.min(s.to, days.length - 1) - s.from + 1) / days.length) * 100}% - 4px)`,
                  top: `${4 + s.lane * LANE}px`,
                } as React.CSSProperties
              }
              onClick={(e) => onOpen(s.event, { x: e.clientX, y: e.clientY })}
            >
              {s.event.title}
            </button>
          ))}
        </div>
      </div>

      <div className="cx-scroll" ref={scroller}>
        <div className="cx-hours-body" style={{ height: `${24 * HOUR}px` }}>
          <div className="cx-gutter cx-hour-labels" aria-hidden="true">
            {Array.from({ length: 24 }, (_, h) =>
              h === 0 ? null : (
                <span key={h} style={{ top: `${h * HOUR}px` }}>
                  {hourLabel(h)}
                </span>
              )
            )}
            {todayColumn >= 0 && (
              <span className="cx-now-label" style={{ top: `${(nowMinutes / 60) * HOUR}px` }}>
                {clock(seconds(today))}
              </span>
            )}
          </div>
          <div
            className="cx-columns"
            ref={columnsEl}
            style={{ "--cols": days.length } as React.CSSProperties}
            onPointerDown={onColumnDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onPointerCancel={() => setDrag(null)}
          >
            {days.map((d, column) => (
              <div className={`cx-column${d.getDay() === 0 || d.getDay() === 6 ? " is-weekend" : ""}`} key={dayKey(d)}>
                {placeDay(events, d).map((p) => {
                  const moving = drag?.kind === "move" && drag.event.guid === p.event.guid ? drag : null;
                  const c = calendars.get(p.event.calendarGuid);
                  return (
                    <button
                      key={p.event.guid}
                      className={`cx-block${moving ? " is-moving" : ""}`}
                      style={
                        {
                          "--cal": tint(c),
                          top: `${((p.top + (moving?.dMinutes ?? 0)) / 60) * HOUR}px`,
                          height: `${Math.max((p.length / 60) * HOUR - 2, 18)}px`,
                          left: `calc(${(p.column / p.columns) * 100}% + ${(moving?.dx ?? 0) * 100}%)`,
                          width: `calc(${100 / p.columns}% - 3px)`,
                        } as React.CSSProperties
                      }
                      onPointerDown={(e) => {
                        if (e.button !== 0) return;
                        e.stopPropagation();
                        (e.currentTarget.closest(".cx-columns") as HTMLElement | null)?.setPointerCapture(e.pointerId);
                        if (canWrite(p.event.calendarGuid)) {
                          setDrag({ kind: "move", event: p.event, startX: e.clientX, startY: e.clientY, dx: 0, dMinutes: 0 });
                        } else {
                          onOpen(p.event, { x: e.clientX, y: e.clientY });
                        }
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          onOpen(p.event, pointOf(e.currentTarget));
                        }
                      }}
                    >
                      <span className="cx-block-title">{p.event.title}</span>
                      <span className="cx-block-time">{clock(p.event.startsAt)}</span>
                      {p.event.location && <span className="cx-block-where">{p.event.location}</span>}
                    </button>
                  );
                })}
                {reminders
                  .filter((r) => r.dueAt !== null && sameDay(dateOf(r.dueAt), d))
                  .map((r) => (
                    <span
                      key={r.guid}
                      className={`cx-reminder${r.completedAt ? " is-done" : ""}`}
                      style={{ top: `${(((dateOf(r.dueAt ?? 0).getHours() * 60 + dateOf(r.dueAt ?? 0).getMinutes()) / 60) * HOUR)}px` }}
                    >
                      <span className="cx-ring" aria-hidden="true" />
                      {r.title}
                    </span>
                  ))}
                {drag?.kind === "create" && drag.column === column && (
                  <span
                    className="cx-block is-draft"
                    style={{ top: `${(drag.from / 60) * HOUR}px`, height: `${((drag.to - drag.from) / 60) * HOUR}px` }}
                  >
                    <span className="cx-block-title">New Event</span>
                  </span>
                )}
              </div>
            ))}
            {todayColumn >= 0 && (
              <span
                className="cx-now"
                aria-hidden="true"
                style={{
                  top: `${(nowMinutes / 60) * HOUR}px`,
                  left: `${(todayColumn / days.length) * 100}%`,
                  width: `${100 / days.length}%`,
                }}
              />
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

const pointOf = (el: Element): Anchor => {
  const r = el.getBoundingClientRect();
  return { x: r.right, y: r.top };
};

/* ---- year --------------------------------------------------------------------------- */

function YearView({
  year,
  today,
  busy,
  onPick,
}: {
  year: number;
  today: Date;
  busy: (d: Date) => boolean;
  onPick: (d: Date) => void;
}) {
  return (
    <div className="cal-year" role="grid" aria-label={`${year}`}>
      {Array.from({ length: 12 }, (_, m) => {
        const first = new Date(year, m, 1);
        const days = monthDays(first);
        const current = today.getFullYear() === year && today.getMonth() === m;
        return (
          <section className="cal-mini" key={m} aria-label={first.toLocaleDateString(undefined, { month: "long" })}>
            <h2 className={`cx-year-month${current ? " is-current" : ""}`}>
              {first.toLocaleDateString(undefined, { month: "long" })}
            </h2>
            <div className="cx-minimonth-grid">
              {days.slice(0, 7).map((d) => (
                <span className="cx-minimonth-wd" key={`wd${d.getDay()}`} aria-hidden="true">
                  {weekdayShort(d).slice(0, 1)}
                </span>
              ))}
              {days.map((d) =>
                d.getMonth() !== m ? (
                  <span key={dayKey(d)} className="cx-minimonth-day is-blank" aria-hidden="true" />
                ) : (
                  <button
                    key={dayKey(d)}
                    className={`cx-minimonth-day${sameDay(d, today) ? " is-today" : ""}${busy(d) ? " is-busy" : ""}`}
                    aria-label={d.toLocaleDateString(undefined, { dateStyle: "full" })}
                    onClick={() => onPick(d)}
                  >
                    {d.getDate()}
                  </button>
                )
              )}
            </div>
          </section>
        );
      })}
    </div>
  );
}

/* ---- the event popover ---------------------------------------------------------------- */

function EventPopover({
  draft,
  at,
  calendars,
  busy,
  onChange,
  onClose,
  onSave,
  onDelete,
}: {
  draft: Draft;
  at: Anchor;
  calendars: Calendar[];
  busy: boolean;
  onChange: (d: Draft) => void;
  onClose: () => void;
  onSave: () => void;
  onDelete?: () => void;
}) {
  const box = useRef<HTMLFormElement | null>(null);
  const [pos, setPos] = useState<{ left: number; top: number }>({ left: at.x, top: at.y });
  const calendar = calendars.find((c) => c.guid === draft.calendarGuid);
  const editable = !draft.guid || (calendar?.writable ?? false);
  const choices = calendars.filter((c) => c.writable || c.guid === draft.calendarGuid);

  // Beside what was clicked, and always wholly on screen.
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const gap = 12;
    let left = at.x + gap;
    if (left + width > window.innerWidth - 8) left = Math.max(8, at.x - width - gap);
    const top = Math.min(Math.max(8, at.y - 24), window.innerHeight - height - 8);
    setPos({ left, top });
  }, [at]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    const onDown = (e: MouseEvent) => {
      if (box.current && !box.current.contains(e.target as Node)) onClose();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onDown);
    };
  }, [onClose]);

  const set = (patch: Partial<Draft>) => onChange({ ...draft, ...patch });
  return (
    <form
      ref={box}
      className="cx-popover"
      role="dialog"
      aria-label={draft.guid ? draft.title || "Event" : "New event"}
      style={{ left: pos.left, top: pos.top, "--cal": tint(calendar) } as React.CSSProperties}
      onSubmit={(e) => {
        e.preventDefault();
        if (editable) onSave();
      }}
    >
      <input
        className="cx-pop-title"
        value={draft.title}
        placeholder="New Event"
        aria-label="Title"
        autoFocus
        readOnly={!editable}
        onChange={(e) => set({ title: e.target.value })}
      />
      <input
        className="cx-pop-line"
        value={draft.location}
        placeholder="Add Location"
        aria-label="Location"
        readOnly={!editable}
        onChange={(e) => set({ location: e.target.value })}
      />
      <div className="cx-pop-rows">
        <label className="cx-pop-row">
          <span>all-day</span>
          <input
            type="checkbox"
            checked={draft.allDay}
            disabled={!editable}
            onChange={(e) => {
              const allDay = e.target.checked;
              const day = midnight(dateOf(draft.startsAt));
              set(
                allDay
                  ? { allDay, startsAt: seconds(day), endsAt: seconds(addDays(day, 1)) }
                  : { allDay, startsAt: seconds(day) + 9 * 3600, endsAt: seconds(day) + 10 * 3600 }
              );
            }}
          />
        </label>
        <label className="cx-pop-row">
          <span>starts</span>
          <input
            type={draft.allDay ? "date" : "datetime-local"}
            value={toLocalInput(draft.startsAt, draft.allDay)}
            readOnly={!editable}
            onChange={(e) => {
              if (!e.target.value) return;
              const startsAt = fromLocalInput(e.target.value);
              // Moving the start keeps the length, as Calendar does.
              set({ startsAt, endsAt: startsAt + (draft.endsAt - draft.startsAt) });
            }}
          />
        </label>
        <label className="cx-pop-row">
          <span>ends</span>
          <input
            type={draft.allDay ? "date" : "datetime-local"}
            value={toLocalInput(draft.allDay ? draft.endsAt - 1 : draft.endsAt, draft.allDay)}
            readOnly={!editable}
            onChange={(e) => {
              if (!e.target.value) return;
              const end = fromLocalInput(e.target.value);
              set({ endsAt: draft.allDay ? seconds(addDays(dateOf(end), 1)) : end });
            }}
          />
        </label>
        <div className="cx-pop-row cal-field">
          <span>calendar</span>
          <Picker
            label="Calendar"
            value={draft.calendarGuid}
            disabled={!editable || choices.length < 2}
            onChange={(guid) => set({ calendarGuid: guid })}
            options={choices.map((c) => ({ value: c.guid, label: c.title }))}
          />
        </div>
      </div>
      <textarea
        className="cx-pop-notes"
        value={draft.notes}
        placeholder="Add Notes"
        aria-label="Notes"
        rows={2}
        readOnly={!editable}
        onChange={(e) => set({ notes: e.target.value })}
      />
      <div className="cx-pop-actions">
        {onDelete && editable ? (
          <button type="button" className="cx-pop-delete" onClick={onDelete} disabled={busy}>
            Delete
          </button>
        ) : (
          <span />
        )}
        <span>
          <button type="button" className="quiet" onClick={onClose}>
            {editable ? "Cancel" : "Close"}
          </button>
          {editable && (
            <button type="submit" className="primary" disabled={busy}>
              {draft.guid ? "Save" : "Add"}
            </button>
          )}
        </span>
      </div>
      {!editable && <p className="cx-pop-note">This calendar can be read here but not changed.</p>}
    </form>
  );
}
