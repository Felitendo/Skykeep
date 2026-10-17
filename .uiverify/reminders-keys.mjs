// Reminders' keys, as on a Mac: Tab tucks a reminder under the one above, at once;
// Shift-Tab brings it back out; Return goes on to a new reminder.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const row = () => page.locator(".rx-row", { has: page.getByLabel("Rename Pick up the parcel") });

await page.getByLabel("Rename Pick up the parcel").focus();
await page.keyboard.press("Tab");
await page.waitForTimeout(150);
ok(await row().evaluate((el) => el.classList.contains("is-subtask")), "Tab tucks a reminder under the one above, at once");
let sent = await page.evaluate(() => window.__edits?.at(-1));
ok(typeof sent?.parentGuid === "string", `and asks Apple to  ${JSON.stringify(sent)}`);

await page.getByLabel("Rename Pick up the parcel").focus();
await page.keyboard.press("Shift+Tab");
await page.waitForTimeout(150);
ok(!(await row().evaluate((el) => el.classList.contains("is-subtask"))), "Shift-Tab brings it back out");
sent = await page.evaluate(() => window.__edits?.at(-1));
ok(sent && sent.parentGuid === null, `and asks Apple to  ${JSON.stringify(sent)}`);

await page.getByLabel("Rename Pick up the parcel").focus();
await page.keyboard.press("Enter");
await page.waitForTimeout(150);
ok(await page.evaluate(() => document.activeElement?.closest(".rx-add") !== null), "Return goes on to a new reminder");
await b.close();
process.exitCode = failures ? 1 : 0;
