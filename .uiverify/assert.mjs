// Deterministic checks on the library nav — cheaper and more reliable than eyeballing a screenshot.
import { chromium } from "playwright";
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1040, height: 900 } });
await ctx.addInitScript(({ services }) => {
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
      if (cmd === "auth_resume") return true;
      if (cmd === "account_services") return services;
      if (cmd === "calendar_events") return [];
      if (cmd === "calendar_list") return [];
      if (cmd === "event_save") return "E-new";
      if (cmd === "event_delete") return null;
      if (cmd === "calendar_sync") return { calendars: 0, events: 0 };
      if (cmd === "reminder_lists") return [];
      if (cmd === "reminders_in") return [];
      if (cmd === "reminders_due_between") return [];
      if (cmd === "reminder_create") return null;
      if (cmd === "reminder_delete") return null;
      if (cmd === "reminder_set_done") return null;
      if (cmd === "reminder_edit") return null;
      if (cmd === "reminders_smart") return [];
      if (cmd === "reminder_list_create" || cmd === "reminder_list_rename" || cmd === "reminder_list_delete") return null;
      if (cmd === "note_folders") return [];
      if (cmd === "notes_in") return [];
      if (cmd === "note_save" || cmd === "note_delete" || cmd === "note_move") return null;
      if (cmd === "note_create") return "Note/new";
      if (cmd === "contacts_list") return [];
      if (cmd === "drive_children") return [];
      if (cmd === "drive_sync_folder") return 0;
      if (cmd === "drive_item") return null;
      // One refresh for every service, and the event it finishes with.
      if (cmd === "sync_all" || cmd === "sync_status")
        return { services: [], asOf: Math.floor(Date.now() / 1000), tookMs: 0 };
      if (cmd === "plugin:event|listen") return 0;
      return null;
    },
  };
}, { services: ["account","archivews","calendar","ckdatabasews","ckdeviceservice","cksharews",
  "contacts","contactsweb","developerapi","docws","drivews","findme","geows","iwmb",
  "iworkexportws","iworkthumbnailws","keyvalue","mail","mcc","mccgateway","mpp","notes",
  "photos","photosupload","premiummailsettings","push","reminders","schoolwork","settings",
  "sharedlibrary","sharedstreams","ubiquity","uploadimagews"] });

const p = await ctx.newPage();
const errs = [];
p.on("pageerror", (e) => errs.push(String(e)));
await p.goto("http://localhost:4173", { waitUntil: "networkidle" });
await p.waitForSelector(".sidebar .nav-item");

const toggle = await p.textContent(".nav-toggle");
const count = await p.locator(".sidebar .nav-item").count();
const first = await p.textContent(".sidebar .nav-item");
const selected = await p.textContent('.nav-item[aria-current="true"]');

console.log("toggle label   :", JSON.stringify(toggle?.trim()));
console.log("nav items      :", count, "(10 primary + 1 toggle expected when collapsed)");
console.log("first item     :", JSON.stringify(first?.trim()));
console.log("selected       :", JSON.stringify(selected?.trim()));
console.log("page errors    :", errs.length ? errs : "none");

const collapsed = toggle?.includes("Show") && count === 11;
// Opening on a screen that works beats opening on a placeholder, so selection must land on
// a built service rather than simply the first one listed.
const selectsBuilt = selected?.trim() === "Calendar";
console.log(collapsed ? "PASS collapsed by default" : "FAIL not collapsed by default");
console.log(selectsBuilt ? "PASS opens on a built screen" : `FAIL opened on ${selected}`);
await b.close();
process.exitCode = collapsed && selectsBuilt ? 0 : 1;
