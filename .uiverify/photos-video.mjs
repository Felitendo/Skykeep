// A video plays in the viewer itself, as Photos plays one: fetched from the app's loopback
// server in ranges, its controls the player's own, seeking anywhere. One this engine cannot
// play gives way to a still whose Play opens the system's player.
import http from "node:http";
import fs from "node:fs";
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const clip = new URL("./clip.webm", import.meta.url);
const len = fs.statSync(clip).size;
const asked = [];
const server = http.createServer((req, res) => {
  asked.push(req.url);
  if (!req.url.startsWith("/s3cret/p3/")) return void res.writeHead(404).end();
  const m = /bytes=(\d*)-(\d*)/.exec(req.headers.range ?? "");
  const first = m && m[1] ? +m[1] : 0;
  const last = m && m[2] ? Math.min(+m[2], len - 1) : len - 1;
  res.writeHead(m ? 206 : 200, { "Content-Type": "video/webm", "Accept-Ranges": "bytes", "Content-Length": last - first + 1, "Content-Range": `bytes ${first}-${last}/${len}` });
  fs.createReadStream(clip, { start: first, end: last }).pipe(res);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/s3cret`;

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript((v) => { window.__videoBase = v; }, base);
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await page.locator('.px-tile[data-id="p3"]').dblclick();
await page.waitForTimeout(600);
const video = page.locator(".px-stage video");
ok((await video.count()) === 1 && (await video.getAttribute("src")) === `${base}/p3/original`, `the video is in the viewer, from the loopback server  ${await video.getAttribute("src").catch(() => "")}`);
ok(await video.evaluate((v) => v.controls && v.autoplay), "with the player's own controls, playing at once");
const played = await video.evaluate((v) => new Promise((r) => {
  v.muted = true;
  void v.play().catch(() => undefined);
  const t0 = performance.now();
  const tick = () => (v.currentTime > 0.5 || v.error || performance.now() - t0 > 5000 ? r({ t: v.currentTime, err: v.error?.code ?? null }) : setTimeout(tick, 100));
  tick();
}));
if (played.err) {
  // This engine has no decoder for it: the still and the system's player take its place.
  await page.waitForTimeout(200);
  ok((await page.locator(".px-stage button.px-video").count()) === 1, `an engine that cannot play it falls back to the system's player  (error ${played.err})`);
} else {
  ok(played.t > 0.5, `it plays  (at ${played.t.toFixed(2)}s)`);
  const sought = await video.evaluate((v) => new Promise((r) => { v.onseeked = () => r(v.currentTime); v.currentTime = 4; }));
  ok(Math.abs(sought - 4) < 0.5, `and seeks  (to ${sought.toFixed(2)}s)`);
  ok(asked.some((u) => u.startsWith("/s3cret/p3/original")), "fetched from the app's server");
}
// Stepping to a photo leaves no video behind.
await page.keyboard.press("ArrowLeft");
await page.waitForTimeout(300);
ok((await page.locator(".px-stage video").count()) === 0, "a step to a photo leaves no video playing");

// No server (it could not start): the system's player, as before.
const p2 = await ctx.newPage();
await p2.addInitScript(() => { window.__videoBase = null; });
await p2.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(p2, "Photos");
await p2.waitForTimeout(400);
await p2.locator('.px-tile[data-id="p3"]').dblclick();
await p2.waitForTimeout(600);
ok((await p2.locator(".px-stage video").count()) === 0 && (await p2.locator(".px-stage button.px-video").count()) === 1, "without the server, the still and the system's player");
await b.close();
server.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
