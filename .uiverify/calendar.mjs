// Calendar's Day and Week grids, checked in a real browser.
import { chromium } from "playwright";

const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
// The moment of the screenshot this came from: 9:51 PM, so "now" sits just above 10 PM.
await ctx.addInitScript(() => {
  const today = new Date(2026, 8, 29, 21, 51).getTime();
  const at = (h, m = 0) => Math.floor(new Date(2026, 8, 29, h, m).getTime() / 1000);
  const r = (guid, title, due) => ({ guid, listGuid: "L", title, notes: null, dueAt: due, completedAt: null,
    priority: 0, flagged: false, parentGuid: null, recurFrequency: null, recurInterval: null, etag: null });
  window.__fixedNow = today;
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
      if (cmd === "auth_resume") return true;
      if (cmd === "account_services") return ["calendar"];
      if (cmd === "calendar_list") return [{ guid: "c", title: "Home", colour: "#0088FF", ctag: null, sortOrder: 0, enabled: true, writable: true, isDefault: true }];
      if (cmd === "calendar_events") return [];
      if (cmd === "reminders_due_between") return [r("a", "Pick up the parcel", at(8)), r("b", "Water the plants", at(8))];
      if (cmd === "calendar_sync") return { calendars: 1, events: 0 };
      if (cmd === "sync_all" || cmd === "sync_status") return { services: [], asOf: 1, tookMs: 0 };
      if (cmd === "plugin:event|listen") return 0;
      return [];
    },
  };
});
const page = await ctx.newPage();
await page.clock.setFixedTime(new Date(2026, 8, 29, 21, 51));
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await page.getByRole("button", { name: "Day", exact: true }).click();
await page.waitForSelector(".cal-hours.is-day");
await page.waitForTimeout(300);

let failures = 0;
const ok = (pass, yes, no) => { console.log(pass ? `PASS ${yes}` : `FAIL ${no}`); if (!pass) failures++; };

const boxes = await page.locator(".cx-reminder").evaluateAll((els) =>
  els.map((e) => { const r = e.getBoundingClientRect(); return [r.top, r.bottom]; }));
const apart = boxes.length === 2 && (boxes[0][1] <= boxes[1][0] || boxes[1][1] <= boxes[0][0]);
ok(apart, "two reminders due at the same time are stacked, not drawn over each other",
  `the reminders at 8 overlap: ${JSON.stringify(boxes)}`);

const covered = await page.evaluate(() => {
  const now = document.querySelector(".cx-now-label")?.getBoundingClientRect();
  if (!now) return ["no now label"];
  return [...document.querySelectorAll(".cx-hour-labels span:not(.cx-now-label)")]
    .filter((s) => { const r = s.getBoundingClientRect(); return r.bottom > now.top && r.top < now.bottom; })
    .map((s) => s.textContent);
});
ok(covered.length === 0, "the time of now never sits over an hour's label", `now covers ${JSON.stringify(covered)}`);

await b.close();
process.exitCode = failures === 0 ? 0 : 1;
