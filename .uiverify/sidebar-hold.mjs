// Holding an arrow key in the sidebar, as on a Mac: the selection walks down the apps at the
// key's own pace, and only the app it settles on is opened — the ones passed on the way are
// never built. A single press still opens the next app at once, and switching to an app
// already open is quick.
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const b = await engine.launch();
const all = { notesFixture: true, calendarFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, ...all });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await page.locator(".nav-item").first().waitFor();
// Every app's code in, as a person meets it after the first moment.
await page.waitForFunction(() => ["NotesView", "CalendarView", "ContactsView", "DriveView", "RemindersView", "PhotosView"]
  .every((n) => performance.getEntriesByType("resource").some((r) => r.name.includes(`/${n}-`))), null, { timeout: 20000 });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const marked = () => page.locator('.nav-item[aria-current="true"]').innerText();
const slots = () => page.locator(".content > .screen-slot").count();
const front = () => page.evaluate(() => document.querySelector(".content")?.dataset.screen ?? null);

// Start on the first app, the sidebar focused.
const names = await page.locator(".nav-item").allInnerTexts();
await page.locator(".nav-item").first().click();
await page.waitForFunction(() => document.querySelectorAll(".content > .screen-slot").length === 1);
await page.locator(".nav-item").first().focus();
const before = await slots();

// Held: the first press and then the key's repeats. Each step is timed in the page, from the
// key's event to the frame that shows it.
await page.evaluate(() => {
  window.__steps = [];
  window.addEventListener("keydown", () => {
    const t = performance.now();
    requestAnimationFrame(() => setTimeout(() => window.__steps.push(performance.now() - t), 0));
  }, true);
});
for (let i = 0; i < names.length - 1; i++) {
  await page.keyboard.down("ArrowDown"); // after the first, Playwright marks these as repeats
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0))));
  ok((await marked()) === names[i + 1], `${engineName}: the selection is on ${names[i + 1]} as the key repeats`);
}
const during = await slots();
ok(during <= before + 1, `${engineName}: holding the key opens no app on the way  (${before} → ${during} screens built)`);
await page.keyboard.up("ArrowDown");
const last = names[names.length - 1];
await page.waitForFunction((n) => document.querySelector(".content > .screen-slot:not([hidden])") &&
  document.querySelector('.nav-item[aria-current="true"]')?.textContent?.includes(n), last, { timeout: 2000 });
const shownKey = await front();
ok((await page.locator(".content > .screen-slot:not([hidden])").count()) === 1, `${engineName}: letting go opens the app it settled on  (${shownKey})`);
ok((await slots()) <= before + 2, `${engineName}: and only that one was built  (${await slots()} screens)`);
const steps = (await page.evaluate(() => window.__steps)).map((t) => Math.round(t)).slice(0, names.length - 1);
const avg = steps.reduce((a, b) => a + b, 0) / steps.length;
console.log(`   held steps (ms): ${steps.join(" ")}  avg ${avg.toFixed(1)}`);
ok(avg < 40, `${engineName}: each step of a held key is quick  (${avg.toFixed(1)} ms)`);

// One press, then let go: the next app is in front straight away.
await page.keyboard.press("ArrowUp");
const target = names[names.length - 2];
const t1 = Date.now();
await page.waitForFunction((n) => document.querySelector(".content > .screen-slot:not([hidden])") &&
  document.querySelector('.nav-item[aria-current="true"]')?.textContent?.includes(n), target, { timeout: 2000 });
const opened = await page.evaluate(() => document.querySelector(".content")?.dataset.screen);
ok(opened !== shownKey, `${engineName}: a single press opens the app above  (${opened}, ${Date.now() - t1} ms)`);

// Switching between apps already open, by click: time from the click to the next painted frame.
const times = [];
for (const name of [names[0], last, names[0], last, names[0]]) {
  times.push(await page.evaluate(async (name) => {
    const btn = [...document.querySelectorAll(".nav-item")].find((b) => b.textContent.includes(name));
    const t = performance.now();
    btn.click();
    await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
    return performance.now() - t;
  }, name));
}
const sw = times.reduce((a, b) => a + b, 0) / times.length;
console.log(`   switch to an open app (ms): ${times.map((t) => t.toFixed(1)).join(" ")}  avg ${sw.toFixed(1)}`);
ok(sw < 120, `${engineName}: switching to an app already open is quick  (${sw.toFixed(1)} ms)`);
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
