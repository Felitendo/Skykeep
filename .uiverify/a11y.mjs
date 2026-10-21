// Accessibility audit with axe-core — the same engine Lighthouse and Deque use.
// Every auth screen, in both themes, at WCAG 2.1 AA.
import { engine as chromium } from "./engine.mjs";
import AxeBuilder from "@axe-core/playwright";

const SERVICES = ["account","archivews","calendar","ckdatabasews","ckdeviceservice",
                  "cksharews","contacts","contactsweb","developerapi","docws","drivews",
                  "findme","geows","iwmb","iworkexportws","iworkthumbnailws","keyvalue",
                  "mail","mcc","mccgateway","mpp","notes","photos","photosupload",
                  "premiummailsettings","push","reminders","settings","sharedlibrary",
                  "sharedstreams","ubiquity","uploadimagews"];

function installStub({ screen, services, twoFactor, driveItems = [] }) {
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd) => {
      switch (cmd) {
        case "auth_status":
          return { hasStoredSession: screen === "library", signedIn: screen === "library" };
        case "auth_resume": return screen === "library" ? "library" : "signin";
        case "auth_sign_in": return screen === "code" ? "needs_two_factor" : "complete";
        case "auth_two_factor_options": return twoFactor;
        case "account_services": return services;
        case "calendar_events": return [];
        case "calendar_list": return [];
        case "calendar_last_synced": return Math.floor(Date.now() / 1000);
        case "event_save": return "E-new";
        case "event_delete": return null;
        case "calendar_sync": return { calendars: 0, events: 0 };
        case "reminder_counts": return { today: 4, scheduled: 7, flagged: 2, all: 12, lists: [["L1", 5], ["L2", 3], ["lists", 4]] };
        case "reminder_lists": return [];
        case "reminders_in": return [];
        case "reminders_due_between": return [];
        case "reminder_create": return null;
        case "reminder_delete": return null;
        case "reminder_set_done": return null;
        case "reminder_set_repeat": return null;
        case "reminder_edit": return null;
        case "reminder_list_create": case "reminder_list_rename": case "reminder_list_delete":
          return null;
        case "reminders_smart": return [];
        case "note_folders": return [];
        case "note_save": case "note_delete": case "note_move": return null;
        case "note_create": return "Note/new";
        case "notes_in": return [];
        case "contact_save": return "C-new";
        case "contact_delete": return null;
        case "contacts_list": return [];
        case "contacts_last_synced": return Math.floor(Date.now() / 1000);
        case "contacts_sync": return 0;
        case "drive_children": return driveItems;
        case "drive_sync_folder": return 0;
        case "drive_download": return "/home/user/Downloads/file.pdf";
        case "drive_create_folder": case "drive_rename": case "drive_delete": case "drive_move": return null;
        case "drive_item": return null;
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
}

const TWO_FACTOR = { trustedDeviceCount: 0, phones: [{ id: 1, maskedNumber: "+91 ••••• ••318" }], tooManyCodesSent: false };

const screens = [
  { name: "sign-in", screen: "signin" },
  { name: "code",    screen: "code", act: async (p) => {
      await p.fill('input[aria-label="Apple ID"]', "you@icloud.com");
      await p.fill('input[aria-label="Password"]', "secret");
      await p.click('button[type="submit"]');
      await p.waitForSelector(".code");
    } },
  { name: "library", screen: "library", act: async (p) => {
      await p.waitForSelector(".sidebar .nav-item");
    } },
  { name: "drive-move", screen: "library", driveItems: [
      { drivewsid: "FOLDER::a", parentId: null, name: "Downloads", extension: null,
        kind: "FOLDER", size: null, childCount: 3, createdAt: 1785000000,
        modifiedAt: 1788000000, etag: null },
    ], act: async (p) => {
      await p.click('.nav-item:has-text("iCloud Drive")');
      await p.waitForSelector(".drive-row-hit");
      await p.hover(".drive-row.is-folder");
      await p.click('.drive-row.is-folder .drive-action[aria-label^="Actions for"]');
      await p.getByRole("menuitem", { name: "Move to Folder" }).click();
      await p.waitForSelector(".modal-sheet");
    } },
];

const browser = await chromium.launch();
let total = 0;

for (const theme of ["light", "dark"]) {
  for (const { name, screen, act, driveItems } of screens) {
    const ctx = await browser.newContext({
      viewport: { width: 1040, height: 800 },
      colorScheme: theme,
    });
    await ctx.addInitScript(installStub, {
      screen,
      services: SERVICES,
      twoFactor: TWO_FACTOR,
      driveItems: driveItems ?? [],
    });
    const page = await ctx.newPage();
    await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
    if (act) await act(page);
    await page.waitForTimeout(400);

    const { violations } = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
      .analyze();

    total += violations.length;
    const label = `${name} (${theme})`.padEnd(18);
    if (violations.length === 0) {
      console.log(`${label} clean`);
    } else {
      console.log(`${label} ${violations.length} violation(s)`);
      for (const v of violations) {
        console.log(`   [${v.impact}] ${v.id} — ${v.help}`);
        for (const node of v.nodes.slice(0, 3)) {
          console.log(`      ${node.html.slice(0, 110)}`);
        }
      }
    }
    await ctx.close();
  }
}

await browser.close();
console.log(total === 0 ? "\nPASS no WCAG 2.1 AA violations" : `\nFAIL ${total} violation(s)`);
process.exitCode = total === 0 ? 0 : 1;
