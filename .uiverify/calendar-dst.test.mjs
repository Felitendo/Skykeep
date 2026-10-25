// The hour grid on the two days a year the clock moves: an event is drawn by the clock it is
// at, beside its hour's label — never by the time gone since midnight, which put everything
// after the change an hour off. Run in a zone that moves, whatever this computer's is.
process.env.TZ = "America/New_York";
import { test } from "node:test";
import assert from "node:assert/strict";
import { placeDay } from "../src/screens/calendar/model.ts";

const at = (y, m, d, h = 0, min = 0) => Math.floor(new Date(y, m, d, h, min).getTime() / 1000);
const ev = (guid, startsAt, endsAt) =>
  ({ guid, calendarGuid: "c", title: guid, location: null, startsAt, endsAt, allDay: false, timezone: null, url: null, notes: null, etag: null });

test("the zone moves its clocks (the test means something)", () => {
  // 8 March 2026 is 23 hours long in New York.
  assert.equal(at(2026, 2, 9) - at(2026, 2, 8), 23 * 3600);
});

test("on the day the clocks go forward, nine o'clock is drawn at nine", () => {
  const [p] = placeDay([ev("standup", at(2026, 2, 8, 9), at(2026, 2, 8, 10))], new Date(2026, 2, 8));
  assert.deepEqual([p.top, p.length], [9 * 60, 60]);
});

test("on the day the clocks go back, nine o'clock is drawn at nine", () => {
  const [p] = placeDay([ev("standup", at(2026, 10, 1, 9), at(2026, 10, 1, 10))], new Date(2026, 10, 1));
  assert.deepEqual([p.top, p.length], [9 * 60, 60]);
});

test("an evening run across the change ends at its own hour, and past midnight at the day's end", () => {
  const [a] = placeDay([ev("night", at(2026, 2, 8, 1), at(2026, 2, 8, 4))], new Date(2026, 2, 8));
  // 1 AM to 4 AM on the clock: from the 1 AM line to the 4 AM line.
  assert.deepEqual([a.top, a.length], [60, 180]);
  const [b] = placeDay([ev("late", at(2026, 2, 8, 22), at(2026, 2, 9, 1))], new Date(2026, 2, 8));
  assert.deepEqual([b.top, b.length], [22 * 60, 120]);
});
