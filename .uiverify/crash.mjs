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
        case "event_save": return "E-new";
        case "event_delete": return null;
        case "calendar_sync": return { calendars: 0, events: 0 };
        case "reminders_sync": case "notes_sync": return [0, 0];
        case "contacts_sync": case "drive_sync_folder": return 0;
        // One refresh covers every service, so there is one command to stub rather than
        // a pair per screen.
        case "sync_all":
        case "sync_status":
          return {
            services: [
              { service: "calendar", label: "Calendar", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 120 },
              { service: "reminders", label: "Reminders", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 60 },
              { service: "notes", label: "Notes", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 90 },
              { service: "contacts", label: "Contacts", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 300 },
            ],
            asOf: Math.floor(Date.now() / 1000) - 300,
            tookMs: 40,
          };
        // The event plugin is how a finished refresh reaches the screens. Nothing emits in
        // a plain browser, so listening is a no-op that hands back an unsubscribe id.
        case "plugin:event|listen":
          return 0;
        case "plugin:event|unlisten":
          return null;
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
