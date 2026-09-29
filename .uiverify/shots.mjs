// Screenshot every auth screen against the real built bundle.
// Tauri IPC doesn't exist in a plain browser, so we stub __TAURI_INTERNALS__ with the exact
// shapes the Rust commands return — no invented UI state.
import { chromium } from "playwright";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { SERVICES, installStub } from "./stub.mjs";

const dir = path.dirname(fileURLToPath(import.meta.url));
const BASE = process.env.BASE ?? "http://localhost:4173";



const PUSHED = { trustedDeviceCount: 2, phones: [{ id: 1, maskedNumber: "+91 ••••• ••318" }], tooManyCodesSent: false };
const NOTHING = { trustedDeviceCount: 0, phones: [{ id: 1, maskedNumber: "+91 ••••• ••318" }], tooManyCodesSent: false };
// No device to prompt and no number to text. Rare, and real — an account whose trusted number
// was removed. Without a screen of its own it offers a code that can never arrive.
const NOWHERE = { trustedDeviceCount: 0, phones: [], tooManyCodesSent: false };

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
  { name: "19-code-nowhere", screen: "code", theme: "light", twoFactor: NOWHERE,
    act: async (p) => {
      await p.fill('input[aria-label="Apple ID"]', "you@icloud.com");
      await p.fill('input[aria-label="Password"]', "••••••••••••");
      await p.click('button[type="submit"]');
      await p.waitForSelector(".help", { timeout: 5000 });
    } },
  { name: "06-code-no-device", screen: "code", theme: "light", twoFactor: NOTHING,
    act: async (p) => {
      await p.fill('input[aria-label="Apple ID"]', "you@icloud.com");
      await p.fill('input[aria-label="Password"]', "••••••••••••");
      await p.click('button[type="submit"]');
      await p.waitForSelector(".help", { timeout: 5000 });
    } },
  { name: "07-calendar", screen: "library", theme: "light", calendarFixture: true,
    act: async (p) => { await p.locator(".nav-item", { hasText: "Calendar" }).click(); await p.waitForSelector(".cal-grid"); } },
  { name: "08-calendar-dark", screen: "library", theme: "dark", calendarFixture: true,
    act: async (p) => { await p.locator(".nav-item", { hasText: "Calendar" }).click(); await p.waitForSelector(".cal-grid"); } },
  // The window is often half a screen, where four columns do not fit.
  { name: "09-notes-narrow", screen: "library", theme: "dark", notesFixture: true,
    viewport: { width: 958, height: 1038 },
    act: async (p) => {
      await p.click('.nav-item:has-text("Notes")');
      await p.waitForSelector(".note-rows .note-row");
      await p.click('.note-row:has-text("These sources")');
    } },
  { name: "10-notes-wide", screen: "library", theme: "dark", notesFixture: true,
    viewport: { width: 1600, height: 1000 },
    act: async (p) => {
      await p.click('.nav-item:has-text("Notes")');
      await p.waitForSelector(".note-rows .note-row");
      await p.click('.note-row:has-text("These sources")');
    } },
  { name: "11-contacts", screen: "library", theme: "dark", contactsFixture: true,
    viewport: { width: 1100, height: 820 },
    act: async (p) => {
      await p.click('.nav-item:has-text("Contacts")');
      await p.waitForSelector(".contact-rows .contact-row");
      await p.click('.contact-row:has-text("Zoya")');
    } },
  { name: "12-drive", screen: "library", theme: "dark", driveFixture: true,
    viewport: { width: 1100, height: 760 },
    act: async (p) => {
      await p.click('.nav-item:has-text("iCloud Drive")');
      await p.waitForSelector(".drive-rows .drive-row");
    } },
  { name: "13-reminders", screen: "library", theme: "dark", remindersFixture: true,
    viewport: { width: 1100, height: 760 },
    act: async (p) => {
      await p.click('.nav-item:has-text("Reminders")');
      await p.waitForSelector(".rx-rows li");
      await p.click('.rx-list:has-text("Daily")');
      await p.waitForTimeout(200);
      await p.click('.rx-trailing .rx-icon-button[aria-expanded] >> nth=0');
    } },
  // A full-screen window on a large display is where content stranded in a narrow column
  // becomes obvious, and every shot above is too small to show it.
  { name: "14-reminders-wide", screen: "library", theme: "dark", remindersFixture: true,
    viewport: { width: 1920, height: 1080 },
    act: async (p) => {
      await p.click('.nav-item:has-text("Reminders")');
      await p.waitForSelector(".rx-rows li");
    } },
  { name: "15-notes-full", screen: "library", theme: "dark", notesFixture: true,
    viewport: { width: 1920, height: 1080 },
    act: async (p) => {
      await p.click('.nav-item:has-text("Notes")');
      await p.waitForSelector(".note-rows .note-row");
      await p.click('.note-row:has-text("These sources")');
    } },
  { name: "16-contacts-wide", screen: "library", theme: "dark", contactsFixture: true,
    viewport: { width: 1920, height: 1080 },
    act: async (p) => {
      await p.click('.nav-item:has-text("Contacts")');
      await p.waitForSelector(".contact-rows .contact-row");
      await p.click('.contact-row:has-text("Zoya")');
    } },
  { name: "17-calendar-wide", screen: "library", theme: "dark", calendarFixture: true,
    viewport: { width: 1920, height: 1080 },
    act: async (p) => { await p.locator(".nav-item", { hasText: "Calendar" }).click(); await p.waitForSelector(".cal-grid"); } },
  { name: "18-drive-move", screen: "library", theme: "dark", driveFixture: true,
    act: async (p) => {
      await p.click('.nav-item:has-text("iCloud Drive")');
      await p.waitForSelector(".drive-row-hit");
      await p.hover(".drive-row.is-folder");
      await p.click('.drive-row.is-folder .drive-action[aria-label^="Move"]');
      await p.waitForSelector(".modal-sheet");
    } },
  { name: "04-library", screen: "library", theme: "light" },
  { name: "05-library-dark", screen: "library", theme: "dark" },
];

const browser = await chromium.launch();
for (const { name, screen, theme, act, twoFactor = PUSHED, calendarFixture = false, notesFixture = false, contactsFixture = false, driveFixture = false, remindersFixture = false, viewport } of shots) {
  const ctx = await browser.newContext({
    viewport: viewport ?? { width: 1040, height: 720 },
    deviceScaleFactor: 2,
    colorScheme: theme,
  });
  await ctx.addInitScript(installStub, { screen, services: SERVICES, twoFactor, calendarFixture, notesFixture, contactsFixture, driveFixture, remindersFixture });
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
