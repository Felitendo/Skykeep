// A table in a note, drawn as its grid, its text colour shown, and copied as its cells.
// Editing is off until an edit made here is seen on an iPhone.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(() => { window.__tableNote = true; });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await page.locator(".note-row", { hasText: "Groceries" }).first().click();
await page.waitForSelector(".nf-table");
const grid = await page.locator(".nf-table tr").evaluateAll((rows) => rows.map((r) => [...r.cells].map((c) => c.textContent)));
ok(JSON.stringify(grid) === JSON.stringify([["Item", "Qty"], ["Milk", "2"], ["", ""]]), `the table is drawn as its grid  ${JSON.stringify(grid)}`);

// Read-only for now: Quince's table edits were stored at Apple but did not show on the
// iPhone, so no cell is offered for typing and no table menu opens.
ok((await page.locator(".nf-cell-edit").count()) === 0, "no table cell is offered for typing while table writes are off");
await page.locator(".nf-table tr").nth(1).locator("td").first().click({ button: "right" });
ok((await page.getByRole("menuitem", { name: /Add Row|Delete Row/ }).count()) === 0, "and right-click offers no row or column changes");
await page.keyboard.press("Escape");
const coloured = await page.locator(".note-rich .nf-em-4").evaluateAll((e) => e.map((x) => [x.textContent, getComputedStyle(x).color]));
ok(coloured.length === 1 && coloured[0][0] === "end" && !/rgb\(29, 29, 31\)|rgb\(0, 0, 0\)/.test(coloured[0][1]), `text coloured on a phone shows in its colour  ${JSON.stringify(coloured)}`);
// Copying the note puts the table's cells out, never a placeholder word.
const copied = await page.evaluate(() => {
  const ed = document.querySelector(".note-rich");
  ed.focus();
  const range = document.createRange();
  range.selectNodeContents(ed);
  getSelection().removeAllRanges();
  getSelection().addRange(range);
  return new Promise((done) => setTimeout(() => {
    const data = new DataTransfer();
    ed.dispatchEvent(new ClipboardEvent("copy", { clipboardData: data, bubbles: true, cancelable: true }));
    done({ text: data.getData("text/plain"), html: data.getData("text/html") });
  }, 50));
});
ok(!/Attachment/.test(copied.text + copied.html) && /Item\tQty/.test(copied.text) && /<td>Milk<\/td>/.test(copied.html), `copying a table copies its cells  ${JSON.stringify(copied).slice(0, 300)}`);
// Pasting a copied table never makes a second reference to it: the save holds it once.
await page.evaluate(() => {
  const ed = document.querySelector(".note-rich");
  ed.focus();
  const range = document.createRange();
  range.selectNodeContents(ed);
  getSelection().removeAllRanges();
  getSelection().addRange(range);
});
await page.waitForTimeout(50);
await page.evaluate(() => {
  const ed = document.querySelector(".note-rich");
  const data = new DataTransfer();
  ed.dispatchEvent(new ClipboardEvent("copy", { clipboardData: data, bubbles: true, cancelable: true }));
  const last = ed.lastElementChild;
  const range = document.createRange();
  range.selectNodeContents(last);
  range.collapse(false);
  getSelection().removeAllRanges();
  getSelection().addRange(range);
  window.__pasteData = data;
});
await page.waitForTimeout(80);
await page.evaluate(() => {
  const ed = document.querySelector(".note-rich");
  ed.dispatchEvent(new ClipboardEvent("paste", { clipboardData: window.__pasteData, bubbles: true, cancelable: true }));
});
await page.waitForTimeout(1500);
const pasted = await page.evaluate(() => window.__noteSaves?.at(-1));
const refs = (pasted?.runs ?? []).filter((r) => r.attachment === "T1").length;
ok(pasted && refs === 1 && (pasted.body.match(/\uFFFC/g) ?? []).length === 1 && /Item\tQty\nMilk\t2/.test(pasted.body), `a pasted table brings its cells as text, never a second reference  refs=${refs} ${JSON.stringify(pasted?.body)}`);
// Typed in on a phone while the note is open: the next sync redraws the table, without
// leaving the note.
await page.evaluate(() => {
  window.__table[2] = ["Bread", "1"];
  window.__emit("quince://synced", { services: [], asOf: Date.now() / 1000, tookMs: 5 });
});
await page.waitForTimeout(500);
const after = await page.locator(".nf-table tr").nth(2).evaluate((r) => [...r.cells].map((c) => c.textContent));
ok(JSON.stringify(after) === JSON.stringify(["Bread", "1"]), `a table changed elsewhere redraws after the next sync  ${JSON.stringify(after)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
