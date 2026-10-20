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

// Switching views, frame by frame: the title never stands over another view's photos, a photo
// in both views keeps its picture (no reload, no flash), and the new view starts at its top.
await page.locator(".px-scroll").evaluate((e) => (e.scrollTop = 200));
await page.evaluate(() => {
  window.__frames = [];
  document.querySelector('.px-tile[data-id="p1"] img').__kept = true;
  const tick = () => {
    window.__frames.push({ title: document.querySelector(".px-bar h1")?.textContent, ids: [...document.querySelectorAll(".px-tile")].map((t) => t.dataset.id).join() });
    if (window.__frames.length < 40) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
});
await page.locator(".px-source", { hasText: "Favorites" }).click();
await page.waitForTimeout(900);
const frames = await page.evaluate(() => window.__frames);
const wrong = frames.filter((f) => f.title === "Favorites" && f.ids !== "p1");
ok(wrong.length === 0, `no frame shows Favorites over other photos  (${wrong.length} of ${frames.length})`);
ok(await page.evaluate(() => document.querySelector('.px-tile[data-id="p1"] img').__kept === true), "a photo in both views keeps its picture across the switch");
ok((await page.locator(".px-scroll").evaluate((e) => e.scrollTop)) === 0, "and the new view starts at its top");
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
// Media types as icloud.com lists them: only the kinds the library has — a panorama appears
// once there is one (Apple's assetSubtypeV2 1); no empty ones.
ok((await page.locator(".px-source", { hasText: "Panoramas" }).count()) === 0, "no Panoramas in the sidebar while there are none");
await page.evaluate(() => { window.__photos.find((p) => p.id === "p4").subtype = 1; });
await page.locator(".px-source", { hasText: "Recents" }).first().click();
await page.waitForTimeout(300);
await page.locator(".px-source", { hasText: "Library" }).first().click();
await page.waitForTimeout(400);
ok(/Panoramas\s*1/.test(await page.locator(".px-source", { hasText: "Panoramas" }).innerText()), "and there it is once there is one, with its count");
await page.locator(".px-source", { hasText: "Panoramas" }).click();
await page.waitForTimeout(300);
ok((await page.locator(".px-tile").evaluateAll((t) => t.map((x) => x.dataset.id).join())) === "p4", "holding the panorama");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
