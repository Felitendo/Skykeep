// A window opened as one app (`?app=…`, from its launcher entry or the tray) opens on that app,
// with everything else still there: the other apps, search, Settings and the account.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const [app, marker] of [["notes", ".note-rows"], ["reminders", ".rx-list"], ["calendar", ".cx-side, .cx-month, .cx-grid"]]) {
  const ctx = await b.newContext({ viewport: { width: 1040, height: 700 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true, calendarFixture: true, remindersFixture: true });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto(`http://localhost:4173/?app=${app}`, { waitUntil: "networkidle" });
  await page.waitForTimeout(800);
  ok((await page.locator(`main.content[data-screen="${app}"]`).count()) === 1 && (await page.locator(marker).count()) > 0, `${app}: the window is that app`);
  ok(await page.locator("nav.sidebar").isVisible() && await page.getByRole("combobox", { name: "Search everything" }).isVisible(),
    `${app}: with the other apps, search and Settings still there`);
  await ctx.close();
}
const ctx = await b.newContext({ viewport: { width: 1040, height: 700 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
await page.goto("http://localhost:4173/", { waitUntil: "networkidle" });
await page.waitForTimeout(600);
ok(await page.locator("nav.sidebar").isVisible(), "without ?app, the window is the whole library, its app list there");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
