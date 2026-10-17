// Get Info: a file's kind, size, place and dates, as icloud.com's File Info shows them.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "iCloud Drive");
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await page.locator(".drive-row", { hasText: "Semester Plan" }).hover();
await page.getByRole("button", { name: "Actions for Semester Plan.pdf" }).click();
await page.getByRole("menuitem", { name: "Get Info" }).click();
await page.waitForTimeout(200);
const facts = await page.evaluate(() => Object.fromEntries([...document.querySelectorAll(".info-rows > div")].map((d) => [d.querySelector("dt").textContent, d.querySelector("dd").textContent])));
ok(facts.Kind === "PDF document" && facts.Size === "2.3 MB" && facts.Where === "iCloud Drive", `File Info  ${JSON.stringify(facts)}`);
await page.keyboard.press("Escape");
await page.waitForTimeout(150);
ok((await page.locator(".info-sheet").count()) === 0, "Escape closes it");
// A right-click opens the same menu at the pointer; Escape puts it away.
await page.locator(".drive-row", { hasText: "Downloads" }).click({ button: "right" });
await page.waitForTimeout(150);
const offered = await page.getByRole("menuitem").allInnerTexts();
ok(JSON.stringify(offered) === JSON.stringify(["Open", "Get Info", "Rename", "Move to Folder…", "Delete"]),
  `a right-click on a folder offers its menu  ${JSON.stringify(offered)}`);
await page.keyboard.press("Escape");
await page.waitForTimeout(100);
ok((await page.getByRole("menu").count()) === 0, "and Escape puts it away");
await b.close();
process.exitCode = failures ? 1 : 0;
