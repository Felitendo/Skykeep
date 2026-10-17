// A note formatted on a phone shows formatted here: its checklist with ticks, its dashed
// list; a click into it is editing.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.locator(".note-row", { hasText: "Curtains" }).click();
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ticked = await page.locator(".nf-check.is-done").count();
const marks = await page.locator(".nf-mark").allInnerTexts();
ok(ticked === 1, `a ticked checklist item shows ticked  [${ticked}]`);
ok(marks.length > 0 && marks.every((m) => m === "–"), `a dashed list shows its dashes  ${JSON.stringify(marks.slice(0, 3))}`);
await page.locator(".note-read").click();
await page.waitForTimeout(200);
ok(await page.evaluate(() => document.activeElement?.classList.contains("note-text") && document.activeElement.tagName === "TEXTAREA"),
  "a click into it is editing");
await b.close();
process.exitCode = failures ? 1 : 0;
