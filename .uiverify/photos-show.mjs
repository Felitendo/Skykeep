// A slideshow, as Photos plays one: started from a photo it begins there and goes on by
// itself; from a selection it shows just those; Space pauses, the arrows step, Escape ends
// it and leaves the screen as it was.
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
const count = () => page.locator(".px-show-count").innerText();

await page.locator(".px-tile").nth(1).click({ button: "right" });
await page.locator(".row-menu").getByText("Start Slideshow").click();
await page.waitForTimeout(200);
ok((await page.locator(".px-show").count()) === 1 && (await count()) === "2 of 5", `it begins at the photo it was started from  "${await count()}"`);
const fills = await page.locator(".px-show").evaluate((el) => {
  const r = el.getBoundingClientRect();
  return r.left === 0 && r.top === 0 && Math.round(r.width) === innerWidth && Math.round(r.height) === innerHeight;
});
ok(fills, "it fills the whole window, not a box inside the screen");
await page.waitForTimeout(4400);
ok((await count()) === "3 of 5", `and goes on by itself  "${await count()}"`);
await page.keyboard.press(" ");
await page.waitForTimeout(4400);
ok((await count()) === "3 of 5" && (await page.getByRole("button", { name: "Play" }).count()) === 1, "Space pauses it");
await page.keyboard.press("ArrowRight");
ok((await count()) === "4 of 5", "the right arrow steps on");
await page.keyboard.press("ArrowLeft");
await page.keyboard.press("ArrowLeft");
ok((await count()) === "2 of 5", "and the left back");
await page.keyboard.press("Escape");
await page.waitForTimeout(200);
ok((await page.locator(".px-show").count()) === 0 && (await page.locator(".px-tile").count()) === 5, "Escape ends it, the library as it was");

await page.locator(".px-tile").nth(0).click();
await page.locator(".px-tile").nth(3).click({ modifiers: ["Control"] });
await page.locator(".px-tile").nth(3).click({ button: "right" });
await page.locator(".row-menu").getByText("Start Slideshow").click();
await page.waitForTimeout(200);
ok(/of 2$/.test(await count()), `from a selection it shows just those  "${await count()}"`);
await page.keyboard.press("Escape");
await page.locator(".px-tile").nth(2).dblclick();
await page.waitForTimeout(600);
await page.getByRole("button", { name: "Start Slideshow" }).click();
await page.waitForTimeout(200);
ok((await page.locator(".px-show").count()) === 1, "the viewer starts one too");
await page.keyboard.press("Escape");
await page.waitForTimeout(200);
ok((await page.locator(".px-viewer").count()) === 1, "and Escape ends the slideshow, not the photo behind it");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
