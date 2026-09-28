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
      if (cmd === "calendar_list")
        return [
          { guid: "work", title: "Work", colour: null, ctag: null, sortOrder: 0, enabled: true,
            writable: true, isDefault: false },
          { guid: "home", title: "Home", colour: null, ctag: null, sortOrder: 1, enabled: true,
            writable: true, isDefault: true },
          { guid: "holidays", title: "Holidays in India", colour: null, ctag: null, sortOrder: 2,
            enabled: true, writable: false, isDefault: false },
        ];
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

  // Everything in the Reminders column reaches the same edge. The rename and delete buttons
  // used to sit in each row's flow, which took thirty-seven pixels off every list row and left
  // the column with two widths — the cards one, the lists another.
  const column = await page.evaluate(() => {
    const width = (s) => {
      const el = document.querySelector(s);
      return el ? +el.getBoundingClientRect().width.toFixed(1) : null;
    };
    const cards = [...document.querySelectorAll(".rem-smart")].map(
      (el) => +el.getBoundingClientRect().width.toFixed(1)
    );
    return { grid: width(".rem-smart-grid"), row: width(".rem-list"), cards };
  });
  const evenCards = new Set(column.cards).size <= 1;
  const flush = column.grid !== null && column.row !== null &&
    Math.abs(column.grid - column.row) <= 1;
  console.log(
    evenCards && flush
      ? `PASS the reminders column is one width  [cards ${column.cards[0]}, grid ${column.grid}, row ${column.row}]`
      : `FAIL the reminders column is ragged  [cards ${JSON.stringify(column.cards)}, grid ${column.grid}, row ${column.row}]`
  );
  if (!evenCards || !flush) failures++;

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

// ── What acts on a reminder stays beside it ───────────────────────────────────────────
// The separators span the whole pane, so the list does not end in mid-air on a wide window.
// The controls must not span it too: with the text on one stretching column they were pushed
// to the far edge, about thirteen hundred pixels from the reminder they act on.
{
  const page = await ctx.newPage();
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Reminders", exact: true }).click();
  await page.waitForTimeout(400);

  // Measured from the start of the row, not from the text element — that one stretches to
  // fill its column, so the gap after it is always the grid gap and says nothing at all. The
  // first version of this check passed against the layout it was written to catch.
  const reach = await page.evaluate(() => {
    const row = document.querySelector(".rem-list-items li");
    const actions = row?.querySelector(".rem-trailing");
    if (!row || !actions) return null;
    const left = row.getBoundingClientRect().left;
    return {
      reach: +(actions.getBoundingClientRect().left - left).toFixed(0),
      rowWidth: +row.getBoundingClientRect().width.toFixed(0),
    };
  });
  if (!reach) {
    console.log("FAIL could not measure a reminder row");
    failures++;
  } else {
    const close = reach.reach < 900;
    console.log(
      close
        ? `PASS what acts on a reminder stays beside it  [${reach.reach}px in, row ${reach.rowWidth}px]`
        : `FAIL the controls sit ${reach.reach}px into a ${reach.rowWidth}px row`
    );
    if (!close) failures++;
  }
  await page.close();
}

// ── A new event goes where Apple says, not where the list happens to start ────────────
// Apple names the default calendar outright. Taking the first of the list instead is the same
// mistake that put every new reminder in whichever list sorted first — and the first calendar
// can also be a subscribed feed that nothing may be written to.
{
  const page = await ctx.newPage();
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Calendar", exact: true }).click();
  await page.waitForTimeout(300);
  await page.getByRole("button", { name: "New event" }).click();
  await page.waitForTimeout(250);

  const chosen = await page.locator(".cal-field .picker-button").first().innerText();
  const right = chosen.includes("Home");
  console.log(
    right
      ? `PASS a new event starts in the default calendar  [${chosen.split("\n")[0]}]`
      : `FAIL a new event started in "${chosen.split("\n")[0]}" rather than the default`
  );
  if (!right) failures++;

  await page.locator(".cal-field .picker-button").first().click();
  await page.waitForTimeout(200);
  const offered = await page.locator('[role="option"], .picker-option').allInnerTexts();
  const readOnlyOffered = offered.some((o) => o.includes("Holidays"));
  console.log(
    readOnlyOffered
      ? `FAIL a read-only calendar is offered to put an event in  [${offered.join(", ")}]`
      : `PASS a read-only calendar is not somewhere to put an event  [${offered.join(", ")}]`
  );
  if (readOnlyOffered) failures++;
  await page.close();
}

// ── Notes can be searched ─────────────────────────────────────────────────────────────
{
  const page = await ctx.newPage();
  await page.addInitScript(() => {
    const n = (guid, title) => ({ guid, folderGuid: "F", title, snippet: null, body: null,
      locked: false, editable: true, pinned: false, createdAt: 1, modifiedAt: 1, etag: null });
    window.__TAURI_INTERNALS__ = {
      transformCallback: (cb) => cb,
      invoke: async (cmd, args) => {
        if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
        if (cmd === "auth_resume") return true;
        if (cmd === "account_services") return ["notes"];
        if (cmd === "note_folders") return [{ guid: "F", title: "Notes", sortOrder: 0, parentGuid: null }];
        if (cmd === "notes_in") return [n("A", "Curtains"), n("B", "Shopping")];
        if (cmd === "notes_search") return args.query.toLowerCase().includes("curt") ? [n("A", "Curtains")] : [];
        if (cmd === "note_body") return "";
        if (cmd === "sync_all" || cmd === "sync_status") return { services: [], asOf: 1, tookMs: 0 };
        if (cmd === "plugin:event|listen") return 0;
        return [];
      },
    };
  });
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Notes", exact: true }).click();
  await page.waitForTimeout(300);
  await page.fill('input[aria-label="Search notes"]', "curt");
  await page.waitForTimeout(300);
  const shown = await page.locator(".notes-list").innerText();
  const narrowed = shown.includes("Curtains") && !shown.includes("Shopping");
  await page.press('input[aria-label="Search notes"]', "Escape");
  await page.waitForTimeout(300);
  const back = (await page.locator(".notes-list").innerText()).includes("Shopping");
  console.log(
    narrowed && back
      ? "PASS searching narrows the notes, and Escape brings them back"
      : `FAIL search: narrowed=${narrowed} back=${back}`
  );
  if (!(narrowed && back)) failures++;
  await page.close();
}

// ── Reminders can be searched, across every list ──────────────────────────────────────
{
  const page = await ctx.newPage();
  await page.addInitScript(() => {
    const r = (guid, listGuid, title) => ({ guid, listGuid, title, notes: null, dueAt: null,
      completedAt: null, priority: 0, flagged: false, parentGuid: null, recurFrequency: null,
      recurInterval: null, sectionGuid: null, etag: null });
    window.__TAURI_INTERNALS__ = {
      transformCallback: (cb) => cb,
      invoke: async (cmd, args) => {
        if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
        if (cmd === "auth_resume") return true;
        if (cmd === "account_services") return ["reminders"];
        if (cmd === "reminder_lists") return [
          { guid: "G", title: "GYM", colour: "#0a84ff", ctag: null, sortOrder: 0, enabled: true },
          { guid: "D", title: "Daily", colour: "#ffd60a", ctag: null, sortOrder: 1, enabled: true }];
        if (cmd === "reminder_counts") return { today: 0, scheduled: 0, flagged: 0, all: 2, lists: [] };
        if (cmd === "reminders_smart" || cmd === "reminders_in") return [r("A", "G", "Evening run"), r("B", "D", "Water the plants")];
        if (cmd === "reminders_search") return args.query.includes("plant") ? [r("B", "D", "Water the plants")] : [];
        if (cmd === "sync_all" || cmd === "sync_status") return { services: [], asOf: 1, tookMs: 0 };
        if (cmd === "plugin:event|listen") return 0;
        return [];
      },
    };
  });
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Reminders", exact: true }).click();
  await page.waitForTimeout(300);
  await page.fill('input[aria-label="Search reminders"]', "plant");
  await page.waitForTimeout(300);
  const titles = await page.evaluate(() =>
    [...document.querySelectorAll(".rem-list-items li input")].map((i) => i.value));
  const heads = await page.locator(".rem-section").allInnerTexts();
  const right = JSON.stringify(titles) === JSON.stringify(["Water the plants"]) && heads.includes("Daily");
  console.log(
    right
      ? "PASS searching reminders finds across lists, under the list it came from"
      : `FAIL reminder search showed ${JSON.stringify(titles)} under ${JSON.stringify(heads)}`
  );
  if (!right) failures++;
  await page.close();
}

// ── A list shows its own sections ─────────────────────────────────────────────────────
// Inside a real list, reminders outside any section come first with no heading, then each
// section under its own heading — how Apple shows it, and how the phone showed "🌅".
{
  const page = await ctx.newPage();
  await page.addInitScript(() => {
    const r = (guid, title, sectionGuid) => ({ guid, listGuid: "L", title, notes: null, dueAt: null,
      completedAt: null, priority: 0, flagged: false, parentGuid: null, recurFrequency: null,
      recurInterval: null, sectionGuid, etag: null });
    window.__TAURI_INTERNALS__ = {
      transformCallback: (cb) => cb,
      invoke: async (cmd) => {
        if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
        if (cmd === "auth_resume") return true;
        if (cmd === "account_services") return ["reminders"];
        if (cmd === "reminder_lists") return [{ guid: "L", title: "Life Goals", colour: "#1badf8", ctag: null, sortOrder: 0, enabled: true }];
        if (cmd === "reminder_counts") return { today: 0, scheduled: 0, flagged: 0, all: 3, lists: [["L", 3]] };
        if (cmd === "reminder_sections") return [{ guid: "S", listGuid: "L", title: "Morning Habits", createdAt: 1 }];
        if (cmd === "reminders_in" || cmd === "reminders_smart")
          return [r("A", "Loose one", null), r("B", "In the section", "S"), r("C", "In a deleted section", "GONE")];
        if (cmd === "sync_all" || cmd === "sync_status") return { services: [], asOf: 1, tookMs: 0 };
        if (cmd === "plugin:event|listen") return 0;
        return [];
      },
    };
  });
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Reminders", exact: true }).click();
  await page.waitForTimeout(300);
  await page.locator(".rem-list", { hasText: "Life Goals" }).click();
  await page.waitForTimeout(400);

  const layout = await page.evaluate(() => {
    const out = [];
    for (const el of document.querySelectorAll(".rem-section, .rem-list-items li")) {
      out.push(el.matches(".rem-section") ? `#${el.textContent.trim()}` : (el.querySelector("input")?.value ?? ""));
    }
    return out;
  });
  const want = ["Loose one", "In a deleted section", "#Morning Habits", "In the section"];
  const right = JSON.stringify(layout) === JSON.stringify(want);
  console.log(
    right
      ? "PASS a list shows its sections, loose reminders first"
      : `FAIL sections laid out as ${JSON.stringify(layout)}`
  );
  if (!right) failures++;
  await page.close();
}

// ── Coming back to the window brings it up to date ────────────────────────────────────
// That is the moment somebody looks for a change they just made on their phone, and between
// background passes the window could be five minutes stale right then. It must refresh on
// return — and not again for every alt-tab, which would only ask Apple the same thing twice.
{
  const page = await ctx.newPage();
  await page.addInitScript(() => {
    window.__passes = 0;
    window.__TAURI_INTERNALS__ = {
      transformCallback: (cb) => cb,
      invoke: async (cmd) => {
        if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
        if (cmd === "auth_resume") return true;
        if (cmd === "account_services") return ["reminders"];
        if (cmd === "sync_all") { window.__passes++; return { services: [], asOf: 1, tookMs: 0 }; }
        if (cmd === "sync_status") return { services: [], asOf: 1, tookMs: 0 };
        if (cmd === "plugin:event|listen") return 0;
        return [];
      },
    };
  });
  await page.clock.install();
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await page.waitForTimeout(300);
  const onOpen = await page.evaluate(() => window.__passes);

  const bounce = () => page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await bounce();
  await page.waitForTimeout(100);
  const tooSoon = (await page.evaluate(() => window.__passes)) - onOpen;

  await page.clock.fastForward(31_000);
  await bounce();
  await page.waitForTimeout(200);
  const later = (await page.evaluate(() => window.__passes)) - onOpen;

  const right = tooSoon === 0 && later === 1;
  console.log(
    right
      ? "PASS coming back refreshes, and alt-tabbing does not hammer"
      : `FAIL returning: ${tooSoon} pass(es) straight away, ${later} after half a minute`
  );
  if (!right) failures++;
  await page.close();
}

// ── An account with nowhere to send a code ────────────────────────────────────────────
// No trusted device and no trusted number. The screen must not offer a code that can never
// arrive, and must say what would actually help — otherwise it is a dead end with a text box
// in the middle of it.
{
  for (const [label, twoFactor, wants] of [
    ["nowhere to send", { trustedDeviceCount: 0, phones: [], tooManyCodesSent: false },
      { resend: false, text: false, says: /account\.apple\.com/ }],
    ["no device, but a number", { trustedDeviceCount: 0, phones: [{ id: 1, maskedNumber: "+91 •••318" }], tooManyCodesSent: false },
      { resend: false, text: true, says: /texted/i }],
    ["devices and a number", { trustedDeviceCount: 2, phones: [{ id: 1, maskedNumber: "+91 •••318" }], tooManyCodesSent: false },
      { resend: true, text: true, says: /2 of your devices/ }],
  ]) {
    const page = await ctx.newPage();
    await page.addInitScript((tf) => {
      window.__TAURI_INTERNALS__ = {
        transformCallback: (cb) => cb,
        invoke: async (cmd) => {
          if (cmd === "auth_status") return { hasStoredSession: false, signedIn: false };
          if (cmd === "auth_sign_in") return "needs_two_factor";
          if (cmd === "auth_two_factor_options") return tf;
          if (cmd === "plugin:event|listen") return 0;
          return null;
        },
      };
    }, twoFactor);
    await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
    await page.fill('input[aria-label="Apple ID"]', "you@icloud.com");
    await page.fill('input[aria-label="Password"]', "hunter2hunter2");
    await page.click('button[type="submit"]');
    await page.waitForSelector(".code", { timeout: 5000 });
    // Where no device is reachable the help opens by itself, so there is nothing to click.
    const opener = page.locator("button", { hasText: /get a code/ });
    if (await opener.count()) await opener.click();
    await page.waitForTimeout(250);

    const body = await page.locator(".sheet").innerText();
    const resend = await page.locator('button:has-text("Send to my devices again")').count();
    const text = await page.locator('button:has-text("Text ")').count();

    const right =
      (resend > 0) === wants.resend &&
      (text > 0) === wants.text &&
      wants.says.test(body);
    console.log(
      right
        ? `PASS ${label}: offers only what can work`
        : `FAIL ${label}: resend=${resend} text=${text}, said ${JSON.stringify(body.slice(0, 90))}`
    );
    if (!right) failures++;
    await page.close();
  }
}

// ── What somebody is told before they type an Apple ID in ─────────────────────────────
// Required rather than nice to have: this is software a stranger wrote, reaching iCloud
// through interfaces Apple has not published. Shut, it must not shout; open, it must say so.
{
  const page = await ctx.newPage();
  await page.addInitScript(() => {
    window.__TAURI_INTERNALS__ = {
      transformCallback: (cb) => cb,
      invoke: async (cmd) => {
        if (cmd === "auth_status") return { hasStoredSession: false, signedIn: false };
        if (cmd === "plugin:event|listen") return 0;
        return null;
      },
    };
  });
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await page.waitForTimeout(300);

  const shut = await page.locator(".disclosure").innerText();
  const quiet = shut.length < 120;
  console.log(
    quiet
      ? `PASS the disclosure is one line until asked  [${shut.replace(/\n/g, " ").slice(0, 60)}]`
      : `FAIL the sign-in screen opens with ${shut.length} characters of disclosure`
  );
  if (!quiet) failures++;

  await page.locator(".disclosure > summary").click();
  await page.waitForTimeout(200);
  const open = (await page.locator(".disclosure").innerText()).toLowerCase();
  const says = {
    "that it is unofficial": /unofficial|not affiliated/.test(open),
    "that Apple can change it": /without notice|stop working/.test(open),
    "where the copy is kept": /local\/share|dev\.quince\.client/.test(open),
    "that the password stays here": /never leaves this computer/.test(open),
    "what no client can read": /end to end|encrypted end/.test(open),
  };
  for (const [what, said] of Object.entries(says)) {
    console.log(said ? `PASS it says ${what}` : `FAIL it does not say ${what}`);
    if (!said) failures++;
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
