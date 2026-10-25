// The search across everything, by the keys and under load: Ctrl+K comes to it from anywhere;
// Up and Down move the selection and Return opens it; Escape steps back one thing at a time;
// an answer that arrives after a newer question's is never drawn, and what was found stays on
// screen while the next answer comes (never empty in between); the typed words are marked; and
// a section of 600 is drawn only where it is in view, read a page at a time down to its last
// row, scrolling without a long frame.
//
//   BASE=http://localhost:4240 ENGINE=webkit node .uiverify/search-keys.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const BASE = process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
/** Until `fn` holds in the page (polled each frame), or 8 s; the check after it says what is there. */
const until = (page, fn, arg) => page.waitForFunction(fn, arg, { timeout: 8000, polling: "raf" }).then(() => true, () => false);
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
    notesFixture: true, contactsFixture: true, driveFixture: true });
  await ctx.addInitScript(() => {
    window.__moreContacts = Array.from({ length: 600 }, (_, i) => ({ guid: `m${i}`, firstName: "Marlowe", lastName: `Test ${i + 1}`,
      company: null, isCompany: false, birthday: null, notes: null, etag: null, methods: [] }));
  });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
  await page.goto(BASE, { waitUntil: "networkidle" });
  const field = page.getByRole("combobox", { name: "Search everything" });
  const panel = page.getByRole("listbox", { name: "Found" });
  const options = panel.getByRole("option");
  const selected = () => panel.locator('[role="option"][aria-selected="true"]').innerText().catch(() => "");

  // Ctrl+K from anywhere.
  await page.locator("body").click({ position: { x: 900, y: 400 } });
  await page.keyboard.press("Control+k");
  ok(await field.evaluate((f) => f === document.activeElement), `${scheme}: Ctrl+K comes to the search field`);

  // The keys move the selection, and it stays in view.
  await page.keyboard.type("marlowe");
  await until(page, () => document.querySelectorAll('[role="listbox"][aria-label="Found"] [role="option"]').length >= 5
    && /Marlowe/.test(document.querySelector('[role="listbox"][aria-label="Found"] [aria-selected="true"]')?.textContent ?? ""));
  ok((await options.count()) >= 5, `${scheme}: results at once`);
  ok(/Marlowe/.test(await selected()), `${scheme}: the first is selected`);
  ok((await panel.locator('[aria-selected="true"] mark').first().innerText()).toLowerCase() === "marlowe", `${scheme}: the typed word is marked`);
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  ok(/Test 3\b/.test(await selected()), `${scheme}: Down moves it  (${(await selected()).split("\n")[0]})`);
  await page.keyboard.press("ArrowUp");
  ok(/Test 2\b/.test(await selected()), `${scheme}: Up moves it back`);
  const active = await field.getAttribute("aria-activedescendant");
  ok(active && (await page.locator(`#${active}`).getAttribute("aria-selected")) === "true", `${scheme}: the field names the selected row to assistive tech`);

  // Escape: Show All first, then the results, then the field.
  await panel.getByRole("option", { name: /^Show all 600 in Contacts/ }).click();
  await until(page, () => document.querySelectorAll('[role="listbox"][aria-label="Found"] [role="option"]').length > 10);
  const drawnRows = await options.count();
  ok(drawnRows > 10 && drawnRows < 80, `${scheme}: 600 found, only those in view drawn  (${drawnRows} rows)`);
  await page.keyboard.press("Escape");
  ok(await panel.getByRole("option", { name: /^Show all 600/ }).isVisible(), `${scheme}: Escape leaves Show All first`);
  await page.keyboard.press("Escape");
  ok(!(await panel.isVisible()), `${scheme}: then puts the results away`);
  await page.keyboard.press("Escape");
  ok((await field.inputValue()) === "", `${scheme}: then clears the field`);

  // An answer overtaken is never drawn; what was found stays while the next comes.
  await field.fill("curtains");
  await until(page, () => /Curtains/.test(document.querySelector('[role="listbox"][aria-label="Found"]')?.textContent ?? ""));
  ok(/Curtains/.test(await panel.innerText()), `${scheme}: Curtains found`);
  await page.evaluate(() => { window.__searchDelay = (q) => (q === "zoya" ? 0 : q === "zoy" ? 250 : q.startsWith("z") ? 700 : 0); });
  await page.keyboard.press("Control+a");
  await page.keyboard.type("zo");
  await page.waitForTimeout(60);
  ok(/Curtains/.test(await panel.innerText()) && (await options.count()) > 0, `${scheme}: while the next answer comes, the last stays on screen`);
  await page.keyboard.type("y");
  await page.keyboard.type("a");
  await until(page, () => /Zoya/.test(document.querySelector('[role="listbox"][aria-label="Found"]')?.textContent ?? ""));
  ok(/Zoya/.test(await panel.innerText()), `${scheme}: the newest answer is drawn`);
  // Every older answer landed (the stub counts those on their way), and two frames drawn after.
  await until(page, () => window.__searchesOut === 0);
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  const after = await panel.innerText();
  ok(/Zoya/.test(after) && !/Curtains/.test(after), `${scheme}: answers to older questions arriving after it are dropped`);
  const seqs = await page.evaluate(() => window.__searches.slice(-3).map((s) => s.seq));
  ok(seqs.every((s, i) => i === 0 || s > seqs[i - 1]), `${scheme}: every question is numbered, newest highest  (${seqs.join(", ")})`);
  await page.evaluate(() => { window.__searchDelay = 0; });

  // Return opens the selected one in its own app.
  await page.keyboard.press("Enter");
  await until(page, () => !(document.querySelector('[role="listbox"][aria-label="Found"]')?.getClientRects().length) && /Contacts/.test(document.querySelector('.nav-item[aria-current="true"]')?.textContent ?? ""));
  ok(!(await panel.isVisible()) && (await page.locator('.nav-item[aria-current="true"]').innerText()).includes("Contacts"), `${scheme}: Return opens it in Contacts`);

  // 600 in one section: read a page at a time to the very end, scrolled without a long frame.
  await field.fill("marlowe");
  await panel.getByRole("option", { name: /^Show all 600 in Contacts/ }).click();
  await until(page, () => document.querySelectorAll('#gsearch-results [role="option"]').length > 10);
  const run = await page.evaluate(async () => {
    const list = document.querySelector("#gsearch-results");
    const out = [];
    let last = performance.now();
    for (let i = 0; i < 400 && list.scrollTop + list.clientHeight < list.scrollHeight - 1; i++) {
      list.scrollTop += 80;
      await new Promise((r) => requestAnimationFrame(r));
      const now = performance.now();
      out.push(now - last);
      last = now;
    }
    await new Promise((r) => setTimeout(r, 400));
    list.scrollTop = list.scrollHeight;
    await new Promise((r) => setTimeout(r, 300));
    const rows = [...list.querySelectorAll('[role="option"]')];
    return { max: Math.max(...out), frames: out.length, lastRow: rows.at(-1)?.innerText ?? "", drawn: rows.length };
  });
  ok(/Test 600\b/.test(run.lastRow), `${scheme}: the last of 600 is reached by scrolling  (${run.lastRow.split("\n")[0]})`);
  ok(run.drawn < 80, `${scheme}: and still only those in view are drawn  (${run.drawn})`);
  ok(run.max < 100, `${engineName} ${scheme}: no frame of that scroll past 100 ms  (worst ${run.max.toFixed(0)} ms over ${run.frames})`);
  const reads = await page.evaluate(() => window.__searches.filter((s) => s.only === "contacts").map((s) => s.skip));
  ok(reads.length >= 3 && reads[0] === 5, `${scheme}: read a page at a time  (from ${reads.join(", ")})`);
  await page.screenshot({ path: `${process.env.TMPDIR ?? "/tmp"}/search-keys-${scheme}-${engineName}.png` });
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
