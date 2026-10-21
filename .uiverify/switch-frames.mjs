// Switching apps never shows an empty frame: from the click on, every frame the page draws has
// one screen in front, with something in it, and the sidebar already marking the app chosen —
// for a screen opened for the first time as much as for one coming back.
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const b = await engine.launch();
const all = { notesFixture: true, calendarFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, ...all });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await page.locator(".nav-item").first().waitFor();
// Every app's code is in (fetched 300ms after the window opens) — what a person meets after
// the first moment. Before it is, opening an app waits on its code, which is a different matter.
await page.waitForFunction(() => ["NotesView", "CalendarView", "ContactsView", "DriveView", "RemindersView", "PhotosView"]
  .every((n) => performance.getEntriesByType("resource").some((r) => r.name.includes(`/${n}-`))), null, { timeout: 20000 });
await page.waitForLoadState("networkidle");
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

/** Each frame from the click on, for a few frames: what is in front and what the sidebar marks. */
const switchTo = (name) => page.evaluate(async (name) => {
  const frames = [];
  const look = () => {
    const shown = [...document.querySelectorAll(".content > .screen-slot:not([hidden])")];
    const slot = shown[0];
    // The slot is `display: contents`: what it holds is what is drawn.
    let drawn = 0;
    for (const el of slot?.children ?? []) drawn += el.getBoundingClientRect().height;
    frames.push({
      slots: shown.length,
      drawn: Math.round(drawn),
      kids: [...(slot?.children ?? [])].map((c) => c.className.split(" ")[0]).join("+"),
      opacity: slot?.firstElementChild ? Number(getComputedStyle(slot.firstElementChild).opacity) : null,
      marked: document.querySelector('.nav-item[aria-current="true"]')?.textContent ?? null,
    });
  };
  const btn = [...document.querySelectorAll(".nav-item")].find((b) => b.textContent.includes(name));
  btn.click();
  look(); // straight after the click, before anything is drawn
  for (let i = 0; i < 12; i++) {
    await new Promise((r) => requestAnimationFrame(() => r()));
    look();
  }
  return frames;
}, name);

for (const name of ["Notes", "Calendar", "Contacts", "Drive", "Notes", "Reminders", "Calendar"]) {
  const frames = await switchTo(name);
  // Frame 0 is the state in the click's own task, never painted; frames 1.. are each a paint.
  const painted = frames.slice(1);
  const blank = painted.findIndex((f) => f.slots !== 1 || f.drawn === 0);
  ok(blank === -1, `${engineName} → ${name}: every painted frame has a screen in front, drawn ${blank === -1 ? "" : JSON.stringify(painted[blank])}`);
  if (blank !== -1) console.log("   ", painted.map((f) => `${f.kids || "-"}:${f.drawn}`).join(" "));
  ok(painted.every((f) => f.marked?.includes(name)), `${engineName} → ${name}: the sidebar marks it from the first frame`);
  await page.waitForTimeout(250);
}
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
