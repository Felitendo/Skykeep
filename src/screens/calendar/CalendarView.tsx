import { Notice } from "../../components/Notice";
import { Search } from "lucide-react";
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
  placeDayWith,
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
/** Narrower than this, a month cell draws its items as dots. */
const DOT_CELL = 72;
/** The most dots one row of a narrow cell holds; past that the day is simply busy. */
const MOST_DOTS = 4;

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

/** Where a popover opens: to the right of `x`, or to the left of `left` (or `x`) when the
 *  window has no room on the right. */
type Anchor = { x: number; y: number; left?: number };

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
  /** A found event: its day, in Day view, and the search put away. */
  const openFound = (e: CalendarEvent) => {
    setSelected(midnight(dateOf(e.startsAt)));
    setView("day");
    setQuery("");
  };

  /** The day everything is anchored on: the one selected, and what the views are built around. */
  const [selected, setSelected] = useState<Date>(() => midnight(today));
  // Search, as Calendar's toolbar field: every word, any calendar, soonest first.
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<CalendarEvent[]>([]);
  const searchField = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    const q = query.trim();
    if (!q) return setFound([]);
    let cancelled = false;
    const timer = window.setTimeout(() => {
      api.calendarSearch(q).then((r) => !cancelled && setFound(r)).catch(() => !cancelled && setFound([]));
    }, 120);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [query]);
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
    // What is done is not on the schedule any more; Calendar hides completed reminders.
    setDue(Array.isArray(reminders) ? reminders.filter((r) => !r.completedAt) : []);
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

  /** "New event": the next whole hour on the selected day, for an hour — Apple's default. */
  const newEvent = (e?: React.MouseEvent) => {
    const at = new Date(selected);
    at.setHours(now.getHours() + 1, 0, 0, 0);
    const button = (e?.currentTarget ?? document.querySelector('[aria-label="New event"]'))?.getBoundingClientRect()
      ?? new DOMRect(window.innerWidth - 40, 40, 0, 0);
    // Under the button, never over the toolbar; in Day and Week it moves beside the new event
    // once that is drawn.
    newAt(seconds(at), seconds(at) + 3600, false, { x: button.right, y: button.bottom + 32 });
    setFollowDraft(true);
  };
  const [followDraft, setFollowDraft] = useState(false);
  useLayoutEffect(() => {
    if (!followDraft) return;
    setFollowDraft(false);
    const block = document.querySelector(".cx-block.is-draft");
    if (!block) return;
    block.scrollIntoView({ block: "nearest" });
    const r = block.getBoundingClientRect();
    setAnchor({ x: r.right, y: r.top, left: r.left });
  }, [followDraft]);

  const shift = (by: number) =>
    setSelected((d) => {
      if (view === "day") return addDays(d, by);
      if (view === "week") return addDays(d, by * 7);
      if (view === "month") return new Date(d.getFullYear(), d.getMonth() + by, 1);
      return new Date(d.getFullYear() + by, d.getMonth(), 1);
    });

  // Keyboard, as in Calendar on a Mac (Ctrl here): arrows page, T today, 1–4 the views, N an event.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = (e.target as HTMLElement | null)?.closest("input, textarea, select, [contenteditable]");
      // Find, as everywhere on a Mac: ⌘F (Ctrl+F here) goes to the search field.
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        searchField.current?.focus();
        return;
      }
      if (typing || draft) return;
      const command = (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey;
      const k = e.key.toLowerCase();
      if (command && e.key === "ArrowLeft") shift(-1);
      else if (command && e.key === "ArrowRight") shift(1);
      // ⌘T, and the bare T this screen always had.
      else if (k === "t" && !e.altKey && !e.shiftKey) setSelected(midnight(new Date()));
      // ⌘1–⌘4: Day, Week, Month, Year, in the order the switch shows them.
      else if (command && /^[1-4]$/.test(e.key)) setView(VIEWS[Number(e.key) - 1].key);
      else if (command && k === "n") newEvent();
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  /** The title, and the shorter one a narrow calendar shows in its place ("Sep 2026"). */
  const title = useMemo(() => {
    const short = (d: Date) => d.toLocaleDateString(undefined, { month: "short" });
    if (view === "year") {
      const year = String(selected.getFullYear());
      return { strong: year, rest: "", brief: year };
    }
    if (view === "day")
      return {
        strong: selected.toLocaleDateString(undefined, { day: "numeric", month: "long" }),
        rest: `${selected.getFullYear()}`,
        brief: selected.toLocaleDateString(undefined, { day: "numeric", month: "short" }),
      };
    // A week across two months is named for both, as Calendar does: "Sep – Oct 2026".
    if (view === "week" && week[0].getMonth() !== week[6].getMonth()) {
      const sameYear = week[0].getFullYear() === week[6].getFullYear();
      const strong = `${short(week[0])}${sameYear ? "" : ` ${week[0].getFullYear()}`} – ${short(week[6])}`;
      return { strong, rest: String(week[6].getFullYear()), brief: `${strong} ${week[6].getFullYear()}` };
    }
    const base = view === "week" ? week[6] : selected;
    return {
      strong: base.toLocaleDateString(undefined, { month: "long" }),
      rest: String(base.getFullYear()),
      brief: `${short(base)} ${base.getFullYear()}`,
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
          <h1 className="cx-title" aria-label={`${title.strong} ${title.rest}`.trim()}>
            <span className="cx-title-full" aria-hidden="true">
              <strong>{title.strong}</strong> {title.rest}
            </span>
            <strong className="cx-title-brief" aria-hidden="true">
              {title.brief}
            </strong>
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
            <label className="cx-search">
              <Search size={13} strokeWidth={2.25} aria-hidden="true" />
              <input
                ref={searchField}
                type="search"
                placeholder="Search"
                aria-label="Search events"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setQuery("");
                  if (e.key === "Enter" && found[0]) openFound(found[0]);
                }}
              />
            </label>
          </div>
        </header>

        {query.trim() && (
          <div className="cx-found" role="listbox" aria-label="Events found">
            {found.length === 0 ? (
              <p className="cx-found-none">No events match “{query.trim()}”.</p>
            ) : (
              found.map((e) => (
                <button
                  key={e.guid}
                  className="cx-found-row"
                  role="option"
                  aria-selected={false}
                  style={{ "--cal": tint(byGuid.get(e.calendarGuid)) } as React.CSSProperties}
                  onClick={() => openFound(e)}
                >
                  <span className="cx-dot" aria-hidden="true" />
                  <span className="cx-found-title">{e.title}</span>
                  <span className="cx-found-when">
                    {dateOf(e.startsAt).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short", year: "numeric" })}
                    {e.allDay ? "" : ` · ${clock(e.startsAt)}`}
                  </span>
                </button>
              ))
            )}
          </div>
        )}

        {error && (
          <Notice message={error} onDismiss={() => setError(null)} />
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
          <div className={view === "day" ? "cx-dayview" : "cx-weekview"}>
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
            pending={
              draft && !draft.guid && !draft.allDay
                ? { startsAt: draft.startsAt, endsAt: draft.endsAt, title: draft.title }
                : null
            }
          />
          {view === "day" && (
            <DayPane
              day={selected}
              events={shown}
              reminders={due}
              calendars={byGuid}
              onOpen={(e, at) => openDraft(draftOf(e), at)}
            />
          )}
          </div>
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
  // A cell too narrow for any title shows its day's items as dots, as Calendar does on a
  // small screen; a title cut to two letters and "3 more" squeezed below it said nothing.
  const [dots, setDots] = useState(false);
  useLayoutEffect(() => {
    const el = grid.current;
    if (!el) return undefined;
    const measure = () => {
      const week = el.firstElementChild as HTMLElement | null;
      const height = week?.getBoundingClientRect().height ?? 0;
      const width = (week?.firstElementChild as HTMLElement | null)?.getBoundingClientRect().width ?? 0;
      setRows(Math.max(1, Math.floor((height - 28) / 18)));
      setDots(width > 0 && width < DOT_CELL);
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
          return (
            <div className="cx-week" role="row" key={dayKey(row[0])}>
              {row.map((day, i) => {
                const list = items(day);
                // Only the lanes of the bars that cross this day are kept clear in it: a bar
                // at the end of the week had left the days before it room for nothing.
                const lanesHere = Math.max(0, ...spans.filter((s) => s.from <= i && s.to >= i).map((s) => s.lane + 1));
                const room = Math.max(0, rows - lanesHere);
                // What fits, and when it does not, one line fewer so "N more" has a line too.
                // As dots, a row of them fits in any cell.
                const visible = dots
                  ? list.slice(0, MOST_DOTS)
                  : list.length > room
                    ? list.slice(0, Math.max(0, room - 1))
                    : list;
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
                    <ul className={`cx-items${dots ? " is-dots" : ""}`} style={{ marginTop: `${lanesHere * LANE}px` }}>
                      {visible.map((item) =>
                        item.kind === "event" ? (
                          <li key={item.event.guid}>
                            <button
                              className="cx-item"
                              style={{ "--cal": tint(calendars.get(item.event.calendarGuid)) } as React.CSSProperties}
                              // Named in full, for the dot that is all a narrow cell shows.
                              aria-label={`${item.event.title}, ${clock(item.event.startsAt)}`}
                              title={dots ? `${item.event.title} · ${clock(item.event.startsAt)}` : undefined}
                              onClick={(e) => onOpen(item.event, { x: e.clientX, y: e.clientY })}
                            >
                              <span className="cx-dot" aria-hidden="true" />
                              <span className="cx-item-title">{item.event.title}</span>
                              <span className="cx-item-time">{clock(item.event.startsAt)}</span>
                            </button>
                          </li>
                        ) : (
                          <li key={item.reminder.guid}>
                            <span
                              className={`cx-item is-reminder${item.reminder.completedAt ? " is-done" : ""}`}
                              title={dots ? `${item.reminder.title} · ${clock(item.at)}` : undefined}
                            >
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
  | { kind: "move"; event: CalendarEvent; startX: number; startY: number; dx: number; dMinutes: number }
  /** Dragging an event's bottom edge: its end moves, its start stays. */
  | { kind: "resize"; event: CalendarEvent; startY: number; dMinutes: number };

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
  pending,
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
  /** A new event being named in the popover, held on the grid where it was made. */
  pending?: { startsAt: number; endsAt: number; title: string } | null;
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
    // A little above the hour, so its label — centred on the line — shows whole.
    el.scrollTop = Math.max(0, hour * HOUR - 12);
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

  /** Beside the day column, not under the pointer: the popover must not cover the event. */
  const besideColumn = (column: number, y: number): Anchor => {
    const r = columnsEl.current?.getBoundingClientRect();
    if (!r) return { x: 0, y };
    const width = r.width / days.length;
    return { x: r.left + (column + 1) * width, y, left: r.left + column * width } as Anchor;
  };

  /*
   * A drag is followed from the window, not from the grid, and it ends on whichever comes
   * first: the button's release, a move with no button held, the window losing focus, or
   * Escape. In the app's WebKitGTK the release did not always reach the grid — a click left a
   * sliver of "New Event" behind that then grew with every move of the mouse — so a release
   * that never arrives is inferred from the next move, where the buttons say it is up.
   */
  const dragRef = useRef<Drag | null>(null);
  const setDragNow = (d: Drag | null) => {
    dragRef.current = d;
    setDrag(d);
  };

  const follow = (clientX: number, clientY: number) => {
    const current = dragRef.current;
    const at = where(clientX, clientY);
    if (!current || !at) return;
    if (current.kind === "create") {
      setDragNow({ ...current, to: Math.max(current.from + 15, snap(at.minutes)) });
    } else if (current.kind === "resize") {
      // Never shorter than a quarter hour, as Calendar holds it.
      const length = (current.event.endsAt - current.event.startsAt) / 60;
      setDragNow({ ...current, dMinutes: Math.max(15 - length, snap(((clientY - current.startY) / HOUR) * 60)) });
    } else {
      const el = columnsEl.current;
      const width = el ? el.getBoundingClientRect().width / days.length : 1;
      setDragNow({
        ...current,
        dx: Math.round((clientX - current.startX) / width),
        dMinutes: snap(((clientY - current.startY) / HOUR) * 60),
      });
    }
  };

  const finish = (clientX: number, clientY: number) => {
    const done = dragRef.current;
    if (!done) return;
    setDragNow(null);
    if (done.kind === "create") {
      // A click on empty time does nothing, as in Calendar — only a drag or a double-click
      // makes an event, so looking around the week never opens a form by accident.
      if (done.to - done.from > 15) {
        const base = seconds(done.day);
        onCreate(base + done.from * 60, base + done.to * 60, false, besideColumn(done.column, clientY));
      }
    } else if (done.kind === "resize") {
      if (done.dMinutes !== 0) onMove(done.event, done.event.startsAt, done.event.endsAt + done.dMinutes * 60);
    } else if (done.dx !== 0 || done.dMinutes !== 0) {
      const shift = done.dMinutes * 60;
      const moved = (t: number) => {
        const d = dateOf(t + shift);
        return seconds(new Date(d.getFullYear(), d.getMonth(), d.getDate() + done.dx, d.getHours(), d.getMinutes()));
      };
      onMove(done.event, moved(done.event.startsAt), moved(done.event.endsAt));
    } else {
      onOpen(done.event, { x: clientX, y: clientY });
    }
  };

  // The newest handlers, for listeners attached once per drag.
  const handlers = useRef({ follow, finish });
  handlers.current = { follow, finish };
  const dragging = drag !== null;
  useEffect(() => {
    if (!dragging) return undefined;
    const move = (e: PointerEvent) => {
      if (e.buttons === 0) handlers.current.finish(e.clientX, e.clientY);
      else handlers.current.follow(e.clientX, e.clientY);
    };
    const up = (e: PointerEvent | MouseEvent) => handlers.current.finish(e.clientX, e.clientY);
    const drop = () => setDragNow(null);
    const key = (e: KeyboardEvent) => e.key === "Escape" && drop();
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("mouseup", up);
    window.addEventListener("pointercancel", drop);
    window.addEventListener("blur", drop);
    window.addEventListener("keydown", key);
    // A native drag, if one starts anyway, takes the pointer until it ends: the drag here
    // ends with it rather than waiting for a release that will not come.
    window.addEventListener("dragstart", drop);
    window.addEventListener("dragend", drop);
    return () => {
      window.removeEventListener("dragstart", drop);
      window.removeEventListener("dragend", drop);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("mouseup", up);
      window.removeEventListener("pointercancel", drop);
      window.removeEventListener("blur", drop);
      window.removeEventListener("keydown", key);
    };
  }, [dragging]);

  const onColumnDown = (e: React.PointerEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest(".cx-block")) return;
    const at = where(e.clientX, e.clientY);
    if (!at) return;
    // No text selection or native drag while making an event.
    e.preventDefault();
    const from = snap(at.minutes) - (snap(at.minutes) === 24 * 60 ? 15 : 0);
    setDragNow({ kind: "create", day: days[at.column], column: at.column, from, to: from + 15 });
  };

  // Each day's events and timed reminders, laid out together so neither is drawn over the other.
  const layouts = days.map((d) => {
    const due = stackReminders(reminders.filter((r) => r.dueAt !== null && sameDay(dateOf(r.dueAt), d)));
    const marks = due.map(({ reminder, top }) => ({
      id: reminder.guid,
      top: (top / HOUR) * 60,
      length: (REMINDER_LINE / HOUR) * 60,
    }));
    // The event being named takes its own column too, beside what it overlaps, not over it.
    if (pending && sameDay(dateOf(pending.startsAt), d)) {
      const from = dateOf(pending.startsAt);
      marks.push({
        id: PENDING,
        top: from.getHours() * 60 + from.getMinutes(),
        length: Math.max((pending.endsAt - pending.startsAt) / 60, 15),
      });
    }
    const laid = placeDayWith(events, d, marks);
    return {
      events: laid.events,
      marks: laid.marks.filter((m) => m.id !== PENDING),
      pending: laid.marks.find((m) => m.id === PENDING) ?? null,
      reminders: new Map(due.map((x) => [x.reminder.guid, x.reminder])),
    };
  });

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
              // An hour the red "now" label sits over is left out, as Calendar does, rather
              // than drawn underneath it.
              // Measured in pixels, since that is what overlaps: two labels about 14px tall
              // touch until they are 18px apart, whatever an hour is drawn as.
              h === 0 || (todayColumn >= 0 && (Math.abs(h * 60 - nowMinutes) / 60) * HOUR < 18) ? null : (
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
            // A press on the grid starts making an event, never a text selection or a native
            // drag: in WebKitGTK either would take the pointer and the events that end the
            // drag with it.
            onMouseDown={(e) => e.preventDefault()}
            onDragStart={(e) => e.preventDefault()}
            onDoubleClick={(e) => {
              if ((e.target as HTMLElement).closest(".cx-block")) return;
              const at = where(e.clientX, e.clientY);
              if (!at) return;
              const start = Math.min(snap(Math.floor(at.minutes / 30) * 30), 23 * 60);
              const base = seconds(days[at.column]);
              onCreate(base + start * 60, base + (start + 60) * 60, false, besideColumn(at.column, e.clientY));
            }}
          >
            {days.map((d, column) => (
              <div className={`cx-column${d.getDay() === 0 || d.getDay() === 6 ? " is-weekend" : ""}`} key={dayKey(d)}>
                {layouts[column].events.map((p) => {
                  const moving = drag?.kind === "move" && drag.event.guid === p.event.guid ? drag : null;
                  const sizing = drag?.kind === "resize" && drag.event.guid === p.event.guid ? drag : null;
                  const c = calendars.get(p.event.calendarGuid);
                  return (
                    <button
                      key={p.event.guid}
                      className={`cx-block${moving || sizing ? " is-moving" : ""}${p.length < 45 ? " is-short" : ""}`}
                      style={
                        {
                          "--cal": tint(c),
                          top: `${((p.top + (moving?.dMinutes ?? 0)) / 60) * HOUR}px`,
                          height: `${Math.max(((p.length + (sizing?.dMinutes ?? 0)) / 60) * HOUR - 2, 20)}px`,
                          left: `calc(${(p.column / p.columns) * 100}% + ${(moving?.dx ?? 0) * 100}%)`,
                          width: `calc(${100 / p.columns}% - 3px)`,
                        } as React.CSSProperties
                      }
                      onPointerDown={(e) => {
                        if (e.button !== 0) return;
                        e.stopPropagation();
                        e.preventDefault();
                        if (canWrite(p.event.calendarGuid)) {
                          setDragNow({ kind: "move", event: p.event, startX: e.clientX, startY: e.clientY, dx: 0, dMinutes: 0 });
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
                      {canWrite(p.event.calendarGuid) && (
                        <span
                          className="cx-block-grip"
                          aria-hidden="true"
                          onPointerDown={(e) => {
                            if (e.button !== 0) return;
                            e.stopPropagation();
                            e.preventDefault();
                            setDragNow({ kind: "resize", event: p.event, startY: e.clientY, dMinutes: 0 });
                          }}
                        />
                      )}
                    </button>
                  );
                })}
                {layouts[column].marks.map((m) => {
                  const r = layouts[column].reminders.get(m.id)!;
                  return (
                    <span
                      key={r.guid}
                      className={`cx-reminder${r.completedAt ? " is-done" : ""}`}
                      style={{
                        top: `${(m.top / 60) * HOUR}px`,
                        left: `calc(${(m.column / m.columns) * 100}% + 4px)`,
                        maxWidth: `calc(${100 / m.columns}% - 8px)`,
                      }}
                    >
                      <span className="cx-ring" aria-hidden="true" />
                      {r.title}
                    </span>
                  );
                })}
                {pending && !drag && layouts[column].pending && (
                  <span
                    className="cx-block is-draft"
                    style={{
                      top: `${(layouts[column].pending!.top / 60) * HOUR}px`,
                      height: `${Math.max((layouts[column].pending!.length / 60) * HOUR - 2, 20)}px`,
                      left: `${(layouts[column].pending!.column / layouts[column].pending!.columns) * 100}%`,
                      width: `calc(${100 / layouts[column].pending!.columns}% - 3px)`,
                    }}
                  >
                    <span className="cx-block-title">{pending.title.trim() || "New Event"}</span>
                    <span className="cx-block-time">{clock(pending.startsAt)}</span>
                  </span>
                )}
                {/* Drawn once the drag is one, never for a press held still: a
                    quarter-hour sliver that release then discards reads as a bug. */}
                {drag?.kind === "create" && drag.column === column && drag.to - drag.from > 15 && (
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

/**
 * Where each reminder sits on a day's hour grid. Two due at the same time — a morning routine
 * set for eight — stack one under the other instead of being drawn over each other.
 */
const REMINDER_LINE = 18;
/** The layout's name for the event being named, which has no guid yet. */
const PENDING = "\u0000pending";
export function stackReminders(list: Reminder[]): { reminder: Reminder; top: number }[] {
  const LINE = REMINDER_LINE;
  const sorted = [...list].sort((a, b) => (a.dueAt ?? 0) - (b.dueAt ?? 0) || a.title.localeCompare(b.title));
  let lastBottom = -Infinity;
  return sorted.map((reminder) => {
    const at = dateOf(reminder.dueAt ?? 0);
    const wanted = ((at.getHours() * 60 + at.getMinutes()) / 60) * HOUR;
    const top = Math.max(wanted, lastBottom);
    lastBottom = top + LINE;
    return { reminder, top };
  });
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

/** A date and, unless it is all day, a time — two compact fields, as Calendar lays them out. */
function When({
  at,
  allDay,
  editable,
  label,
  onChange,
}: {
  at: number;
  allDay: boolean;
  editable: boolean;
  label: string;
  onChange: (s: number) => void;
}) {
  const d = dateOf(at);
  const day = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  const time = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  return (
    <span className="cx-when">
      <input
        type="date"
        aria-label={`${label} date`}
        value={day}
        readOnly={!editable}
        onChange={(e) => e.target.value && onChange(fromLocalInput(`${e.target.value}T${time}`))}
      />
      {!allDay && (
        <input
          type="time"
          aria-label={`${label} time`}
          step={300}
          value={time}
          readOnly={!editable}
          onChange={(e) => e.target.value && onChange(fromLocalInput(`${day}T${e.target.value}`))}
        />
      )}
    </span>
  );
}

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
    if (left + width > window.innerWidth - 8) left = (at.left ?? at.x) - width - gap;
    // Neither side free — a block as wide as Day's column — so over its far end, clear of
    // its title, which begins at the other.
    if (left < 8) left = Math.max(8, window.innerWidth - width - 8);
    const top = Math.min(Math.max(8, at.y - 24), window.innerHeight - height - 8);
    setPos({ left, top });
  }, [at]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") return onClose();
      // Delete, as in Calendar: an open event goes when Delete is pressed outside its fields —
      // and Undo brings it back, since iCloud keeps no bin.
      const typing = (e.target as HTMLElement | null)?.closest("input, textarea, select, [contenteditable]");
      if ((e.key === "Delete" || e.key === "Backspace") && !typing && onDelete && editable) {
        e.preventDefault();
        onDelete();
      }
    };
    const onDown = (e: MouseEvent) => {
      if (box.current && !box.current.contains(e.target as Node)) onClose();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onDown);
    };
  }, [onClose, onDelete, editable]);

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
        <div className="cx-pop-row">
          <span>starts</span>
          <When
            at={draft.startsAt}
            allDay={draft.allDay}
            editable={editable}
            label="Starts"
            // Moving the start keeps the length, as Calendar does.
            onChange={(startsAt) => set({ startsAt, endsAt: startsAt + (draft.endsAt - draft.startsAt) })}
          />
        </div>
        <div className="cx-pop-row">
          <span>ends</span>
          <When
            at={draft.allDay ? draft.endsAt - 1 : draft.endsAt}
            allDay={draft.allDay}
            editable={editable}
            label="Ends"
            onChange={(end) =>
              set({ endsAt: draft.allDay ? seconds(addDays(midnight(dateOf(end)), 1)) : Math.max(end, draft.startsAt + 300) })
            }
          />
        </div>
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

/**
 * The day at a glance, beside Day's timeline — as Calendar sets it: the date, then what fills
 * it, all-day first and the rest in time order. A wide window gave the timeline the whole
 * width and left most of it empty; this is what Calendar puts there.
 */
function DayPane({
  day,
  events,
  reminders,
  calendars,
  onOpen,
}: {
  day: Date;
  events: CalendarEvent[];
  reminders: Reminder[];
  calendars: Map<string, Calendar>;
  onOpen: (e: CalendarEvent, at: Anchor) => void;
}) {
  const from = seconds(day);
  const to = seconds(addDays(day, 1));
  const today = events.filter((e) => e.startsAt < to && e.endsAt > from);
  const allDay = today.filter((e) => spansDays(e));
  const timed = today.filter((e) => !spansDays(e)).sort((a, b) => a.startsAt - b.startsAt);
  const due = reminders
    .filter((r) => r.dueAt !== null && r.dueAt >= from && r.dueAt < to && !r.completedAt)
    .sort((a, b) => (a.dueAt ?? 0) - (b.dueAt ?? 0));
  const row = (e: CalendarEvent, when: string) => (
    <li key={e.guid}>
      <button
        className="cx-pane-event"
        style={{ "--cal": tint(calendars.get(e.calendarGuid)) } as React.CSSProperties}
        onClick={(ev) => onOpen(e, pointOf(ev.currentTarget))}
      >
        <span className="cx-pane-title">{e.title}</span>
        <span className="cx-pane-when">{when}</span>
        {e.location && <span className="cx-pane-where">{e.location}</span>}
      </button>
    </li>
  );
  const nothing = allDay.length + timed.length + due.length === 0;
  return (
    <aside className="cx-pane" aria-label={day.toLocaleDateString(undefined, { dateStyle: "full" })}>
      <h2 className="cx-pane-day">{day.toLocaleDateString(undefined, { weekday: "long" })}</h2>
      <p className="cx-pane-date">{day.toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" })}</p>
      {nothing ? (
        <p className="cx-pane-empty">Nothing on this day.</p>
      ) : (
        <ul className="cx-pane-list">
          {allDay.map((e) => row(e, "all-day"))}
          {timed.map((e) => row(e, `${clock(e.startsAt)} – ${clock(e.endsAt)}`))}
          {due.map((r) => (
            <li key={r.guid} className="cx-pane-reminder">
              <span className="cx-ring" aria-hidden="true" />
              <span className="cx-pane-title">{r.title}</span>
              <span className="cx-pane-when">{clock(r.dueAt ?? 0)}</span>
            </li>
          ))}
        </ul>
      )}
    </aside>
  );
}
