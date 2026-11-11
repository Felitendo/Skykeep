// German: a desktop set to German (Germany, Austria or Switzerland) gets the interface in German
// with nothing asked: the launcher and Settings, sign-in and the code screen. Every German line
// fits where the English one did, sizes take the system's decimal comma, and choosing English
// in Settings turns the window English at once, Automatic German again.
//
//   ENGINE=webkit node .uiverify/german.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";

const BASE = process.env.URL ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };

/** Text a box cuts off: wider than the box that hides what doesn't fit. */
const clipped = (page) =>
  page.evaluate(() =>
    [...document.querySelectorAll("body *")]
      .filter((el) => {
        if (!el.getClientRects().length || !el.textContent.trim()) return false;
        const cs = getComputedStyle(el);
        const hides = cs.overflowX === "hidden" || cs.overflowX === "clip" || cs.textOverflow === "ellipsis";
        return hides && el.scrollWidth > el.clientWidth + 1;
      })
      .map((el) => el.textContent.trim().slice(0, 60)),
  );

async function open(locale, screen, options = {}) {
  const ctx = await b.newContext({ viewport: options.viewport ?? { width: 1180, height: 820 }, locale });
  await ctx.addInitScript(installStub, { screen, services: SERVICES, twoFactor: options.twoFactor ?? null, notesFixture: true });
  if (options.busy) await ctx.addInitScript((busy) => {
    window.__syncExtra = [{ service: "calendar", label: "Calendar", outcome: "failed", detail: busy, lastSyncedAt: Math.floor(Date.now() / 1000) - 600 }];
  }, options.busy);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(BASE, { waitUntil: "networkidle" });
  return { ctx, page };
}

// Settings and the launcher, at a wide window and a narrow one.
for (const [w, h] of [[1180, 820], [820, 640]]) {
  const { ctx, page } = await open("de-DE", "library", { viewport: { width: w, height: h }, busy: "iCloud is busy right now. Nothing was changed — try again in a minute." });
  await openSettings(page);
  await page.getByText("Dieser Computer", { exact: true }).waitFor();
  await page.waitForTimeout(400);
  ok((await page.evaluate(() => document.documentElement.lang)) === "de", `${w}: the document says it is German, for screen readers and hyphenation`);
  ok(await page.getByRole("button", { name: "Jetzt synchronisieren" }).isVisible(), `${w}: the launcher is German`);
  ok((await page.getByRole("button", { name: "Sprache" }).innerText()).includes("Automatisch (Deutsch)"), `${w}: Language follows the system, and says it is German`);
  const tile = page.locator(".launch-tile", { hasText: "Kalender" });
  ok((await tile.locator(".launch-fresh-text").innerText()) === "iCloud ausgelastet, neuer Versuch", `${w}: a busy sync says so in German, in a few words`);
  const text = await page.locator(".settings").innerText();
  ok(/4,2 GB/.test(text) && /4,9 GB auf diesem Computer gespeichert/.test(text) && !/\d\.\d GB/.test(text), `${w}: sizes on this computer take the decimal comma, as Apple's storage does`);
  ok(/Freie Software unter der GPL-3\.0/.test(text), `${w}: the licence line is German`);
  const cut = await clipped(page);
  ok(cut.length === 0, `${w}: no German line is cut off${cut.length ? `  [${cut.join(" | ")}]` : ""}`);
  const spills = await page.evaluate(() =>
    [...document.querySelectorAll(".launch-tile")].filter((t) => {
      const box = t.getBoundingClientRect();
      return [...t.querySelectorAll("*")].some((c) => c.getClientRects().length && c.getBoundingClientRect().right > box.right + 1);
    }).length,
  );
  ok(spills === 0, `${w}: every tile holds its German words`);

  if (w === 1180) {
    // English chosen applies at once, and Automatic brings the system's German back.
    await page.getByRole("button", { name: "Sprache" }).click();
    await page.getByRole("option", { name: "English" }).click();
    await page.getByText("This Computer", { exact: true }).waitFor({ timeout: 3000 }).catch(() => {});
    ok(await page.getByText("This Computer", { exact: true }).isVisible(), "English chosen turns the window English at once");
    ok(/4,2 GB/.test(await page.locator(".settings").innerText()), "while sizes keep the German system's decimal comma: formats follow the system");
    await page.reload({ waitUntil: "networkidle" });
    await openSettings(page);
    ok(await page.getByText("This Computer", { exact: true }).isVisible(), "and stays English after a restart");
    await page.getByRole("button", { name: "Language" }).click();
    await page.getByRole("option", { name: "Automatic (Deutsch)" }).click();
    await page.getByText("Dieser Computer", { exact: true }).waitFor({ timeout: 3000 }).catch(() => {});
    ok(await page.getByText("Dieser Computer", { exact: true }).isVisible(), "Automatic follows the system's German again");
  }
  await ctx.close();
}

// Austrian and Swiss German are German too.
for (const locale of ["de-AT", "de-CH"]) {
  const { ctx, page } = await open(locale, "library");
  await openSettings(page);
  ok(await page.getByText("Allgemein", { exact: true }).isVisible(), `${locale} gets the German interface`);
  await ctx.close();
}

// Sign-in, with its privacy notes open.
{
  const { ctx, page } = await open("de-DE", "signin");
  await page.getByRole("heading", { name: "iCloud verbinden" }).waitFor();
  ok(await page.getByRole("textbox", { name: "Apple Account" }).isVisible(), "sign-in asks for the Apple Account in German");
  await page.getByText("Wie Skykeep deine Privatsphäre schützt").click();
  await page.waitForTimeout(300);
  ok(await page.getByText("Privat von Grund auf.").isVisible(), "and explains its privacy in German");
  const cut = await clipped(page);
  ok(cut.length === 0, `no sign-in line is cut off${cut.length ? `  [${cut.join(" | ")}]` : ""}`);
  await ctx.close();
}

// The code screen: what it says, and what a screen reader hears of its boxes.
{
  const twoFactor = { trustedDeviceCount: 2, phones: [{ id: 1, maskedNumber: "+49 ••• ••42" }], tooManyCodesSent: false };
  const { ctx, page } = await open("de-DE", "code", { twoFactor });
  await page.getByRole("textbox", { name: "Apple Account" }).fill("you@icloud.com");
  await page.getByRole("textbox", { name: "Passwort" }).fill("secret");
  await page.getByRole("button", { name: "Anmelden" }).click();
  await page.getByRole("group", { name: "Sechsstelliger Bestätigungscode" }).waitFor({ timeout: 5000 }).catch(() => {});
  ok(await page.getByText("Apple hat einen Code an 2 deiner Geräte gesendet.").isVisible(), "the code screen says where the code went, in German");
  ok(await page.getByRole("group", { name: "Sechsstelliger Bestätigungscode" }).isVisible(), "the code's boxes are named in German");
  ok((await page.evaluate(() => document.activeElement?.getAttribute("aria-label"))) === "Ziffer 1", "and the first box, which has focus, too");
  const cut = await clipped(page);
  ok(cut.length === 0, `no code-screen line is cut off${cut.length ? `  [${cut.join(" | ")}]` : ""}`);
  await ctx.close();
}

// English stays English, its sizes with the point an American system writes.
{
  const { ctx, page } = await open("en-US", "library");
  await openSettings(page);
  await page.getByText("This Computer", { exact: true }).waitFor();
  await page.waitForTimeout(300);
  ok(/4\.2 GB/.test(await page.locator(".settings").innerText()), "an American system keeps its decimal point");
  ok((await page.getByRole("button", { name: "Language" }).innerText()).includes("Automatic (English)"), "and its interface in English");
  await ctx.close();
}

await b.close();
process.exitCode = failures === 0 ? 0 : 1;
