// A note into the editor and back: Apple's text and runs survive the round trip exactly.
import { test } from "node:test";
import assert from "node:assert/strict";
import { toDoc, fromDoc, carryRuns, STYLE } from "../src/screens/notes/rich.ts";

const r = (length, extra = {}) => ({ length, style: null, indent: 0, checked: null, bold: false, italic: false,
  underline: false, strike: false, link: null, ...extra });

test("a formatted note goes in and comes out the same", () => {
  const text = "Shopping\nBuy milk now\nCall the bank\n• Fan";
  const runs = [
    r(9, { style: STYLE.title }),
    r(4, { style: STYLE.checklist, checked: true }), r(4, { style: STYLE.checklist, checked: true, bold: true }),
    r(5, { style: STYLE.checklist, checked: true }),
    r(14, { style: STYLE.dashed }),
    r(5, { italic: true, link: "https://example.com" }),
  ];
  const back = fromDoc(toDoc(text, runs));
  assert.equal(back.text, text);
  assert.deepEqual(back.runs, runs);
});

test("attachments and line breaks keep their place", () => {
  const text = "a￼b c";
  const back = fromDoc(toDoc(text, [r(text.length)]));
  assert.equal(back.text, text);
});

test("without runs that fit, every line is plain and the text is kept", () => {
  const text = "One\nTwo\n";
  const back = fromDoc(toDoc(text, [r(2)]));
  assert.equal(back.text, text);
  assert.ok(back.runs.every((x) => x.style === null && !x.bold));
});

test("runs follow another device's edit merged in", () => {
  const out = carryRuns("abc\ndef", [r(3, { bold: true }), r(4)], "abcXY\ndef");
  assert.deepEqual(out.map((x) => [x.length, x.bold]), [[5, true], [4, false]]);
});

import { mergeEdits } from "../src/screens/notes/model.ts";
test("two edits merge as Notes merges them — nothing typed is lost", () => {
  assert.equal(mergeEdits("Tea filter\n", "Tea filter\nSpoon", "Tea filter\nKnife"), "Tea filter\nKnifeSpoon");
  assert.equal(mergeEdits("a b c", "a X c", "a Y c"), "a YX c");
  assert.equal(mergeEdits("milk\neggs\nbread\njam", "milk!\neggs\nbread\njam?", "milk\neggs\nbutter\njam"), "milk!\neggs\nbutter\njam?");
  assert.equal(mergeEdits("one two three", "one three", "one two three!"), "one three!");
});

import { checklistMove } from "../src/screens/notes/rich.ts";
// A note of lines, each "[ ] text", "[x] text", "  [ ] nested", or plain text.
const listDoc = (lines) => {
  const text = lines.map((l) => l.replace(/^\s*\[[ x]\] /, "")).join("\n");
  const runs = lines.map((l, i) => {
    const m = /^(\s*)\[([ x])\] /.exec(l);
    const len = l.replace(/^\s*\[[ x]\] /, "").length + (i < lines.length - 1 ? 1 : 0);
    return m ? r(len, { style: STYLE.checklist, checked: m[2] === "x", indent: m[1].length / 2 }) : r(len);
  });
  return toDoc(text, runs);
};

test("ticked, an item goes to the end of its own checklist, nested items with it", () => {
  const doc = listDoc(["Title", "[x] a", "[ ] b", "  [ ] b1", "[ ] c", "after"]);
  assert.deepEqual(checklistMove(doc, 1, true), { from: 1, to: 2, before: 5 });
  assert.deepEqual(checklistMove(doc, 2, true), { from: 2, to: 4, before: 5 });
});

test("ticked, the last item already at the end stays; a nested item never moves on its own", () => {
  const doc = listDoc(["[ ] a", "[ ] b", "  [ ] b1"]);
  assert.equal(checklistMove(doc, 1, true), null);
  assert.equal(checklistMove(doc, 2, true), null);
});

test("unticked, an item goes back above the ticked ones", () => {
  const doc = listDoc(["[ ] a", "[x] b", "[x] c", "[ ] d"]);
  assert.deepEqual(checklistMove(doc, 2, false), { from: 2, to: 3, before: 1 });
  assert.equal(checklistMove(doc, 0, false), null, "already above them");
});

test("only checklist lines move, and only within their own run", () => {
  const doc = listDoc(["[ ] a", "plain", "[ ] x", "[ ] y"]);
  assert.equal(checklistMove(doc, 1, true), null, "a plain line is not an item");
  assert.equal(checklistMove(doc, 0, true), null, "alone in its run, it stays");
  assert.deepEqual(checklistMove(doc, 2, true), { from: 2, to: 3, before: 4 });
});

test("the empty line after a checklist's closing newline is plain, not one more item", () => {
  // As a phone stores it: every checklist line, the last one too, ends with its own newline.
  const text = "Saturday\nFarmers market before 10\nCall Alex about the cabin\n";
  const runs = [r(9, { style: STYLE.title }), r(25, { style: STYLE.checklist, checked: false }), r(26, { style: STYLE.checklist, checked: false })];
  const doc = toDoc(text, runs);
  const last = doc.child(doc.childCount - 1);
  assert.equal(last.textContent, "");
  assert.equal(last.attrs.style, null, "no empty checklist item under the last one");
  assert.equal(doc.child(doc.childCount - 2).attrs.style, STYLE.checklist);
  const back = fromDoc(doc);
  assert.equal(back.text, text);
  // The two checklist lines come back as one run, as Apple's identical runs merge.
  assert.deepEqual(back.runs, [r(9, { style: STYLE.title }), r(51, { style: STYLE.checklist, checked: false })]);
});
