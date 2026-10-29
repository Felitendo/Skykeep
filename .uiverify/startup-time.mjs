// How long the window takes to open, in the page: from navigation to the window being told it may
// show (window_ready), to its first paint, and to the last-open app (Contacts) drawn with rows; and
// the first open of another app (Notes), whose code comes in then. Median of N runs (N=9).
// A measure, not a pass or fail: run before and after a change, the same way.
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/startup-time.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const N = Number(process.env.N ?? 9);
const b = await engine.launch();
const ready = [], painted = [], rows = [], firstSwitch = [];
for (let i = 0; i < N; i++) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
  await ctx.addInitScript(() => { try { localStorage.setItem("quince.library.screen", "contacts"); } catch {} });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true, notesFixture: true, calendarFixture: true, remindersFixture: true, driveFixture: true });
  await ctx.addInitScript(() => {
    const wrap = () => {
      const t = window.__TAURI_INTERNALS__;
      if (!t || t.__timed) return;
      const inner = t.invoke;
      t.invoke = (cmd, args) => { if (cmd === "window_ready") window.__ready ??= performance.now(); return inner(cmd, args); };
      t.__timed = true;
    };
    wrap();
    document.addEventListener("DOMContentLoaded", wrap);
    // The moment the first contact row is in the page.
    new MutationObserver((_, o) => {
      if (document.querySelector(".contact-row")) { window.__rows ??= performance.now(); o.disconnect(); }
    }).observe(document, { childList: true, subtree: true });
  });
  const page = await ctx.newPage();
  await page.goto(process.env.BASE ?? "http://localhost:4173");
  await page.waitForFunction(() => window.__ready && window.__rows, null, { timeout: 20000 });
  const r = await page.evaluate(() => ({
    ready: window.__ready,
    rows: window.__rows,
    paint: performance.getEntriesByType("paint").find((p) => p.name === "first-contentful-paint")?.startTime ?? NaN,
  }));
  ready.push(r.ready);
  rows.push(r.rows);
  painted.push(r.paint);
  await page.waitForLoadState("networkidle");
  await page.waitForTimeout(400);
  firstSwitch.push(await page.evaluate(async () => {
    const btn = [...document.querySelectorAll(".nav-item")].find((x) => x.textContent.includes("Notes"));
    const t = performance.now();
    btn.click();
    await new Promise((res) => { const look = () => document.querySelector(".content > .screen-slot:not([hidden]) .notes") ? requestAnimationFrame(() => setTimeout(res, 0)) : requestAnimationFrame(look); look(); });
    return performance.now() - t;
  }));
  await ctx.close();
}
const med = (a) => { const s = a.filter((x) => !Number.isNaN(x)).sort((x, y) => x - y); return s.length ? s[Math.floor(s.length / 2)].toFixed(0) : "n/a"; };
const all = (a) => a.map((x) => x.toFixed(0)).join(" ");
console.log(`${engineName}: window_ready ${med(ready)} ms [${all(ready)}]; first paint ${med(painted)} ms; contacts drawn ${med(rows)} ms [${all(rows)}]; first open of Notes ${med(firstSwitch)} ms`);
await b.close();
