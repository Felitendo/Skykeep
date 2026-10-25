// Quick Event: a line read into a title, a day, a time, a length, a repeat and a place.
import { test } from "node:test";
import assert from "node:assert/strict";
import { parseQuick } from "../src/screens/calendar/quick.ts";

// Wednesday 21 October 2026, 10:15 in the morning.
const now = new Date(2026, 9, 21, 10, 15);
const at = (y, m, d, h = 0, min = 0) => Math.floor(new Date(y, m, d, h, min).getTime() / 1000);
const q = (s, dmy = true) => parseQuick(s, now, dmy);

test("lunch with someone tomorrow at one", () => {
  const r = q("Lunch with Asha tomorrow 1pm");
  assert.equal(r.title, "Lunch with Asha");
  assert.equal(r.startsAt, at(2026, 9, 22, 13));
  assert.equal(r.endsAt, at(2026, 9, 22, 14));
  assert.equal(r.allDay, false);
  assert.equal(r.repeat, null);
});

test("a weekly repeat on a named day", () => {
  const r = q("Gym every Monday 7am");
  assert.equal(r.title, "Gym");
  assert.equal(r.startsAt, at(2026, 9, 26, 7));
  assert.deepEqual(r.repeat, { freq: "weekly", interval: 1, count: null });
});

test("a range with no am or pm", () => {
  const r = q("Flight Fri 6:30–9");
  assert.equal(r.title, "Flight");
  assert.equal(r.startsAt, at(2026, 9, 23, 18, 30));
  assert.equal(r.endsAt, at(2026, 9, 23, 21));
  const morning = q("Flight Fri 6:30am-9");
  assert.equal(morning.startsAt, at(2026, 9, 23, 6, 30));
  assert.equal(morning.endsAt, at(2026, 9, 23, 9));
  const span = q("Workshop 1-2pm");
  assert.equal(span.startsAt, at(2026, 9, 21, 13));
  assert.equal(span.endsAt, at(2026, 9, 21, 14));
  const noon = q("Talk 11am to 1");
  assert.equal(noon.endsAt - noon.startsAt, 2 * 3600);
});

test("a day alone is all day; nothing said is the next hour", () => {
  const r = q("Mum's birthday on 3 Nov");
  assert.equal(r.title, "Mum's birthday");
  assert.equal(r.allDay, true);
  assert.equal(r.startsAt, at(2026, 10, 3));
  assert.equal(r.endsAt, at(2026, 10, 4));
  const bare = q("Call the bank");
  assert.equal(bare.title, "Call the bank");
  assert.equal(bare.startsAt, at(2026, 9, 21, 11));
  assert.equal(bare.dated || bare.timed, false);
});

test("places, lengths, dates and months", () => {
  const r = q("Coffee with Ravi at Third Wave tomorrow at 4 for 30 min");
  assert.equal(r.title, "Coffee with Ravi");
  assert.equal(r.location, "Third Wave");
  assert.equal(r.startsAt, at(2026, 9, 22, 16));
  assert.equal(r.endsAt - r.startsAt, 1800);
  assert.equal(q("Review Oct 30 at 2:30pm").startsAt, at(2026, 9, 30, 14, 30));
  assert.equal(q("Exam 2027-01-12 9am").startsAt, at(2027, 0, 12, 9));
  assert.equal(q("Dentist 12/11 10am", true).startsAt, at(2026, 10, 12, 10));
  assert.equal(q("Dentist 12/11 10am", false).startsAt, at(2026, 11, 11, 10));
  // A month gone this year is next year's.
  assert.equal(q("Trip March 3").startsAt, at(2027, 2, 3));
  assert.deepEqual(q("Rent every month").repeat, { freq: "monthly", interval: 1, count: null });
  assert.deepEqual(q("Payroll every 2 weeks").repeat, { freq: "weekly", interval: 2, count: null });
  assert.equal(q("Standup daily 9:30").repeat.freq, "daily");
});

test("a time already gone today is tomorrow's; tonight is the evening", () => {
  assert.equal(q("Stretch 8am").startsAt, at(2026, 9, 22, 8));
  assert.equal(q("Dinner tonight").startsAt, at(2026, 9, 21, 19));
  assert.equal(q("Dinner tonight at 8:30").startsAt, at(2026, 9, 21, 20, 30));
  assert.equal(q("Movie at noon").startsAt, at(2026, 9, 21, 12));
});

test("numbers that are not times stay in the title", () => {
  assert.equal(q("Read chapter 4 tomorrow").title, "Read chapter 4");
  assert.equal(q("Room 101 meeting Thursday 3pm").title, "Room 101 meeting");
  assert.equal(q("Room 101 meeting Thursday 3pm").startsAt, at(2026, 9, 22, 15));
});

test("a range whose am or pm comes at its end is read on that side, on the day named", () => {
  // "6-7am" is six to seven in the morning: the bare 6 had been read as 6 PM, the end pushed into
  // the next morning, and the whole event then moved on a day — tomorrow's gym the day after.
  const gym = q("Gym tomorrow 6-7am");
  assert.deepEqual([gym.startsAt, gym.endsAt], [at(2026, 9, 22, 6), at(2026, 9, 22, 7)]);
  const early = q("Swim Fri 5:30-6:30am");
  assert.deepEqual([early.startsAt, early.endsAt], [at(2026, 9, 23, 5, 30), at(2026, 9, 23, 6, 30)]);
  // And as before: the start on the end's side where it can be, crossing noon or midnight
  // where it has to.
  assert.deepEqual([q("Class tomorrow 7-9pm").startsAt, q("Class tomorrow 7-9pm").endsAt], [at(2026, 9, 22, 19), at(2026, 9, 22, 21)]);
  assert.deepEqual([q("Shift tomorrow 9-5pm").startsAt, q("Shift tomorrow 9-5pm").endsAt], [at(2026, 9, 22, 9), at(2026, 9, 22, 17)]);
  assert.deepEqual([q("Party tomorrow 11-1am").startsAt, q("Party tomorrow 11-1am").endsAt], [at(2026, 9, 22, 23), at(2026, 9, 23, 1)]);
});
