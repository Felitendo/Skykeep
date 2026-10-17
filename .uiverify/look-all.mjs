// Not a check: every screen with fixture data, to look at. node look-all.mjs <dir> [w] [h] [scheme]
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const [out = ".", w = "1400", h = "850", scheme = "dark"] = process.argv.slice(2);
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: +w, height: +h }, colorScheme: scheme });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: true, notesFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true });
const p = await ctx.newPage();
await p.goto("http://localhost:4173", { waitUntil: "networkidle" });
for (const s of ["Reminders", "Notes", "iCloud Drive", "Calendar", "Contacts"]) {
  await openScreen(p, s);
  await p.waitForTimeout(500);
  await p.screenshot({ path: `${out}/all-${s.replace(/\W/g, "")}.png` });
}
await b.close();
