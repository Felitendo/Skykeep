// The app never looks stuck: a view whose photos are slow to come has its title up at once and
// its tiles as plain boxes until they do; one that comes quickly shows no boxes at all (nothing
// to flash); and a month far down, not built yet, is plain boxes, never an empty page.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(() => {
  const P = (i, t) => ({ id: `l${i}`, master: `ml${i}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024, duration: null, filename: `I${i}`, size: 1, uti: "public.jpeg", favorite: i % 3 === 0, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  const out = []; let i = 0;
  for (let m = 0; m < 30; m++) for (let k = 0; k < 14; k++) out.push(P(i++, Date.UTC(2026, 9 - m, 20 - k) / 1000));
  window.__photos = out;
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(500);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

// A month far down, not built: boxes, laid out on the grid.
const far = await page.evaluate(() => {
  const later = [...document.querySelectorAll(".px-later")];
  return { months: later.length, boxes: later.reduce((n, l) => n + l.querySelectorAll(".px-ph").length, 0) };
});
ok(far.months > 0 && far.boxes === far.months * 14, `months not yet built show their tiles as boxes  (${far.months} months, ${far.boxes} boxes)`);

// A slow view: title at once, boxes, then the photos.
await page.evaluate(() => { window.__slowView = "favorites"; });
await page.locator(".px-source", { hasText: "Favorites" }).first().click();
await page.waitForTimeout(250);
ok((await page.locator(".px-bar h1").innerText()) === "Favorites" && (await page.locator(".px-month[aria-busy] .px-ph").count()) > 0 && (await page.locator(".px-tile").count()) === 0,
  "a slow view has its title up at once and its tiles as boxes, not the last view's photos");
await page.waitForTimeout(700);
ok((await page.locator(".px-month[aria-busy]").count()) === 0 && (await page.locator(".px-tile").count()) > 0, "and its photos take their place when they come");

// A quick view: never a box, frame by frame.
await page.evaluate(() => { window.__slowView = null; });
const boxed = await page.evaluate(() => new Promise((resolve) => {
  let seen = false;
  const t0 = performance.now();
  const tick = () => { if (document.querySelector(".px-month[aria-busy]")) seen = true; if (performance.now() - t0 < 600) requestAnimationFrame(tick); else resolve(seen); };
  requestAnimationFrame(tick);
  [...document.querySelectorAll(".px-source")].find((e) => /Library/.test(e.textContent)).click();
}));
ok(!boxed, "a view that comes quickly never shows the boxes — nothing flashes");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
