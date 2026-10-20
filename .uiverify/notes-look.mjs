// Notes at a given size, in the chosen engine, for looking at.
//   ENGINE=webkit node .uiverify/notes-look.mjs <out-dir> [width] [height]
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
// Never into the repository: with no folder given, the system's temporary one.
const [out = (await import("node:os")).tmpdir(), w = "1920", h = "1040"] = process.argv.slice(2);
const b = await engine.launch();
for (const scheme of ["dark", "light"]) {
  const ctx = await b.newContext({ viewport: { width: +w, height: +h }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
    calendarFixture: true, notesFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true });
  const p = await ctx.newPage();
  await p.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await p.locator(".nav-item", { hasText: "Notes" }).click();
  await p.waitForTimeout(500);
  await p.screenshot({ path: `${out}/notes-${scheme}-${w}.png` });
  await ctx.close();
}
await b.close();
