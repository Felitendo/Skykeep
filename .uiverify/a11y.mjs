// Accessibility audit with axe-core — the same engine Lighthouse and Deque use.
// Every auth screen, in both themes, at WCAG 2.1 AA.
import { chromium } from "playwright";
import AxeBuilder from "@axe-core/playwright";

const SERVICES = ["account","archivews","calendar","ckdatabasews","cksharews","contacts",
  "contactsweb","developerapi","docws","drivews","findme","geows","iwmb","iworkexportws",
  "iworkthumbnailws","keyvalue","mail","mcc","mccgateway","mpp","notes","photos",
  "photosupload","premiummailsettings","push","reminders","settings","sharedlibrary",
  "sharedstreams","ubiquity","uploadimagews"];

function installStub({ screen, services, twoFactor }) {
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd) => {
      switch (cmd) {
        case "auth_status":
          return { hasStoredSession: screen === "library", signedIn: screen === "library" };
        case "auth_resume": return screen === "library";
        case "auth_sign_in": return screen === "code" ? "needs_two_factor" : "complete";
        case "auth_two_factor_options": return twoFactor;
        case "account_services": return services;
        case "calendar_events": return [];
        case "calendar_list": return [];
        case "calendar_last_synced": return Math.floor(Date.now() / 1000);
        case "calendar_sync": return { calendars: 0, events: 0 };
        case "reminder_lists": return [];
        case "reminders_in": return [];
        case "reminders_last_synced": return Math.floor(Date.now() / 1000);
        case "reminders_due_between": return [];
        case "reminders_sync": return [0, 0];
        case "reminder_create": return null;
        case "reminder_delete": return null;
        case "reminder_set_done": return null;
        case "note_folders": return [];
        case "notes_in": return [];
        case "notes_last_synced": return Math.floor(Date.now() / 1000);
        case "notes_sync": return [0, 0];
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
      await p.click(".nav-toggle"); // audit the expanded state too
    } },
];

const browser = await chromium.launch();
let total = 0;

for (const theme of ["light", "dark"]) {
  for (const { name, screen, act } of screens) {
    const ctx = await browser.newContext({
      viewport: { width: 1040, height: 800 },
      colorScheme: theme,
    });
    await ctx.addInitScript(installStub, { screen, services: SERVICES, twoFactor: TWO_FACTOR });
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
