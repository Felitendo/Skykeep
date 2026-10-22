// One search across everything, at the head of the sidebar: typing finds notes, contacts and
// files at once, grouped by app; the keys move through what was found and Return opens it in
// its own app, on that very thing; Escape puts it away; nothing found says so.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const BASE = process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
    notesFixture: true, contactsFixture: true, driveFixture: true });
  // Enough people named alike that their section has more than it shows.
  await ctx.addInitScript(() => {
    window.__moreContacts = Array.from({ length: 8 }, (_, i) => ({ guid: `m${i}`, firstName: "Marlowe", lastName: `Test ${i + 1}`,
      company: null, isCompany: false, birthday: null, notes: null, etag: null, methods: [] }));
  });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto(BASE, { waitUntil: "networkidle" });
  const field = page.getByRole("combobox", { name: "Search everything" });
  ok(await field.isVisible(), `${scheme}: a search field heads the sidebar`);
  await field.fill("zoya");
  await page.waitForTimeout(300);
  const panel = page.getByRole("listbox", { name: "Found" });
  ok(await panel.isVisible(), `${scheme}: what it finds shows at once`);
  ok((await panel.getByRole("group", { name: "Contacts" }).count()) === 1, `${scheme}: grouped by app`);
  await page.screenshot({ path: `${process.env.TMPDIR ?? "/tmp"}/search-${scheme}-${process.env.ENGINE}.png` });
  await page.keyboard.press("Enter");
  await page.waitForTimeout(500);
  ok(!(await panel.isVisible()), `${scheme}: Return opens it, and the results go`);
  ok(/Zoya/.test(await page.locator(".contact-detail, .contacts-detail, .contact-card").first().innerText().catch(() => "")) ||
    (await page.locator('.nav-item[aria-current="true"]').innerText()).includes("Contacts"), `${scheme}: in Contacts, on that person`);
  await field.fill("downloads");
  await page.waitForTimeout(300);
  ok((await panel.getByRole("group", { name: "Drive" }).count()) === 1, `${scheme}: files found by name`);
  await page.keyboard.press("Escape");
  ok(!(await panel.isVisible()), `${scheme}: Escape puts the results away`);
  await page.keyboard.press("Escape");
  ok((await field.inputValue()) === "", `${scheme}: and a second clears the field`);
  // A section with more than it shows: Show All shows that one whole, in place; Show Less
  // puts it back; the keys reach every row of it.
  await field.fill("marlowe");
  await page.waitForTimeout(300);
  const more = panel.getByRole("option", { name: /^Show all \d+ in / }).first();
  const label = await more.getAttribute("aria-label").catch(() => null);
  if (!label) ok(false, `${scheme}: some section has more than it shows`);
  else {
    const [, total, name] = /Show all (\d+) in (.+)/.exec(label);
    const region = panel.getByRole("group", { name });
    const rows = () => region.getByRole("option").filter({ hasNotText: /^Show (All|Less)$/ }).count();
    const short = await rows();
    await more.click();
    await page.waitForTimeout(300);
    const all = await rows();
    ok(short === 5 && all === Number(total), `${scheme}: Show All shows ${name} whole  (${short} → ${all} of ${total})`);
    ok(await field.evaluate((f) => f === document.activeElement), `${scheme}: and the field keeps the keys`);
    await panel.getByRole("option", { name: `Show less of ${name}` }).click();
    await page.waitForTimeout(300);
    ok((await rows()) === 5, `${scheme}: Show Less puts it back`);
  }
  await field.fill("qqqzzz");
  await page.waitForTimeout(300);
  ok(/No results for “qqqzzz”/.test(await page.locator("#gsearch-results").innerText()), `${scheme}: nothing found says so`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
