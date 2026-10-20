// A photo at full quality from the first frame: once its whole picture is decoded — as it is
// fetched before it opens, and for the photos either side while one shows — the viewer shows
// that alone, never a smaller copy first; stepping lands on a whole picture too.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
// Pictures here cannot be fetched; decoding is made to succeed, as it does in the app.
await ctx.addInitScript(() => { HTMLImageElement.prototype.decode = function () { return Promise.resolve(); }; });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const shown = () => page.locator(".px-stage .px-shown").getAttribute("src");

await page.locator(".px-tile").nth(0).dblclick();
await page.waitForTimeout(80);
ok(/full/.test((await shown()) ?? "") && (await page.locator(".px-sharp").count()) === 0, `it opens on the whole picture, alone  ${await shown()}`);
await page.keyboard.press("ArrowRight");
await page.waitForTimeout(30);
ok(/p2.*full|full.*p2/.test(decodeURIComponent((await shown()) ?? "")), `a step lands on the next whole picture at once  ${await shown()}`);
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
