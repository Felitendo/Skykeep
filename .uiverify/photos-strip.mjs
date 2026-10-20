// The viewer's filmstrip, as icloud.com's: the photos either side as slips, the shown one wider
// in the middle; a click goes to it and the strip glides to keep it in the middle; in a library
// of thousands only the nearest are built.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(() => {
  const P = (id, t) => ({ id, master: `m${id}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024, duration: null,
    filename: `IMG_${id}.JPG`, size: 1, uti: "public.jpeg", favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  window.__photos = Array.from({ length: 2000 }, (_, i) => P(`q${i}`, 1792600000 - i * 40000));
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(500);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const centred = () => page.evaluate(() => {
  const s = document.querySelector(".px-slip.is-shown").getBoundingClientRect(), st = document.querySelector(".px-strip").getBoundingClientRect();
  return Math.abs(s.left + s.width / 2 - (st.left + st.width / 2));
});

await page.locator(".px-tile").nth(40).dblclick();
await page.waitForTimeout(800);
ok((await page.locator(".px-slip").count()) <= 61 && (await page.locator(".px-slip").count()) > 20, `only the nearest slips are built  (${await page.locator(".px-slip").count()} of 2000)`);
ok((await centred()) <= 1, "the shown photo's slip sits in the middle");
await page.locator(".px-slip").nth(25).click();
await page.waitForTimeout(700);
const shown = await page.locator(".px-shown").getAttribute("src");
const slip = await page.locator(".px-slip.is-shown img").getAttribute("src");
ok(shown.split("pic%2F")[1]?.split("%2F")[0] === slip.split("pic%2F")[1]?.split("%2F")[0], "a click on a slip goes to that photo");
ok((await centred()) <= 1, "and the strip glides to keep it in the middle");
for (let i = 0; i < 40; i++) await page.keyboard.press("ArrowRight");
await page.waitForTimeout(700);
ok((await centred()) <= 1 && (await page.locator(".px-slip").count()) <= 61, "stepping far along, still in the middle and still few");
// A wheel or a two-finger swipe over the strip skims: four notches of 40 go four photos on,
// and the slips glide on the compositor, never jump.
const which = async () => (await page.locator(".px-slip.is-shown img").getAttribute("src")).split("pic%2F")[1].split("%2F")[0];
const before = await which();
const strip = await page.locator(".px-strip").boundingBox();
await page.mouse.move(strip.x + strip.width / 2, strip.y + strip.height / 2);
for (let i = 0; i < 4; i++) { await page.mouse.wheel(0, 40); await page.waitForTimeout(60); }
await page.waitForTimeout(700);
const after = await which();
const ids = await page.evaluate(() => window.__photos.map((p) => p.id));
ok(ids.indexOf(after) - ids.indexOf(before) === 4, `scrolling over the strip skims through the photos  (${ids.indexOf(before)} → ${ids.indexOf(after)})`);
ok((await centred()) <= 1, "and the strip keeps the shown one in the middle");
ok(await page.locator(".px-slip").first().evaluate((e) => getComputedStyle(e).transform !== "none" && /transform/.test(getComputedStyle(e).transitionProperty)),
  "slips are placed and eased by transform");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
