// Deterministic checks on the library nav — cheaper and more reliable than eyeballing a screenshot.
import { chromium } from "playwright";
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1040, height: 900 } });
await ctx.addInitScript(({ services }) => {
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
      if (cmd === "auth_resume") return true;
      if (cmd === "account_services") return services;
      return null;
    },
  };
}, { services: ["account","archivews","calendar","ckdatabasews","ckdeviceservice","cksharews",
  "contacts","contactsweb","developerapi","docws","drivews","findme","geows","iwmb",
  "iworkexportws","iworkthumbnailws","keyvalue","mail","mcc","mccgateway","mpp","notes",
  "photos","photosupload","premiummailsettings","push","reminders","schoolwork","settings",
  "sharedlibrary","sharedstreams","ubiquity","uploadimagews"] });

const p = await ctx.newPage();
const errs = [];
p.on("pageerror", (e) => errs.push(String(e)));
await p.goto("http://localhost:4173", { waitUntil: "networkidle" });
await p.waitForSelector(".sidebar .nav-item");

const toggle = await p.textContent(".nav-toggle");
const count = await p.locator(".sidebar .nav-item").count();
const first = await p.textContent(".sidebar .nav-item");
const selected = await p.textContent('.nav-item[aria-current="true"]');

console.log("toggle label   :", JSON.stringify(toggle?.trim()));
console.log("nav items      :", count, "(10 primary + 1 toggle expected when collapsed)");
console.log("first item     :", JSON.stringify(first?.trim()));
console.log("selected       :", JSON.stringify(selected?.trim()));
console.log("page errors    :", errs.length ? errs : "none");

const collapsed = toggle?.includes("Show") && count === 11;
console.log(collapsed ? "PASS collapsed by default" : "FAIL not collapsed by default");
await b.close();
process.exitCode = collapsed ? 0 : 1;
