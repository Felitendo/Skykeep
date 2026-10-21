// Which browser engine a suite drives. Quince draws in WebKitGTK, so WebKit is the one that
// matters; Chromium stays the default only because it needs nothing extra to run.
//
//   ENGINE=webkit node .uiverify/calendar.mjs   WebKit's WPE port, off-screen
//   ENGINE=gtk    node .uiverify/calendar.mjs   WebKit's GTK port — Quince's own — in a real
//                                              window on this display
//   ENGINE=msedge node .uiverify/calendar.mjs   Microsoft Edge — the engine WebView2 is, with
//                                              Windows' own fonts — on a Windows machine
//
// Playwright's WebKit build wants a few libraries this machine's distribution names
// differently; .uiverify/webkit-libs.sh fetches them into a private folder, no root needed.
import { chromium, webkit } from "playwright";

export const engineName = ["webkit", "gtk", "msedge"].includes(process.env.ENGINE) ? process.env.ENGINE : "chromium";
// Edge is Chromium (the engine WebView2 is); only the two WebKit ports take Playwright's WebKit.
const base = engineName === "webkit" || engineName === "gtk" ? webkit : chromium;
/**
 * In the GTK port the window is a real one on this desktop, and a tiling window manager sizes
 * it — Playwright's attempt to resize it is refused ("Failed to resize window"). So a GTK run
 * takes whatever size it is given: contexts get no fixed viewport and pages ignore requests to
 * change it. The suites that matter here measure from the elements themselves.
 */
export const engine = {
  launch: async (options = {}) => {
    const browser = await base.launch({
      ...options,
      ...(engineName === "gtk" ? { headless: false } : {}),
      ...(engineName === "msedge" ? { channel: "msedge" } : {}),
    });
    if (engineName !== "gtk") return browser;
    const newContext = browser.newContext.bind(browser);
    browser.newContext = async (opts = {}) => {
      const ctx = await newContext({ ...opts, viewport: null });
      const newPage = ctx.newPage.bind(ctx);
      ctx.newPage = async () => {
        const page = await newPage();
        page.setViewportSize = async () => {};
        return page;
      };
      return ctx;
    };
    browser.newPage = async (opts = {}) => (await browser.newContext(opts)).newPage();
    return browser;
  },
};
