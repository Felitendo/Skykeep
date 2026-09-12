import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, messageOf, type Calendar, type CalendarEvent } from "../lib/ipc";

const DAY = 86_400;
const ZONE = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";

const isoDate = (seconds: number) =>
  new Date(seconds * 1000).toISOString().slice(0, 10);

const dayKey = (seconds: number) =>
  new Date(seconds * 1000).toLocaleDateString(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  });

const dayHeading = (seconds: number) =>
  new Date(seconds * 1000).toLocaleDateString(undefined, {
    weekday: "long",
    day: "numeric",
    month: "long",
  });

const clockTime = (seconds: number) =>
  new Date(seconds * 1000).toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });

function relative(seconds: number | null): string {
  if (seconds === null) return "not yet";
  const ago = Math.max(0, Math.floor(Date.now() / 1000) - seconds);
  if (ago < 60) return "just now";
  if (ago < 3600) return `${Math.floor(ago / 60)} min ago`;
  if (ago < 86_400) return `${Math.floor(ago / 3600)} h ago`;
  return `${Math.floor(ago / 86_400)} d ago`;
}

export function CalendarView() {
  const [events, setEvents] = useState<CalendarEvent[]>([]);
  const [calendars, setCalendars] = useState<Calendar[]>([]);
  const [lastSynced, setLastSynced] = useState<number | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // A four-week window from a week ago: enough to be useful without pulling a decade.
  const window = useMemo(() => {
    const now = Math.floor(Date.now() / 1000);
    return { from: now - 7 * DAY, to: now + 21 * DAY };
  }, []);

  const read = useCallback(async () => {
    const [found, cals, synced] = await Promise.all([
      api.calendarEvents(window.from, window.to),
      api.calendarList(),
      api.calendarLastSynced(),
    ]);
    setEvents(found);
    setCalendars(cals);
    setLastSynced(synced);
    setLoaded(true);
  }, [window]);

  // Render from the library first — instant, and works offline — then refresh behind it.
  // On a first ever open there is nothing stored, so fetch straight away rather than
  // showing an empty screen and asking the person to press a button.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        await read();
        if (cancelled) return;
        const synced = await api.calendarLastSynced();
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
  }, [read]);

  // Held in a ref so the first-open effect can trigger a refresh without depending on it.
  const refreshRef = useRef<null | (() => Promise<void>)>(null);

  const refresh = useCallback(async () => {
    setSyncing(true);
    setError(null);
    try {
      await api.calendarSync(isoDate(window.from), isoDate(window.to), ZONE);
      await read();
    } catch (e) {
      // The library still holds the last good data, so the view stays usable.
      setError(messageOf(e));
    } finally {
      setSyncing(false);
    }
  }, [window, read]);

  refreshRef.current = refresh;

  const colours = useMemo(
    () => new Map(calendars.map((c) => [c.guid, c.colour])),
    [calendars]
  );

  const days = useMemo(() => {
    const grouped = new Map<string, CalendarEvent[]>();
    for (const event of events) {
      const key = dayKey(event.startsAt);
      const bucket = grouped.get(key);
      if (bucket) bucket.push(event);
      else grouped.set(key, [event]);
    }
    return [...grouped.values()];
  }, [events]);

  return (
    <>
      <div className="toolbar">
        <span className="badge">
          {lastSynced === null && loaded ? "Never synced" : `Updated ${relative(lastSynced)}`}
        </span>
        <button className="quiet" onClick={refresh} disabled={syncing}>
          {syncing ? "Refreshing…" : "Refresh"}
        </button>
      </div>

      <h1>Calendar</h1>

      {error && <p className="error" role="alert">{error}</p>}

      {loaded && events.length === 0 && (
        <p>
          Nothing here yet. Refresh to pull the next few weeks from iCloud — after that it
          opens instantly, with or without a connection.
        </p>
      )}

      <div className="agenda">
        {days.map((day) => (
          <section key={dayKey(day[0].startsAt)} className="agenda-day">
            <h2>{dayHeading(day[0].startsAt)}</h2>
            <ul>
              {day.map((event) => (
                <li key={event.guid}>
                  <span
                    className="dot"
                    style={{ background: colours.get(event.calendarGuid) || "var(--gold)" }}
                    aria-hidden="true"
                  />
                  <span className="when">
                    {event.allDay ? "All day" : clockTime(event.startsAt)}
                  </span>
                  <span className="what">
                    {event.title}
                    {event.location && <span className="where">{event.location}</span>}
                  </span>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
    </>
  );
}
