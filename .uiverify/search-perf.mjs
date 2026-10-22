// How the search across everything feels, measured: from a key pressed to the results it
// brings painted, and the frames while a long list of results scrolls. Prints numbers; fails
// only on what no one should see — results settling later than a quarter second after a key,
// or a scroll whose frames run past a sixth of a second.
//
//   BASE=http://localhost:4240 ENGINE=webkit node .uiverify/search-perf.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const BASE = process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  notesFixture: true, contactsFixture: true, driveFixture: true });
await ctx.addInitScript(() => {
  window.__moreContacts = Array.from({ length: 600 }, (_, i) => ({ guid: `m${i}`, firstName: "Marlowe", lastName: `Test ${i + 1}`,
    company: null, isCompany: false, birthday: null, notes: null, etag: null, methods: [{ kind: "phone", label: "mobile", value: `+1 555 01${String(i).padStart(3, "0")}` }] }));
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
await page.goto(BASE, { waitUntil: "networkidle" });
const field = page.getByRole("combobox", { name: "Search everything" });
await field.click();

// Each key: the time to the last change it caused in the results, then the frame that shows it.
await page.evaluate(() => {
  window.__keys = [];
  document.addEventListener("keydown", () => window.__keys.push({ at: performance.now(), last: null }), true);
  new MutationObserver(() => {
    const k = window.__keys[window.__keys.length - 1];
    if (!k) return;
    requestAnimationFrame(() => { k.last = performance.now() - k.at; });
  }).observe(document.body, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ["aria-selected", "class"] });
});
for (const word of ["marlowe", "zoya", "curtains"]) {
  await field.fill("");
  await page.waitForTimeout(300);
  await page.evaluate(() => { window.__keys = []; });
  for (const c of word) {
    await page.keyboard.type(c);
    await page.waitForTimeout(180);
  }
  await page.waitForTimeout(400);
  const keys = await page.evaluate(() => window.__keys.map((k) => k.last));
  const settled = keys.filter((x) => x !== null);
  const worst = Math.max(...settled);
  const mean = settled.reduce((a, x) => a + x, 0) / settled.length;
  console.log(`${engineName} "${word}": key → painted results mean ${mean.toFixed(1)} ms, worst ${worst.toFixed(1)} ms  [${settled.map((x) => x.toFixed(0)).join(", ")}]`);
  ok(worst < 250, `${engineName}: every key of "${word}" painted within 250 ms`);
}

// A long section shown whole, then scrolled as a wheel scrolls it: every frame's length.
await field.fill("marlowe");
await page.waitForTimeout(400);
const panel = page.getByRole("listbox", { name: "Found" });
const more = panel.getByRole("option", { name: /^Show all \d+ in Contacts/ });
const t0 = Date.now();
await more.click();
await page.waitForFunction(() => document.querySelectorAll('#gsearch-results [role="option"]').length > 20);
const opened = Date.now() - t0;
const frames = await page.evaluate(async () => {
  const list = document.querySelector("#gsearch-results [data-scroller]") ?? document.querySelector("#gsearch-results");
  const out = [];
  let last = performance.now();
  for (let i = 0; i < 240; i++) {
    await new Promise((r) => requestAnimationFrame(r));
    const now = performance.now();
    out.push(now - last);
    last = now;
    list.scrollTop += 48;
    list.dispatchEvent(new Event("scroll"));
  }
  return { out, height: list.scrollHeight, rows: document.querySelectorAll('#gsearch-results [role="option"]').length };
});
const sorted = [...frames.out].sort((a, b) => a - b);
const p = (q) => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))];
const long = frames.out.filter((x) => x > 20).length;
console.log(`${engineName} scroll: frames p50 ${p(0.5).toFixed(1)} ms, p95 ${p(0.95).toFixed(1)} ms, max ${sorted.at(-1).toFixed(1)} ms, ${long} over 20 ms of ${frames.out.length}; ${frames.rows} rows in the page; Show All painted in ${opened} ms`);
ok(sorted.at(-1) < 160, `${engineName}: no frame of the scroll past 160 ms`);
await ctx.close();
await b.close();
process.exitCode = failures ? 1 : 0;
