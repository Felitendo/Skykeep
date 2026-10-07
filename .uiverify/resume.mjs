// Coming back to a lapsed session: Quince signs itself back in, and when Apple wants a code
// before trusting this computer again, the code screen is what opens — not the library, which
// could not refresh, and not the sign-in screen, whose password is already kept.
import { engine } from "./engine.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1040, height: 720 } });
await ctx.addInitScript(() => {
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: false };
      if (cmd === "auth_resume") return "code";
      if (cmd === "auth_two_factor_options") return { trustedDeviceCount: 1, phones: [], tooManyCodesSent: false };
      if (cmd === "plugin:event|listen") return 0;
      return [];
    },
  };
});
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await page.waitForTimeout(500);
const where = await page.evaluate(() => ({
  code: Boolean(document.querySelector(".code, input[autocomplete='one-time-code']")),
  library: Boolean(document.querySelector(".sidebar")),
  signIn: Boolean(document.querySelector("input[type='password']")),
}));
const ok = where.code && !where.library && !where.signIn;
console.log(ok ? "PASS a lapsed session that needs a code opens on the code screen" : `FAIL it opened on: ${JSON.stringify(where)}`);
await b.close();
process.exitCode = ok ? 0 : 1;
