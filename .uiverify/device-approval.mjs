// Advanced Data Protection (issue #4): Notes, Reminders, Photos and iCloud Drive open only once
// the person approves Skykeep on one of their devices. A screen with nothing on this computer
// yet is replaced by one calm page saying so, with Try Again; a screen that has its copy here
// keeps it, with one card at the foot. Calendar and Contacts are never held up. Once the
// approval lands, it all goes away by itself; with web access off, the existing page shows.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.BASE ?? process.env.URL ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ALL = ["notes", "reminders", "photos", "drive"];
const HEADING = "Approve on your iPhone, iPad or Mac";

async function open(access, afterAsk) {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
  await ctx.addInitScript(({ access, afterAsk }) => {
    window.__dataAccess = access;
    window.__dataAccessAfterAsk = afterAsk;
  }, { access, afterAsk });
  const page = await ctx.newPage();
  await page.goto(BASE, { waitUntil: "networkidle" });
  await page.waitForTimeout(300);
  return { ctx, page };
}
const say = (page, access) => page.evaluate((a) => window.__emit?.("quince://data-access", a), access);
const heading = (page) => page.getByRole("heading", { name: HEADING });

// iCloud Drive has nothing on this computer yet: the page stands in for it.
{
  const { ctx, page } = await open({ state: "waiting", services: ALL, problem: null });
  await openScreen(page, "Drive");
  await page.waitForTimeout(200);
  ok(await heading(page).isVisible(), "Drive, with nothing here yet, shows the approval page");
  const text = await page.locator(".device-approval").innerText();
  ok(text.includes("Advanced Data Protection, which keeps iCloud Drive end-to-end encrypted"), "it names the service and why");
  ok(text.includes("Waiting for your iPhone, iPad or Mac…"), "while it is on its way, it says it is waiting");
  ok(await page.locator(".device-approval .spinner").isVisible(), "with a spinner");
  ok(await page.getByRole("button", { name: "Try Again" }).isVisible(), "and Try Again");
  ok(!(await page.locator(".drive").first().isVisible()), "the empty Drive screen is not drawn beneath it");

  // Calendar and Contacts need no approval under Advanced Data Protection.
  await openScreen(page, "Calendar");
  await page.waitForTimeout(200);
  ok(!(await heading(page).isVisible()), "Calendar is never held up");
  ok(await page.locator(".cx").first().isVisible(), "and is drawn as usual");

  // Approved on the phone: the backend says so, and Drive opens by itself.
  await openScreen(page, "Drive");
  await say(page, { state: "open", services: [], problem: null });
  await page.waitForTimeout(250);
  ok(!(await heading(page).isVisible()), "once approved, the page goes away by itself");
  ok(await page.locator(".drive").first().isVisible(), "and Drive is drawn");
  await ctx.close();
}

// Notes already has its copy here: kept on screen, with one card at the foot.
{
  const { ctx, page } = await open({ state: "needed", services: ALL, problem: null }, { state: "waiting", services: ALL, problem: null });
  await openScreen(page, "Notes");
  await page.waitForTimeout(250);
  ok(!(await heading(page).isVisible()), "Notes with its copy here is not replaced");
  ok(await page.locator(".notes").first().isVisible(), "its notes stay readable");
  const card = page.locator(".device-approval-notice");
  ok(await card.isVisible(), "a card at the foot says what would bring it up to date");
  ok((await card.innerText()).includes("To bring Notes up to date, approve Skykeep on your iPhone, iPad or Mac."), "in plain words");
  await card.getByRole("button", { name: "Try Again" }).click();
  await page.waitForTimeout(200);
  ok((await page.evaluate(() => window.__dataAccessAsks)) === 1, "Try Again asks Apple once");
  ok((await card.innerText()).includes("Approve Skykeep on your iPhone, iPad or Mac to bring Notes up to date."), "and the card then says the request is on its way");
  ok(!(await card.getByRole("button", { name: "Try Again" }).isVisible()), "without a second button to press meanwhile");
  await ctx.close();
}

// Apple couldn't reach any of the person's devices; then, Try Again on the page.
{
  const { ctx, page } = await open({ state: "notSent", services: ["photos"], problem: null }, { state: "waiting", services: ["photos"], problem: null });
  await page.locator(".nav-item", { hasText: "Photos" }).click();
  await page.waitForTimeout(400);
  ok(await heading(page).isVisible(), "Photos shows the approval page");
  ok((await page.locator(".device-approval").innerText()).includes("Apple couldn’t reach your devices."), "and says Apple couldn't reach the devices");
  await page.getByRole("button", { name: "Try Again" }).click();
  await page.waitForTimeout(200);
  ok((await page.locator(".device-approval").innerText()).includes("Waiting for your iPhone, iPad or Mac…"), "Try Again puts it on its way");
  // A service not waiting on it opens as usual.
  await openScreen(page, "Notes");
  await page.waitForTimeout(200);
  ok(!(await page.locator(".device-approval-notice").isVisible()) && !(await heading(page).isVisible()), "Notes, not waiting on it, shows nothing extra");
  await ctx.close();
}

// "Access iCloud Data on the Web" turned off: the page that says how to turn it on.
{
  const { ctx, page } = await open({ state: "open", services: [], problem: null });
  await openScreen(page, "Notes");
  await say(page, { state: "webAccessOff", services: [], problem: null });
  await page.waitForTimeout(250);
  ok(await page.getByRole("heading", { name: "Your iCloud data is kept off the web" }).isVisible(), "web access off shows the page with Apple's switch");
  ok(!(await heading(page).isVisible()), "not the approval page");
  await ctx.close();
}

// The page is reachable by keyboard and reads as a status for a screen reader.
{
  const { ctx, page } = await open({ state: "needed", services: ALL, problem: null });
  await page.locator(".nav-item", { hasText: "Drive" }).click();
  await page.waitForTimeout(300);
  ok((await page.locator(".device-approval [role=status]").count()) === 1, "where it stands is announced politely");
  await page.getByRole("button", { name: "Try Again" }).focus();
  ok(await page.evaluate(() => document.activeElement?.textContent === "Try Again"), "Try Again takes focus");
  await ctx.close();
}

await b.close();
if (failures) { console.log(`${failures} failed`); process.exit(1); }
console.log("all passed");
