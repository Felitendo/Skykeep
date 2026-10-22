// Settings' Photos Cache: what the originals and previews take against the cap; the cap is a
// ruler of the sizes offered, moved by the keys or a click on a size, sent once the hand stops;
// a lower cap's trim reports back and the figure follows; Clear Cache lets go of them and says
// how much; a disk short of space says the cap is held lower for now.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const open = async (init) => {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
  if (init) await ctx.addInitScript(init);
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await page.locator(".sidebar-account").click();
  await page.waitForSelector(".settings");
  await page.locator(".settings-group[aria-label='Photos Cache']").waitFor();
  return { ctx, page };
};
const heading = (page) => page.locator(".settings-heading", { hasText: "Photos Cache" }).innerText();

{
  const { ctx, page } = await open();
  ok(/1\.8 GB of 2 GB/.test(await heading(page)), `the cache's use against its cap is on its heading  ${JSON.stringify(await heading(page))}`);
  const meter = page.getByRole("meter", { name: "Photos cache used" });
  ok(Number(await meter.getAttribute("aria-valuenow")) === 88, "the meter fills to the share of the cap used");
  const slider = page.getByLabel("Keep up to");
  ok((await slider.getAttribute("aria-valuetext")) === "2 GB", "the ruler rests on the cap chosen, said in words");
  // Two steps down by the keys: shown at once, sent once, after the hand stops.
  await slider.focus();
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("ArrowLeft");
  ok(/of 512 MB/.test(await heading(page)) && (await page.evaluate(() => window.__capSets?.length ?? 0)) === 0, "the cap moves at once, nothing sent while it moves");
  await page.waitForTimeout(500);
  ok(JSON.stringify(await page.evaluate(() => window.__capSets)) === JSON.stringify([512 * 1024 ** 2]), "then the cap is sent, once");
  await page.waitForTimeout(300);
  ok(/^Photos Cache\s*461 MB of 512 MB$/.test((await heading(page)).trim()), `the trim a lower cap starts reports back  ${JSON.stringify(await heading(page))}`);
  // A size under the ruler, clicked.
  await page.locator(".cap-ruler-stops button", { hasText: "10 GB" }).click();
  await page.waitForTimeout(500);
  ok((await page.evaluate(() => window.__capSets.at(-1))) === 10 * 1024 ** 3 && (await slider.getAttribute("aria-valuetext")) === "10 GB", "a size under the ruler sets it");
  // Clear Cache.
  const clear = page.getByRole("button", { name: "Clear Cache" });
  ok(!(await clear.isDisabled()), "with something cached, Clear Cache is offered");
  await clear.click();
  await page.waitForTimeout(200);
  const row = await page.locator(".settings-row", { hasText: "Clear Cache" }).innerText();
  ok((await page.evaluate(() => window.__cacheCleared)) === 1 && /Cleared 461 MB/.test(row), `Clear Cache asks once and says what it freed  ${JSON.stringify(row)}`);
  ok(await clear.isDisabled(), "and rests with nothing left");
  const labels = await page.locator(".settings button").allInnerTexts();
  ok(!labels.some((l) => /…|\.\.\./.test(l)), "no button label trails off");
  await ctx.close();
}
{
  // A disk short of space: said plainly, and true.
  const { ctx, page } = await open(() => {
    window.__cache = { used: 900_000_000, cap: 5 * 1024 ** 3, limit: 1_200_000_000, thumbnails: 1, steps: [512 * 1024 ** 2, 1024 ** 3, 2 * 1024 ** 3, 5 * 1024 ** 3] };
    window.__cacheClearFail = "The photo cache could not be read.";
  });
  ok((await page.locator(".settings-group[aria-label='Photos Cache']").innerText()).includes("Using up to 1.1 GB for now, to leave room on this disk."), "a disk short of space says the cap is held lower for now");
  await page.getByRole("button", { name: "Clear Cache" }).click();
  await page.waitForTimeout(200);
  ok((await page.locator(".toast.is-error").innerText()).includes("could not be read"), "a failed clear says why");
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
