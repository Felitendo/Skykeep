// A reminder typed in one line: day, time, repeat and priority read out; everything else —
// including words that only look like dates — stays the title.
import { test } from "node:test";
import assert from "node:assert/strict";
import { parseReminder, understood } from "../src/screens/reminderQuick.ts";

const now = new Date(2026, 9, 22, 10, 0); // Thursday
const local = (y, m, d, h = 0, min = 0) => Math.floor(new Date(y, m, d, h, min).getTime() / 1000);

test("a day and a time make it due then", () => {
  const q = parseReminder("Call mom tomorrow 6pm", now, true);
  assert.deepEqual([q.title, q.dueAt, q.allDay, q.repeat, q.priority], ["Call mom", local(2026, 9, 23, 18), false, null, 0]);
});

test("a day alone is all day", () => {
  const q = parseReminder("Submit report friday", now, true);
  assert.deepEqual([q.title, q.dueAt, q.allDay], ["Submit report", local(2026, 9, 23), true]);
});

test("a repeat maps to Apple's numbering, starting today when no day is named", () => {
  assert.deepEqual(parseReminder("Water plants daily", now, true).repeat, { frequency: 0, interval: 1 });
  const gym = parseReminder("Gym every Monday 7am", now, true);
  assert.deepEqual([gym.title, gym.repeat, gym.dueAt], ["Gym", { frequency: 1, interval: 1 }, local(2026, 9, 26, 7)]);
  const water = parseReminder("Water plants daily", now, true);
  assert.deepEqual([water.dueAt, water.allDay], [local(2026, 9, 22), true]);
});

test("! marks priority, leading or trailing, and goes from the title", () => {
  assert.deepEqual([parseReminder("!!! Pay rent", now, true).priority, parseReminder("!!! Pay rent", now, true).title], [1, "Pay rent"]);
  assert.equal(parseReminder("Pay rent !!", now, true).priority, 5);
  assert.equal(parseReminder("! Pay rent", now, true).priority, 9);
  assert.equal(parseReminder("Wow!", now, true).priority, 0, "a ! inside a word is just a !");
  assert.equal(parseReminder("Wow!", now, true).title, "Wow!");
});

test("words that only look like dates stay the title", () => {
  for (const t of ["Call May", "Read 20 pages", "Buy milk", "Email Sunday school", "Book: March of the Penguins"]) {
    const q = parseReminder(t, now, true);
    if (q.dueAt === null) assert.equal(q.title, t);
  }
  assert.equal(understood(parseReminder("Buy milk", now, true)), false);
  assert.equal(understood(parseReminder("Call May", now, true)), false);
});

test("a place is kept in the title", () => {
  const q = parseReminder("Pick up parcel at the post office tomorrow", now, true);
  assert.match(q.title, /post office/);
});

test("how long it takes is part of what to do: it stays in the title", () => {
  // A reminder has no length. Read as one, "for 20 minutes" went from the title unsaid.
  const read = parseReminder("Read for 20 minutes daily", now, true);
  assert.deepEqual([read.title, read.repeat], ["Read for 20 minutes", { frequency: 0, interval: 1 }]);
  const piano = parseReminder("Practice piano for an hour tomorrow", now, true);
  assert.deepEqual([piano.title, piano.dueAt, piano.allDay], ["Practice piano for an hour", local(2026, 9, 23), true]);
  const run = parseReminder("Run for 30 mins tomorrow 7am", now, true);
  assert.deepEqual([run.title, run.dueAt, run.allDay], ["Run for 30 mins", local(2026, 9, 23, 7), false]);
});
