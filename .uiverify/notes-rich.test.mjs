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
