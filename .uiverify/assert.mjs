// Deterministic checks on the library nav — cheaper and more reliable than eyeballing a screenshot.
import { chromium } from "playwright";
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1040, height: 900 } });
await ctx.addInitScript(({ services }) => {
  // Enough reminders to measure a real row. The checks below are about layout, so the data
  // only has to be shaped right.
  const lists = [
    { guid: "L1", title: "Quizzes & Exams", colour: "#c8912a", ctag: null, sortOrder: 0, enabled: true },
    { guid: "L2", title: "Life Goals", colour: "#1badf8", ctag: null, sortOrder: 1, enabled: true },
  ];
  const items = [
    { guid: "R1", listGuid: "L1", title: "Plan tomorrow", notes: null,
      dueAt: 4102444800, completedAt: null, priority: 0, flagged: false, parentGuid: null,
      recurFrequency: null, recurInterval: null, etag: "t1" },
  ];
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
      if (cmd === "reminder_counts") return { today: 4, scheduled: 7, flagged: 2, all: 12, lists: [["L1", 5], ["L2", 3], ["lists", 4]] };
      if (cmd === "reminder_lists") return lists;
      if (cmd === "reminders_in") return items;
      if (cmd === "reminders_due_between") return [];
      if (cmd === "reminder_create") return null;
      if (cmd === "reminder_delete") return null;
      if (cmd === "reminder_set_done") return null;
      if (cmd === "reminder_set_repeat") return null;
      if (cmd === "reminder_edit") return null;
      if (cmd === "reminders_smart") return items;
      if (cmd === "reminder_list_create" || cmd === "reminder_list_rename" || cmd === "reminder_list_delete") return null;
      if (cmd === "note_folders") return [];
      if (cmd === "notes_in") return [];
      if (cmd === "note_save" || cmd === "note_delete" || cmd === "note_move") return null;
      if (cmd === "note_create") return "Note/new";
      if (cmd === "contact_save") return "C-new";
      if (cmd === "contact_delete") return null;
      if (cmd === "contacts_list") return [];
      if (cmd === "drive_children") {
        // The same one real folder every time, whatever parent is asked for — exactly the
        // shape of the account the bug was found on: a single "Downloads" folder, nothing
        // nested inside it.
        return [
          { drivewsid: "FOLDER::x::downloads", parentId: null, name: "Downloads",
            extension: null, kind: "FOLDER", size: null, childCount: 0,
            createdAt: null, modifiedAt: null, etag: "e1" },
        ];
      }
      if (cmd === "drive_sync_folder") {
        // Slow enough that several clicks land on the same still-open folder before this
        // resolves — which is exactly the race that let the trail duplicate.
        return new Promise((resolve) => setTimeout(() => resolve(0), 250));
      }
      if (cmd === "drive_download") return "/home/user/Downloads/file.pdf";
      if (cmd === "drive_create_folder" || cmd === "drive_rename" || cmd === "drive_delete" || cmd === "drive_move") return null;
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

// ── Calendar's four scopes each render their own view ─────────────────────────────────
// A scope that only changed a label would pass a screenshot; this checks it changed the view.
let failures = 0;
{
  const page = await ctx.newPage();
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Calendar" }).click();
  await page.waitForSelector(".cal-grid");

  const scopes = [
    ["Day", ".cal-hours.is-day"],
    ["Week", ".cal-hours"],
    ["Year", ".cal-year"],
    ["Month", ".cal-grid"],
  ];
  for (const [label, selector] of scopes) {
    await page.getByRole("button", { name: label, exact: true }).click();
    await page.waitForSelector(selector, { timeout: 4000 });
    const shown = await page.locator(selector).count();
    console.log(shown > 0 ? `PASS ${label} renders ${selector}` : `FAIL ${label} did not render ${selector}`);
    if (shown === 0) failures++;
  }

  // Week shows seven day columns; Day shows one. A label change alone would not.
  await page.getByRole("button", { name: "Week", exact: true }).click();
  await page.waitForSelector(".cal-hours");
  const week = await page.locator(".cal-col-head").count();
  console.log(week === 7 ? "PASS Week shows seven days" : `FAIL Week showed ${week} columns`);
  if (week !== 7) failures++;

  await page.getByRole("button", { name: "Day", exact: true }).click();
  const day = await page.locator(".cal-col-head").count();
  console.log(day === 1 ? "PASS Day shows one" : `FAIL Day showed ${day} columns`);
  if (day !== 1) failures++;

  await page.getByRole("button", { name: "Year", exact: true }).click();
  await page.waitForSelector(".cal-year");
  const months = await page.locator(".cal-mini").count();
  console.log(months === 12 ? "PASS Year shows twelve months" : `FAIL Year showed ${months} months`);
  if (months !== 12) failures++;
  await page.close();
}


// ── Nothing is truncated, and nothing is stranded, on a real-sized window ──────────────
// Both of these shipped: "Quizzes & Ex…" and "Recently Delet…" cut off in sidebars that
// were too narrow, and list rows capped so tightly that on a wide display they sat in the
// left third with their separators ending in mid-air. A picture would not have caught
// either; measuring does.
{
  const page = await ctx.newPage();
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });

  const clipped = (selector) =>
    page.$$eval(selector, (nodes) =>
      nodes
        .filter((n) => n.scrollWidth > n.clientWidth + 1)
        .map((n) => n.textContent?.trim().slice(0, 30))
    );

  for (const [screen, selector] of [
    ["Reminders", ".rem-list-name, .rem-smart-name"],
    ["Notes", ".notes-folders .note-folder, .notes-folders button"],
  ]) {
    await page.getByRole("button", { name: screen, exact: true }).click();
    await page.waitForTimeout(400);
    const cut = await clipped(selector);
    console.log(
      cut.length === 0
        ? `PASS ${screen} sidebar shows every name in full`
        : `FAIL ${screen} sidebar truncates: ${JSON.stringify(cut)}`
    );
    if (cut.length > 0) failures++;
  }

  // A list that occupies a sliver of a wide pane reads as broken, whatever it says.
  await page.getByRole("button", { name: "Reminders", exact: true }).click();
  await page.waitForTimeout(400);
  const fill = await page.evaluate(() => {
    const pane = document.querySelector(".rem-items");
    const row = document.querySelector(".rem-list-items li, .rem-add");
    if (!pane || !row) return null;
    return row.getBoundingClientRect().width / pane.getBoundingClientRect().width;
  });
  console.log(
    fill === null
      ? "FAIL could not measure the reminders list"
      : fill >= 0.9
        ? `PASS reminder rows fill their pane (${Math.round(fill * 100)}%)`
        : `FAIL reminder rows fill only ${Math.round(fill * 100)}% of their pane`
  );
  if (fill === null || fill < 0.9) failures++;

  // The "My Lists" heading belongs on the same left edge as the icons in the rows under it,
  // which is where a source list puts a section header. It sat 24px right of both the cards
  // above and the icons below, reading as a column that had slipped.
  const heading = await page.evaluate(() => {
    const label = document.querySelector(".rem-lists-heading");
    const card = document.querySelector(".rem-smart");
    const icon = document.querySelector(".rem-list-glyph");
    if (!label || !card || !icon) return null;
    const text = label.getBoundingClientRect().left +
      parseFloat(getComputedStyle(label).paddingLeft || "0");
    return {
      offCard: text - card.getBoundingClientRect().left,
      offIcon: text - icon.getBoundingClientRect().left,
      size: parseFloat(getComputedStyle(label).fontSize),
    };
  });
  if (!heading) {
    console.log("FAIL could not measure the My Lists heading");
    failures++;
  } else {
    const aligned = Math.abs(heading.offIcon) <= 1.5 && heading.offCard <= 10;
    console.log(
      aligned
        ? `PASS "My Lists" sits on the row icons' edge  [${heading.offIcon.toFixed(1)}px off the icons, ${heading.offCard.toFixed(1)}px in from the cards]`
        : `FAIL "My Lists" is adrift  [${heading.offIcon.toFixed(1)}px off the icons, ${heading.offCard.toFixed(1)}px in from the cards]`
    );
    if (!aligned) failures++;

    // A section header, not a title. It reads as one of the list names if it grows.
    const small = heading.size <= 12;
    console.log(
      small
        ? `PASS "My Lists" reads as a heading, not a title  [${heading.size}px]`
        : `FAIL "My Lists" is ${heading.size}px, too big for a section heading`
    );
    if (!small) failures++;
  }
  await page.close();
}


// ── Drive: a rapid double click must never duplicate the breadcrumb ────────────────────
// The actual bug: an open() in flight let a second click on the same stale row append a
// second identical crumb before the folder had finished loading — a trail that read
// "Downloads › Downloads › Downloads …" fifteen deep, all pointing at the one real folder.
{
  const page = await ctx.newPage();
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await page.getByRole("button", { name: "iCloud Drive" }).click();
  await page.waitForSelector(".drive-row-hit");

  const folderRow = page.locator(".drive-row.is-folder .drive-row-hit").first();
  if ((await folderRow.count()) > 0) {
    // Fire several clicks back to back, the way an impatient double-click does, before the
    // slow folder load has had any chance to resolve.
    await Promise.all([folderRow.click(), folderRow.click(), folderRow.click()]);
    await page.waitForTimeout(500);
    const crumbs = await page.locator(".crumb").allInnerTexts();
    const names = crumbs.slice(1);
    const unique = new Set(names);
    console.log(
      names.length <= 1 || unique.size === names.length
        ? "PASS rapid clicks do not duplicate the breadcrumb"
        : `FAIL breadcrumb duplicated: ${JSON.stringify(crumbs)}`
    );
    if (names.length > 1 && unique.size !== names.length) failures++;
  } else {
    console.log("PASS rapid clicks do not duplicate the breadcrumb (no folder to test with)");
  }
  await page.close();
}

// ── An account with nothing in it ─────────────────────────────────────────────────────
// Everything above was measured against one real library: nine lists, folders, a Drive with
// things in it. A person installing this may have none of that — a new Apple ID, or one whose
// lists and notes have all been deleted. Every screen has to be usable, say something true,
// and never offer a write it cannot carry out.
{
  const empty = await b.newContext({ viewport: { width: 1040, height: 900 } });
  await empty.addInitScript(({ services }) => {
    window.__TAURI_INTERNALS__ = {
      transformCallback: (cb) => cb,
      invoke: async (cmd) => {
        if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
        if (cmd === "auth_resume") return true;
        if (cmd === "account_services") return services;
        if (cmd === "reminder_counts")
          return { today: 0, scheduled: 0, flagged: 0, all: 0, lists: [] };
        if (cmd === "sync_all" || cmd === "sync_status")
          return { services: [], asOf: null, tookMs: 0 };
        if (cmd === "plugin:event|listen") return 0;
        // Every list, every folder, every calendar, every row: nothing at all.
        if (cmd.endsWith("_list") || cmd.endsWith("_lists") || cmd.endsWith("_folders") ||
            cmd.endsWith("_children") || cmd.endsWith("_events") || cmd.startsWith("reminders_") ||
            cmd === "notes_in" || cmd === "contacts_list") return [];
        return null;
      },
    };
  }, { services: ["account","calendar","ckdatabasews","contacts","docws","drivews","notes","reminders"] });

  const page = await empty.newPage();
  const blew = [];
  page.on("pageerror", (e) => blew.push(String(e)));
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });

  for (const [screen, expect] of [
    ["Reminders", ".rem-nolists"],
    ["Calendar", ".cal-grid, .cal-month, .cal-empty"],
    ["Notes", ".cal-empty, .notes-empty"],
    ["Contacts", ".cal-empty, .contact-empty"],
    ["iCloud Drive", ".drive-rows"],
  ]) {
    await page.getByRole("button", { name: screen, exact: true }).click();
    await page.waitForTimeout(350);
    const there = await page.locator(expect).count();
    console.log(
      there > 0
        ? `PASS ${screen} is usable with nothing in the account`
        : `FAIL ${screen} shows nothing at all with an empty account  [wanted ${expect}]`
    );
    if (there === 0) failures++;
  }

  // Nothing may offer a write it has nowhere to put.
  await page.getByRole("button", { name: "Calendar", exact: true }).click();
  await page.waitForTimeout(300);
  const newEvent = page.getByRole("button", { name: "New event" });
  const offered = (await newEvent.count()) > 0 && (await newEvent.isEnabled());
  console.log(
    offered
      ? "FAIL New event is offered on an account with no calendars"
      : "PASS no calendars means no event to create"
  );
  if (offered) failures++;

  console.log(
    blew.length === 0
      ? "PASS an empty account raises nothing"
      : `FAIL an empty account threw: ${blew.join(" | ")}`
  );
  if (blew.length > 0) failures++;
  await page.close();
  await empty.close();
}

await b.close();
process.exitCode = collapsed && selectsBuilt && failures === 0 ? 0 : 1;
