// A tap shows at once, as on the phone: adding, ticking and deleting do not wait on Apple.
// The stub holds every write for 1.5 s, as a slow network would.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
await ctx.addInitScript(() => {
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__slow) return;
    const inner = t.invoke;
    t.invoke = async (cmd, args) => {
      if (/^reminder_(create|delete|set_done|edit)$/.test(cmd)) await new Promise((r) => setTimeout(r, 1500));
      return inner(cmd, args);
    };
    t.__slow = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).first().click();
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

const field = page.getByPlaceholder(/New Reminder/i).first();
await field.fill("Buy milk");
await field.press("Enter");
await page.waitForTimeout(150);
ok(await page.getByRole("checkbox", { name: "Buy milk" }).count() > 0, "a new reminder shows the moment Return is pressed");

const tick = page.getByRole("checkbox", { name: "Plan tomorrow" });
await tick.click();
await page.waitForTimeout(150);
const ticked = (await tick.getAttribute("aria-checked")) === "true";
ok(ticked, "a tick shows at once");
await b.close();
process.exitCode = failures ? 1 : 0;
