// Keyboard-only operability. axe cannot judge focus flow, and this is what a keyboard or
// screen-reader user actually experiences.
import { chromium } from "playwright";

const SERVICES = ["account","archivews","calendar","ckdatabasews","ckdeviceservice",
                  "cksharews","contacts","contactsweb","developerapi","docws","drivews",
                  "findme","geows","iwmb","iworkexportws","iworkthumbnailws","keyvalue",
                  "mail","mcc","mccgateway","mpp","notes","photos","photosupload",
                  "premiummailsettings","push","reminders","settings","sharedlibrary",
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
        case "reminder_edit": return null;
        case "reminder_list_create": case "reminder_list_rename": case "reminder_list_delete":
          return null;
        case "reminders_smart": return [];
        case "note_folders": return [];
        case "notes_in": return [];
        case "notes_last_synced": return Math.floor(Date.now() / 1000);
        case "notes_sync": return [0, 0];
        case "contacts_list": return [];
        case "contacts_last_synced": return Math.floor(Date.now() / 1000);
        case "contacts_sync": return 0;
        case "drive_children": return [];
        case "drive_sync_folder": return 0;
        case "drive_item": return null;
        default: return null;
      }
    },
  };
}
const TWO_FACTOR = { trustedDeviceCount: 2, phones: [], tooManyCodesSent: false };

const browser = await chromium.launch();
const checks = [];
const check = (name, ok, detail = "") => checks.push({ name, ok, detail });

async function open(screen) {
  const ctx = await browser.newContext({ viewport: { width: 1040, height: 800 } });
  await ctx.addInitScript(installStub, { screen, services: SERVICES, twoFactor: TWO_FACTOR });
  const page = await ctx.newPage();
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await page.waitForTimeout(300);
  return { ctx, page };
}

const focused = (page) =>
  page.evaluate(() => {
    const el = document.activeElement;
    if (!el) return "none";
    return el.getAttribute("aria-label") || el.textContent?.trim().slice(0, 24) || el.tagName;
  });

// --- sign-in: every control reachable, in a sensible order ---
{
  const { ctx, page } = await open("signin");
  const seen = [];
  for (let i = 0; i < 4; i++) {
    await page.keyboard.press("Tab");
    seen.push(await focused(page));
  }
  check("sign-in reaches Apple ID, Password, Sign in by Tab",
    seen.includes("Apple ID") && seen.includes("Password") && seen.some((s) => /Sign in/.test(s)),
    seen.join(" → "));

  // A visible focus ring is the whole point of keyboard focus.
  await page.focus('input[aria-label="Apple ID"]');
  const ring = await page.evaluate(() => {
    const g = document.querySelector(".field-group");
    return g ? getComputedStyle(g).boxShadow : "";
  });
  check("focused field shows a visible ring", ring.includes("rgb"), ring.slice(0, 48));
  await ctx.close();
}

// --- code: one tab stop, not six ---
{
  const { ctx, page } = await open("code");
  await page.fill('input[aria-label="Apple ID"]', "you@icloud.com");
  await page.fill('input[aria-label="Password"]', "secret");
  await page.click('button[type="submit"]');
  await page.waitForSelector(".code");

  check("code field takes focus automatically", (await focused(page)) === "Digit 1");

  const stops = await page.evaluate(
    () => document.querySelectorAll('.code input[tabindex="0"]').length
  );
  check("six boxes are ONE tab stop, not six", stops === 1, `${stops} tabbable`);

  // Typing advances; the whole code submits without touching a mouse.
  await page.keyboard.type("4812");
  check("typing advances between boxes", (await focused(page)) === "Digit 5");
  await ctx.close();
}

// --- library: arrow keys, single tab stop ---
{
  const { ctx, page } = await open("library");
  await page.waitForSelector(".sidebar .nav-item");

  const stops = await page.evaluate(
    () => document.querySelectorAll('.sidebar .nav-item[tabindex="0"]').length
  );
  check("sidebar is ONE tab stop", stops === 1, `${stops} tabbable`);

  await page.focus('.nav-item[aria-current="true"]');
  const before = await focused(page);
  await page.keyboard.press("ArrowDown");
  const after = await focused(page);
  check("ArrowDown moves down the sidebar", before !== after, `${before} → ${after}`);

  await page.keyboard.press("End");
  const end = await focused(page);
  await page.keyboard.press("Home");
  const home = await focused(page);
  check("Home/End jump to the ends", end !== home, `End=${end} Home=${home}`);
  await ctx.close();
}

await browser.close();
let failed = 0;
for (const c of checks) {
  if (!c.ok) failed++;
  console.log(`${c.ok ? "PASS" : "FAIL"} ${c.name}${c.detail ? `  [${c.detail}]` : ""}`);
}
console.log(failed === 0 ? "\nPASS keyboard-only operation works" : `\nFAIL ${failed} check(s)`);
process.exitCode = failed === 0 ? 0 : 1;
