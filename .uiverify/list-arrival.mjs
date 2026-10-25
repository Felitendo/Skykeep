// A reminder, a note or a card made on the phone lands in the list on screen and fades in,
// briefly and alone: no other row fades, moves above it or is made again. Never one made here,
// never a batch, never on opening or switching a list, never with reduced motion.
//
//   URL=http://localhost:4173 node .uiverify/list-arrival.mjs   (ENGINE=webkit for WebKit)
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
import { instrument, land } from "./arrival.mjs";

const URL = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const say = (r) => `faded ${r.fadedFresh} new + [${r.fadedOther.join(",")}], remade ${r.remade}, fresh ${r.fresh}`;
// What the person does sets nothing fading for a moment after (lib/arrival): wait it out.
const still = (page) => page.waitForTimeout(4300);

function extra() {
  const internals = window.__TAURI_INTERNALS__;
  const invoke = internals.invoke;
  internals.invoke = async (cmd, args) => {
    const answer = await invoke(cmd, args);
    if (cmd === "notes_in" && Array.isArray(answer)) return [...(window.__phoneNotes ?? []), ...answer];
    if (cmd === "contacts_list" && Array.isArray(answer)) return [...answer, ...(window.__phoneCards ?? [])];
    return answer;
  };
}
async function open(screen, fixture, reduced = false) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, reducedMotion: reduced ? "reduce" : "no-preference" });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, ...fixture });
  await ctx.addInitScript(instrument);
  await ctx.addInitScript(extra);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(URL, { waitUntil: "networkidle" });
  await openScreen(page, screen);
  await page.waitForTimeout(600);
  return { ctx, page };
}
const reminder = (guid, title) => ({ guid, listGuid: "l1", title, notes: null, dueAt: null, completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: "x" });

// Reminders.
{
  const { ctx, page } = await open("Reminders", { remindersFixture: true });
  await page.evaluate(() => { window.__filterLists = true; });
  await page.locator(".rx-list, .source-row", { hasText: "Daily" }).first().click();
  await page.locator(".rx-row").first().waitFor();
  await still(page);
  const cfg = { items: ".rx-row", idAttr: "data-guid", frames: 30 };
  let r = await land(page, cfg, { changed: ["reminders"], mutate: `window.__extraReminders = [${JSON.stringify(reminder("rP1", "Added on the phone"))}]` });
  ok(r.fresh === 1 && r.fadedFresh === 1 && r.fadedOther.length === 0 && r.remade === 0, `Reminders: one added on the phone fades in alone (${say(r)})`);
  r = await land(page, cfg, { changed: ["reminders"], mutate: `window.__extraReminders = [...window.__extraReminders, ...Array.from({ length: 20 }, (_, i) => (${JSON.stringify(reminder("x", "From the phone"))}))].map((x, i) => (i ? { ...x, guid: "rB" + i, title: "From the phone " + i } : x))` });
  ok(r.fresh === 20 && r.fadedFresh === 0 && r.fadedOther.length === 0, `Reminders: twenty at once simply appear (${say(r)})`);
  // Its own: typed in here, it shows as it is made, never fading in.
  await page.locator(".rx-add input, form.rx-add input").first().click().catch(() => {});
  await page.keyboard.type("Made here");
  await page.evaluate((cfg) => window.__arrival.arm(cfg), { ...cfg, frames: 90 });
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => window.__arrival.done, null, { timeout: 10000 });
  await page.evaluate(() => window.__emit("quince://synced", { services: [], asOf: 1, tookMs: 1, changed: ["reminders"] }));
  await page.waitForTimeout(600);
  r = await page.evaluate(() => window.__arrival.read());
  ok(r.fadedFresh === 0 && r.fadedOther.length === 0, `Reminders: one made here never fades in (${say(r)})`);
  // Another list: all its rows are new, none arrived.
  await still(page);
  await page.evaluate((cfg) => window.__arrival.arm(cfg), { ...cfg, frames: 40 });
  await page.evaluate(() => [...document.querySelectorAll(".rx-list, .source-row")].find((e) => /GYM/.test(e.textContent))?.click());
  await page.waitForFunction(() => window.__arrival.done, null, { timeout: 10000 });
  r = await page.evaluate(() => window.__arrival.read());
  ok(r.fresh > 0 && r.fadedFresh === 0 && r.fadedOther.length === 0, `Reminders: switching lists fades nothing (${say(r)})`);
  await ctx.close();
}

// Notes.
{
  const { ctx, page } = await open("Notes", { notesFixture: true });
  await page.locator(".note-row").first().waitFor();
  await still(page);
  const cfg = { items: ".note-row", idAttr: "data-guid", frames: 30 };
  const note = { guid: "nP1", folderGuid: "f1", title: "From my phone", snippet: "Written there", body: null, locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1700000000, etag: "p" };
  const r = await land(page, cfg, { changed: ["notes"], mutate: `window.__phoneNotes = [${JSON.stringify(note)}]` });
  ok(r.fresh === 1 && r.fadedFresh === 1 && r.fadedOther.length === 0, `Notes: a note written on the phone fades in alone (${say(r)})`);
  await ctx.close();
}

// Contacts, and reduced motion.
for (const reduced of [false, true]) {
  const { ctx, page } = await open("Contacts", { contactsFixture: true }, reduced);
  await page.locator(".contact-row").first().waitFor();
  await still(page);
  const cfg = { items: ".contact-row", idAttr: "data-guid", frames: 30 };
  const card = { guid: "cP1", firstName: "Aaron", lastName: "Phone", company: null, isCompany: false, birthday: null, notes: null, etag: null, methods: [] };
  const r = await land(page, cfg, { changed: ["contacts"], mutate: `window.__phoneCards = [${JSON.stringify(card)}]` });
  if (reduced) ok(r.fresh === 1 && r.fadedFresh === 0 && r.fadedOther.length === 0, `Contacts: with reduced motion a new card is there at once (${say(r)})`);
  else ok(r.fresh === 1 && r.fadedFresh === 1 && r.fadedOther.length === 0, `Contacts: a card made on the phone fades in alone (${say(r)})`);
  await ctx.close();
}

await b.close();
console.log(failures ? `${failures} failed` : "all passed");
process.exit(failures ? 1 : 0);
