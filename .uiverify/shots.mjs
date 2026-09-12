// Screenshot every auth screen against the real built bundle.
// Tauri IPC doesn't exist in a plain browser, so we stub __TAURI_INTERNALS__ with the exact
// shapes the Rust commands return — no invented UI state.
import { chromium } from "playwright";
import { fileURLToPath } from "node:url";
import path from "node:path";

const dir = path.dirname(fileURLToPath(import.meta.url));
const BASE = process.env.BASE ?? "http://localhost:4173";

const SERVICES = ["account","calendar","ckdatabasews","contacts","drivews","findme",
                  "mail","reminders","settings","streams","uploadimagews"];

/* addInitScript SERIALIZES this function — closures do not cross into the page, so every
   value it needs must arrive through the single argument. */
/* Fixture events, so the agenda's rendering is verified without inventing rows in anyone's
   real library. Times are unix seconds relative to now. */
function installStub({ screen, services, twoFactor, calendarFixture }) {
  const HOUR = 3600, DAY = 86400;
  const base = Math.floor(Date.now() / 1000 / DAY) * DAY + 9 * HOUR;
  const events = !calendarFixture ? [] : [
    { guid: "e1", calendarGuid: "work", title: "Design review", location: "Studio",
      startsAt: base + HOUR, endsAt: base + 2 * HOUR, allDay: false,
      timezone: "Asia/Kolkata", url: null, notes: null, etag: null },
    { guid: "e2", calendarGuid: "home", title: "Dentist", location: null,
      startsAt: base + 5 * HOUR, endsAt: base + 6 * HOUR, allDay: false,
      timezone: "Asia/Kolkata", url: null, notes: null, etag: null },
    { guid: "e3", calendarGuid: "work", title: "Sprint planning", location: "Room 2",
      startsAt: base + DAY + 2 * HOUR, endsAt: base + DAY + 3 * HOUR, allDay: false,
      timezone: "Asia/Kolkata", url: null, notes: null, etag: null },
    { guid: "e4", calendarGuid: "personal", title: "Flight to Hyderabad", location: null,
      startsAt: base + 3 * DAY, endsAt: base + 4 * DAY, allDay: true,
      timezone: "Asia/Kolkata", url: null, notes: null, etag: null },
  ];
  const calendars = [
    { guid: "work", title: "Work", colour: "#cc73e1", ctag: null, sortOrder: 0, enabled: true },
    { guid: "home", title: "Home", colour: "#0088ff", ctag: null, sortOrder: 1, enabled: true },
    { guid: "personal", title: "Asha", colour: "#1badf8", ctag: null, sortOrder: 2, enabled: true },
  ];
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd) => {
      switch (cmd) {
        case "auth_status":
          return { hasStoredSession: screen === "library", signedIn: screen === "library" };
        case "auth_resume":      return screen === "library";
        case "auth_sign_in":     return screen === "code" ? "needs_two_factor" : "complete";
        case "auth_submit_code": return null;
        case "account_services": return services;
        case "calendar_events": return events;
        case "calendar_list": return calendars;
        case "calendar_last_synced": return calendarFixture ? Math.floor(Date.now()/1000) - 120 : null;
        case "calendar_sync": return { calendars: calendars.length, events: events.length };
        case "auth_two_factor_options": return twoFactor;
        case "auth_resend_code":    return null;
        case "auth_send_sms":       return null;
        case "auth_submit_sms_code": return null;
        case "auth_sign_out":    return null;
        default: throw new Error(`unstubbed command: ${cmd}`);
      }
    },
  };
}

const PUSHED = { trustedDeviceCount: 2, phones: [{ id: 1, maskedNumber: "+91 ••••• ••318" }], tooManyCodesSent: false };
const NOTHING = { trustedDeviceCount: 0, phones: [{ id: 1, maskedNumber: "+91 ••••• ••318" }], tooManyCodesSent: false };

const shots = [
  { name: "01-signin",  screen: "signin",  theme: "light" },
  { name: "02-signin-dark", screen: "signin", theme: "dark" },
  { name: "03-code",    screen: "code",    theme: "light",
    act: async (p) => {
      await p.fill('input[aria-label="Apple ID"]', "you@icloud.com");
      await p.fill('input[aria-label="Password"]', "••••••••••••");
      await p.click('button[type="submit"]');
      await p.waitForSelector(".code", { timeout: 5000 });
      await p.fill('input[aria-label="Digit 1"]', "4");
      await p.fill('input[aria-label="Digit 2"]', "8");
      await p.fill('input[aria-label="Digit 3"]', "1");
    } },
  { name: "06-code-no-device", screen: "code", theme: "light", twoFactor: NOTHING,
    act: async (p) => {
      await p.fill('input[aria-label="Apple ID"]', "you@icloud.com");
      await p.fill('input[aria-label="Password"]', "••••••••••••");
      await p.click('button[type="submit"]');
      await p.waitForSelector(".help", { timeout: 5000 });
    } },
  { name: "07-calendar", screen: "library", theme: "light", calendarFixture: true,
    act: async (p) => { await p.waitForSelector(".cal-grid"); } },
  { name: "08-calendar-dark", screen: "library", theme: "dark", calendarFixture: true,
    act: async (p) => { await p.waitForSelector(".cal-grid"); } },
  { name: "04-library", screen: "library", theme: "light" },
  { name: "05-library-dark", screen: "library", theme: "dark" },
];

const browser = await chromium.launch();
for (const { name, screen, theme, act, twoFactor = PUSHED, calendarFixture = false } of shots) {
  const ctx = await browser.newContext({
    viewport: { width: 1040, height: 720 },
    deviceScaleFactor: 2,
    colorScheme: theme,
  });
  await ctx.addInitScript(installStub, { screen, services: SERVICES, twoFactor, calendarFixture });
  const page = await ctx.newPage();
  const errors = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto(BASE, { waitUntil: "networkidle" });
  await page.waitForTimeout(300);
  if (act) await act(page);
  await page.waitForTimeout(700); // let entrance animations settle
  await page.screenshot({ path: path.join(dir, `${name}.png`) });
  if (errors.length) {
    console.error(`${name.padEnd(18)} CONSOLE ERRORS: ${errors.join(" | ")}`);
    process.exitCode = 1;
  } else {
    console.log(`${name.padEnd(18)} clean`);
  }
  await ctx.close();
}
await browser.close();
