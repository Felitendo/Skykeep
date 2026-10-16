// Calendar's Mac shortcuts, Ctrl for ⌘: 1–4 switch the view, T goes to today, N starts an event.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.waitForTimeout(300);
let failures = 0;
const ok = (pass, msg) => { console.log(`${pass ? "PASS" : "FAIL"} ${msg}`); if (!pass) failures++; };
for (const [key, name] of [["1", "Day"], ["2", "Week"], ["3", "Month"], ["4", "Year"]]) {
  await page.keyboard.press(`Control+${key}`);
  await page.waitForTimeout(200);
  const now = await page.evaluate(() => [...document.querySelectorAll("button")].find((x) =>
    ["Day", "Week", "Month", "Year"].includes(x.textContent.trim()) &&
    (x.getAttribute("aria-pressed") === "true" || x.getAttribute("aria-selected") === "true" || x.className.includes("is-"))
  )?.textContent.trim());
  ok(now === name, `Ctrl+${key} shows ${name}  [${now}]`);
}
await page.keyboard.press("Control+2");
await page.waitForTimeout(200);
await page.keyboard.press("Control+n");
await page.waitForTimeout(400);
ok(await page.locator(".cx-block.is-draft").count() === 1, "Ctrl+N starts a new event");
await b.close();
process.exitCode = failures ? 1 : 0;
