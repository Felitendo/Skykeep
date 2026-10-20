// A caption, as Photos' Info has one: typed in place, saved on Return, kept when the photo is
// opened again; the keys typed into it are words (an "f" does not favorite); Escape puts it
// back and leaves the photo open.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(600);
await page.keyboard.press("i");
const field = page.getByLabel("Caption");
ok((await field.getAttribute("placeholder")) === "Add a Caption", "Info offers Add a Caption");
await field.click();
await page.keyboard.type("Ferry at half five, fish and hills");
ok((await page.evaluate(() => (window.__flags ?? []).length)) === 0 && (await page.locator(".px-viewer").count()) === 1,
  "the letters typed are words, not the photo's keys");
await page.keyboard.press("Enter");
await page.waitForTimeout(200);
ok(JSON.stringify(await page.evaluate(() => window.__captions?.at(-1))) === JSON.stringify({ id: "p2", caption: "Ferry at half five, fish and hills" }),
  "Return saves it to iCloud");
await page.keyboard.press("Escape");
await page.waitForTimeout(650);
await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(600);
// Info stays open from one photo to the next, as in Photos.
ok((await page.locator(".px-info").count()) === 1, "Info is still open on the next photo opened");
ok((await page.getByLabel("Caption").inputValue()) === "Ferry at half five, fish and hills", "and it is there when the photo is opened again");
await page.getByLabel("Caption").click();
await page.keyboard.type(" — not this");
await page.keyboard.press("Escape");
await page.waitForTimeout(200);
ok((await page.getByLabel("Caption").inputValue()) === "Ferry at half five, fish and hills" && (await page.locator(".px-viewer").count()) === 1,
  "Escape puts it back and keeps the photo open");
ok((await page.evaluate(() => window.__captions.length)) === 1, "and sends nothing");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
