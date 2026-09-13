// Load exactly what the app loads (the dev server) and surface any error that kills render.
import { chromium } from "playwright";

const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1100, height: 800 } });
await ctx.addInitScript(() => {
  window.__TAURI_INTERNALS__ = {
    transformCallback: (c) => c,
    invoke: async (cmd) => {
      switch (cmd) {
        case "auth_status": return { hasStoredSession: true, signedIn: true };
        case "auth_resume": return true;
        case "account_services": return ["calendar", "reminders", "notes", "contacts", "drivews", "photos"];
        case "calendar_events": case "calendar_list": case "reminders_due_between": return [];
        case "calendar_last_synced": case "reminders_last_synced": case "notes_last_synced":
        case "contacts_last_synced": return Math.floor(Date.now() / 1000);
        case "reminder_lists": case "reminders_in": case "reminders_smart":
        case "note_folders": case "notes_in": case "contacts_list": case "drive_children": return [];
        case "calendar_sync": return { calendars: 0, events: 0 };
        case "reminders_sync": case "notes_sync": return [0, 0];
        case "contacts_sync": case "drive_sync_folder": return 0;
        default: return null;
      }
    },
  };
});
const p = await ctx.newPage();
const errors = [];
p.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
p.on("console", (m) => m.type() === "error" && errors.push(`console: ${m.text()}`));

await p.goto(process.env.TARGET ?? "http://localhost:5273", { waitUntil: "networkidle" });
await p.waitForTimeout(1200);

const rendered = (await p.innerText("#root")).trim();
console.log("rendered:", rendered ? `${rendered.slice(0, 90).replace(/\n/g, " ")}…` : "NOTHING — blank");
console.log(errors.length ? "errors:\n  " + errors.join("\n  ") : "no errors");
await b.close();
process.exitCode = rendered ? 0 : 1;
