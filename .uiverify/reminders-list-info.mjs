// Show List Info: Apple's twelve colours; choosing one shows at once and is sent as its
// CloudKit name.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await page.locator(".rx-list", { hasText: "Daily" }).hover();
await page.getByRole("button", { name: "Show List Info for Daily" }).click();
await page.waitForTimeout(200);
ok((await page.getByRole("radio").count()) === 12, "Apple's twelve colours are offered");
await page.getByRole("radio", { name: "purple" }).click();
await page.getByRole("button", { name: "Done" }).click();
await page.waitForTimeout(150);
const dot = await page.locator(".rx-list", { hasText: "Daily" }).evaluate((el) => el.outerHTML.toLowerCase().includes("c077db") || el.outerHTML.includes("192, 119, 219"));
ok(dot, "the list takes the colour at once");
await page.waitForTimeout(300);
const sent = await page.evaluate(() => window.__listColours);
ok(JSON.stringify(sent) === JSON.stringify([["l1", "purple"]]), `sent by its CloudKit name  ${JSON.stringify(sent)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
