// Faults seen in the real window: a slow read of one view landing after another was chosen
// drew its photos under the other's title; Ctrl+A with the app list focused selected the
// page's text; the viewer showed the screen-sized picture, not the whole one.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await page.evaluate(() => { window.__slowView = "recents"; });
await page.locator(".px-source", { hasText: "Recents" }).click();
await page.locator(".px-source", { hasText: "Favorites" }).click();
await page.waitForTimeout(900);
ok((await page.locator(".px-bar h1").innerText()) === "Favorites" && (await page.locator(".px-tile").count()) === 1,
  `a slow read of another view does not take the screen  (${await page.locator(".px-tile").evaluateAll((t) => t.map((x) => x.dataset.id).join())} under Favorites)`);
await page.evaluate(() => { window.__slowView = null; });
// The same without the wait: from Recents (by date added) straight to Favorites, the months
// are each other's and never come twice.
await page.locator(".px-source", { hasText: "Recents" }).click();
await page.locator(".px-source", { hasText: "Favorites" }).click();
await page.waitForTimeout(400);
const titles = await page.locator(".px-month-title").allInnerTexts();
ok(new Set(titles).size === titles.length && (await page.locator(".px-tile").count()) === 1,
  `switching views at once leaves no month twice, nor another view's photos  ${JSON.stringify(titles)}`);
await page.locator(".px-source", { hasText: "Library" }).click();
await page.waitForTimeout(300);

await page.locator(".nav-item", { hasText: "Photos" }).focus();
await page.keyboard.press("Control+a");
await page.waitForTimeout(100);
ok((await page.evaluate(() => window.getSelection()?.toString() ?? "")) === "", "Ctrl+A with the app list focused selects no text");
ok((await page.locator('.px-tile[aria-selected="true"]').count()) === 5, "and selects every photo");
await page.keyboard.press("Escape");

await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(500);
ok(/full/.test((await page.locator(".px-sharp").getAttribute("src")) ?? ""), "an opened photo fetches the whole picture at once, not only when zoomed");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
