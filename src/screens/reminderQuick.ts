/**
 * A reminder typed in one line, read the way Reminders reads one: "Call mom tomorrow 6pm",
 * "Water plants daily", "!! Pay rent every month". The day, time and repeat come from Quick
 * Event's reader (calendar/quick.ts); a leading or trailing `!`, `!!` or `!!!` is the
 * priority, as the phone's list shows it. Anything else stays in the title — a place is not
 * taken out, since a reminder's place is set on the phone. Nothing here is saved: the add
 * field shows what was understood first, and can add the line as typed instead.
 */
import { parseQuick } from "./calendar/quick.ts";

export type QuickReminder = {
  title: string;
  /** Local unix seconds of the moment due, or, all day, of that day's local midnight. */
  dueAt: number | null;
  allDay: boolean;
  /** Apple's repeat: 0 daily, 1 weekly, 2 monthly, 3 yearly; with how many between. */
  repeat: { frequency: number; interval: number } | null;
  /** Apple's scale: 0 none, 1 high, 5 medium, 9 low. */
  priority: number;
};

const FREQUENCY = { daily: 0, weekly: 1, monthly: 2, yearly: 3 } as const;
const PRIORITY: Record<number, number> = { 1: 9, 2: 5, 3: 1 };

export function parseReminder(text: string, now: Date = new Date(), dayFirst?: boolean): QuickReminder {
  let line = text.trim();
  let priority = 0;
  const bang = /^(!{1,3})\s+|\s+(!{1,3})$/.exec(line);
  if (bang) {
    priority = PRIORITY[(bang[1] ?? bang[2]).length];
    line = (line.slice(0, bang.index) + " " + line.slice(bang.index + bang[0].length)).trim();
  }
  // How long is not a reminder's to know ("Read for 20 minutes daily"): those words stay.
  const q = parseQuick(line, now, dayFirst, false);
  // Where Quick Event took out a place, it goes back: a reminder keeps "at the office" as
  // words, as the phone does when typed.
  let title = q.location ? `${q.title} at ${q.location}`.trim() : q.title;
  if (!title) title = line;
  const repeat = q.repeat ? { frequency: FREQUENCY[q.repeat.freq], interval: q.repeat.interval } : null;
  const midnight = (d: Date) => Math.floor(new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() / 1000);
  let dueAt: number | null = null;
  let allDay = false;
  if (q.timed) {
    dueAt = q.startsAt;
  } else if (q.dated) {
    dueAt = q.startsAt;
    allDay = true;
  } else if (repeat) {
    // A repeat needs a first day, as on the phone: today.
    dueAt = midnight(now);
    allDay = true;
  }
  return { title, dueAt, allDay, repeat, priority };
}

/** Whether a line says more than a title — the field then shows what it understood. */
export const understood = (q: QuickReminder) => q.dueAt !== null || q.repeat !== null || q.priority !== 0;
