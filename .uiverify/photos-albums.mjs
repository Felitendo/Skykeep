// Photos albums: a new one from the sidebar's +, named in place, first in the list; a photo
// added from its right-click menu and from the viewer; taken out again in the album, staying
// in the library; renamed and deleted from the album's own right-click menu. Each shows at
// once and reaches iCloud.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
page.on("dialog", (d) => void d.accept());
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const albumNames = () => page.locator(".px-side .px-source").evaluateAll((els) => els.map((e) => e.querySelector("span")?.textContent ?? "").slice(6));

await page.getByRole("button", { name: "New Album" }).click();
await page.getByLabel("Album name").fill("Beach");
await page.keyboard.press("Enter");
await page.waitForTimeout(250);
ok(JSON.stringify(await albumNames()) === JSON.stringify(["Beach", "Trips"]), `a new album is named in place and sorts first  ${JSON.stringify(await albumNames())}`);
ok((await page.evaluate(() => window.__albumCalls.filter((c) => c.cmd === "photos_album_create").length)) === 1, "and is made once, not twice for Enter and the blur after it");

await page.locator(".px-tile").first().click({ button: "right" });
await page.locator(".row-menu").getByText("Add to Album").hover();
await page.waitForTimeout(200);
await page.locator(".row-menu").getByText("Beach", { exact: true }).click();
await page.waitForTimeout(250);
const add = await page.evaluate(() => window.__albumCalls.find((c) => c.cmd === "photos_album_add"));
ok(add && add.args.ids[0] === "p1", "right-click → Add to Album puts the photo in it");
ok(/1/.test(await page.locator(".px-source", { hasText: "Beach" }).innerText()), "and its count shows at once");

await page.locator(".px-tile").nth(1).click();
await page.getByRole("button", { name: "Add to Album" }).click();
await page.locator(".row-menu").getByText("Beach", { exact: true }).click();
await page.waitForTimeout(250);
ok((await page.evaluate(() => window.__albumCalls.filter((c) => c.cmd === "photos_album_add").length)) === 2, "the viewer's Add to Album does the same");
await page.keyboard.press("Escape");

await page.locator(".px-source", { hasText: "Beach" }).click();
await page.waitForTimeout(250);
ok((await page.locator(".px-tile").count()) === 2, "the album holds both");
await page.locator(".px-tile").first().click({ button: "right" });
ok((await page.locator(".row-menu").getByText("Delete", { exact: true }).count()) === 0, "in an album the menu offers Remove, not Delete");
await page.locator(".row-menu").getByText("Remove from Album").click();
await page.waitForTimeout(250);
ok((await page.locator(".px-tile").count()) === 1, "Remove from Album takes it out at once");
ok((await page.evaluate(() => window.__photos.filter((p) => !p.deleted).length)) === 5, "and the photo stays in the library");

await page.locator(".px-source", { hasText: "Beach" }).click({ button: "right" });
await page.locator(".row-menu").getByText("Rename Album").click();
await page.getByLabel("Album name").fill("Sea");
await page.keyboard.press("Enter");
await page.waitForTimeout(250);
ok((await page.locator(".px-bar h1").innerText()) === "Sea" && (await albumNames()).includes("Sea"), "Rename Album renames it in place and in the title");

await page.locator(".px-source", { hasText: "Sea" }).click({ button: "right" });
await page.locator(".row-menu").getByText("Delete Album").click();
await page.waitForTimeout(250);
ok(!(await albumNames()).includes("Sea") && (await page.locator(".px-bar h1").innerText()) === "Library", "Delete Album, once agreed, removes it and goes back to the library");

await page.evaluate(() => { window.__albumFail = true; });
await page.getByRole("button", { name: "New Album" }).click();
await page.getByLabel("Album name").fill("Nope");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok(!(await albumNames()).includes("Nope") && (await page.locator(".notice").count()) === 1, "an album iCloud refuses goes away again, and says why");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
