// The viewer as icloud.com's: where the photo is among them ("2 of 5") under its date, and a
// button for full screen; Escape leaves full screen before it closes the photo, and closing a
// photo leaves full screen too.
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
const asked = () => page.evaluate(() => window.__fullscreen ?? []);

await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(300);
ok(/2 of 5/.test(await page.locator(".px-viewer-when").innerText()), `where the photo is among them, under its date  "${(await page.locator(".px-viewer-when").innerText()).replace(/\n/g, " ")}"`);
await page.keyboard.press("ArrowRight");
await page.waitForTimeout(150);
ok(/3 of 5/.test(await page.locator(".px-viewer-when").innerText()), "and it follows the arrows");
await page.getByRole("button", { name: "Enter Full Screen" }).click();
await page.waitForTimeout(150);
ok(JSON.stringify(await asked()) === "[true]" && (await page.getByRole("button", { name: "Exit Full Screen" }).count()) === 1, "the button takes the window full screen, and becomes Exit");
await page.keyboard.press("Escape");
await page.waitForTimeout(150);
ok(JSON.stringify(await asked()) === "[true,false]" && (await page.locator(".px-viewer").count()) === 1, "Escape leaves full screen first, the photo still open");
await page.keyboard.press("Escape");
await page.waitForTimeout(150);
ok((await page.locator(".px-viewer").count()) === 0, "and Escape again closes the photo");
await page.locator(".px-tile").nth(0).dblclick();
await page.waitForTimeout(300);
await page.getByRole("button", { name: "Enter Full Screen" }).click();
await page.waitForTimeout(100);
await page.getByRole("button", { name: "Close" }).click();
await page.waitForTimeout(200);
ok((await asked()).at(-1) === false, "closing a photo leaves full screen too");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
