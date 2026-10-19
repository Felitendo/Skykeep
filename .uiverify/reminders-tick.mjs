// Ticking a reminder is instant, as on the phone: the circle fills at once, the row stays a
// moment and then leaves, never waiting on Apple; unticking in that moment keeps it, and both
// ticks reach Apple in order.
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
await page.evaluate(() => { window.__doneDelay = 4000; });
const row = (t) => page.locator(".rx-row", { has: page.getByLabel(`Rename ${t}`) });

await row("Pick up the parcel").locator(".rx-tick").click();
await page.waitForTimeout(80);
ok((await row("Pick up the parcel").locator(".rx-tick").getAttribute("aria-checked")) === "true", "the circle fills at once, while Apple is still answering");
await page.waitForTimeout(1500);
ok((await row("Pick up the parcel").count()) === 0, "and the row leaves after a moment, before Apple has answered");

await row("Plan tomorrow").locator(".rx-tick").click();
await page.waitForTimeout(300);
await row("Plan tomorrow").locator(".rx-tick").click();
await page.waitForTimeout(1600);
ok((await row("Plan tomorrow").count()) === 1 && (await row("Plan tomorrow").locator(".rx-tick").getAttribute("aria-checked")) === "false", "unticked in that moment, it stays");
await page.waitForTimeout(9000);
const sent = await page.evaluate(() => window.__done.filter((d) => d.guid === "r1").map((d) => d.done));
ok(JSON.stringify(sent) === JSON.stringify([true, false]), `and Apple gets both ticks, in order  ${JSON.stringify(sent)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
