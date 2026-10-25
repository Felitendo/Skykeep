// What a sync landing does to the screen, frame by frame: the instruments the live-sync suites
// share (photos-arrival, notes-live-caret, sync-quiet). Installed before the app, armed just
// before a landing is played in, read once it has settled.
//
// Per frame it counts the items drawn again from nothing (same id, a new element), the new ones,
// the ones on screen whose picture is not showing, and how far each item that was on screen
// has moved; over the whole landing, the pictures fetched again, the elements given a part-way
// opacity (a fade), and every element added and taken away.

/** In the page: `window.__arrival.arm(cfg)`, then `window.__arrival.done` once it has watched. */
export function instrument() {
  const A = (window.__arrival = { armed: false, done: false });
  document.addEventListener("load", (e) => {
    if (A.armed && e.target instanceof HTMLImageElement) A.loads.push({ old: !!e.target.__seen, src: e.target.getAttribute("src") });
  }, true);
  const label = (n) => (n.nodeType !== 1 ? null : `${n.tagName.toLowerCase()}${typeof n.className === "string" && n.className.trim() ? "." + n.className.trim().split(/\s+/)[0] : ""}`);
  A.arm = (cfg) => {
    A.cfg = cfg;
    A.t0 = performance.now();
    A.armed = true;
    A.done = false;
    A.loads = [];
    A.srcChanges = 0;
    A.removed = {};
    A.added = {};
    A.faded = new Map();
    A.frames = [];
    const vh = window.innerHeight;
    const before = new Map();
    for (const el of document.querySelectorAll(cfg.items)) {
      el.__seen = 1;
      const id = el.getAttribute(cfg.idAttr);
      const r = el.getBoundingClientRect();
      if (id) before.set(id, { el, top: Math.round(r.top), left: Math.round(r.left), shown: r.bottom > 0 && r.top < vh && r.width > 0 });
    }
    for (const img of document.querySelectorAll("img")) img.__seen = 1;
    A.before = before;
    const opacityOf = (el) => {
      const o = el.style?.opacity;
      if (o !== "" && o !== undefined && +o > 0 && +o < 1) {
        const item = el.closest?.(cfg.items);
        const id = item?.getAttribute(cfg.idAttr) ?? label(el);
        A.faded.set(id, { item: !!item, fresh: !!item && !before.has(item.getAttribute(cfg.idAttr)) });
      }
    };
    A.mo = new MutationObserver((ms) => {
      for (const m of ms) {
        if (m.type === "childList") {
          for (const n of m.removedNodes) { const l = label(n); if (l) A.removed[l] = (A.removed[l] ?? 0) + 1; }
          for (const n of m.addedNodes) { const l = label(n); if (l) A.added[l] = (A.added[l] ?? 0) + 1; }
        } else if (m.attributeName === "src" && m.target.__seen) A.srcChanges++;
        else if (m.attributeName === "style") opacityOf(m.target);
      }
    });
    A.mo.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["src", "style"] });
    let n = 0;
    const frame = () => {
      const f = { t: Math.round(performance.now() - A.t0) };
      const items = [...document.querySelectorAll(cfg.items)];
      const shown = (el) => { const r = el.getBoundingClientRect(); return r.bottom > 0 && r.top < vh && r.width > 0; };
      let remade = 0, fresh = 0, blank = 0, moved = 0;
      const here = new Set();
      for (const el of items) {
        const id = el.getAttribute(cfg.idAttr);
        const was = before.get(id);
        if (!el.__seen) { if (was) remade++; else fresh++; }
        if (was && was.shown) {
          if (shown(el)) here.add(id);
          if (cfg.pictures) {
            const img = el.querySelector("img");
            if (!img || !img.complete || !img.naturalWidth || getComputedStyle(img).opacity === "0") blank++;
          }
          const r = el.getBoundingClientRect();
          moved = Math.max(moved, Math.abs(Math.round(r.top) - was.top), Math.abs(Math.round(r.left) - was.left));
        }
      }
      // An item that was on screen and is gone (a whole grid swapped for its placeholder).
      let lost = 0;
      for (const [id, was] of before) if (was.shown && !here.has(id)) lost++;
      Object.assign(f, { items: items.length, remade, fresh, blank, moved, lost });
      for (const el of document.querySelectorAll(cfg.items)) opacityOf(el), el.querySelectorAll("[style]").forEach(opacityOf);
      if (cfg.probe) Object.assign(f, (0, eval)(cfg.probe));
      A.frames.push(f);
      if (++n < (cfg.frames ?? 60)) requestAnimationFrame(frame);
      else { A.mo.disconnect(); A.armed = false; A.done = true; }
    };
    requestAnimationFrame(frame);
  };
  A.read = () => {
    const max = (k) => Math.max(0, ...A.frames.map((f) => (typeof f[k] === "number" ? f[k] : 0)));
    const faded = [...A.faded.entries()];
    return {
      remade: max("remade"), fresh: max("fresh"), blank: max("blank"), moved: max("moved"), lost: max("lost"),
      oldLoads: A.loads.filter((l) => l.old).length, loads: A.loads.length, srcChanges: A.srcChanges,
      fadedFresh: faded.filter(([, v]) => v.fresh).length, fadedOther: faded.filter(([, v]) => !v.fresh).map(([k]) => k),
      removed: A.removed, added: A.added, last: A.frames.at(-1), frames: A.frames,
    };
  };
}

export const status = (changed) => ({
  services: [{ service: "photos", label: "Photos", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) }],
  asOf: Math.floor(Date.now() / 1000), tookMs: 10, ...(changed === undefined ? {} : { changed }),
});

/** One landing, measured: `mutate` changes what the library holds, then the pass lands. */
export async function land(page, cfg, { mutate, changed, settle = 300 } = {}) {
  await page.mouse.move(2, 2);
  if (mutate) await page.evaluate(mutate);
  await page.evaluate(([cfg, payload]) => { window.__arrival.arm(cfg); window.__emit("quince://synced", payload); }, [cfg, status(changed)]);
  await page.waitForFunction(() => window.__arrival.done, null, { timeout: 20000 });
  await page.waitForTimeout(settle);
  return page.evaluate(() => window.__arrival.read());
}

/** Pictures served for the stub's `quince://` addresses (see stub's convertFileSrc), a few ms late as from disk. */
export const PICS = "http://pics.test";
export async function servePictures(ctx, { delay = 15 } = {}) {
  await ctx.addInitScript(([pics]) => { window.__picBase = pics; }, [PICS]);
  ctx.__pictureHits = [];
  await ctx.route(`${PICS}/**`, async (route) => {
    const path = decodeURIComponent(route.request().url().slice(PICS.length + 1));
    ctx.__pictureHits.push(path);
    const [, id = "x"] = /(?:pic|note|rimg)\/([^/~]+)/.exec(path) ?? [];
    let h = 0;
    for (const c of id) h = (h * 31 + c.charCodeAt(0)) >>> 0;
    const col = [h % 200 + 30, (h >> 8) % 200 + 30, (h >> 16) % 200 + 30];
    await new Promise((r) => setTimeout(r, delay));
    await route.fulfill({ contentType: "image/svg+xml", headers: { "cache-control": "max-age=3600" },
      body: `<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256"><rect width="256" height="256" fill="rgb(${col.join(",")})"/></svg>` });
  });
}
