// Screenshot every auth screen against the real built bundle.
// Tauri IPC doesn't exist in a plain browser, so we stub __TAURI_INTERNALS__ with the exact
// shapes the Rust commands return — no invented UI state.
import { chromium } from "playwright";
import { fileURLToPath } from "node:url";
import path from "node:path";

const dir = path.dirname(fileURLToPath(import.meta.url));
const BASE = process.env.BASE ?? "http://localhost:4173";

const SERVICES = ["account","archivews","calendar","ckdatabasews","ckdeviceservice",
                  "cksharews","contacts","contactsweb","developerapi","docws","drivews",
                  "findme","geows","iwmb","iworkexportws","iworkthumbnailws","keyvalue",
                  "mail","mcc","mccgateway","mpp","notes","photos","photosupload",
                  "premiummailsettings","push","reminders","settings","sharedlibrary",
                  "sharedstreams","ubiquity","uploadimagews"];

/* addInitScript SERIALIZES this function — closures do not cross into the page, so every
   value it needs must arrive through the single argument. */
/* Fixture events, so the agenda's rendering is verified without inventing rows in anyone's
   real library. Times are unix seconds relative to now. */
function installStub({ screen, services, twoFactor, calendarFixture, notesFixture, contactsFixture, driveFixture, remindersFixture }) {
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
  const dueReminders = !calendarFixture ? [] : [
    { guid: "r1", listGuid: "daily", title: "Water the plants", notes: null,
      dueAt: base + DAY + 8 * HOUR, completedAt: null, priority: 1, etag: null },
    { guid: "r2", listGuid: "health", title: "Evening run", notes: null,
      dueAt: base + 6 * HOUR, completedAt: null, priority: 0, etag: null },
    { guid: "r3", listGuid: "daily", title: "Plan tomorrow", notes: null,
      dueAt: base + 13 * HOUR, completedAt: base + 13 * HOUR, priority: 0, etag: null },
  ];
  const noteFolders = [
    { guid: "f1", title: "Notes", sortOrder: 0 },
    { guid: "f2", title: "Recently Deleted", sortOrder: 1 },
  ];
  const noteList = !notesFixture ? [] : [
    { guid: "n1", folderGuid: "f1", title: "Curtains",
      snippet: "Bathroom accessories like mirror holder…",
      body: "Curtains\nBathroom accessories like mirror holder, soap holder, shampoo, clothes hanger\nFan\nwall clock\nBed\nDining table\nTea filter\nKnife\nWashing Machine cover",
      locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1788500000, etag: null },
    { guid: "n2", folderGuid: "f1", title: "🔴 UNIT 1: SOFTWARE TESTING METHODOLOGIES",
      snippet: "📌 SECTION A: 2-MARK HIGH-YIELD…",
      body: "🔴 UNIT 1: SOFTWARE TESTING METHODOLOGIES\n\n📌 SECTION A: 2-MARK HIGH-YIELD QUESTIONS\n\nSoftware testing is the process of evaluating a system to find whether it satisfies the specified requirements, and to detect the differences between expected and actual results.",
      locked: false, editable: true, pinned: true, createdAt: null, modifiedAt: 1787900000, etag: null },
    { guid: "n3", folderGuid: "f1",
      title: "These sources outline the fundamental principles and methodologies",
      snippet: "of **Software Testing Methodologies…",
      body: "These sources outline the fundamental principles and methodologies of **Software Testing Methodologies (CSEN3311)**, emphasizing its evolution from simple debugging to a structured, process-oriented discipline. The material details the **Software Testing Life Cycle (STLC)**, covering essential phases from initial planning and test design to execution and post-implementation review.",
      locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1787800000, etag: null },
  ];
  const contactList = !contactsFixture ? [] : [
    { guid: "c1", firstName: "Zoya", lastName: "Ahuja", company: null, isCompany: false,
      birthday: null, notes: null, etag: null,
      methods: [{ kind: "phone", label: "mobile", value: "+91 90000 00001" },
                { kind: "email", label: "home", value: "zoya@example.com" }] },
    { guid: "c2", firstName: "Amma", lastName: null, company: null, isCompany: false,
      birthday: null, notes: null, etag: null,
      methods: [{ kind: "phone", label: null, value: "+91 98765 43210" }] },
    { guid: "c3", firstName: null, lastName: null, company: "Northfield University",
      isCompany: true, birthday: null, notes: null, etag: null,
      methods: [{ kind: "phone", label: "work", value: "+91 891 284 0000" },
                { kind: "url", label: null, value: "northfield.example" }] },
    { guid: "c4", firstName: null, lastName: null, company: null, isCompany: false,
      birthday: null, notes: null, etag: null,
      methods: [{ kind: "phone", label: null, value: "+91 90000 11111" }] },
  ];
  const driveItems = !driveFixture ? [] : [
    { drivewsid: "FOLDER::a", parentId: null, name: "Downloads", extension: null,
      kind: "FOLDER", size: null, childCount: 3, createdAt: 1785000000,
      modifiedAt: 1788000000, etag: null },
    { drivewsid: "APPLIB::b", parentId: null, name: "Keynote", extension: null,
      kind: "APP_LIBRARY", size: null, childCount: null, createdAt: 1780000000,
      modifiedAt: null, etag: null },
    { drivewsid: "APPLIB::c", parentId: null, name: "Shortcuts", extension: null,
      kind: "APP_LIBRARY", size: null, childCount: null, createdAt: 1780000000,
      modifiedAt: null, etag: null },
    { drivewsid: "FILE::d", parentId: null, name: "Semester Plan", extension: "pdf",
      kind: "FILE", size: 2411724, childCount: null, createdAt: 1787000000,
      modifiedAt: 1788600000, etag: null },
  ];
  const reminderLists = !remindersFixture ? [] : [
    { guid: "l1", title: "Daily", colour: "#63da38", ctag: null, sortOrder: 0, enabled: true },
    { guid: "l2", title: "GYM", colour: "#cc73e1", ctag: null, sortOrder: 1, enabled: true },
    { guid: "l3", title: "Life Goals", colour: "#1badf8", ctag: null, sortOrder: 2, enabled: true },
  ];
  const reminderItems = !remindersFixture ? [] : [
    { guid: "r1", listGuid: "l1", title: "Plan tomorrow", notes: null,
      dueAt: base - 2 * HOUR, completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null },
    { guid: "r2", listGuid: "l1", title: "Pick up the parcel", notes: null,
      dueAt: base + DAY + 7 * HOUR, completedAt: null, priority: 0, flagged: true, parentGuid: null, etag: null },
    { guid: "r3", listGuid: "l2", title: "Evening run", notes: "Legs today",
      dueAt: base + 6 * HOUR, completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null },
    { guid: "r4", listGuid: "l1", title: "Water the plants", notes: null,
      dueAt: base - DAY, completedAt: base - DAY, priority: 0, flagged: false, parentGuid: null, etag: null },
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
        case "event_save": return "E-new";
        case "event_delete": return null;
        case "calendar_sync": return { calendars: calendars.length, events: events.length };
        case "reminder_counts": return { today: 4, scheduled: 7, flagged: 2, all: 12, lists: [["L1", 5], ["L2", 3], ["lists", 4]] };
        case "reminder_lists": return !remindersFixture ? [] : reminderLists;
        case "reminders_in": return !remindersFixture ? [] : reminderItems;
        case "reminders_due_between": return calendarFixture ? dueReminders : [];
        case "reminder_create": return null;
        case "reminder_delete": return null;
        case "reminder_set_done": return null;
        case "reminder_set_repeat": return null;
        case "reminder_edit": return null;
        case "reminder_list_create": case "reminder_list_rename": case "reminder_list_delete":
          return null;
        case "reminders_smart": return !remindersFixture ? [] : reminderItems.filter(r => !r.completedAt);
        case "note_folders": return !notesFixture ? [] : noteFolders;
        case "note_save": case "note_delete": case "note_move": return null;
        case "note_create": return "Note/new";
        case "notes_in": return !notesFixture ? [] : noteList;
        case "contact_save": return "C-new";
        case "contact_delete": return null;
        case "contacts_list": return !contactsFixture ? [] : contactList;
        case "drive_children": return !driveFixture ? [] : driveItems;
        case "drive_sync_folder": return driveItems.length;
        case "drive_download": return "/home/user/Downloads/file.pdf";
        case "drive_create_folder": case "drive_rename": case "drive_delete": case "drive_move": return null;
        case "drive_item": return null;
        case "auth_two_factor_options": return twoFactor;
        case "auth_resend_code":    return null;
        case "auth_send_sms":       return null;
        case "auth_submit_sms_code": return null;
        case "auth_sign_out":    return null;
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
      await p.waitForSelector(".rem-list-items li");
      await p.click('.rem-list:has-text("Daily")');
      await p.waitForTimeout(200);
      await p.click('.rem-more >> nth=0');
    } },
  // A full-screen window on a large display is where content stranded in a narrow column
  // becomes obvious, and every shot above is too small to show it.
  { name: "14-reminders-wide", screen: "library", theme: "dark", remindersFixture: true,
    viewport: { width: 1920, height: 1080 },
    act: async (p) => {
      await p.click('.nav-item:has-text("Reminders")');
      await p.waitForSelector(".rem-list-items li");
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
    act: async (p) => { await p.waitForSelector(".cal-grid"); } },
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
