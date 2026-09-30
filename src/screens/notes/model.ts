/**
 * The rules Notes' list and editor follow, kept apart from drawing them so they can be tested.
 */
import type { Note } from "../../lib/ipc";

const midnight = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
const daysBetween = (from: Date, to: Date) =>
  Math.round((midnight(to).getTime() - midnight(from).getTime()) / 864e5);

/** One run of the note list under a heading. */
export type Section = { key: string; heading: string; notes: Note[] };

/**
 * The list the way Notes groups it: Pinned first, then Today, Yesterday, Previous 7 Days,
 * Previous 30 Days, then each earlier month of this year by name, then each earlier year.
 * Within a group, most recently changed first. A group with nothing in it is not shown.
 */
export function groupNotes(notes: Note[], now = new Date()): Section[] {
  const out = new Map<string, Section>();
  const add = (key: string, heading: string, note: Note) => {
    const s = out.get(key) ?? { key, heading, notes: [] };
    s.notes.push(note);
    out.set(key, s);
  };
  const sorted = [...notes].sort((a, b) => (b.modifiedAt ?? 0) - (a.modifiedAt ?? 0));
  for (const note of sorted) {
    if (note.pinned) {
      add("pinned", "Pinned", note);
      continue;
    }
    if (note.modifiedAt === null) {
      add("undated", "Earlier", note);
      continue;
    }
    const when = new Date(note.modifiedAt * 1000);
    const ago = daysBetween(when, now);
    if (ago <= 0) add("today", "Today", note);
    else if (ago === 1) add("yesterday", "Yesterday", note);
    else if (ago <= 7) add("week", "Previous 7 Days", note);
    else if (ago <= 30) add("month", "Previous 30 Days", note);
    else if (when.getFullYear() === now.getFullYear())
      add(`m${when.getMonth()}`, when.toLocaleDateString(undefined, { month: "long" }), note);
    else add(`y${when.getFullYear()}`, String(when.getFullYear()), note);
  }
  // Map keeps insertion order, and notes went in newest first, so groups already run
  // newest first — Pinned aside, which always leads.
  const sections = [...out.values()];
  const pinned = sections.filter((s) => s.key === "pinned");
  return [...pinned, ...sections.filter((s) => s.key !== "pinned")];
}

/** The date a row shows: the time today, "Yesterday", the weekday this week, else the date. */
export function listDate(seconds: number | null, now = new Date()): string {
  if (seconds === null) return "";
  const when = new Date(seconds * 1000);
  const ago = daysBetween(when, now);
  if (ago <= 0) return when.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  if (ago === 1) return "Yesterday";
  if (ago < 7) return when.toLocaleDateString(undefined, { weekday: "long" });
  return when.toLocaleDateString(undefined, { day: "numeric", month: "numeric", year: "numeric" });
}

/** The date at the top of the editor: "30 September 2026 at 10:02 PM". */
export function editorDate(seconds: number | null): string {
  if (seconds === null) return "";
  const when = new Date(seconds * 1000);
  const day = when.toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" });
  const time = when.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  return `${day} at ${time}`;
}

/**
 * A note's text as the editor shows it: its first line, set as the title, and the rest.
 * Joining them back gives exactly the text that went in, so nothing is gained or lost by
 * editing the two halves.
 */
export function splitTitle(text: string): { title: string; rest: string } {
  const at = text.indexOf("\n");
  return at < 0 ? { title: text, rest: "" } : { title: text.slice(0, at), rest: text.slice(at + 1) };
}

export function joinTitle(title: string, rest: string, hadBreak: boolean): string {
  return rest.length > 0 || hadBreak ? `${title}\n${rest}` : title;
}
