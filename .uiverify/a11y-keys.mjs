// What axe cannot judge, checked as a keyboard and screen-reader user meets it: the search is
// reachable however the sidebar is drawn (whole, folded to icons, or put away in a narrow
// window) and its Show All is reached by the arrows; Escape closes the topmost layer and the
// focus goes back to what opened it — menus, sheets, popovers, the photo viewer; modal sheets
// keep Tab inside; one Tab stop for a date grid, with the arrows inside it; the note editor
// lets the keys out; states are named (aria-checked, aria-current, aria-pressed); notices are
// said by a live region; Reduce Motion stops motion; High Contrast keeps edges and rings;
// Increase Contrast firms the focus ring.
//
//   BASE=http://localhost:4260 node .uiverify/a11y-keys.mjs
//   BASE=http://localhost:4260 ENGINE=webkit node .uiverify/a11y-keys.mjs
//   ONLY=search                              only the checks whose group matches
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.BASE ?? "http://localhost:4173";
const ONLY = process.env.ONLY ? new RegExp(process.env.ONLY, "i") : null;
const FIXTURES = { calendarFixture: true, notesFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true };
const b = await engine.launch();
let failures = 0;
const ok = (pass, m) => {
  console.log(`${pass ? "PASS" : "FAIL"} ${engineName} ${m}`);
  if (!pass) failures++;
};
const wait = (p, ms = 250) => p.waitForTimeout(ms);
/** A description of what has the focus: its role, name and class, for the messages. */
const focused = (p) =>
  p.evaluate(() => {
    const el = document.activeElement;
    if (!el || el === document.body) return "(the page)";
    return `${el.getAttribute("role") ?? el.tagName.toLowerCase()} "${(el.getAttribute("aria-label") ?? el.textContent ?? "").trim().slice(0, 40)}"`;
  });
const has = (p, selector) => p.evaluate((s) => Boolean(document.activeElement?.closest(s)), selector);

async function fresh(opts = {}, extra, fixtures = FIXTURES) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, ...opts });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, ...fixtures });
  if (extra) await ctx.addInitScript(extra);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(BASE, { waitUntil: "networkidle" });
  await page.waitForSelector(".sidebar .nav-item", { state: "attached" });
  return { ctx, page };
}
async function group(name, run) {
  if (ONLY && !ONLY.test(name)) return;
  try {
    await run();
  } catch (e) {
    ok(false, `${name}: ${String(e.message).split("\n")[0]}`);
  }
}
/** Focus a control the way the keys would land on it, so :focus-visible holds. */
const keyFocus = async (p, locator) => {
  await locator.evaluate((el) => el.focus());
  await p.keyboard.press("Shift");
};

// ---- Search ----------------------------------------------------------------------------------

const manyContacts = () => {
  window.__moreContacts = Array.from({ length: 40 }, (_, i) => ({ guid: `m${i}`, firstName: "Marlowe", lastName: `Test ${i + 1}`,
    company: null, isCompany: false, birthday: null, notes: null, etag: null, methods: [] }));
};

await group("search", async () => {
  const { ctx, page } = await fresh({}, manyContacts);
  const field = page.getByRole("combobox", { name: "Search everything" });
  const list = page.getByRole("listbox", { name: "Found" });
  await openScreen(page, "Drive");
  await page.keyboard.press("Control+k");
  ok(await field.evaluate((f) => f === document.activeElement), "search: Ctrl+K comes to the field");
  await page.keyboard.type("marlowe");
  await wait(page, 300);
  ok(/\d+ results?/.test(await page.locator(".gsearch [role=status]").innerText()), `search: how many were found is said  (${await page.locator(".gsearch [role=status]").innerText()})`);
  const sel = () => list.locator('[role="option"][aria-selected="true"]').innerText().catch(() => "");
  ok(/Marlowe/.test(await sel()), "search: the first result is selected, not Show All");
  await page.keyboard.press("ArrowUp");
  ok(/Show All/.test(await sel()), `search: Up from the first result reaches Show All  (${await sel()})`);
  const active = await field.getAttribute("aria-activedescendant");
  ok(active && /Show all 40 in Contacts/.test((await page.locator(`#${active}`).getAttribute("aria-label")) ?? ""), "search: and the field names it to assistive tech");
  await page.keyboard.press("Enter");
  await wait(page);
  ok(await list.getByRole("option", { name: /^Show less of Contacts/ }).isVisible(), "search: Return on Show All shows the app whole, with Show Less");
  await page.keyboard.press("Escape");
  await wait(page);
  ok(/Show All/.test(await sel()), "search: Escape comes back to every app, on the Show All it left from");
  ok(await field.evaluate((f) => f === document.activeElement), "search: the focus never left the field");
  const rows = await list.locator('[role="option"]').evaluateAll((els) => els.map((e) => e.tabIndex));
  ok(rows.every((t) => t === -1), "search: results are no Tab stops of their own (the field holds the keys)");
  await ctx.close();
});

await group("search folded", async () => {
  const { ctx, page } = await fresh();
  // Notes first: the suites' openScreen presses anything named "Show Sidebar…".
  await openScreen(page, "Notes");
  await page.getByRole("button", { name: "Fold Sidebar to Icons" }).click();
  await wait(page);
  const magnifier = page.locator(".gsearch-magnifier");
  ok(await magnifier.isVisible(), "search folded: the rail keeps a magnifier");
  await page.locator("main.content").click({ position: { x: 600, y: 400 } });
  await page.keyboard.press("Control+k");
  await wait(page);
  const field = page.getByRole("combobox", { name: "Search everything" });
  ok((await field.isVisible()) && (await field.evaluate((f) => f === document.activeElement)), "search folded: Ctrl+K brings the field out, focused");
  await page.keyboard.type("curtains");
  await wait(page, 300);
  ok(await page.getByRole("listbox", { name: "Found" }).isVisible(), "search folded: results show under it");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await wait(page);
  ok(!(await field.isVisible()) && (await has(page, ".gsearch-magnifier")), `search folded: Escape puts it away, the focus back on the magnifier  (${await focused(page)})`);
  await page.keyboard.press("Enter");
  await wait(page);
  ok(await field.evaluate((f) => f === document.activeElement), "search folded: the magnifier brings it out by the keys too");
  await ctx.close();
});

await group("search narrow", async () => {
  const { ctx, page } = await fresh({ viewport: { width: 800, height: 700 } });
  await page.locator("main.content").click({ position: { x: 400, y: 400 } });
  await page.keyboard.press("Control+k");
  await wait(page, 400);
  const field = page.getByRole("combobox", { name: "Search everything" });
  ok((await field.isVisible()) && (await field.evaluate((f) => f === document.activeElement)), "search narrow: Ctrl+K brings the sidebar out, the field focused");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await wait(page, 400);
  ok(await has(page, ".sidebar-toggle"), `search narrow: put away, the focus is on Show Sidebar  (${await focused(page)})`);
  await ctx.close();
});

// ---- Menus, sheets and popovers: Escape, and the focus back ---------------------------------

await group("menus and sheets", async () => {
  const { ctx, page } = await fresh();
  await openScreen(page, "Drive");
  await wait(page, 400);
  const actions = page.getByRole("button", { name: "Actions for Semester Plan.pdf" });
  await keyFocus(page, actions);
  await page.keyboard.press("Enter");
  await wait(page);
  ok(await page.locator(".row-menu").isVisible(), "menu: Return on ⋯ opens the menu");
  await page.keyboard.press("Escape");
  await wait(page);
  ok(await actions.evaluate((el) => el === document.activeElement), `menu: Escape closes it, the focus back on ⋯  (${await focused(page)})`);

  for (const item of ["Get Info", "Move to Folder"]) {
    await keyFocus(page, actions);
    await page.keyboard.press("Enter");
    await wait(page);
    await page.getByRole("menuitem", { name: item }).focus();
    await page.keyboard.press("Enter");
    await wait(page, 400);
    const sheet = page.locator('[role="dialog"][aria-modal="true"]');
    ok((await sheet.count()) === 1, `${item}: a modal sheet, marked aria-modal`);
    ok(await has(page, '[aria-modal="true"]'), `${item}: the focus is in the sheet  (${await focused(page)})`);
    let inside = true;
    for (let i = 0; i < 12; i++) {
      await page.keyboard.press(i % 3 === 2 ? "Shift+Tab" : "Tab");
      if (!(await has(page, '[aria-modal="true"]'))) inside = false;
    }
    ok(inside, `${item}: Tab and Shift-Tab stay in the sheet`);
    await page.keyboard.press("Escape");
    await wait(page, 300);
    ok((await sheet.count()) === 0 && (await actions.evaluate((el) => el === document.activeElement)), `${item}: Escape closes it, the focus back on ⋯  (${await focused(page)})`);
  }

  await openScreen(page, "Reminders");
  await wait(page, 400);
  const listActions = page.getByRole("button", { name: "Actions for Daily" });
  await keyFocus(page, listActions);
  await page.keyboard.press("Enter");
  await wait(page);
  await page.getByRole("menuitem", { name: "Show List Info" }).focus();
  await page.keyboard.press("Enter");
  await wait(page, 400);
  ok(await has(page, '.rx-info[aria-modal="true"]'), `list info: a modal sheet with the focus in it  (${await focused(page)})`);
  await page.keyboard.press("Escape");
  await wait(page, 300);
  ok(await listActions.evaluate((el) => el === document.activeElement), `list info: Escape returns the focus to ⋯  (${await focused(page)})`);

  // A question: Escape is Cancel, and the focus goes back to what asked it.
  await page.locator(".sidebar-account").click();
  await wait(page, 500);
  const signOut = page.getByRole("button", { name: "Sign Out" });
  await keyFocus(page, signOut);
  await page.keyboard.press("Enter");
  await wait(page);
  ok(await has(page, '[role="alertdialog"]'), "sign out: the question takes the focus");
  await page.keyboard.press("Escape");
  await wait(page);
  ok(await signOut.evaluate((el) => el === document.activeElement), `sign out: Escape cancels, the focus back on Sign Out  (${await focused(page)})`);
  await ctx.close();
});

await group("calendar", async () => {
  const { ctx, page } = await fresh();
  await openScreen(page, "Calendar");
  await wait(page, 500);
  // Month: the event's popover, and back.
  const event = page.locator(".cx-item", { hasText: "Design review" }).first();
  await keyFocus(page, event);
  await page.keyboard.press("Enter");
  await wait(page, 400);
  ok(await page.locator(".cx-popover").isVisible(), "calendar: Return on an event opens its popover");
  await page.keyboard.press("Escape");
  await wait(page, 300);
  ok((await page.locator(".cx-popover").count()) === 0 && (await event.evaluate((el) => el === document.activeElement)),
    `calendar: Escape closes it, the focus back on the event  (${await focused(page)})`);

  // The mini month: one Tab stop, the arrows inside it, today and the chosen day named.
  const mini = page.locator(".cx-minimonth-grid").first();
  const stops = await mini.locator("button").evaluateAll((els) => els.filter((e) => e.tabIndex === 0).length);
  ok(stops === 1, `calendar: the mini month is one Tab stop  (${stops})`);
  ok((await mini.locator('[aria-current="date"]').count()) === 1, "calendar: today is marked aria-current=date");
  ok((await mini.locator('[aria-pressed="true"]').count()) === 1, "calendar: the chosen day is marked pressed");
  const start = mini.locator('button[tabindex="0"]');
  const before = await start.getAttribute("aria-label");
  await keyFocus(page, start);
  await page.keyboard.press("ArrowRight");
  await wait(page, 150);
  const after = await page.evaluate(() => document.activeElement?.getAttribute("aria-label"));
  ok(after && after !== before && (await has(page, ".cx-minimonth-grid")), `calendar: Right moves to the next day  (${before} → ${after})`);
  await page.keyboard.press("ArrowDown");
  await wait(page, 150);
  ok(await has(page, ".cx-minimonth-grid"), "calendar: Down moves a week, the focus kept in the grid");

  // Month view targets: at least 24px, WCAG 2.2.
  const small = await page.locator(".cx-items:not(.is-dots) .cx-item, .cx-more").evaluateAll((els) => els.filter((e) => e.getBoundingClientRect().height < 24).length);
  ok(small === 0, `calendar: every month line is a 24px target  (${small} smaller)`);

  // Year: one Tab stop for the year's days.
  await page.keyboard.press("Control+4");
  await wait(page, 500);
  const yearStops = await page.locator(".cal-year .cx-minimonth-day").evaluateAll((els) => els.filter((e) => e.tabIndex === 0).length);
  ok(yearStops === 1, `calendar: the year is one Tab stop  (${yearStops})`);
  ok((await page.locator(".cal-year[role=grid]").count()) === 0, "calendar: the year claims no grid it does not have");
  await ctx.close();
});

await group("photos", async () => {
  const { ctx, page } = await fresh();
  await openScreen(page, "Photos");
  await page.locator(".px-tile").first().waitFor();
  const tile = page.locator(".px-tile").nth(1);
  await keyFocus(page, tile);
  await page.keyboard.press("Enter");
  await wait(page, 400);
  ok(await page.locator(".px-viewer").isVisible(), "photos: Return on a photo opens it");
  await page.keyboard.press("Escape");
  await wait(page, 400);
  ok((await page.locator(".px-viewer").count()) === 0 && (await has(page, ".px-tile")), `photos: Escape closes it, the focus back on the photo  (${await focused(page)})`);
  await ctx.close();
});

// ---- Notes: the keys can leave the editor ---------------------------------------------------

await group("notes", async () => {
  const { ctx, page } = await fresh();
  await openScreen(page, "Notes");
  await page.locator(".note-row").first().click();
  await wait(page, 400);
  const editor = page.locator(".note-rich").first();
  await editor.click();
  await page.keyboard.press("End");
  await page.keyboard.press("Tab");
  ok(await has(page, ".note-rich"), "notes: Tab in a note stays in it (it indents or types a tab)");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Tab");
  await wait(page, 100);
  ok(!(await has(page, ".note-rich")), `notes: Escape, then Tab, moves on  (${await focused(page)})`);
  await editor.click();
  await page.keyboard.press("End");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Shift+Tab");
  await wait(page, 100);
  ok(!(await has(page, ".note-rich")), `notes: and Escape, then Shift-Tab, moves back  (${await focused(page)})`);
  await ctx.close();
});

// ---- Names and states -----------------------------------------------------------------------

await group("states", async () => {
  // Reminders' sample alone: Calendar's sample reminders share its ids.
  const { ctx, page } = await fresh({}, null, { remindersFixture: true, notesFixture: true, contactsFixture: true, calendarFixture: false });
  ok((await page.locator('.nav-item[aria-current="true"]').count()) === 1, "states: the sidebar marks its current app aria-current");
  await openScreen(page, "Reminders");
  await wait(page, 400);
  const name = await page.locator('.rx-tick[role="checkbox"]').first().getAttribute("aria-label");
  const ring = page.locator(`.rx-tick[role="checkbox"][aria-label="${name}"]`);
  const was = await ring.getAttribute("aria-checked");
  ok(was === "true" || was === "false", `states: a reminder's ring is a checkbox with aria-checked  (${was})`);
  // The sample library forgets a tick when it answers; held back, the tick shows as it would.
  await page.evaluate(() => { window.__doneDelay = 4000; });
  await ring.click();
  await wait(page, 80);
  ok((await ring.getAttribute("aria-checked")) !== was, "states: and ticking it changes aria-checked");
  await openScreen(page, "Calendar");
  await wait(page, 400);
  ok((await page.locator('.cx-day-hit[aria-current="date"]').count()) === 1, "states: today's day in the month is aria-current=date");
  ok((await page.locator('.cx-day[role="gridcell"][aria-selected="true"]').count()) === 1, "states: the chosen day is aria-selected in the month grid");
  await openScreen(page, "Notes");
  await wait(page, 300);
  ok((await page.locator('.note-row[aria-selected="true"]').count()) === 1, "states: the open note is marked in its list");
  // A list box of options, not a list of buttons marked current: the open row is selected.
  const notesBox = page.getByRole("listbox", { name: "Notes" });
  ok((await notesBox.count()) === 1 && (await notesBox.getByRole("option", { selected: true }).count()) === 1
    && (await page.locator('.note-row[aria-current]').count()) === 0, "states: Notes is a list box whose open note is its selected option");
  ok((await notesBox.getByRole("group").count()) >= 1, "states: each heading of notes is a named group");
  await openScreen(page, "Contacts");
  await wait(page, 300);
  const peopleBox = page.getByRole("listbox", { name: "Contacts" });
  const people = await page.locator(".contact-row").evaluateAll((rows) => rows.map((r) => [r.getAttribute("role"), r.getAttribute("aria-selected"), r.hasAttribute("aria-current")]));
  ok((await peopleBox.count()) === 1 && (await peopleBox.getAttribute("aria-multiselectable")) === "true" && people.length > 0
    && people.every(([role, sel, cur]) => role === "option" && (sel === "true" || sel === "false") && !cur), `states: Contacts is a multi-select list box of options, each selected or not  ${JSON.stringify(people.slice(0, 3))}`);
  // A screen's own list box is not a menu: its keys still answer.
  await page.keyboard.press("Control+f");
  ok(await page.evaluate(() => document.activeElement?.matches?.('input[type="search"], .contacts input')), "states: the screen's keys answer beside its own list box");
  await page.keyboard.press("Escape");
  // A notice is said by a live region that was there before it.
  const region = page.locator('.visually-hidden[role="status"][aria-live="polite"]').last();
  ok((await region.count()) === 1, "states: a polite live region waits for notices");
  await ctx.close();
});

// ---- Reduce Motion, High Contrast, Increase Contrast ---------------------------------------

await group("reduced motion", async () => {
  const { ctx, page } = await fresh({ reducedMotion: "reduce" });
  await openScreen(page, "Calendar");
  await wait(page, 400);
  await page.locator(".cx-item", { hasText: "Design review" }).first().click();
  await wait(page, 300);
  const moving = await page.evaluate(() =>
    document.getAnimations().filter((a) => {
      const t = a.effect?.getTiming?.();
      const el = a.effect?.target;
      return t && Number(t.duration) > 1 && !el?.closest?.(".spinner, .sync-dot, .is-working");
    }).map((a) => `${a.effect.target?.className ?? "?"} ${a.effect.getTiming().duration}ms`),
  );
  ok(moving.length === 0, `reduced motion: nothing moves  (${moving.slice(0, 3).join("; ")})`);
  const slow = await page.evaluate(() =>
    [...document.querySelectorAll("body *")].filter((el) => {
      const s = getComputedStyle(el);
      return s.transitionDuration.split(",").some((d) => parseFloat(d) > 0.001) && !el.closest(".spinner");
    }).length,
  );
  ok(slow === 0, `reduced motion: no transition longer than a frame  (${slow})`);
  await ctx.close();
});

if (engineName === "chromium" || engineName === "msedge") {
  await group("forced colours", async () => {
    const { ctx, page } = await fresh({ forcedColors: "active" });
    await openScreen(page, "Drive");
    await wait(page, 400);
    await page.getByRole("button", { name: "Actions for Semester Plan.pdf" }).click();
    await wait(page);
    const edge = async (sel) => page.locator(sel).first().evaluate((el) => {
      const s = getComputedStyle(el);
      return parseFloat(s.borderTopWidth) >= 1 && s.borderTopStyle !== "none" ? "border" : parseFloat(s.outlineWidth) >= 1 && s.outlineStyle !== "none" ? "outline" : "";
    });
    ok(Boolean(await edge(".row-menu")), "forced colours: a menu keeps a drawn edge");
    await page.getByRole("menuitem", { name: "Get Info" }).click();
    await wait(page, 400);
    ok(Boolean(await edge(".modal-sheet")), "forced colours: a sheet keeps a drawn edge");
    await page.keyboard.press("Tab");
    const ring = await page.evaluate(() => {
      const s = getComputedStyle(document.activeElement);
      return `${s.outlineStyle} ${s.outlineWidth} ${s.outlineColor}`;
    });
    ok(/solid 2px/.test(ring), `forced colours: focus is a solid ring in the system highlight  (${ring})`);
    await page.keyboard.press("Escape");
    await page.getByRole("combobox", { name: "Search everything" }).fill("a");
    await wait(page, 300);
    ok(Boolean(await edge(".gsearch-panel")), "forced colours: the search's results keep a drawn edge");
    ok(Boolean(await edge(".gsearch-row.is-at")) || (await page.locator(".gsearch-row.is-at").evaluate((el) => getComputedStyle(el).backgroundColor !== "rgba(0, 0, 0, 0)")),
      "forced colours: the selected result is drawn in the highlight");
    await ctx.close();
  });
}

await group("increase contrast", async () => {
  const { ctx, page } = await fresh({ contrast: "more" });
  await openScreen(page, "Drive");
  await wait(page, 400);
  const ring = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--focus-ring").trim());
  ok(/2px solid/.test(ring) && /var\(--text\)|rgb\(29, 29, 31\)|#1d1d1f/i.test(ring), `increase contrast: the focus ring is drawn in the text colour  (${ring})`);
  const rows = page.getByRole("button", { name: "Actions for Semester Plan.pdf" });
  // Reached by Tab, as WebKit shows a focus ring only for focus the keys moved.
  await keyFocus(page, rows);
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Tab");
  const drawn = await page.evaluate(() => {
    const s = getComputedStyle(document.activeElement);
    return `${s.outlineStyle} ${s.outlineWidth}`;
  });
  ok(/solid 2px/.test(drawn), `increase contrast: a focused control draws it  (${drawn})`);
  await ctx.close();
});

// The desktop's own switch, where the engine does not map it (Linux: the Settings portal, read
// by appearance.rs): the same firmer look, and it follows the switch both ways while open.
await group("contrast from the desktop", async () => {
  const { ctx, page } = await fresh({}, () => { window.__contrast = true; });
  await wait(page, 300);
  const look = () => page.evaluate(() => ({
    mark: document.documentElement.dataset.contrast ?? null,
    sep: getComputedStyle(document.documentElement).getPropertyValue("--separator").trim(),
    glass: getComputedStyle(document.documentElement).getPropertyValue("--glass-blur").trim(),
  }));
  let now = await look();
  ok(now.mark === "more" && /rgba\(0, 0, 0, 0?\.42\)/.test(now.sep) && now.glass === "none", `desktop contrast: the root is marked and the edges firm up  ${JSON.stringify(now)}`);
  await page.evaluate(() => window.__emit("quince://contrast", false));
  await wait(page, 100);
  now = await look();
  ok(now.mark === null && !/rgba\(0, 0, 0, 0?\.42\)/.test(now.sep), `desktop contrast: switched off, it goes back  ${JSON.stringify(now)}`);
  await page.evaluate(() => window.__emit("quince://contrast", true));
  await wait(page, 100);
  ok((await look()).mark === "more", "desktop contrast: switched on while open, it applies at once");
  await ctx.close();
});

await b.close();
console.log(failures ? `\nFAIL ${failures} check(s) (${engineName})` : `\nPASS keyboard, screen reader and display settings (${engineName})`);
process.exitCode = failures ? 1 : 0;
