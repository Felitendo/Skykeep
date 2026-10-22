// An event found by the search across everything opens in Calendar as Calendar opens one
// found: on its day, its calendar shown if it was hidden — and kept shown, as the tick in the
// list says, after a restart too — and the event's own popover open on it.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const BASE = process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
  await ctx.addInitScript(() => {
    if (!sessionStorage.getItem("seeded")) {
      localStorage.setItem("quince.calendar.hidden", JSON.stringify(["work"]));
      sessionStorage.setItem("seeded", "1");
    }
  });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto(BASE, { waitUntil: "networkidle" });
  const field = page.getByRole("combobox", { name: "Search everything" });
  await field.fill("sprint");
  await page.waitForTimeout(300);
  const panel = page.getByRole("listbox", { name: "Found" });
  ok((await panel.getByRole("group", { name: "Calendar" }).count()) === 1, `${scheme}: the event is found`);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(800);
  const pop = page.locator(".cx-popover");
  ok((await pop.count()) === 1 && /Sprint planning/.test(await pop.innerText().catch(() => "") + (await pop.locator("input").first().inputValue().catch(() => ""))),
    `${scheme}: its popover is open on it`);
  const work = page.locator(".cx-cal", { hasText: "Work" }).locator("input");
  ok(await work.isChecked(), `${scheme}: its hidden calendar is shown, and ticked`);
  ok(!(await page.evaluate(() => localStorage.getItem("quince.calendar.hidden") ?? "")).includes("work"), `${scheme}: and stays shown`);
  await page.reload({ waitUntil: "networkidle" });
  await page.getByRole("link", { name: /Calendar/ }).or(page.locator(".nav-item", { hasText: "Calendar" })).first().click();
  await page.waitForTimeout(500);
  ok(await page.locator(".cx-cal", { hasText: "Work" }).locator("input").isChecked(), `${scheme}: after a restart too`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
