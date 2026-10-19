// Folding the sidebar and opening it again is one step: the sidebar is at its old width or
// its new one, never in between, and the content jumps once. On WebKitGTK every frame in
// between redrew the sidebar's rows and showed the content under it: the flicker he saw.
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
    const side = document.querySelector(".sidebar");
    window.__ws = [];
    const tick = () => { window.__xs.push(Math.round(h1.getBoundingClientRect().left)); window.__ws.push(Math.round(side.getBoundingClientRect().width)); if (window.__xs.length < 30) requestAnimationFrame(tick); };
    requestAnimationFrame(tick);
  });
  await page.getByRole("button", { name }).click();
  await page.waitForTimeout(700);
  const xs = await page.evaluate(() => window.__xs);
  const dir = Math.sign(xs[xs.length - 1] - xs[0]);
  const back = xs.slice(1).filter((x, i) => Math.sign(x - xs[i]) === -dir && Math.abs(x - xs[i]) > 2).length;
  console.log(`${back ? "FAIL" : "PASS"} ${name}: the content moves one way  [${xs.join(" ")}]`);
  if (back) failures++;
  const ws = await page.evaluate(() => window.__ws);
  const widths = new Set(ws).size, places = new Set(xs).size;
  const step = widths <= 2 && places <= 2;
  console.log(`${step ? "PASS" : "FAIL"} ${name}: one step, no frames in between  widths [${[...new Set(ws)].join(" ")}]`);
  if (!step) failures++;
}
await b.close();
process.exitCode = failures ? 1 : 0;
