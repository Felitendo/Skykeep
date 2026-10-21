import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const S = "/tmp/claude-1000/-home-yash-Projects/2540edda-3efd-42b2-8acc-a7662fcef7d7/scratchpad";
const tag = process.argv[2] || "before";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 }, colorScheme: process.argv[3] || "light" });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => console.log("pageerror", e.message));
await page.goto("http://localhost:4181", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
for (const v of ["Month", "Week", "Day", "Year"]) {
  await page.getByRole("button", { name: v, exact: true }).click();
  await page.waitForTimeout(400);
  await page.screenshot({ path: `${S}/${tag}-${engineName}-${v}.png` });
}
await page.getByRole("button", { name: "Week", exact: true }).click();
await page.waitForTimeout(300);
await page.locator(".cx-block", { hasText: "Design review" }).first().click();
await page.waitForTimeout(300);
await page.screenshot({ path: `${S}/${tag}-${engineName}-pop.png` });
await b.close();
