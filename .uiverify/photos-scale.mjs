// Photos at the size of a real library — twenty thousand: only the months near the view are
// built, a month not yet built is exactly as tall as it will be (nothing jumps), a click that
// selects stays quick, and the arrows keep going into months that were not built.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(() => {
  const P = (id, t) => ({ id, master: `m${id}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024, duration: null,
    filename: `IMG_${id}.JPG`, size: 2400000, uti: "public.jpeg", favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  window.__photos = Array.from({ length: 20000 }, (_, i) => P(`q${i}`, 1792600000 - i * 40000));
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await openScreen(page, "Photos");
await page.locator(".px-tile").first().waitFor();
await page.waitForTimeout(500);

const built = await page.locator(".px-tile").count();
ok(built > 0 && built < 1500, `only the months near the view are built  (${built} of 20000 tiles)`);

const sizes = await page.evaluate(async () => {
  const months = [...document.querySelectorAll(".px-month")];
  const later = months.find((s) => s.querySelector(".px-later"));
  const before = later.getBoundingClientRect().height;
  later.scrollIntoView();
  await new Promise((r) => setTimeout(r, 400));
  return { before, after: later.getBoundingClientRect().height, built: !later.querySelector(".px-later") };
});
ok(sizes.built && Math.abs(sizes.before - sizes.after) < 1,
  `a month is as tall before it is built as after  ${sizes.before.toFixed(1)} → ${sizes.after.toFixed(1)}px`);

// The same with whole photos, whose rows sit further apart.
await page.locator(".px-scroll").evaluate((e) => (e.scrollTop = 0));
await page.getByRole("button", { name: "Show Whole Photos" }).click();
await page.waitForTimeout(400);
const fitted = await page.evaluate(async () => {
  const months = [...document.querySelectorAll(".px-month")];
  const later = months.find((s) => s.querySelector(".px-later"));
  const before = later.getBoundingClientRect().height;
  later.scrollIntoView();
  await new Promise((r) => setTimeout(r, 400));
  return { before, after: later.getBoundingClientRect().height, built: !later.querySelector(".px-later") };
});
ok(fitted.built && Math.abs(fitted.before - fitted.after) < 1,
  `with whole photos too  ${fitted.before.toFixed(1)} → ${fitted.after.toFixed(1)}px`);
await page.getByRole("button", { name: "Show Square Photos" }).click();
await page.waitForTimeout(200);
await page.locator(".px-scroll").evaluate((e) => (e.scrollTop = 0));
await page.waitForTimeout(300);
const ms = await page.evaluate(async () => {
  const t = performance.now();
  document.querySelectorAll(".px-tile")[3].click();
  await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  return performance.now() - t;
});
ok(ms < 150, `a click that selects stays quick  (${Math.round(ms)}ms, two frames included)`);

// Down, row after row, past the months that were built when it began — after a real click,
// with the app list holding the focus first, as it does once Photos is chosen from it.
await page.locator(".nav-item", { hasText: "Photos" }).focus();
await page.locator(".px-tile").nth(3).click();
const start = await page.locator('.px-tile[aria-selected="true"]').getAttribute("data-index");
for (let i = 0; i < 60; i++) await page.keyboard.press("ArrowDown");
await page.waitForTimeout(300);
const end = await page.locator('.px-tile[aria-selected="true"]').getAttribute("data-index");
ok(Number(end) > built, `the down arrow keeps going into months not built at first  (tile ${start} → ${end}, ${built} built at first)`);
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
