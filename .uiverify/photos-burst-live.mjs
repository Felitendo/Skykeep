// A burst shows as its chosen photo; in the viewer its BURST mark goes through all of it —
// "Burst · 2 of 3" — and Done (or closing) brings the library back on the chosen one. A Live
// Photo's LIVE mark plays its moving part over the still while the pointer rests on it.
import http from "node:http";
import fs from "node:fs";
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
// The moving part: a short clip from a server of the app's kind.
const clip = new URL("./clip.webm", import.meta.url);
const len = fs.statSync(clip).size;
const server = http.createServer((req, res) => {
  const m = /bytes=(\d*)-(\d*)/.exec(req.headers.range ?? "");
  const f = m && m[1] ? +m[1] : 0, l = m && m[2] ? Math.min(+m[2], len - 1) : len - 1;
  res.writeHead(m ? 206 : 200, { "Content-Type": "video/webm", "Accept-Ranges": "bytes", "Content-Length": l - f + 1, "Content-Range": `bytes ${f}-${l}/${len}` });
  fs.createReadStream(clip, { start: f, end: l }).pipe(res);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/s3cret`;
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript((v) => {
  window.__videoBase = v;
  // p2 is a burst's chosen photo (of 3), p4 a Live Photo.
  window.__photoExtra = { p2: { burst: "B1", burstPick: true, burstCount: 3 }, p4: { kind: "live" } };
}, base);
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const tiles = () => page.locator(".px-tile").count();
const before = await tiles();

await page.locator('.px-tile[data-id="p2"]').dblclick();
await page.waitForTimeout(600);
const burst = page.getByRole("button", { name: /View all 3 photos in this burst/ });
ok((await burst.count()) === 1 && /BURST/.test(await burst.innerText()), "a burst's chosen photo is marked BURST, with how many it holds");
await burst.click();
await page.waitForTimeout(400);
ok(/Burst · 2 of 3/.test(await page.locator(".px-mark.is-burst").innerText()), "BURST goes through the burst: the chosen one is 2 of 3");
await page.keyboard.press("ArrowLeft");
await page.waitForTimeout(250);
ok(/1 of 3/.test(await page.locator(".px-mark.is-burst").innerText()) && /p2-s0/.test(decodeURIComponent(await page.locator(".px-stage .px-shown").getAttribute("src"))),
  "the arrows step through its shots");
await page.getByRole("button", { name: "Done" }).click();
await page.waitForTimeout(500);
ok((await page.locator(".px-mark.is-burst").count()) === 0 && /p2/.test(decodeURIComponent(await page.locator(".px-stage .px-shown").getAttribute("src"))) && !/p2-s/.test(decodeURIComponent(await page.locator(".px-stage .px-shown").getAttribute("src"))),
  "Done brings the library back, on the chosen photo");
// Into the burst again, then closing the viewer: the library is whole again behind it.
await page.getByRole("button", { name: /View all 3 photos/ }).click();
await page.waitForTimeout(300);
await page.keyboard.press("Escape");
await page.waitForTimeout(900);
ok((await page.locator(".px-viewer").count()) === 0 && (await tiles()) === before && (await page.locator('.px-tile[data-id^="p2-s"]').count()) === 0,
  `closing inside a burst leaves the library as it was  (${await tiles()} tiles)`);

// A Live Photo: LIVE, and resting on it plays the moving part over the still.
await page.locator('.px-tile[data-id="p4"]').dblclick();
await page.waitForTimeout(600);
const live = page.getByRole("button", { name: "Play Live Photo" });
ok((await live.count()) === 1 && /LIVE/.test(await live.innerText()), "a Live Photo is marked LIVE");
await live.hover();
await page.waitForTimeout(200);
const v = page.locator(".px-pan video.px-live");
ok((await v.count()) === 1 && (await v.getAttribute("src")) === `${base}/p4/video`, `resting on LIVE plays its moving part over the still  ${await v.getAttribute("src").catch(() => "")}`);
await page.keyboard.press("ArrowRight");
await page.waitForTimeout(250);
ok((await page.locator("video.px-live").count()) === 0, "and it stops when the photo changes");
await b.close();
server.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
