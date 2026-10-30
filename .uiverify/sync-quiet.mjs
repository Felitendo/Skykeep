// A sync that brought nothing new for a list draws none of its rows again, and one that changed
// a single row draws that row alone: each row read again unchanged keeps the object it had
// (lib/same), so the rows around it — memo'd by their row — stay as they were. Measured by the
// props each row was last drawn with: a row drawn again was handed new ones.
//
//   URL=http://localhost:4173 node .uiverify/sync-quiet.mjs   (ENGINE=webkit for WebKit)
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const URL = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

/**
 * Each row's component, by its element: the nearest drawn-by-a-function fiber above it, and
 * the props both halves of its fiber pair hold. The element keeps whichever half drew it last,
 * and a row skipped by its memo only hands the props it already had to the other half — so a
 * row is drawn again only when a props object neither half held before turns up. (Reading the
 * one half the element points at counted rows of a memoised list as drawn when they were not.)
 */
// (Each sent into the page whole, so each carries its own fiberOf.)
const mark = (rows) => {
  const fiberOf = (el) => {
    const key = Object.keys(el).find((k) => k.startsWith("__reactFiber$"));
    let f = key ? el[key] : null;
    while (f && ![0, 14, 15].includes(f.tag)) f = f.return;
    return f;
  };
  window.__rowProps = new Map([...document.querySelectorAll(rows)].map((el) => {
    const f = fiberOf(el);
    return [el, new Set([f?.memoizedProps, f?.alternate?.memoizedProps].filter(Boolean))];
  }));
};
/** How many of the rows marked were drawn again since, and how many are new. */
const drawn = (rows) => {
  const fiberOf = (el) => {
    const key = Object.keys(el).find((k) => k.startsWith("__reactFiber$"));
    let f = key ? el[key] : null;
    while (f && ![0, 14, 15].includes(f.tag)) f = f.return;
    return f;
  };
  let again = 0, fresh = 0;
  for (const el of document.querySelectorAll(rows)) {
    const f = fiberOf(el);
    if (!window.__rowProps.has(el)) { fresh++; continue; }
    const had = window.__rowProps.get(el);
    const now = [f?.memoizedProps, f?.alternate?.memoizedProps].filter(Boolean);
    const drewAgain = now.find((p) => !had.has(p));
    if (drewAgain) {
      again++;
      const was = [...had][0] ?? {};
      (window.__why ??= new Set());
      for (const k of Object.keys(drewAgain)) if (was[k] !== drewAgain[k]) window.__why.add(k);
    }
  }
  return { again, fresh, rows: document.querySelectorAll(rows).length };
};
const landing = async (page, changed, mutate) => {
  if (mutate) await page.evaluate(mutate);
  await page.evaluate((c) => window.__emit("quince://synced", { services: [], asOf: Math.floor(Date.now() / 1000), tookMs: 1, changed: c }), changed);
  await page.waitForTimeout(500);
};

async function screen(name, fixture, setup) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, ...fixture });
  if (setup) await ctx.addInitScript(setup);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(URL, { waitUntil: "networkidle" });
  await openScreen(page, name);
  await page.waitForTimeout(800);
  return { ctx, page };
}

// Reminders: the fixture's list, read through a hook that lets the test change one row.
{
  const { ctx, page } = await screen("Reminders", { remindersFixture: true }, () => {
    const internals = window.__TAURI_INTERNALS__;
    const invoke = internals.invoke;
    internals.invoke = async (cmd, args) => {
      const answer = await invoke(cmd, args);
      if (Array.isArray(answer) && /^reminders_(in|smart|search)$/.test(cmd) && window.__retitle)
        return answer.map((r, i) => (i === 0 ? { ...r, title: window.__retitle } : r));
      return answer;
    };
  });
  const rows = ".rx-row";
  await page.locator(rows).first().waitFor();
  await page.evaluate(mark, rows);
  await landing(page, ["reminders"]);
  let d = await page.evaluate(drawn, rows);
  ok(d.again === 0, `Reminders: a sync with nothing new draws no row again (${d.again} of ${d.rows})`);
  await page.evaluate(mark, rows);
  await landing(page, ["reminders"], "window.__retitle = 'Renamed on the phone'");
  d = await page.evaluate(drawn, rows);
  const titles = await page.evaluate(() => [...document.querySelectorAll(".rx-row")].map((r) => r.querySelector("input.rx-title")?.value ?? r.textContent));
  ok(titles.filter((t) => t.includes("Renamed on the phone")).length === 1, `Reminders: the reminder renamed on the phone shows its new name (${JSON.stringify(titles)})`);
  ok(d.again <= 1, `Reminders: …and only its row is drawn again (${d.again} of ${d.rows}; ${await page.evaluate(() => [...(window.__why ?? [])].join(","))})`);
  await ctx.close();
}

// Contacts.
{
  const { ctx, page } = await screen("Contacts", { contactsFixture: true }, () => {
    const internals = window.__TAURI_INTERNALS__;
    const invoke = internals.invoke;
    internals.invoke = async (cmd, args) => {
      const answer = await invoke(cmd, args);
      if (cmd === "contacts_list" && Array.isArray(answer) && window.__recompany)
        return answer.map((c, i) => (i === 1 ? { ...c, company: window.__recompany } : c));
      return answer;
    };
  });
  const rows = ".contact-row";
  await page.locator(rows).first().waitFor();
  await page.evaluate(mark, rows);
  await landing(page, ["contacts"]);
  let d = await page.evaluate(drawn, rows);
  ok(d.again === 0, `Contacts: a sync with nothing new draws no row again (${d.again} of ${d.rows})`);
  await page.evaluate(mark, rows);
  await landing(page, ["contacts"], "window.__recompany = 'Changed on the phone'");
  d = await page.evaluate(drawn, rows);
  ok(d.again <= 1, `Contacts: one card changed on the phone draws its row alone (${d.again} of ${d.rows})`);
  await ctx.close();
}

// Notes: the folder's list.
{
  const { ctx, page } = await screen("Notes", { notesFixture: true }, () => {
    const internals = window.__TAURI_INTERNALS__;
    const invoke = internals.invoke;
    internals.invoke = async (cmd, args) => {
      const answer = await invoke(cmd, args);
      if (cmd === "notes_in" && Array.isArray(answer) && window.__resnip)
        return answer.map((n) => (n.guid === "n3" ? { ...n, snippet: window.__resnip } : n));
      return answer;
    };
  });
  const rows = ".note-row";
  await page.locator(rows).first().waitFor();
  await page.evaluate(mark, rows);
  await landing(page, ["notes"]);
  let d = await page.evaluate(drawn, rows);
  ok(d.again === 0, `Notes: a sync with nothing new draws no row again (${d.again} of ${d.rows})`);
  await page.evaluate(mark, rows);
  await landing(page, ["notes"], "window.__resnip = 'Typed on the phone'");
  d = await page.evaluate(drawn, rows);
  ok((await page.locator(rows, { hasText: "Typed on the phone" }).count()) === 1, "Notes: the note changed on the phone shows its new words");
  await ctx.close();
}

await b.close();
console.log(failures ? `${failures} failed` : "all passed");
process.exit(failures ? 1 : 0);
