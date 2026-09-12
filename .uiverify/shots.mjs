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
function installStub({ screen, services, twoFactor }) {
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
  { name: "04-library", screen: "library", theme: "light" },
  { name: "05-library-dark", screen: "library", theme: "dark" },
];

const browser = await chromium.launch();
for (const { name, screen, theme, act, twoFactor = PUSHED } of shots) {
  const ctx = await browser.newContext({
    viewport: { width: 1040, height: 720 },
    deviceScaleFactor: 2,
    colorScheme: theme,
  });
  await ctx.addInitScript(installStub, { screen, services: SERVICES, twoFactor });
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
