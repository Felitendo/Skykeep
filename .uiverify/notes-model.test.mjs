// The rules Notes' list and editor follow, checked directly.
import { test } from "node:test";
import assert from "node:assert/strict";
import { groupNotes, listDate, splitTitle, joinTitle } from "../src/screens/notes/model.ts";

const now = new Date(2026, 8, 30, 12, 0);
const at = (y, m, d, h = 9) => Math.floor(new Date(y, m, d, h).getTime() / 1000);
const note = (guid, modifiedAt, pinned = false) => ({ guid, folderGuid: null, title: guid, snippet: null, body: null,
  locked: false, pinned, createdAt: null, modifiedAt, editable: true, etag: null });

test("the list is grouped the way Notes groups it, pinned first", () => {
  const sections = groupNotes([
    note("old", at(2025, 2, 1)), note("aug", at(2026, 7, 10)), note("today", at(2026, 8, 30)),
    note("pin", at(2026, 0, 1), true), note("yday", at(2026, 8, 29)), note("week", at(2026, 8, 25)),
    note("month", at(2026, 8, 10)),
  ], now);
  assert.deepEqual(sections.map((s) => [s.key, s.notes.map((n) => n.guid)]), [
    ["pinned", ["pin"]], ["today", ["today"]], ["yesterday", ["yday"]], ["week", ["week"]],
    ["month", ["month"]], ["m7", ["aug"]], ["y2025", ["old"]],
  ]);
});

test("within a group the newest comes first", () => {
  const [today] = groupNotes([note("a", at(2026, 8, 30, 8)), note("b", at(2026, 8, 30, 11))], now);
  assert.deepEqual(today.notes.map((n) => n.guid), ["b", "a"]);
});

test("a row says the time today, Yesterday, the weekday, or the date", () => {
  assert.match(listDate(at(2026, 8, 30, 9), now), /9/);
  assert.equal(listDate(at(2026, 8, 29), now), "Yesterday");
  assert.equal(listDate(at(2026, 8, 27), now), new Date(2026, 8, 27).toLocaleDateString(undefined, { weekday: "long" }));
  assert.match(listDate(at(2026, 5, 1), now), /2026/);
});

test("splitting a note into title and the rest loses nothing", () => {
  for (const text of ["", "Title", "Title\n", "Title\nbody", "\nbody", "T\n\n\nlast\n"]) {
    const { title, rest } = splitTitle(text);
    assert.equal(joinTitle(title, rest, text.includes("\n")), text, JSON.stringify(text));
  }
});
