// Pictures for the README's demo library, drawn on a canvas: skies, ridges, lakes, dunes and
// a campfire, each from a seed. Nobody's photos; nothing real. Runs in a browser page:
// `await page.evaluate(drawPictures, specs)` answers { name: base64 JPEG }.
//
// A spec is { name, w, h, scene, seed } with scene one of: landscape, lake, dunes, sea, night,
// fire, bloom, map, page.
export function drawPictures(specs) {
  const rng = (seed) => () => { seed |= 0; seed = (seed + 0x6d2b79f5) | 0; let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
  const hex = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  const mix = (a, b, t) => { const x = hex(a), y = hex(b); return `rgb(${x.map((v, i) => Math.round(v + (y[i] - v) * t)).join(",")})`; };
  const PAL = [
    { sky: ["#27467a", "#e3a3a6", "#fdd9a6"], sun: "#fff3cf", far: "#a7a3c6", near: "#2a2747" },
    { sky: ["#3f68a3", "#f3bd78", "#fbe5a6"], sun: "#fff6d6", far: "#c9967a", near: "#36262a" },
    { sky: ["#4f9be0", "#a7d1f4", "#eaf5fc"], sun: null, far: "#b9cde2", near: "#1f3650" },
    { sky: ["#2a1a4f", "#c2456d", "#f6a65e"], sun: "#ffe2b0", far: "#80406d", near: "#1a0f2e" },
    { sky: ["#86aec4", "#c6d9e1", "#ecf1f2"], sun: "#ffffff", far: "#a2b6bd", near: "#3a5360" },
    { sky: ["#3a8bb0", "#9fd4da", "#e6f5ef"], sun: "#fffbe6", far: "#7aa9a0", near: "#1d3b37" },
    { sky: ["#5b7fc7", "#c5b6e3", "#fbd9cf"], sun: "#fff0e8", far: "#9c95c6", near: "#2b2850" },
    { sky: ["#1f6fb8", "#7fc0ec", "#dff1fb"], sun: "#fffdf0", far: "#8fb9a0", near: "#1e3f2c" },
  ];
  const out = {};
  for (const spec of specs) {
    const { w, h, scene } = spec;
    const r = rng(spec.seed * 7919 + 13);
    const c = document.createElement("canvas"); c.width = w; c.height = h;
    const g = c.getContext("2d");
    const pal = PAL[spec.seed % PAL.length];
    const sky = (top, mid, low, y1 = h) => { const s = g.createLinearGradient(0, 0, 0, y1); s.addColorStop(0, top); s.addColorStop(0.55, mid); s.addColorStop(1, low); g.fillStyle = s; g.fillRect(0, 0, w, h); };
    const glow = (x, y, rad, col, a = 0.9) => { const s = g.createRadialGradient(x, y, 0, x, y, rad); s.addColorStop(0, col); s.addColorStop(0.18, col); s.addColorStop(1, "rgba(255,255,255,0)"); g.globalAlpha = a; g.fillStyle = s; g.fillRect(0, 0, w, h); g.globalAlpha = 1; };
    // A ridge line by midpoint displacement, filled down to the bottom.
    const ridge = (base, rough, col, smooth = false) => {
      let pts = [base + (r() - 0.5) * rough, base + (r() - 0.5) * rough];
      for (let d = rough; pts.length < 257; d *= 0.55) {
        const next = [];
        for (let i = 0; i < pts.length - 1; i++) next.push(pts[i], (pts[i] + pts[i + 1]) / 2 + (r() - 0.5) * d);
        next.push(pts[pts.length - 1]); pts = next;
      }
      g.beginPath(); g.moveTo(0, h);
      pts.forEach((y, i) => { const x = (i / (pts.length - 1)) * w; smooth ? g.lineTo(x, y) : g.lineTo(x, y); });
      g.lineTo(w, h); g.closePath(); g.fillStyle = col; g.fill();
      return pts;
    };
    const clouds = (n, col, y0, y1) => {
      for (let i = 0; i < n; i++) {
        const x = r() * w, y = y0 + r() * (y1 - y0), rw = w * (0.12 + r() * 0.22), rh = rw * (0.12 + r() * 0.1);
        const s = g.createRadialGradient(x, y, 0, x, y, rw); s.addColorStop(0, col); s.addColorStop(1, "rgba(255,255,255,0)");
        g.save(); g.translate(x, y); g.scale(1, rh / rw); g.translate(-x, -y); g.fillStyle = s; g.globalAlpha = 0.55; g.beginPath(); g.arc(x, y, rw, 0, 7); g.fill(); g.restore();
      }
      g.globalAlpha = 1;
    };
    const grain = (amt) => {
      const d = g.getImageData(0, 0, w, h), p = d.data;
      for (let i = 0; i < p.length; i += 4) { const n = (r() - 0.5) * amt; p[i] += n; p[i + 1] += n; p[i + 2] += n; }
      g.putImageData(d, 0, 0);
    };
    if (scene === "landscape" || scene === "lake") {
      const horizon = h * (scene === "lake" ? 0.56 : 0.62);
      sky(pal.sky[0], pal.sky[1], pal.sky[2], horizon);
      g.fillStyle = pal.sky[2]; g.fillRect(0, horizon, w, h);
      const sx = w * (0.25 + r() * 0.5), sy = horizon - h * (0.08 + r() * 0.18);
      if (pal.sun) { glow(sx, sy, h * 0.55, pal.sun, 0.55); g.fillStyle = pal.sun; g.beginPath(); g.arc(sx, sy, h * 0.035, 0, 7); g.fill(); }
      clouds(5, "rgba(255,255,255,0.9)", h * 0.08, horizon * 0.7);
      const layers = scene === "lake" ? 3 : 5;
      for (let k = 0; k < layers; k++) {
        const t = k / (layers - 1);
        const base = scene === "lake" ? horizon - h * (0.2 - t * 0.17) : horizon - h * 0.22 + t * h * 0.34;
        ridge(base, h * (0.3 - t * 0.12), mix(pal.far, pal.near, t));
        // Haze between layers.
        const hz = g.createLinearGradient(0, base - h * 0.1, 0, h); hz.addColorStop(0, "rgba(255,255,255,0)"); hz.addColorStop(1, pal.sky[2]);
        g.globalAlpha = 0.18 * (1 - t); g.fillStyle = hz; g.fillRect(0, 0, w, h); g.globalAlpha = 1;
      }
      if (scene === "lake") {
        // The water: the sky turned over, darker, with ripples of light.
        g.save(); g.translate(0, horizon * 2); g.scale(1, -1); g.globalAlpha = 0.85; g.drawImage(c, 0, 0, w, horizon, 0, 0, w, horizon); g.restore();
        g.globalAlpha = 1;
        const water = g.createLinearGradient(0, horizon, 0, h); water.addColorStop(0, "rgba(10,30,60,0.15)"); water.addColorStop(1, "rgba(10,25,50,0.55)");
        g.fillStyle = water; g.fillRect(0, horizon, w, h - horizon);
        for (let i = 0; i < 140; i++) { const y = horizon + Math.pow(r(), 1.6) * (h - horizon); g.fillStyle = `rgba(255,255,255,${0.05 + r() * 0.12})`; g.fillRect(r() * w, y, w * (0.02 + r() * 0.08), 1 + (y - horizon) / 160); }
        ridge(h * 0.94, h * 0.06, mix(pal.near, "#000000", 0.4));
      }
      grain(10);
    } else if (scene === "dunes") {
      sky("#3b77c4", "#a5cdec", "#f4dcc0", h * 0.5);
      glow(w * 0.7, h * 0.3, h * 0.5, "#fff4dc", 0.5);
      for (let k = 0; k < 5; k++) {
        const t = k / 4, base = h * (0.48 + t * 0.12), amp = h * (0.06 + r() * 0.05), ph = r() * 6, f = 1.2 + r() * 1.6;
        g.beginPath(); g.moveTo(0, h);
        for (let x = 0; x <= w; x += 4) g.lineTo(x, base + Math.sin(x / w * Math.PI * f + ph) * amp + Math.sin(x / w * 9 + ph) * amp * 0.15);
        g.lineTo(w, h); g.closePath();
        const s = g.createLinearGradient(0, base - amp, w, h); s.addColorStop(0, mix("#f1c48e", "#c9773e", t)); s.addColorStop(1, mix("#d79b62", "#7c3f1f", t));
        g.fillStyle = s; g.fill();
      }
      grain(12);
    } else if (scene === "sea") {
      const horizon = h * 0.5;
      sky(pal.sky[0], pal.sky[1], pal.sky[2], horizon);
      if (pal.sun) { glow(w * 0.5, horizon - h * 0.06, h * 0.6, pal.sun, 0.6); }
      clouds(7, "rgba(255,255,255,0.85)", h * 0.05, horizon * 0.8);
      const sea = g.createLinearGradient(0, horizon, 0, h); sea.addColorStop(0, mix(pal.sky[2], "#1d4f7a", 0.5)); sea.addColorStop(1, "#0e2f4f");
      g.fillStyle = sea; g.fillRect(0, horizon, w, h - horizon);
      for (let i = 0; i < 260; i++) { const y = horizon + Math.pow(r(), 1.8) * (h - horizon); const cx = w * 0.5 + (r() - 0.5) * w * (0.15 + (y - horizon) / h);
        g.fillStyle = `rgba(255,240,210,${0.08 + r() * 0.25})`; g.fillRect(cx, y, 8 + r() * 40, 1 + (y - horizon) / 220); }
      grain(9);
    } else if (scene === "night") {
      sky("#070d24", "#16285a", "#3a4f8c", h * 0.75);
      for (let i = 0; i < 420; i++) { g.fillStyle = `rgba(255,255,255,${r() * 0.9})`; const s = r() < 0.05 ? 2.2 : 1.1; g.fillRect(r() * w, r() * h * 0.7, s, s); }
      glow(w * 0.75, h * 0.2, h * 0.25, "#e8eeff", 0.35); g.fillStyle = "#f4f6ff"; g.beginPath(); g.arc(w * 0.75, h * 0.2, h * 0.03, 0, 7); g.fill();
      for (let k = 0; k < 3; k++) ridge(h * (0.62 + k * 0.1), h * 0.2, mix("#2b3d70", "#05081a", k / 2));
      grain(8);
    } else if (scene === "fire") {
      g.fillStyle = "#0b0706"; g.fillRect(0, 0, w, h);
      glow(w * 0.5, h * 0.72, h * 0.8, "#ff8a2a", 0.55); glow(w * 0.5, h * 0.75, h * 0.3, "#ffd27a", 0.7);
      for (let i = 0; i < 90; i++) { const x = w * 0.5 + (r() - 0.5) * w * 0.5, y = h * (0.15 + r() * 0.55), s = 1 + r() * 3;
        g.fillStyle = `rgba(255,${150 + r() * 90 | 0},60,${0.3 + r() * 0.6})`; g.beginPath(); g.arc(x, y, s, 0, 7); g.fill(); }
      for (let i = 0; i < 7; i++) { g.save(); g.translate(w * 0.5, h * 0.86); g.rotate((i / 7 - 0.5) * 1.6); g.fillStyle = "#1a0f0b"; g.fillRect(-w * 0.2, -h * 0.02, w * 0.4, h * 0.04); g.restore(); }
      ridge(h * 0.9, h * 0.05, "#050303");
      grain(10);
    } else if (scene === "bloom") {
      // Soft out-of-focus garden light.
      const base = pal.sky;
      sky(mix(base[0], "#1d3b2a", 0.5), mix(base[1], "#5f8f4e", 0.4), mix(base[2], "#c9e3a5", 0.3));
      for (let i = 0; i < 40; i++) { const x = r() * w, y = r() * h, rad = h * (0.03 + r() * 0.12);
        const col = [pal.sun ?? "#ffffff", "#ffd6e0", "#fff2b8", "#d8f5c8"][i % 4];
        const s = g.createRadialGradient(x, y, rad * 0.6, x, y, rad); s.addColorStop(0, col); s.addColorStop(1, "rgba(255,255,255,0)");
        g.globalAlpha = 0.25 + r() * 0.35; g.fillStyle = s; g.beginPath(); g.arc(x, y, rad, 0, 7); g.fill(); }
      g.globalAlpha = 1; grain(8);
    } else if (scene === "map") {
      g.fillStyle = "#f3efe2"; g.fillRect(0, 0, w, h);
      const cx = w * 0.55, cy = h * 0.45;
      for (let k = 1; k < 16; k++) { g.beginPath(); for (let a = 0; a <= 64; a++) { const t = a / 64 * Math.PI * 2, rad = k * h * 0.035 * (1 + 0.18 * Math.sin(t * 3 + k * 0.4) + 0.08 * Math.sin(t * 7));
        const x = cx + Math.cos(t) * rad * 1.3, y = cy + Math.sin(t) * rad; a ? g.lineTo(x, y) : g.moveTo(x, y); } g.strokeStyle = k % 5 ? "rgba(150,110,60,0.45)" : "rgba(120,80,40,0.8)"; g.lineWidth = k % 5 ? 1.2 : 2; g.stroke(); }
      g.fillStyle = "rgba(90,150,210,0.55)"; g.beginPath(); g.ellipse(w * 0.2, h * 0.75, w * 0.14, h * 0.09, 0.3, 0, 7); g.fill();
      g.setLineDash([10, 8]); g.strokeStyle = "#d2452c"; g.lineWidth = 4; g.beginPath(); g.moveTo(w * 0.1, h * 0.95); g.bezierCurveTo(w * 0.3, h * 0.6, w * 0.4, h * 0.7, cx, cy); g.stroke(); g.setLineDash([]);
    } else if (scene === "page") {
      g.fillStyle = "#ffffff"; g.fillRect(0, 0, w, h);
      g.fillStyle = "#2f6fd6"; g.fillRect(w * 0.1, h * 0.08, w * 0.5, h * 0.035);
      for (let i = 0; i < 22; i++) { g.fillStyle = i % 6 === 0 ? "#44474f" : "#b9bdc6"; g.fillRect(w * 0.1, h * (0.17 + i * 0.034), w * (i % 6 === 0 ? 0.42 : 0.6 + r() * 0.2), h * 0.012); }
    }
    out[spec.name] = c.toDataURL("image/jpeg", 0.86).split(",")[1];
  }
  return out;
}
