#!/usr/bin/env python3
"""
Frame times in the real WebKitGTK — the system's own, the one Skykeep's window is on Linux —
for what glass-frames.mjs measures in Playwright's engines: switching apps, scrolling Photos,
Notes' list and Drive's list, opening a row's menu, sliding a segmented control's thumb.

This drives WebKitGTK directly through its GObject bindings, in a window on a headless Wayland
display — never the desktop's. The display needs weston's kiosk shell: under its desktop shell
a headless window is never given a frame to draw, and requestAnimationFrame never fires.

    weston --backend=headless --renderer=gl --shell=kiosk --socket=<name> --width=1280 --height=800 &
    WAYLAND_DISPLAY=<name> GDK_BACKEND=wayland python3 .uiverify/gtk-frames.py http://localhost:4173 [label]

The page gets the same stub the browser suites use (stub.mjs installStub, read through node),
with 2,000 photos served from here as real JPEGs, 400 notes and 400 files. Prints one line per
action and, last, the numbers as JSON. Exits non-zero when a frame is held past the bound.
"""
import io
import json
import os
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import GLib, Gtk, WebKit2  # noqa: E402

URL = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:4173"
LABEL = sys.argv[2] if len(sys.argv) > 2 else ""
HERE = os.path.dirname(os.path.abspath(__file__))
BOUND = 900  # ms: a frozen frame, not a slow one


def pictures():
    """Twelve photo-like JPEGs, as glass-frames.mjs draws them."""
    from PIL import Image, ImageDraw

    out = []
    for i in range(12):
        img = Image.new("RGB", (384, 288))
        d = ImageDraw.Draw(img)
        for y in range(288):
            h = (i * 30 + y // 3) % 360
            d.line([(0, y), (384, y)], fill=f"hsl({h},65%,{60 - y // 12}%)")
        for k in range(60):
            x, yy, r = (k * 97) % 384, (k * 53) % 288, 10 + (k % 7) * 6
            d.ellipse([x - r, yy - r, x + r, yy + r], fill=f"hsl({(i * 30 + k * 13) % 360},80%,60%)")
        b = io.BytesIO()
        img.save(b, "JPEG", quality=80)
        out.append(b.getvalue())
    return out


PICS = pictures()


class Pics(BaseHTTPRequestHandler):
    def do_GET(self):
        import re
        from urllib.parse import unquote

        m = re.search(r"q(\d+)", unquote(self.path))
        body = PICS[int(m.group(1)) % len(PICS) if m else 0]
        self.send_response(200)
        self.send_header("content-type", "image/jpeg")
        self.send_header("cache-control", "max-age=3600")
        self.send_header("access-control-allow-origin", "*")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


server = ThreadingHTTPServer(("127.0.0.1", 0), Pics)
threading.Thread(target=server.serve_forever, daemon=True).start()
pic_base = f"http://127.0.0.1:{server.server_address[1]}"

# The stub, as a script run before the page's own: installStub's source, called with its options.
stub = subprocess.run(
    ["node", "--input-type=module", "-e",
     "import { installStub, SERVICES } from './stub.mjs';"
     "const opts = { screen: 'library', services: SERVICES, twoFactor: null, notesFixture: true, calendarFixture: true,"
     " contactsFixture: true, driveFixture: true, remindersFixture: true };"
     "process.stdout.write(`(${installStub.toString()})(${JSON.stringify(opts)});`);"],
    cwd=HERE, capture_output=True, text=True, check=True).stdout

LIBRARY = """
window.__picBase = %s;
const P = (i, t) => ({ id: `q${i}`, master: `mq${i}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024,
  duration: null, filename: `IMG_${i}.JPG`, size: 2400000, uti: "public.jpeg", favorite: i %% 9 === 0, hidden: false,
  deleted: false, deletedAt: null, subtype: 0, changeTag: null });
window.__photos = Array.from({ length: 2000 }, (_, i) => P(i, 1792600000 - i * 20000));
const words = ["Garden plan", "Trip to the coast", "Recipes", "Book list", "Meeting with Sam", "Gift ideas", "Running log", "Maya's birthday"];
window.__notesExtra = Array.from({ length: 400 }, (_, i) => ({ guid: `x${i}`, folderGuid: "f1", title: `${words[i %% words.length]} ${i}`,
  snippet: "A few lines about it, to read in the list.", body: `${words[i %% words.length]} ${i}\\nA few lines about it, to read in the list.`,
  locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1788400000 - i * 600, etag: null }));
window.__driveExtra = Array.from({ length: 400 }, (_, i) => ({ drivewsid: `FILE::x${i}`, parentId: null, name: `${words[i %% words.length]} ${i}`,
  extension: ["pdf", "pages", "png", "txt"][i %% 4], kind: "FILE", size: 20000 + i * 911, childCount: null, createdAt: 1787000000,
  modifiedAt: 1788000000 - i * 600, etag: null }));
""" % json.dumps(pic_base)

RUN = r"""
const raf = () => new Promise((r) => requestAnimationFrame(r));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (sel, n = 1, ms = 15000) => {
  const t = performance.now();
  while (document.querySelectorAll(sel).length < n) { if (performance.now() - t > ms) throw new Error("never drawn: " + sel); await sleep(50); }
};
const nav = (name) => [...document.querySelectorAll(".nav-item")].find((b) => b.textContent.includes(name));
const open = async (name, root) => { nav(name).click(); if (root) await until(root); await sleep(400); };
const after = async (act, n) => {
  await raf();
  let last = performance.now();
  act();
  const gaps = [];
  for (let i = 0; i < n; i++) { const now = await raf(); gaps.push(now - last); last = now; }
  return gaps;
};
const scroll = (sel, speed, ms) => new Promise((done) => {
  const el = document.querySelector(sel);
  el.scrollTop = 0;
  const gaps = [];
  requestAnimationFrame((t0) => {
    let last = t0;
    const step = (now) => {
      gaps.push(now - last);
      el.scrollTop += (speed * (now - last)) / 1000;
      last = now;
      if (now - t0 < ms && el.scrollTop + el.clientHeight < el.scrollHeight - 2) requestAnimationFrame(step);
      else done(gaps.slice(2));
    };
    requestAnimationFrame(step);
  });
});
const res = { engine: document.documentElement.dataset.engine ?? null, size: [innerWidth, innerHeight] };
await until(".nav-item");
for (const [n, r] of [["Notes", ".notes"], ["Calendar", ".cx"], ["Contacts", ".contacts"], ["Drive", ".drive"], ["Reminders", ".rx"], ["Photos", ".px"]]) await open(n, r);
let g = [];
for (const name of ["Notes", "Calendar", "Contacts", "Drive", "Reminders", "Photos", "Notes", "Drive"]) {
  g.push(...await after(() => nav(name).click(), 12));
  await sleep(250);
}
res["switching apps"] = g;
await open("Photos", ".px-tile");
await sleep(800);
res["scrolling Photos"] = await scroll(".px-scroll", 3000, 3000);
await open("Notes", ".note-row");
await until(".note-row", 40);
res["scrolling Notes' list"] = await scroll(".note-rows", 2000, 2500);
await open("Drive", ".drive-row");
await until(".drive-row", 40);
res["scrolling Drive's list"] = await scroll(".drive-rows", 2000, 2500);
document.querySelector(".drive-rows").scrollTop = 0;
await sleep(200);
g = [];
for (let i = 0; i < 8; i++) {
  g.push(...await after(() => {
    const row = document.querySelectorAll(".drive-row")[4 + i];
    const b = row.getBoundingClientRect();
    row.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: b.left + 60, clientY: b.top + b.height / 2, button: 2 }));
  }, 10));
  if (!document.querySelector(".row-menu")) throw new Error("the row's menu did not open");
  (document.activeElement ?? document).dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  for (let k = 0; k < 40 && document.querySelector(".row-menu"); k++) await sleep(25);
  await sleep(120);
}
res["opening a menu"] = g;
await open("Calendar", ".cx");
g = [];
for (let i = 0; i < 8; i++) {
  const segs = document.querySelectorAll('.segmented[aria-label="View"] .segment');
  g.push(...await after(() => segs[i % 2 ? 1 : 2].click(), 14));
  await sleep(200);
}
res["sliding a segmented control"] = g;
return JSON.stringify(res);
"""


def stats(gaps):
    s = sorted(gaps)
    q = lambda p: s[min(len(s) - 1, int(p * len(s)))]  # noqa: E731
    return {"frames": len(s), "p50": q(0.5), "p95": q(0.95), "max": s[-1], "late": sum(1 for x in gaps if x > 25)}


Gtk.init(None)
win = Gtk.Window(title="gtk-frames")
win.set_default_size(1280, 800)
manager = WebKit2.UserContentManager()
manager.add_script(WebKit2.UserScript(stub + LIBRARY, WebKit2.UserContentInjectedFrames.TOP_FRAME,
                                      WebKit2.UserScriptInjectionTime.START, None, None))
# GLASS=live: the blurred tier here too, to measure what it would cost in WebKitGTK.
if os.environ.get("GLASS") == "live":
    manager.add_script(WebKit2.UserScript(
        "new MutationObserver(() => { if (document.documentElement?.dataset.engine) delete document.documentElement.dataset.engine; })"
        ".observe(document, { subtree: true, attributes: true, attributeFilter: ['data-engine'] });",
        WebKit2.UserContentInjectedFrames.TOP_FRAME, WebKit2.UserScriptInjectionTime.START, None, None))
view = WebKit2.WebView.new_with_user_content_manager(manager)
settings = view.get_settings()
settings.set_hardware_acceleration_policy(WebKit2.HardwareAccelerationPolicy.ALWAYS)
win.add(view)
win.show_all()
code = {"rc": 1}


def done(v, result, _):
    try:
        out = json.loads(v.call_async_javascript_function_finish(result).to_string())
        table = {k: stats(g) for k, g in out.items() if isinstance(g, list) and k != "size"}
        print(f"WebKitGTK {WebKit2.get_major_version()}.{WebKit2.get_minor_version()}.{WebKit2.get_micro_version()}"
              f" {LABEL} engine={out['engine']} window={out['size']}")
        frozen = False
        for k, st in table.items():
            print(f"  {k}: {st['frames']} frames, p50 {st['p50']:.1f} ms, p95 {st['p95']:.1f} ms, max {st['max']:.1f} ms, {st['late']} late (>25 ms)")
            frozen |= st["max"] >= BOUND
        print(json.dumps({"engine": "gtk", "label": LABEL, "results": table}))
        code["rc"] = 1 if frozen else 0
    except Exception as e:  # noqa: BLE001
        print("FAIL", e)
    Gtk.main_quit()


def loaded(v, event):
    if event == WebKit2.LoadEvent.FINISHED:
        GLib.timeout_add(1500, lambda: (v.call_async_javascript_function(RUN, -1, None, None, None, None, done, None), False)[1])


view.connect("load-changed", loaded)
view.load_uri(URL)
GLib.timeout_add_seconds(300, lambda: (print("FAIL timed out"), Gtk.main_quit()))
Gtk.main()
server.shutdown()
sys.exit(code["rc"])
