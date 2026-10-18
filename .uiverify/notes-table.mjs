// A table in a note, drawn as its grid and edited in place: a typed cell takes typing as its
// own (the note's text is untouched), saves when left, and goes back with the reason if Apple
// refuses; a cell never typed in says it cannot be changed here yet.
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
ok((await page.locator(".nf-cell-empty").count()) === 2, "a cell never typed in is shown but not offered for editing");
await b.close();
process.exitCode = failures ? 1 : 0;
