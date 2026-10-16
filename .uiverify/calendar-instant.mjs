// Calendar writes show at once, and an edit keeps what the editor does not show.
// The stub holds every write for 1.5 s, as a slow network would.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
await ctx.addInitScript(() => {
  window.__saves = [];
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__slow) return;
    const inner = t.invoke;
    t.invoke = async (cmd, args) => {
      if (cmd === "event_save") window.__saves.push(args);
      if (cmd === "event_save" || cmd === "event_delete" || cmd === "reminder_set_done") await new Promise((r) => setTimeout(r, 1500));
      const out = await inner(cmd, args);
      if (cmd === "calendar_events" && Array.isArray(out))
        return out.map((e) => (e.guid === "e2" ? { ...e, url: "https://meet.example/dentist" } : e));
      return out;
    };
    t.__slow = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.keyboard.press("Control+2");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

// An edit sends the event's link back as it was.
await page.locator(".cx-block", { hasText: "Dentist" }).first().click();
await page.waitForTimeout(300);
const title = page.locator('input[placeholder="New Event"]').first();
await title.fill("Dentist, again");
await title.press("Enter");
await page.waitForTimeout(150);
ok(await page.locator(".cx-block", { hasText: "Dentist, again" }).count() > 0, "a retitled event shows its new title at once");
await page.waitForTimeout(1800);
const sent = await page.evaluate(() => window.__saves.at(-1));
ok(sent?.draft?.url === "https://meet.example/dentist",
  `an edit keeps the event's link  [${JSON.stringify(sent?.draft?.url)}]`);

// And its own zone, not this machine's: the stub's browser says Asia/Calcutta, the event
// was made in Asia/Kolkata (and a meeting set in New York must stay in New York).
const zoned = await page.evaluate(() => window.__saves.at(-1)?.draft?.timezone);
ok(zoned === "Asia/Kolkata", `an edit keeps the event's own time zone  [${zoned}]`);

// The link can be set from the event itself. (The stub hands back the old title on re-read.)
await page.locator(".cx-block", { hasText: "Dentist" }).first().click();
await page.waitForTimeout(250);
await page.getByLabel("URL").fill("https://example.com/new");
await page.locator('input[placeholder="New Event"]').first().press("Enter");
await page.waitForTimeout(1800);
const linked = await page.evaluate(() => window.__saves.at(-1)?.draft?.url);
ok(linked === "https://example.com/new", `a URL typed into the event is saved  [${linked}]`);

// A delete takes the event off the calendar at once.
await page.locator(".cx-block", { hasText: "Design review" }).first().click();
await page.waitForTimeout(250);
await page.evaluate(() => document.activeElement?.blur());
await page.keyboard.press("Delete");
await page.waitForTimeout(150);
ok(await page.locator(".cx-block", { hasText: "Design review" }).count() === 0, "a deleted event leaves at once");
// A reminder on the calendar ticks there, at once, as in Calendar.
const ring = page.getByRole("checkbox", { name: /^Complete / }).first();
const had = await ring.count();
if (had) {
  await ring.click();
  await page.waitForTimeout(150);
  ok((await ring.getAttribute("aria-checked")) === "true", "a reminder's ring on the calendar ticks it at once");
} else ok(false, "no reminder on the calendar to tick");
await b.close();
process.exitCode = failures ? 1 : 0;
