/**
 * The arithmetic behind Calendar's views, kept apart from drawing them so it can be tested.
 *
 * Every date here is local: a calendar is read in the viewer's own time, and a day is from
 * one local midnight to the next — twenty-three or twenty-five hours on the two days a clock
 * moves, which is why nothing here adds a fixed 86,400 seconds.
 */
import type { CalendarEvent } from "../../lib/ipc";
import { timeText } from "../../lib/when.ts";

export const midnight = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
export const addDays = (d: Date, n: number) =>
  new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
export const seconds = (d: Date) => Math.floor(d.getTime() / 1000);
export const dateOf = (s: number) => new Date(s * 1000);
export const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
export const dayKey = (d: Date) => `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;

/** The Sunday on or before `d`, where Apple starts a week. */
export const startOfWeek = (d: Date) => addDays(midnight(d), -d.getDay());

/** The six weeks a month shows: from the week holding the 1st, forty-two days. */
export function monthDays(anchor: Date): Date[] {
  const start = startOfWeek(new Date(anchor.getFullYear(), anchor.getMonth(), 1));
  return Array.from({ length: 42 }, (_, i) => addDays(start, i));
}

/** An event's first and last local day, the last being the day holding its final moment. */
export function daysOf(event: CalendarEvent): [Date, Date] {
  const first = midnight(dateOf(event.startsAt));
  // An all-day event ends at the next midnight, and one that ends exactly at midnight
  // does not reach into the day after.
  const last = midnight(dateOf(Math.max(event.endsAt - 1, event.startsAt)));
  return [first, last];
}

/**
 * Whether an event is drawn as a bar in the all-day row rather than as a block of time.
 *
 * All-day events, and timed ones of a day or more. An overnight event — eleven to one — is
 * a block on each of the two days it touches, which is how Calendar draws it.
 */
export const spansDays = (event: CalendarEvent) =>
  event.allDay || event.endsAt - event.startsAt >= 24 * 3600;

/** One bar in a week row: the columns it covers and the lane it sits in. */
export type Span = {
  event: CalendarEvent;
  /** 0–6, the first and last column it covers in this week. */
  from: number;
  to: number;
  lane: number;
  /** Whether it carries on from the week before, or into the week after. */
  continuesLeft: boolean;
  continuesRight: boolean;
};

/**
 * The bars of one week: each all-day or multi-day event, cut to the week, stacked into the
 * lowest lane where it collides with nothing — so a three-day trip is one bar, not three.
 */
export function weekSpans(events: CalendarEvent[], weekStart: Date): Span[] {
  const weekEnd = addDays(weekStart, 6);
  const within = events
    .filter(spansDays)
    .map((event) => {
      const [first, last] = daysOf(event);
      return { event, first, last };
    })
    .filter(({ first, last }) => last >= weekStart && first <= weekEnd)
    // Longest first, then earliest: how Apple stacks them.
    .sort(
      (a, b) =>
        b.last.getTime() - b.first.getTime() - (a.last.getTime() - a.first.getTime()) ||
        a.first.getTime() - b.first.getTime() ||
        a.event.title.localeCompare(b.event.title)
    );
  const lanes: boolean[][] = [];
  const out: Span[] = [];
  for (const { event, first, last } of within) {
    const from = Math.max(0, Math.round((midnight(first).getTime() - weekStart.getTime()) / 864e5));
    const to = Math.min(6, Math.round((midnight(last).getTime() - weekStart.getTime()) / 864e5));
    let lane = 0;
    while (lanes[lane]?.slice(from, to + 1).some(Boolean)) lane++;
    (lanes[lane] ??= Array(7).fill(false));
    for (let c = from; c <= to; c++) lanes[lane][c] = true;
    out.push({
      event,
      from,
      to,
      lane,
      continuesLeft: first < weekStart,
      continuesRight: last > weekEnd,
    });
  }
  return out;
}

/** A timed event placed on a day's hour grid. */
export type Placed = {
  event: CalendarEvent;
  /** Minutes from the day's midnight to where the block starts, and how long it is. */
  top: number;
  length: number;
  /** Side-by-side columns for events that overlap, as Apple lays them out. */
  column: number;
  columns: number;
};

/** Something else on the hour grid that takes room there — a reminder due at a time. */
export type Mark = { id: string; top: number; length: number };
export type PlacedMark = Mark & { column: number; columns: number };

/**
 * The timed events of one day, placed and set side by side where they overlap.
 *
 * Events that overlap, directly or through a chain, form a cluster; each takes the first
 * column free at its start, and the whole cluster shares one column count so the blocks line
 * up. A block is never shorter than a quarter hour, or a five-minute call would be unreadable.
 */
export function placeDay(events: CalendarEvent[], day: Date): Placed[] {
  return placeDayWith(events, day, []).events;
}

/**
 * The same, with the day's reminders laid out among the events rather than over them: a
 * reminder due during a meeting sits in a column beside it, as it does in Calendar, and the
 * meeting's title stays readable.
 */
export function placeDayWith(
  events: CalendarEvent[],
  day: Date,
  marks: Mark[],
): { events: Placed[]; marks: PlacedMark[] } {
  const start = seconds(midnight(day));
  const end = seconds(addDays(day, 1));
  type Item = { top: number; length: number; column: number; columns: number } & (
    | { kind: "event"; event: CalendarEvent }
    | { kind: "mark"; mark: Mark }
  );
  const items: Item[] = [
    ...events
      .filter((e) => !spansDays(e) && e.endsAt > start && e.startsAt < end)
      .map((event): Item => {
        const from = Math.max(event.startsAt, start);
        const to = Math.min(Math.max(event.endsAt, event.startsAt), end);
        return { kind: "event", event, top: (from - start) / 60, length: Math.max((to - from) / 60, 15), column: 0, columns: 1 };
      }),
    ...marks.map((mark): Item => ({ kind: "mark", mark, top: mark.top, length: mark.length, column: 0, columns: 1 })),
  ].sort((a, b) => a.top - b.top || b.length - a.length);

  let cluster: Item[] = [];
  let clusterEnd = -1;
  const close = () => {
    const columns = Math.max(1, ...cluster.map((p) => p.column + 1));
    for (const p of cluster) p.columns = columns;
    cluster = [];
  };
  for (const item of items) {
    if (item.top >= clusterEnd && cluster.length) close();
    const busy = cluster.filter((p) => p.top + p.length > item.top).map((p) => p.column);
    let column = 0;
    while (busy.includes(column)) column++;
    item.column = column;
    cluster.push(item);
    clusterEnd = Math.max(clusterEnd, item.top + item.length);
  }
  if (cluster.length) close();

  const placed: Placed[] = [];
  const placedMarks: PlacedMark[] = [];
  for (const p of items) {
    const { top, length, column, columns } = p;
    if (p.kind === "event") placed.push({ event: p.event, top, length, column, columns });
    else placedMarks.push({ ...p.mark, top, length, column, columns });
  }
  return { events: placed, marks: placedMarks };
}

/** Snap a number of minutes to the nearest quarter hour, as dragging in Calendar does. */
export const snap = (minutes: number, step = 15) => Math.round(minutes / step) * step;

/** "9 AM", "Noon", "3 PM" — the hour labels in the viewer's own clock. */
export function hourLabel(hour: number): string {
  if (hour === 12) return "Noon";
  const d = new Date(2024, 0, 1, hour);
  return timeText(d, { hour: "numeric" });
}

/** "9:30 AM", with ":00" left off, as Calendar writes a time. */
export const clock = (s: number) =>
  timeText(dateOf(s), { hour: "numeric", minute: "2-digit" })
    .replace(/:00(?=\D|$)/, "");

/* ---- time zones ------------------------------------------------------------------------ */

/** A clock reading: what a wall clock in some zone shows. */
export type Wall = { y: number; m: number; d: number; h: number; min: number };

const clocks = new Map<string, Intl.DateTimeFormat>();
function clockIn(zone: string): Intl.DateTimeFormat {
  let f = clocks.get(zone);
  if (!f) {
    f = new Intl.DateTimeFormat("en-US", {
      timeZone: zone, hourCycle: "h23",
      year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "numeric",
    });
    clocks.set(zone, f);
  }
  return f;
}

/** What a clock in `zone` shows at the instant `s` (seconds). */
export function wallIn(zone: string, s: number): Wall {
  const parts: Record<string, number> = {};
  for (const p of clockIn(zone).formatToParts(new Date(s * 1000))) {
    if (p.type !== "literal") parts[p.type] = Number(p.value);
  }
  return { y: parts.year, m: parts.month, d: parts.day, h: parts.hour % 24, min: parts.minute };
}

/**
 * The instant a clock in `zone` shows `w`. A reading a clock skips (the hour lost in spring)
 * comes out an hour on, as Calendar moves one; one a clock shows twice, the first time.
 */
export function instantIn(zone: string, w: Wall): number {
  const asUtc = Date.UTC(w.y, w.m - 1, w.d, w.h, w.min) / 1000;
  const offsetAt = (s: number) => {
    const c = wallIn(zone, s);
    return Date.UTC(c.y, c.m - 1, c.d, c.h, c.min) / 1000 - s;
  };
  // The two offsets either side of any clock change near it: a reading shown under both is
  // taken the first time; one shown under neither (skipped) is taken an hour on.
  const tries = [asUtc - offsetAt(asUtc - 86400), asUtc - offsetAt(asUtc + 86400)];
  const same = (s: number) => {
    const c = wallIn(zone, s);
    return c.y === w.y && c.m === w.m && c.d === w.d && c.h === w.h && c.min === w.min;
  };
  const shown = tries.filter(same);
  return shown.length ? Math.min(...shown) : Math.max(...tries);
}

/** An instant's clock reading moved to another zone: 9:00 in one is 9:00 in the other. */
export function sameClockIn(from: string, to: string, s: number): number {
  return instantIn(to, wallIn(from, s));
}

/** Every zone this computer knows, by its IANA name. */
export function allZones(): string[] {
  const known = (Intl as { supportedValuesOf?: (k: string) => string[] }).supportedValuesOf?.("timeZone") ?? [];
  return known.length ? known : ["UTC"];
}

/** A zone as Calendar names one: its city, and how far from GMT it is at `s`. */
export function zoneName(zone: string, s: number): { city: string; offset: string } {
  const city = (zone.split("/").pop() ?? zone).replace(/_/g, " ");
  const w = wallIn(zone, s);
  const minutes = Math.round((Date.UTC(w.y, w.m - 1, w.d, w.h, w.min) / 1000 - Math.floor(s / 60) * 60) / 60);
  const sign = minutes < 0 ? "−" : "+";
  const a = Math.abs(minutes);
  const offset = minutes === 0 ? "GMT" : `GMT${sign}${Math.floor(a / 60)}${a % 60 ? `:${String(a % 60).padStart(2, "0")}` : ""}`;
  return { city, offset };
}
