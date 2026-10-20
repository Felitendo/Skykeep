// Closing a photo never flashes it: frame by frame the photo only shrinks back into its tile,
// and no frame after it arrives shows it larger again before the viewer is gone. (WebKit drew
// the photo at full size for one frame as the animation ended — a whole-window flash.)
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

for (const round of [1, 2]) {
  await page.locator(".px-tile").nth(round).dblclick();
  await page.waitForTimeout(700);
  const trace = await page.evaluate(() => new Promise((resolve) => {
    const frames = [];
    const tick = () => {
      const v = document.querySelector(".px-viewer");
      const p = document.querySelector(".px-pan, .px-video");
      if (!v || !p) { frames.push(null); return resolve(frames); }
      const shown = getComputedStyle(v).visibility !== "hidden";
      frames.push(shown ? Math.round(p.getBoundingClientRect().width) : 0);
      if (frames.length < 120) requestAnimationFrame(tick); else resolve(frames);
    };
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    requestAnimationFrame(tick);
  }));
  const seen = trace.filter((w) => w !== null && w > 0);
  const grew = seen.some((w, i) => i > 0 && w > seen[i - 1] + 1);
  ok(!grew && trace.at(-1) === null, `close ${round}: the photo only shrinks, then the viewer is gone — no frame drawn larger  ${JSON.stringify(seen)}`);
  await page.waitForTimeout(300);
}
// No frame can flash however the engine starts or ends an animation: the backdrop's own
// style is its first frame (hidden) the moment the viewer appears, its last (shown) once the
// fade runs; closing, its own style is gone (hidden) while it fades out.
const own = await page.evaluate(() => new Promise((resolve) => {
  const seen = {};
  new MutationObserver((_, mo) => {
    const bd = document.querySelector(".px-backdrop");
    if (bd && seen.first === undefined) {
      seen.first = bd.style.opacity;
      mo.disconnect();
      document.getAnimations().forEach((a) => a.ready.then(() => { seen.running = bd.style.opacity; }));
      setTimeout(() => resolve(seen), 400);
    }
  }).observe(document.body, { childList: true, subtree: true });
  document.querySelectorAll(".px-tile")[2].dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
}));
ok(own.first === "0" && own.running === "1", `opening: the backdrop's own style is hidden before its fade, shown once it runs  ${JSON.stringify(own)}`);
await page.waitForTimeout(500);
const closingOwn = await page.evaluate(() => new Promise((resolve) => {
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  const bd = document.querySelector(".px-backdrop");
  Promise.all(bd.getAnimations().map((a) => a.ready)).then(() => resolve(bd.style.opacity));
}));
ok(closingOwn === "0", `closing: its own style is already gone while it fades  ${JSON.stringify(closingOwn)}`);
await page.waitForTimeout(700);

// The window's own colour follows what fills its edges, so a resize shows no lighter band.
const rgb = (sel) => page.evaluate((s) => /(\d+), (\d+), (\d+)/.exec(getComputedStyle(document.querySelector(s)).backgroundColor).slice(1).join(","), sel);
const body = await rgb("body");
ok((await page.evaluate(() => window.__tints?.at(-1))) === body, `the window is the app's colour  ${body}`);
await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(600);
const backdrop = await rgb(".px-backdrop");
ok((await page.evaluate(() => window.__tints.at(-1))) === backdrop, `the viewer's own colour while a photo is open  ${backdrop}`);
await page.keyboard.press("Escape");
await page.waitForTimeout(600);
ok((await page.evaluate(() => window.__tints.at(-1))) === body, "and the app's colour again once it closes");

// Opened again at once after a close: shown, not left hidden.
await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(600);
ok(await page.locator(".px-viewer").evaluate((v) => getComputedStyle(v).visibility === "visible" && getComputedStyle(v.querySelector(".px-backdrop")).opacity === "1"), "a photo opened again is fully shown");
// In dark, the viewer's near-black: the band a resize shows is never lighter than the photo's surround.
const dark = await b.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: "dark" });
await dark.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const dp = await dark.newPage();
await dp.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(dp, "Photos");
await dp.waitForTimeout(400);
await dp.locator(".px-tile").nth(1).dblclick();
await dp.waitForTimeout(600);
const tint = await dp.evaluate(() => window.__tints.at(-1));
ok(tint.split(",").every((v) => Number(v) < 20), `in dark, the window goes near-black with the viewer  ${tint}`);
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
