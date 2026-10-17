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
// A tap on a ticked circle unticks it at once, and asks Apple to.
const circle = page.locator(".nf-check .nf-circle").first();
await circle.click();
await page.waitForTimeout(150);
ok((await circle.getAttribute("aria-checked")) === "false", "a tap on a ticked circle unticks it at once");
const sent = await page.evaluate(() => window.__paragraphs?.at(-1));
ok(sent?.checked === false && typeof sent?.start === "number" && sent.start > 0, `and asks Apple for that line  ${JSON.stringify(sent)}`);
ok(await page.evaluate(() => document.activeElement?.tagName !== "TEXTAREA"), "a tick does not start editing");
await page.locator(".nf-body, .nf-item:not(.nf-check)").first().click();
await page.waitForTimeout(200);
ok(await page.evaluate(() => document.activeElement?.classList.contains("note-text") && document.activeElement.tagName === "TEXTAREA"),
  "a click into it is editing");
// Aa: a line clicked, then Heading — shown at once, and asked of Apple for that line alone.
await page.keyboard.press("Escape");
await page.locator(".note-row", { hasText: "Curtains" }).click();
await page.waitForTimeout(300);
const fan = page.locator(".note-read [data-start]", { hasText: "Fan" }).first();
await fan.click();
await page.waitForTimeout(200);
await page.getByRole("button", { name: "Format" }).click();
await page.getByRole("menuitem", { name: "Heading", exact: true }).click();
await page.waitForTimeout(300);
ok((await page.locator(".nf-heading", { hasText: "Fan" }).count()) === 1, "Aa → Heading shows the line as a heading at once");
const styled = await page.evaluate(() => window.__paragraphs?.at(-1));
ok(styled?.style === 1 && styled?.expect === "Fan", `and asks Apple for that line alone  ${JSON.stringify(styled)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
