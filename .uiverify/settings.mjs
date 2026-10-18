// Settings: opened from the account at the foot of the sidebar; every switch is the
// backend's and moves at once, going back with the reason if it could not be kept; the
// appearance chosen applies at once and survives a restart; the sidebar's selection is
// neutral, and folding it slides the content rather than laying it out frame by frame.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

const card = page.locator(".sidebar-account");
ok((await card.innerText()).includes("Asha") && !(await card.innerText()).includes("…"), "the sidebar names who is signed in, whole");
await card.click();
await page.waitForSelector(".settings");
ok(await page.getByRole("heading", { name: "Settings" }).isVisible(), "the account opens Settings");
ok((await card.getAttribute("aria-current")) === "true", "and is the current item while Settings is open");

const sw = page.getByRole("switch", { name: "Calendar alerts" });
await page.waitForFunction(() => !document.querySelector('[role=switch]')?.disabled);
await sw.click();
ok((await sw.getAttribute("aria-checked")) === "false", "a switch moves the moment it is pressed");
ok((await page.evaluate(() => window.__settings?.eventAlerts)) === false, "and the backend is told");
await page.evaluate(() => { window.__settingsFail = true; });
await sw.click();
await page.waitForTimeout(100);
ok((await sw.getAttribute("aria-checked")) === "false" && (await page.locator(".settings-error").isVisible()), "a setting that could not be kept goes back, and says why");
await page.evaluate(() => { window.__settingsFail = false; });

await page.getByRole("button", { name: "Dark", exact: true }).click();
ok((await page.evaluate(() => document.documentElement.dataset.theme)) === "dark", "Dark applies at once");
const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
ok(bg !== "rgb(245, 245, 247)", `the window turns dark whatever the system says  [${bg}]`);
await page.reload({ waitUntil: "networkidle" });
ok((await page.evaluate(() => document.documentElement.dataset.theme)) === "dark", "and is still chosen after a restart");
await page.getByRole("button", { name: "Automatic", exact: true }).click();
ok((await page.evaluate(() => document.documentElement.dataset.theme)) === undefined, "Automatic follows the system again");

// The selected app is a neutral highlight: three apps used to select in the same blue.
await page.locator(".nav-item", { hasText: "Reminders" }).click();
await page.waitForTimeout(200);
const sel = await page.evaluate(() => {
  const el = document.querySelector('.nav-item[aria-current="true"]');
  return { color: getComputedStyle(el).color, text: getComputedStyle(document.body).color };
});
ok(sel.color === sel.text, `the current app is not drawn in an accent  [${sel.color}]`);

// Folding: the content's padding is never animated; the content slides instead.
const padTransition = await page.evaluate(() => {
  const cs = getComputedStyle(document.querySelector(".content"));
  return `${cs.transitionProperty} ${cs.transitionDuration}`;
});
ok(/^(all|none) 0s$/.test(padTransition) || !/padding/.test(padTransition) && !/all/.test(padTransition), `the content's layout is not animated  [${padTransition}]`);
await page.evaluate(() => {
  window.__slid = false;
  const el = document.querySelector(".content");
  const orig = el.animate.bind(el);
  el.animate = (...a) => { window.__slid = true; return orig(...a); };
});
await page.getByRole("button", { name: "Fold Sidebar to Icons" }).click();
await page.waitForTimeout(400);
ok(await page.evaluate(() => window.__slid), "folding slides the content on the compositor");
// The account's picture is the photo on the person's own card, where it has one.
// (A plain browser cannot load quince:// — the picture falls back to initials, as it should —
// so what is checked is the picture asked for.)
await ctx.addInitScript(() => {
  window.__mePhoto = "me-card";
  window.__imgs = [];
  new MutationObserver((m) => {
    for (const r of m) for (const n of r.addedNodes) {
      if (n.nodeType !== 1) continue;
      for (const img of [n, ...n.querySelectorAll?.("img") ?? []]) if (img.tagName === "IMG") window.__imgs.push(img.getAttribute("src"));
    }
  }).observe(document, { childList: true, subtree: true });
});
await page.reload({ waitUntil: "networkidle" });
await page.waitForTimeout(300);
const asked = await page.evaluate(() => window.__imgs);
ok(asked.some((s) => s?.includes("me-card")), `the account shows the photo on the person's own card  [${asked.join(", ")}]`);
ok((await page.locator(".sidebar-account .account-avatar").innerText()).trim() === "AR", "and its initials hold the place where the photo cannot be had");
await b.close();
process.exitCode = failures ? 1 : 0;
