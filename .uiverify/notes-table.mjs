// A table in a note, drawn as its grid, its text colour shown, and copied as its cells. Its
// cells are edited where they stand, and right-click adds and deletes rows and columns, as
// Notes does (each made here showed on an iPhone). A table this app cannot read says so where
// it stands — never a file to open.
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

// A cell typed into is saved to Apple, that cell alone.
const empty = page.locator(".nf-table tr").nth(2).locator(".nf-cell-edit").first();
ok((await empty.count()) === 1, "a table's cells are offered for typing, an empty one too");
await empty.click();
await page.keyboard.type("Eggs");
await page.keyboard.press("Enter");
await page.waitForTimeout(500);
const wrote = await page.evaluate(() => window.__cellWrites ?? []);
ok(wrote.length === 1 && wrote[0].id === "T1" && wrote[0].row === 2 && wrote[0].column === 0 && wrote[0].text === "Eggs",
  `typed into a cell, that cell is saved  ${JSON.stringify(wrote)}`);
await page.evaluate(() => { window.__table[2] = ["", ""]; });
const shape = () => page.locator(".nf-table tr").evaluateAll((rows) => [rows.length, rows[0]?.cells.length ?? 0]);
const menu = async (r, c, name) => {
  await page.locator(".nf-table tr").nth(r).locator("td").nth(c).click({ button: "right" });
  await page.waitForTimeout(100);
  await page.getByRole("menuitem", { name, exact: true }).click();
  await page.waitForTimeout(300);
};
await menu(1, 0, "Add Row Below");
ok(JSON.stringify(await shape()) === "[4,2]" && JSON.stringify(await page.evaluate(() => window.__rowAdds)) === JSON.stringify([{ id: "T1", at: 2, column: false }]),
  `Add Row Below adds a row under that one, at Apple  ${JSON.stringify(await page.evaluate(() => window.__rowAdds))}`);
await menu(0, 1, "Add Column After");
ok(JSON.stringify(await shape()) === "[4,3]", "Add Column After adds a column beside it");
await menu(0, 2, "Delete Column");
await menu(2, 0, "Delete Row");
ok(JSON.stringify(await shape()) === "[3,2]" && (await page.evaluate(() => window.__removes?.length)) === 2, "Delete Row and Delete Column take them away again");
await page.evaluate(() => { window.__table = [["Item", "Qty"], ["Milk", "2"], ["", ""]]; });
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

// A table this app cannot read: said so where it stands, never a file chip to open.
const ctx2 = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx2.addInitScript(() => { window.__tableNote = true; window.__unreadTable = true; });
await ctx2.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const p2 = await ctx2.newPage();
await p2.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(p2, "Notes");
await p2.locator(".note-row", { hasText: "Groceries" }).first().click();
await p2.waitForTimeout(600);
ok((await p2.locator(".nf-table-unread").count()) === 1 && (await p2.locator(".nf-file").count()) === 0,
  `a table that can't be read says so, and is no file to open  ${await p2.locator(".nf-attachment").first().innerText().catch(() => "")}`);
await b.close();
process.exitCode = failures ? 1 : 0;
