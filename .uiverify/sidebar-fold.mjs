// Folding the sidebar and opening it again: the content travels one way from where it was
// to where it goes, every frame — never drawn at the destination first and then thrown
// back, which was the whole window glitching.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.waitForTimeout(500);
let failures = 0;
for (const name of ["Fold Sidebar to Icons", "Show Sidebar Names"]) {
  await page.evaluate(() => {
    window.__xs = [];
    const h1 = document.querySelector(".cx-title");
    const tick = () => { window.__xs.push(Math.round(h1.getBoundingClientRect().left)); if (window.__xs.length < 30) requestAnimationFrame(tick); };
    requestAnimationFrame(tick);
  });
  await page.getByRole("button", { name }).click();
  await page.waitForTimeout(700);
  const xs = await page.evaluate(() => window.__xs);
  const dir = Math.sign(xs[xs.length - 1] - xs[0]);
  const back = xs.slice(1).filter((x, i) => Math.sign(x - xs[i]) === -dir && Math.abs(x - xs[i]) > 2).length;
  console.log(`${back ? "FAIL" : "PASS"} ${name}: the content moves one way  [${xs.join(" ")}]`);
  if (back) failures++;
}
await b.close();
process.exitCode = failures ? 1 : 0;
