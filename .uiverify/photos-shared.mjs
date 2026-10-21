// Sharing, as icloud.com lists it: Shared Albums from Apple's own service — empty, a new one
// named in a sheet and on the page at once, deleted from its menu after asking; a refusal puts
// things back and says why.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(() => { window.__shared = [{ guid: "J1", name: "Family", createdAt: 1790000000, owned: false, public: false, contributions: true }]; });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const cards = () => page.locator(".px-albums-page .px-card").allInnerTexts();

await page.locator(".px-source", { hasText: "Shared Albums" }).click();
await page.waitForTimeout(300);
ok((await page.locator(".px-albums-page h1").innerText()) === "Shared Albums" && (await page.locator(".px-tile:visible").count()) === 0, "Sharing → Shared Albums opens its page in place of the photos");
ok((await page.locator(".px-source", { hasText: "Library" }).getAttribute("aria-current")) !== "true" && (await page.locator(".px-source", { hasText: "Shared Albums" }).getAttribute("aria-current")) === "true",
  "and the sidebar marks it, not the view before");
ok(/Family[\s\S]*Shared with you/.test((await cards()).join()), `an album shared with this account says so  ${JSON.stringify(await cards())}`);

await page.getByRole("button", { name: "New Shared Album" }).first().click();
await page.waitForTimeout(200);
await page.getByLabel("Album Name").fill("Beach Trip");
await page.keyboard.press("Enter");
await page.waitForTimeout(250);
ok(/Beach Trip[\s\S]*Shared by you/.test((await cards())[0] ?? "") && (await page.getByRole("dialog", { name: "New Shared Album" }).count()) === 0, "a new one, named in a sheet, is on the page at once — shared by you");
ok(await page.evaluate(() => window.__shared.some((a) => a.name === "Beach Trip")), "and made at Apple");

// Deleted from its menu, after asking.
await page.locator(".px-albums-page .px-card", { hasText: "Beach Trip" }).click({ button: "right" });
await page.locator(".row-menu").getByText("Delete Shared Album").click();
ok(/everyone it is shared with/.test(await page.locator(".confirm").innerText()), "deleting asks first, saying it goes for everyone");
await page.locator(".confirm .confirm-answer").click();
await page.waitForTimeout(250);
ok(!(await cards()).join().includes("Beach Trip") && !(await page.evaluate(() => window.__shared.some((a) => a.name === "Beach Trip"))), "and it is gone, here and at Apple");
// One shared with this account cannot be deleted from here.
await page.locator(".px-albums-page .px-card", { hasText: "Family" }).click({ button: "right" });
ok((await page.locator(".row-menu").getByText("Delete Shared Album").count()) === 0, "one shared with you offers no delete — only its owner can");
await page.keyboard.press("Escape");

// Apple refuses: the new one goes again, and the reason is said.
await page.evaluate(() => { window.__sharedFail = true; });
await page.getByRole("button", { name: "New Shared Album" }).first().click();
await page.getByLabel("Album Name").fill("Nope");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok(!(await cards()).join().includes("Nope") && /refused/.test(await page.locator(".toast.is-error").innerText().catch(() => "")), "a refusal takes it back and says why");
await page.evaluate(() => { window.__sharedFail = false; });

// Back to the library.
await page.locator(".px-source", { hasText: "Library" }).click();
await page.waitForTimeout(300);
ok((await page.locator(".px-albums-page").count()) === 0 && (await page.locator(".px-tile:visible").count()) > 0, "Library brings the photos back");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
