// A note open here, changed on the phone: the change lands in place. The caret stays on the
// character it was on (a line added above it no longer moves it to another line), the lines the
// phone did not touch keep their elements (a picture in them is not drawn again), and the view
// scrolled part way down a long note stays on the lines it showed.
//
//   URL=http://localhost:4173 node .uiverify/notes-live-caret.mjs   (ENGINE=webkit for WebKit)
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const URL = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

// One note whose text the test plays in as the phone changes it.
function live() {
  const internals = window.__TAURI_INTERNALS__;
  const invoke = internals.invoke;
  window.__live = { body: "Shopping\nLine two\nmilk", at: 1788600000 };
  internals.invoke = async (cmd, args) => {
    if (args?.guid === "nLIVE") {
      if (cmd === "note_body") return window.__live.body;
      if (cmd === "note_runs") return [];
      if (cmd === "note_row") return null;
    }
    const answer = await invoke(cmd, args);
    if (cmd === "notes_in" && Array.isArray(answer)) {
      const lines = window.__live.body.split("\n");
      return [{ guid: "nLIVE", folderGuid: "f1", title: lines[0], snippet: lines.slice(1, 3).join(" "), body: null,
        locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: window.__live.at, etag: String(window.__live.at) }, ...answer];
    }
    return answer;
  };
}

const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
await ctx.addInitScript(live);
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(URL, { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.locator('.note-row[data-guid="nLIVE"]').click();
await page.locator(".note-editor .ProseMirror").waitFor();
await page.waitForTimeout(500);

let at = 1792600000;
/** The phone saves `body`; a sync lands with it. */
const phone = async (body) => {
  at += 10;
  await page.evaluate(([body, at]) => {
    window.__live = { body, at };
    window.__emit("quince://synced", { services: [], asOf: at, tookMs: 1, changed: ["notes"] });
  }, [body, at]);
  await page.waitForFunction((want) => {
    const ed = document.querySelector(".note-editor .ProseMirror");
    return ed && [...ed.children].map((c) => c.textContent).join("\n") === want;
  }, body, { timeout: 5000 }).catch(() => {});
  await page.waitForTimeout(150);
};
/** Where the caret is: the line it is in, and how far along it. */
const caret = () => page.evaluate(() => {
  const ed = document.querySelector(".note-editor .ProseMirror");
  const sel = getSelection();
  if (!sel.rangeCount || !ed.contains(sel.anchorNode)) return null;
  const line = [...ed.children].find((c) => c.contains(sel.anchorNode));
  const r = document.createRange();
  r.setStart(line, 0);
  r.setEnd(sel.anchorNode, sel.anchorOffset);
  return { line: line.textContent, at: r.toString().length };
});
const text = () => page.evaluate(() => [...document.querySelector(".note-editor .ProseMirror").children].map((c) => c.textContent).join("\n"));

// The caret after "Line two".
await page.evaluate(() => {
  const ed = document.querySelector(".note-editor .ProseMirror");
  const p = [...ed.children].find((c) => c.textContent === "Line two");
  const t = document.createTreeWalker(p, NodeFilter.SHOW_TEXT).nextNode();
  ed.focus();
  getSelection().collapse(t, t.data.length);
});
await page.waitForTimeout(150);
// Each line's element, marked, to tell one kept from one made again.
await page.evaluate(() => { [...document.querySelector(".note-editor .ProseMirror").children].forEach((c, i) => { c.__mark = i + 1; }); });

await phone("Shopping\nLine two\nmilk and eggs");
let c = await caret();
ok((await text()) === "Shopping\nLine two\nmilk and eggs", "words typed on the phone arrive in the open note");
ok(c?.line === "Line two" && c.at === 8, `…and the caret stays where it was (${JSON.stringify(c)})`);
const kept = await page.evaluate(() => [...document.querySelector(".note-editor .ProseMirror").children].slice(0, 2).every((c, i) => c.__mark === i + 1));
ok(kept, "the lines the phone did not touch keep their elements");

await phone("Shopping\nNew first item\nLine two\nmilk and eggs");
c = await caret();
ok((await text()) === "Shopping\nNew first item\nLine two\nmilk and eggs", "a line added on the phone above the caret arrives");
ok(c?.line === "Line two" && c.at === 8, `…and the caret stays on its own line, not the one now in its place (${JSON.stringify(c)})`);

await phone("Shopping list\nNew first item\nLine two\nmilk and eggs");
c = await caret();
ok(c?.line === "Line two" && c.at === 8, `words added on the phone earlier in the note leave the caret on its character (${JSON.stringify(c)})`);

await phone("Shopping list\nLine two\nmilk and eggs");
c = await caret();
ok(c?.line === "Line two" && c.at === 8, `a line taken out above the caret leaves it on its own (${JSON.stringify(c)})`);

// A long note, scrolled part way, the caret elsewhere: lines added at its top leave the view still.
const long = ["Long list", ...Array.from({ length: 120 }, (_, i) => `Item ${i + 1}`)].join("\n");
await phone(long);
await page.evaluate(() => document.activeElement?.blur());
const scroller = ".note-editor";
await page.evaluate((s) => { document.querySelector(s).scrollTop = 1200; }, scroller);
await page.waitForTimeout(200);
const where = () => page.evaluate((s) => {
  const sc = document.querySelector(s);
  const top = sc.getBoundingClientRect().top;
  const line = [...sc.querySelectorAll(".ProseMirror > *")].find((c) => c.getBoundingClientRect().top >= top);
  return { line: line?.textContent, top: Math.round(line?.getBoundingClientRect().top ?? 0) };
}, scroller);
const was = await where();
await phone(long.replace("Long list\n", "Long list\nAdded on the phone\nAnd another\n"));
const now = await where();
ok(now.line === was.line && Math.abs(now.top - was.top) <= 1, `lines added above the view leave it on the lines it showed (${JSON.stringify(was)} → ${JSON.stringify(now)})`);

await b.close();
console.log(failures ? `${failures} failed` : "all passed");
process.exit(failures ? 1 : 0);
