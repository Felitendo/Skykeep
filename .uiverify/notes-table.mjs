// A table in a note, drawn as its grid and edited in place: a typed cell takes typing as its
// own (the note's text is untouched), saves when left, and goes back with the reason if Apple
// refuses; an empty cell takes typing like any other.
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

const milk = page.locator(".nf-cell-edit", { hasText: "Milk" });
await milk.click();
await page.keyboard.press("End");
await page.keyboard.type(" (oat)");
await page.keyboard.press("Enter");
await page.waitForTimeout(100);
ok((await milk.textContent()) === "Milk (oat)", "a cell takes typing and shows it at once");
await page.waitForTimeout(400);
const writes = await page.evaluate(() => window.__cellWrites);
ok(writes?.length === 1 && writes[0].row === 1 && writes[0].column === 0 && writes[0].text === "Milk (oat)", `the cell, and only it, goes to Apple  ${JSON.stringify(writes)}`);
const saves = await page.evaluate(() => window.__noteSaves?.length ?? 0);
ok(saves === 0, "typing in a cell leaves the note's own text alone");

await page.evaluate(() => { window.__cellFail = true; });
const qty = page.locator(".nf-cell-edit", { hasText: "2" });
await qty.click();
await page.keyboard.press("End");
await page.keyboard.type("0");
await page.keyboard.press("Enter");
await page.waitForTimeout(500);
ok((await qty.textContent()) === "2" && /refused/.test((await qty.getAttribute("title")) ?? ""), "a change Apple refuses goes back, and says why");
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
ok(!/Attachment/.test(copied.text + copied.html) && /Item\tQty/.test(copied.text) && /<td>Milk \(oat\)<\/td>/.test(copied.html), `copying a table copies its cells  ${JSON.stringify(copied).slice(0, 300)}`);
// An empty cell takes typing too: tables made on a phone hold an empty text for every cell.
const empty = page.locator(".nf-table tr").nth(2).locator(".nf-cell-edit").first();
ok((await page.locator(".nf-cell-empty").count()) === 0 && (await empty.count()) === 1, "an empty cell is offered for editing");
await page.evaluate(() => { window.__cellFail = false; window.__cellWrites = []; });
await empty.click();
await page.keyboard.type("Bread");
await page.keyboard.press("Enter");
await page.waitForTimeout(400);
const w2 = await page.evaluate(() => window.__cellWrites);
ok((await empty.textContent()) === "Bread" && w2?.length === 1 && w2[0].row === 2 && w2[0].column === 0 && w2[0].text === "Bread", `typing in an empty cell saves it  ${JSON.stringify(w2)}`);
// Right-click a cell: a row added below it, as Notes adds one.
await page.locator(".nf-table tr").nth(1).locator("td").first().click({ button: "right" });
await page.getByRole("menuitem", { name: "Add Row Below" }).click();
await page.waitForTimeout(300);
const grown = await page.locator(".nf-table tr").evaluateAll((rows) => rows.map((r) => [...r.cells].map((c) => c.textContent)));
const asked = await page.evaluate(() => window.__rowAdds?.at(-1));
ok(grown.length === 4 && grown[2].every((c) => c === "") && asked?.at === 2, `Add Row Below adds an empty row under the one clicked  ${JSON.stringify(grown)} ${JSON.stringify(asked)}`);
await page.locator(".nf-table tr").first().locator("td").nth(1).click({ button: "right" });
await page.getByRole("menuitem", { name: "Add Column Before" }).click();
await page.waitForTimeout(300);
const wider = await page.locator(".nf-table tr").first().evaluate((r) => [...r.cells].map((c) => c.textContent));
ok(JSON.stringify(wider) === JSON.stringify(["Item", "", "Qty"]), `Add Column Before adds an empty column before the one clicked  ${JSON.stringify(wider)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
