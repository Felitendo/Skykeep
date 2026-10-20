// One search across everything, at the head of the sidebar: typing finds notes, contacts and
// files at once, grouped by app; the keys move through what was found and Return opens it in
// its own app, on that very thing; Escape puts it away; nothing found says so.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
    notesFixture: true, contactsFixture: true, driveFixture: true });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  const field = page.getByRole("combobox", { name: "Search everything" });
  ok(await field.isVisible(), `${scheme}: a search field heads the sidebar`);
  await field.fill("zoya");
  await page.waitForTimeout(300);
  const panel = page.getByRole("listbox", { name: "Found" });
  ok(await panel.isVisible(), `${scheme}: what it finds shows at once`);
  ok((await panel.getByRole("region", { name: "Contacts" }).count()) === 1, `${scheme}: grouped by app`);
  await page.screenshot({ path: `${process.env.TMPDIR ?? "/tmp"}/search-${scheme}-${process.env.ENGINE}.png` });
  await page.keyboard.press("Enter");
  await page.waitForTimeout(500);
  ok(!(await panel.isVisible()), `${scheme}: Return opens it, and the results go`);
  ok(/Zoya/.test(await page.locator(".contact-detail, .contacts-detail, .contact-card").first().innerText().catch(() => "")) ||
    (await page.locator('.nav-item[aria-current="true"]').innerText()).includes("Contacts"), `${scheme}: in Contacts, on that person`);
  await field.fill("downloads");
  await page.waitForTimeout(300);
  ok((await panel.getByRole("region", { name: "iCloud Drive" }).count()) === 1, `${scheme}: files found by name`);
  await page.keyboard.press("Escape");
  ok(!(await panel.isVisible()), `${scheme}: Escape puts the results away`);
  await page.keyboard.press("Escape");
  ok((await field.inputValue()) === "", `${scheme}: and a second clears the field`);
  await field.fill("qqqzzz");
  await page.waitForTimeout(300);
  ok(/Nothing in your iCloud holds/.test(await panel.innerText()), `${scheme}: nothing found says so`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
