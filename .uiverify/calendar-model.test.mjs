// The arithmetic Calendar's views are drawn from, checked directly.
import { test } from "node:test";
import assert from "node:assert/strict";
import { placeDay, placeDayWith, weekSpans, monthDays, daysOf, snap, clock } from "../src/screens/calendar/model.ts";

const at = (y, m, d, h = 0, min = 0) => Math.floor(new Date(y, m, d, h, min).getTime() / 1000);
const ev = (guid, startsAt, endsAt, allDay = false) =>
  ({ guid, calendarGuid: "c", title: guid, location: null, startsAt, endsAt, allDay, timezone: null, url: null, notes: null, etag: null });

test("a month is six weeks from the Sunday on or before the 1st", () => {
  const days = monthDays(new Date(2026, 8, 15));
  assert.equal(days.length, 42);
  assert.equal(days[0].getDay(), 0);
  assert.deepEqual([days[0].getMonth(), days[0].getDate()], [7, 30]);
});

test("an all-day event ends the day before its end midnight", () => {
  const [first, last] = daysOf(ev("a", at(2026, 9, 2), at(2026, 9, 4), true));
  assert.equal(first.getDate(), 2);
  assert.equal(last.getDate(), 3);
});

test("a trip across a weekend is one bar per week, carrying on across the break", () => {
  const trip = ev("trip", at(2026, 9, 2), at(2026, 9, 6), true); // Fri 2 – Mon 5
  const first = weekSpans([trip], new Date(2026, 8, 27));
  const second = weekSpans([trip], new Date(2026, 9, 4));
  assert.deepEqual([first[0].from, first[0].to, first[0].continuesRight], [5, 6, true]);
  assert.deepEqual([second[0].from, second[0].to, second[0].continuesLeft], [0, 1, true]);
});

test("overlapping bars take separate lanes, and a free lane is reused", () => {
  const week = new Date(2026, 8, 27);
  const spans = weekSpans([
    ev("long", at(2026, 8, 27), at(2026, 9, 1), true),
    ev("mid", at(2026, 8, 28), at(2026, 8, 30), true),
    ev("late", at(2026, 9, 1), at(2026, 9, 3), true),
  ], week);
  const lane = Object.fromEntries(spans.map((s) => [s.event.guid, s.lane]));
  assert.equal(lane.long, 0);
  assert.equal(lane.mid, 1);
  assert.equal(lane.late, 0);
});

test("overlapping meetings sit side by side; a later one gets the width back", () => {
  const day = new Date(2026, 8, 29);
  const placed = placeDay([
    ev("a", at(2026, 8, 29, 9), at(2026, 8, 29, 10)),
    ev("b", at(2026, 8, 29, 9, 30), at(2026, 8, 29, 11)),
    ev("c", at(2026, 8, 29, 14), at(2026, 8, 29, 15)),
  ], day);
  const by = Object.fromEntries(placed.map((p) => [p.event.guid, p]));
  assert.deepEqual([by.a.column, by.a.columns], [0, 2]);
  assert.deepEqual([by.b.column, by.b.columns], [1, 2]);
  assert.deepEqual([by.c.column, by.c.columns], [0, 1]);
  assert.equal(by.a.top, 540);
});

test("a reminder due during a meeting takes a column beside it", () => {
  const day = new Date(2026, 8, 29);
  const { events, marks } = placeDayWith([ev("m", at(2026, 8, 29, 19), at(2026, 8, 29, 20))], day,
    [{ id: "r", top: 19 * 60, length: 22.5 }, { id: "late", top: 21 * 60, length: 22.5 }]);
  assert.deepEqual([events[0].column, events[0].columns], [0, 2]);
  const by = Object.fromEntries(marks.map((m) => [m.id, m]));
  assert.deepEqual([by.r.column, by.r.columns], [1, 2]);
  assert.deepEqual([by.late.column, by.late.columns], [0, 1]);
});

test("a five-minute call is still a quarter hour tall", () => {
  const [p] = placeDay([ev("x", at(2026, 8, 29, 9), at(2026, 8, 29, 9, 5))], new Date(2026, 8, 29));
  assert.equal(p.length, 15);
});

test("an evening that runs past midnight is a block on each day it touches", () => {
  const late = ev("late", at(2026, 8, 29, 23), at(2026, 8, 30, 1));
  const [tue] = placeDay([late], new Date(2026, 8, 29));
  const [wed] = placeDay([late], new Date(2026, 8, 30));
  assert.deepEqual([tue.top, tue.length], [23 * 60, 60]);
  assert.deepEqual([wed.top, wed.length], [0, 60]);
  assert.equal(weekSpans([late], new Date(2026, 8, 27)).length, 0);
});

test("a timed event of a day or more is a bar", () => {
  const conference = ev("conf", at(2026, 8, 29, 9), at(2026, 9, 1, 17));
  assert.equal(placeDay([conference], new Date(2026, 8, 30)).length, 0);
  assert.equal(weekSpans([conference], new Date(2026, 8, 27))[0].to, 4);
});

test("dragging snaps to the quarter hour", () => {
  assert.equal(snap(7), 0);
  assert.equal(snap(8), 15);
  assert.equal(snap(52), 45);
});

test("a time on the hour leaves off the minutes", () => {
  assert.doesNotMatch(clock(at(2026, 8, 29, 15)), /:00/);
  assert.match(clock(at(2026, 8, 29, 15, 30)), /30/);
});
