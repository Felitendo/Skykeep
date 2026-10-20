// Photos: the library grouped by month, newest first; views from the sidebar; a photo opens on
// a dark stage, the arrows go through, Escape closes; the original downloads.
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

const months = await page.locator(".px-month-title").allInnerTexts();
ok(months.length === 2 && /2026/.test(months[0]), `the library is grouped by month, newest first  ${JSON.stringify(months)}`);
ok((await page.locator(".px-tile").count()) === 5, "every photo in the library is a tile; a deleted one is not");
ok((await page.locator(".px-duration").innerText()).includes("0:12"), "a video shows its length");
ok((await page.locator(".px-tile .px-heart").count()) === 1, "a favorite shows its heart");

await page.locator(".px-source", { hasText: "Favorites" }).click();
await page.waitForTimeout(200);
ok((await page.locator(".px-tile").count()) === 1, "Favorites holds the favorite");
await page.locator(".px-source", { hasText: "Recently Deleted" }).click();
await page.waitForTimeout(200);
ok((await page.locator(".px-tile").count()) === 1, "Recently Deleted holds the deleted one");
// An album: in the sidebar with its count, its photos in one grid with no month headings.
await page.locator(".px-source", { hasText: "Trips" }).click();
await page.waitForTimeout(200);
ok((await page.locator(".px-tile").count()) === 2 && (await page.locator(".px-month-title").count()) === 0 &&
  (await page.locator(".px-bar h1").innerText()) === "Trips", "an album opens to its photos, one grid, titled with its name");
await page.locator(".px-source", { hasText: "Library" }).click();
await page.waitForTimeout(200);

await page.locator(".px-tile").first().click();
await page.waitForTimeout(150);
ok((await page.locator(".px-viewer").count()) === 1, "a photo opens in the viewer");
const first = await page.locator(".px-stage img").getAttribute("src");
await page.keyboard.press("ArrowRight");
await page.waitForTimeout(100);
const second = await page.locator(".px-stage img").getAttribute("src");
ok(first && second && first !== second && /medium/.test(second), "the right arrow goes to the next photo, at screen size");
await page.getByRole("button", { name: "Download" }).click();
await page.waitForTimeout(200);
ok((await page.evaluate(() => window.__downloads?.at(-1)?.id)) === "p2", "Download saves that photo's original");
// Favorite, from the heart and from ".", at once; then Delete, after asking, moves on and the
// photo is in Recently Deleted, from where Recover brings it back.
await page.getByRole("button", { name: "Favorite", exact: true }).click();
ok((await page.getByRole("button", { name: "Unfavorite" }).count()) === 1, "the heart fills at once");
await page.keyboard.press(".");
await page.waitForTimeout(200);
ok((await page.getByRole("button", { name: "Favorite", exact: true }).count()) === 1, '"." takes it back');
const flags = await page.evaluate(() => window.__flags.map((f) => `${f.id}:${f.flag}:${f.on}`));
ok(JSON.stringify(flags) === JSON.stringify(["p2:favorite:true", "p2:favorite:false"]), `and each goes to iCloud  ${JSON.stringify(flags)}`);
page.once("dialog", (d) => void d.accept());
await page.getByRole("button", { name: "Delete", exact: true }).click();
await page.waitForTimeout(300);
const next = await page.locator(".px-stage img, .px-stage video").first().getAttribute("src");
ok(/p3|p4/.test(next ?? "") && (await page.locator(".px-tile").count()) === 4, "Delete, once agreed, takes it out and shows the next");
// A video shows its still; Play opens it in the computer's own player.
if (await page.getByRole("button", { name: "Play in your video player" }).count()) {
  await page.getByRole("button", { name: "Play in your video player" }).click();
  await page.waitForTimeout(150);
  ok((await page.evaluate(() => window.__plays?.at(-1)?.id)) === "p3", "a video plays in the computer's own player");
}
await page.keyboard.press("Escape");
await page.locator(".px-source", { hasText: "Recently Deleted" }).click();
await page.waitForTimeout(200);
ok((await page.locator(".px-tile").count()) === 2, "the deleted photo is in Recently Deleted");
await page.locator(".px-tile").first().click();
await page.getByRole("button", { name: "Recover" }).click();
await page.waitForTimeout(300);
ok((await page.evaluate(() => window.__flags.at(-1))).on === false, "Recover brings it back");
await page.keyboard.press("Escape");
await page.locator(".px-source", { hasText: "Library" }).click();
await page.waitForTimeout(200);
await page.evaluate(() => { window.__flagFail = true; });
await page.locator(".px-tile").nth(1).click();
await page.getByRole("button", { name: "Favorite", exact: true }).click();
await page.waitForTimeout(300);
ok((await page.getByRole("button", { name: "Favorite", exact: true }).count()) === 1 && (await page.locator(".notice").count()) === 1,
  "a change iCloud refuses goes back, and says why");
await page.keyboard.press("Escape");
await page.evaluate(() => { window.__flagFail = false; });
ok((await page.locator(".px-viewer").count()) === 0, "Escape closes the viewer");

const across = async () => page.locator(".px-grid").first().evaluate((g) => getComputedStyle(g).gridTemplateColumns.split(" ").length);
const before = await across();
await page.getByRole("button", { name: "Zoom In" }).click();
await page.getByRole("button", { name: "Zoom In" }).click();
await page.waitForTimeout(100);
ok((await across()) < before, `Zoom In shows fewer across  ${before} → ${await across()}`);
ok(await page.getByRole("button", { name: "Zoom In" }).isDisabled(), "and stops at the closest");
await page.keyboard.press("Control+-");
await page.waitForTimeout(100);
ok((await page.getByRole("button", { name: "Zoom In" }).isEnabled()), "Ctrl − zooms back out, as in Photos");
ok(/Photos?, 1 Video/.test(await page.locator(".px-tally").innerText()), `the foot of the library says what it holds  ${await page.locator(".px-tally").innerText()}`);

// Adding: the chosen photos go to the library, each shown in the tray as it goes; a file
// Photos can't take says why at once; Cancel stops the rest; a clean finish clears itself.
await page.evaluate(() => { window.__pick = ["/home/test/Pictures/beach.jpg", "/home/test/Pictures/notes.txt", "/home/test/Pictures/dog.heic"]; });
await page.getByRole("button", { name: "Add Photos" }).click();
await page.waitForTimeout(300);
ok(JSON.stringify(await page.evaluate(() => window.__photoUploads?.at(-1))) === JSON.stringify(["/home/test/Pictures/beach.jpg", "/home/test/Pictures/notes.txt", "/home/test/Pictures/dog.heic"]),
  "Add Photos sends the chosen files to the library");
ok((await page.locator(".upload-tray").count()) === 1 && /can’t add this kind/.test(await page.locator(".upload-tray").innerText()),
  "the tray shows the files, and why one can't be added");
await page.evaluate(() => window.__emit("quince://photo-upload", { id: "B1:0", batch: "B1", name: "beach.jpg", within: "", folder: false, error: null, state: "sending", sent: 500, total: 1000 }));
await page.waitForTimeout(100);
ok(/Uploading/.test(await page.locator(".upload-tray-title").innerText()), "a file on its way shows as uploading");
await page.locator(".upload-tray").getByRole("button", { name: "Cancel", exact: true }).click();
await page.waitForTimeout(200);
ok(JSON.stringify(await page.evaluate(() => window.__photoCancels?.at(-1))) === JSON.stringify({ ids: [], batch: "B1" }),
  "Cancel stops the batch through Photos, not Drive");
await page.evaluate(() => window.__emit("quince://photo-upload", { id: "B1:0", batch: "B1", name: "beach.jpg", within: "", folder: false, error: null, state: "cancelled", sent: 0, total: 1000 }));
await page.waitForTimeout(100);
ok(!/Uploading/.test(await page.locator(".upload-tray-title").innerText()), "once cancelled, nothing reads as still going");
await page.getByRole("button", { name: "Close uploads" }).click();
ok((await page.locator(".upload-tray").count()) === 0, "Done clears the tray");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
