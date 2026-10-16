// Today with nothing due: no empty Morning / Afternoon / Tonight headings, only that there
// is nothing — as icloud.com shows it.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: "lists" });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-tile", { hasText: "Today" }).click();
await page.waitForTimeout(400);
const seen = await page.evaluate(() => ({
  headings: [...document.querySelectorAll(".rx-section")].map((h) => h.textContent.trim()),
  empty: document.querySelector(".rx-empty")?.textContent.trim() ?? null,
}));
const pass = seen.headings.length === 0 && seen.empty === "No Reminders";
console.log(pass ? "PASS an empty Today says No Reminders, with no empty headings"
  : `FAIL an empty Today: ${JSON.stringify(seen)}`);
await b.close();
process.exitCode = pass ? 0 : 1;
