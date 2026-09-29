// Which browser engine a suite drives. Quince draws in WebKitGTK, so WebKit is the one that
// matters; Chromium stays the default only because it needs nothing extra to run.
//
//   ENGINE=webkit node .uiverify/calendar.mjs
//
// Playwright's WebKit build wants a few libraries this machine's distribution names
// differently; .uiverify/webkit-libs.sh fetches them into a private folder, no root needed.
import { chromium, webkit } from "playwright";

export const engineName = process.env.ENGINE === "webkit" ? "webkit" : "chromium";
export const engine = engineName === "webkit" ? webkit : chromium;
