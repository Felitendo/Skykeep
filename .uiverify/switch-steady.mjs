// Switching apps moves nothing: no layout shift once a screen has drawn, at Windows' 125%
// scaling and with a Windows user agent. A title measured as not fitting a frame after it drew
// moved every app's content up 6px on each switch, and its own height change measured it again,
// so on Windows the content shook. A long title still steps down a size; "Today" never does.
// Chromium only: the layout-shift record is Chromium's.
import { chromium } from "playwright";
import { SERVICES, installStub } from "./stub.mjs";
const b = await chromium.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0";
const ctx = await b.newContext({ viewport: { width: 1536, height: 864 }, deviceScaleFactor: 1.25, userAgent: UA });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true, remindersFixture: true, calendarFixture: true, contactsFixture: true });
await ctx.addInitScript(() => {
  window.__shifts = [];
  new PerformanceObserver((list) => { for (const e of list.getEntries()) window.__shifts.push(e.value); }).observe({ type: "layout-shift", buffered: true });
});
const page = await ctx.newPage();
await page.goto(process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await page.waitForTimeout(1200);
// Each app once, so every screen has been drawn; then round again, which is what a person does.
for (const s of ["Notes", "Calendar", "Contacts", "Drive", "Photos", "Reminders"]) {
  await page.locator(".nav-item", { hasText: s }).first().click();
  await page.waitForTimeout(700);
}
for (const s of ["Notes", "Calendar", "Contacts", "Drive", "Photos", "Reminders"]) {
  await page.evaluate(() => (window.__shifts = []));
  await page.locator(".nav-item", { hasText: s }).first().click();
  await page.waitForTimeout(900);
  const shifts = await page.evaluate(() => window.__shifts);
  ok(shifts.length === 0, `switching to ${s} moves nothing  ${JSON.stringify(shifts.map((v) => +v.toFixed(4)))}`);
}
// Reminders' "Today" keeps its full size.
const today = page.locator(".pane-title h1").first();
ok(!(await today.evaluate((el) => el.classList.contains("is-small"))), `a short title keeps its size  [${await today.innerText()}]`);
// A title too long for two lines steps down a size once its column narrows.
const long = await today.evaluate(async (el) => {
  el.textContent = "A list with a name long enough that two lines of it at the full size cannot hold it all, ".repeat(4);
  await new Promise((r) => setTimeout(r, 50));
  return el.classList.contains("is-small");
});
await page.setViewportSize({ width: 1200, height: 864 });
await page.waitForTimeout(300);
ok(await today.evaluate((el) => el.classList.contains("is-small")) || long, "a title too long for two lines steps down a size");
await b.close();
process.exitCode = failures ? 1 : 0;
