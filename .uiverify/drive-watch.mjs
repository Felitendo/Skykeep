// The folder in front is checked again while it shows: a file saved on the phone appears
// within ten seconds, unasked — and a check that finds nothing new redraws nothing, and stands
// aside while a change of this computer's own is on its way.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: false, notesFixture: false, contactsFixture: false, driveFixture: true, remindersFixture: false });
await ctx.addInitScript(() => {
  const internals = window.__TAURI_INTERNALS__;
  const invoke = internals.invoke;
  internals.invoke = async (cmd, args) => {
    if (cmd === "drive_sync_folder") window.__checks = (window.__checks ?? 0) + 1;
    if (cmd === "drive_rename") await new Promise((r) => setTimeout(r, 12_000));
    const answer = await invoke(cmd, args);
    if (cmd === "drive_children" && window.__phone && Array.isArray(answer)) {
      return [...answer, { drivewsid: "FILE::phone", parentId: null, name: "Scan from phone", extension: "pdf",
        kind: "FILE", size: 2048, childCount: null, createdAt: 1792500000, modifiedAt: 1792500000, etag: "p1" }];
    }
    return answer;
  };
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await page.locator(".nav-item", { hasText: "Drive" }).click();
await page.waitForTimeout(600);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const names = () => page.locator(".drive-name").allInnerTexts();

// Nothing new: the rows stay the very same elements, so a selection or a hover is never lost.
await page.evaluate(() => { window.__firstRow = document.querySelector(".drive-name"); });
const before = await page.evaluate(() => window.__checks);
await page.waitForTimeout(10_600);
ok((await page.evaluate(() => window.__checks)) > before, "the open folder is checked again after ten seconds");
ok(await page.evaluate(() => document.querySelector(".drive-name") === window.__firstRow), "and a check that found nothing redraws nothing");

// Saved on the phone: here without anyone opening the folder again.
await page.evaluate(() => { window.__phone = true; });
await page.waitForTimeout(10_600);
ok((await names()).some((n) => n.startsWith("Scan from phone")), "a file saved elsewhere appears unasked");

// A rename on its way (Apple slow to answer): the timed check stands aside.
await page.evaluate(() => { window.__phone = false; });
await page.locator(".drive-row-hit", { hasText: "Downloads" }).click({ button: "right" });
await page.getByRole("menuitem", { name: "Rename" }).click();
await page.keyboard.press("Control+a");
await page.keyboard.type("Downloads 2");
await page.keyboard.press("Enter");
const during = await page.evaluate(() => window.__checks);
await page.waitForTimeout(10_600);
ok((await page.evaluate(() => window.__checks)) === during, "no check while a rename is on its way");
ok((await names()).some((n) => n.startsWith("Downloads 2")), "and the new name stays on screen meanwhile");

// The window open but not in front (behind another, on another workspace): no timed checks —
// and focus coming back checks at once.
await page.waitForTimeout(2500); // the rename's answer lands
await page.evaluate(() => { document.hasFocus = () => false; });
const away = await page.evaluate(() => window.__checks);
await page.waitForTimeout(10_600);
ok((await page.evaluate(() => window.__checks)) === away, "no timed check while the window is in the background");
await page.evaluate(() => { document.hasFocus = () => true; window.dispatchEvent(new Event("focus")); });
await page.waitForTimeout(300);
ok((await page.evaluate(() => window.__checks)) > away, "and one at once when it comes to the front");

await b.close();
if (failures) process.exitCode = 1;
