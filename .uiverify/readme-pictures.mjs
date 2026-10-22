// Pictures for the README's demo library, drawn on a canvas: skies, ridges, lakes, a city at
// dusk, tulips, coffee, forests, beaches, illustrated contact portraits and document pages,
// each from a seed. Nobody's photos; nothing real. Runs in a browser page:
// `await page.evaluate(drawPictures, specs)` answers { name: base64 JPEG }.
//
// A spec is { name, w, h, scene, seed, grey? } with scene one of: landscape, lake, dunes, sea, night,
// fire, bloom, city, tulips, coffee, forest, aurora, beach, portrait, map, page, sheet, slides.
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
    } else if (scene === "city") {
      // A skyline at dusk over a river, its windows lit.
      const horizon = h * 0.7;
      sky(["#1d2a5c", "#2a2a6b", "#14204a"][spec.seed % 3], ["#b2557a", "#d0607a", "#8b5aa8"][spec.seed % 3], ["#f6b37a", "#ffcf8f", "#f59f7a"][spec.seed % 3], horizon);
      glow(w * (0.3 + r() * 0.4), horizon, h * 0.5, "#ffd9a0", 0.35);
      for (const [k, far] of [[0, true], [1, false]]) {
        let x = -10;
        while (x < w) {
          const bw = w * (0.04 + r() * 0.07), bh = h * (far ? 0.12 + r() * 0.22 : 0.08 + r() * 0.36);
          g.fillStyle = far ? "#3a3566" : "#16142c";
          g.fillRect(x, horizon - bh, bw, bh);
          if (!far && r() < 0.3) g.fillRect(x + bw * 0.45, horizon - bh - h * 0.05, bw * 0.1, h * 0.05);
          if (!far) for (let wy = horizon - bh + 6; wy < horizon - 6; wy += h * 0.022)
            for (let wx = x + 4; wx < x + bw - 6; wx += w * 0.012)
              if (r() < 0.32) { g.fillStyle = `rgba(255,${200 + r() * 40 | 0},${120 + r() * 60 | 0},${0.55 + r() * 0.4})`; g.fillRect(wx, wy, w * 0.005, h * 0.008); }
          x += bw + (far ? 0 : w * 0.004);
        }
      }
      g.save(); g.translate(0, horizon * 2); g.scale(1, -1); g.globalAlpha = 0.45; g.drawImage(c, 0, 0, w, horizon, 0, 0, w, horizon); g.restore(); g.globalAlpha = 1;
      const water = g.createLinearGradient(0, horizon, 0, h); water.addColorStop(0, "rgba(20,18,50,0.35)"); water.addColorStop(1, "rgba(8,8,24,0.85)");
      g.fillStyle = water; g.fillRect(0, horizon, w, h - horizon);
      grain(8);
    } else if (scene === "tulips") {
      // A field of tulips under a soft sky.
      sky("#7fb5ea", "#c8e2f6", "#eef6e4", h * 0.45);
      ridge(h * 0.42, h * 0.06, "#8fb07a");
      const field = g.createLinearGradient(0, h * 0.42, 0, h); field.addColorStop(0, "#5f8f45"); field.addColorStop(1, "#2f5a26");
      g.fillStyle = field; g.fillRect(0, h * 0.44, w, h);
      const cols = [["#e8384f", "#b81f3a"], ["#ff9fb5", "#e0607f"], ["#ffd34d", "#e8a91c"], ["#f6f1ea", "#d9cfc4"]][spec.seed % 4];
      for (let row = 0; row < 26; row++) {
        const t = row / 25, y = h * (0.46 + Math.pow(t, 1.6) * 0.6), s = h * (0.008 + Math.pow(t, 2) * 0.06);
        for (let x = -s; x < w + s; x += s * (2.2 + r())) {
          const xx = x + (r() - 0.5) * s;
          g.strokeStyle = "#3e6b2c"; g.lineWidth = Math.max(1, s * 0.18); g.beginPath(); g.moveTo(xx, y); g.lineTo(xx, y + s * 2.5); g.stroke();
          const tg = g.createLinearGradient(xx, y - s, xx, y + s); tg.addColorStop(0, cols[0]); tg.addColorStop(1, cols[1]);
          g.fillStyle = tg; g.beginPath(); g.ellipse(xx, y, s * 0.6, s * 0.85, 0, 0, 7); g.fill();
          g.beginPath(); g.moveTo(xx - s * 0.6, y - s * 0.2); g.lineTo(xx - s * 0.3, y - s * 1.05); g.lineTo(xx, y - s * 0.4); g.lineTo(xx + s * 0.3, y - s * 1.05); g.lineTo(xx + s * 0.6, y - s * 0.2); g.fill();
        }
      }
      grain(9);
    } else if (scene === "coffee") {
      // A flat white from above, on a wooden table.
      const wood = ["#8a5a3b", "#a7744f", "#6e4630"][spec.seed % 3];
      g.fillStyle = wood; g.fillRect(0, 0, w, h);
      for (let i = 0; i < 70; i++) { g.strokeStyle = `rgba(${r() < 0.5 ? "40,20,10" : "255,220,180"},${0.05 + r() * 0.08})`; g.lineWidth = 1 + r() * 4; g.beginPath();
        const y = r() * h; g.moveTo(0, y); g.bezierCurveTo(w * 0.3, y + (r() - 0.5) * 30, w * 0.6, y + (r() - 0.5) * 30, w, y + (r() - 0.5) * 20); g.stroke(); }
      const cx = w * (0.42 + r() * 0.16), cy = h * 0.5, R = Math.min(w, h) * 0.3;
      g.shadowColor = "rgba(0,0,0,0.35)"; g.shadowBlur = R * 0.25; g.shadowOffsetY = R * 0.06;
      g.fillStyle = "#f4f1ec"; g.beginPath(); g.arc(cx, cy, R * 1.35, 0, 7); g.fill();
      g.shadowBlur = R * 0.12; g.fillStyle = "#ffffff"; g.beginPath(); g.arc(cx, cy, R, 0, 7); g.fill();
      g.shadowColor = "transparent";
      g.beginPath(); g.ellipse(cx + R * 1.02, cy + R * 0.2, R * 0.32, R * 0.14, 0.4, 0, 7); g.fillStyle = "#f7f4ef"; g.fill();
      const cof = g.createRadialGradient(cx, cy, 0, cx, cy, R * 0.82); cof.addColorStop(0, "#c08a5c"); cof.addColorStop(0.75, "#9a6337"); cof.addColorStop(1, "#5e3517");
      g.fillStyle = cof; g.beginPath(); g.arc(cx, cy, R * 0.82, 0, 7); g.fill();
      // A heart in the foam.
      g.fillStyle = "rgba(250,240,225,0.95)"; g.beginPath();
      const hs = R * 0.36; g.moveTo(cx, cy + hs * 0.9);
      g.bezierCurveTo(cx - hs * 1.6, cy - hs * 0.1, cx - hs * 0.7, cy - hs * 1.2, cx, cy - hs * 0.45);
      g.bezierCurveTo(cx + hs * 0.7, cy - hs * 1.2, cx + hs * 1.6, cy - hs * 0.1, cx, cy + hs * 0.9); g.fill();
      grain(7);
    } else if (scene === "forest") {
      // Pines fading into morning fog.
      sky("#c9d6dc", "#dfe6e6", "#eef0ea");
      for (let k = 0; k < 5; k++) {
        const t = k / 4, base = h * (0.45 + t * 0.14), col = mix("#a9bcbf", "#1d3a30", t), size = h * (0.12 + t * 0.16);
        for (let x = -size; x < w + size; x += size * (0.32 + r() * 0.35)) {
          const top = base - size * (0.7 + r() * 0.6);
          g.fillStyle = col; g.beginPath(); g.moveTo(x, top);
          for (let s = 1; s <= 4; s++) { const yy = top + (base - top) * (s / 4); g.lineTo(x + size * 0.14 * s, yy); g.lineTo(x + size * 0.07 * s, yy); }
          for (let s = 4; s >= 1; s--) { const yy = top + (base - top) * (s / 4); g.lineTo(x - size * 0.07 * s, yy); g.lineTo(x - size * 0.14 * s, yy); }
          g.closePath(); g.fill(); g.fillRect(x - size * 0.6, base - 1, size * 1.2, h);
        }
        const fog = g.createLinearGradient(0, base - size, 0, h); fog.addColorStop(0, "rgba(235,240,238,0)"); fog.addColorStop(1, "rgba(235,240,238,0.55)");
        g.globalAlpha = 1 - t * 0.7; g.fillStyle = fog; g.fillRect(0, 0, w, h); g.globalAlpha = 1;
      }
      glow(w * 0.7, h * 0.15, h * 0.5, "#fffbea", 0.4);
      grain(8);
    } else if (scene === "aurora") {
      sky("#020814", "#071a2c", "#0f2f3c", h * 0.8);
      for (let i = 0; i < 300; i++) { g.fillStyle = `rgba(255,255,255,${r() * 0.8})`; g.fillRect(r() * w, r() * h * 0.6, 1.3, 1.3); }
      g.globalCompositeOperation = "lighter";
      for (let k = 0; k < 4; k++) {
        const y0 = h * (0.25 + k * 0.07), amp = h * 0.1;
        g.shadowColor = k % 2 ? "#5cffb0" : "#46e08c"; g.shadowBlur = h * 0.08;
        g.strokeStyle = `rgba(${k === 3 ? "170,120,255" : "80,240,160"},0.22)`; g.lineWidth = h * (0.05 + r() * 0.05);
        g.beginPath(); g.moveTo(-20, y0);
        g.bezierCurveTo(w * 0.3, y0 - amp * (1 + r()), w * 0.6, y0 + amp * r(), w + 20, y0 - amp * 0.6); g.stroke();
      }
      g.shadowBlur = 0; g.globalCompositeOperation = "source-over";
      ridge(h * 0.7, h * 0.12, "#04101a");
      const water = g.createLinearGradient(0, h * 0.8, 0, h); water.addColorStop(0, "#0b2a30"); water.addColorStop(1, "#02070c");
      g.fillStyle = water; g.fillRect(0, h * 0.82, w, h);
      grain(8);
    } else if (scene === "beach") {
      // Turquoise water meeting the sand, waves foaming in.
      const horizon = h * 0.38;
      sky("#4aa3e8", "#9fd2f5", "#e8f6fd", horizon);
      clouds(5, "rgba(255,255,255,0.95)", h * 0.06, horizon * 0.7);
      const sea = g.createLinearGradient(0, horizon, 0, h * 0.7); sea.addColorStop(0, "#1f6fa8"); sea.addColorStop(0.6, "#2fb3c4"); sea.addColorStop(1, "#7fe0d6");
      g.fillStyle = sea; g.fillRect(0, horizon, w, h);
      const shore = (y, col, amp) => { g.beginPath(); g.moveTo(0, h); for (let x = 0; x <= w; x += 6) g.lineTo(x, y + Math.sin(x / w * 7 + spec.seed) * amp + Math.sin(x / w * 19) * amp * 0.3); g.lineTo(w, h); g.closePath(); g.fillStyle = col; g.fill(); };
      shore(h * 0.66, "rgba(255,255,255,0.85)", h * 0.02);
      shore(h * 0.69, "#f0dcb4", h * 0.02);
      const sand = g.createLinearGradient(0, h * 0.7, 0, h); sand.addColorStop(0, "rgba(240,220,180,0)"); sand.addColorStop(1, "#d9b98a");
      g.fillStyle = sand; g.fillRect(0, h * 0.7, w, h);
      grain(10);
    } else if (scene === "portrait") {
      // A flat illustrated portrait, as people often choose for a contact: a figure on colour.
      const bgs = [["#ffd8c2", "#ffb48f"], ["#cfe6ff", "#9cc8f5"], ["#d9f2d0", "#a8d99a"], ["#efe0ff", "#c9b0f0"], ["#fff1b8", "#f7d36b"], ["#ffd1dc", "#f7a1b6"], ["#cdeff0", "#8fd3d6"]];
      const skins = ["#f6d2b8", "#e8b48f", "#c98b62", "#a86b45", "#7a4a2e", "#f1c6a4"];
      const hairs = ["#2b1d16", "#5a3a22", "#a8643a", "#d9b26a", "#1a1a1a"];
      const shirts = ["#2f6fd6", "#e8594a", "#2f9e6b", "#7b4fd6", "#f2a33a", "#283043", "#e5e1d8"];
      const pick = (a, k) => a[(spec.seed * [1, 5, 3, 3][k] + k) % a.length];
      const bg = pick(bgs, 0), skin = pick(skins, 1), hair = spec.grey ? "#a7a7a7" : pick(hairs, 2), shirt = pick(shirts, 3), style = spec.seed % 4;
      const s = g.createLinearGradient(0, 0, w, h); s.addColorStop(0, bg[0]); s.addColorStop(1, bg[1]); g.fillStyle = s; g.fillRect(0, 0, w, h);
      const cx = w / 2, hy = h * 0.43, rx = w * 0.17, ry = h * 0.2;
      if (style === 1) { g.fillStyle = hair; g.beginPath(); g.roundRect(cx - rx * 1.25, hy - ry * 0.6, rx * 2.5, ry * 2.3, rx * 0.9); g.fill(); }
      g.fillStyle = shirt; g.beginPath(); g.ellipse(cx, h * 1.02, w * 0.36, h * 0.28, 0, 0, 7); g.fill();
      g.fillStyle = mix(skin, "#000000", 0.12); g.fillRect(cx - rx * 0.42, hy + ry * 0.6, rx * 0.84, h * 0.17);
      g.fillStyle = skin; g.beginPath(); g.ellipse(cx, hy, rx, ry, 0, 0, 7); g.fill();
      g.beginPath(); g.ellipse(cx - rx * 0.98, hy + ry * 0.1, rx * 0.16, ry * 0.18, 0, 0, 7); g.ellipse(cx + rx * 0.98, hy + ry * 0.1, rx * 0.16, ry * 0.18, 0, 0, 7); g.fill();
      g.fillStyle = hair;
      if (style === 2) { for (let i = 0; i < 16; i++) { const a = Math.PI * (1.02 + i / 15 * 0.96); g.beginPath(); g.arc(cx + Math.cos(a) * rx * 0.95, hy - ry * 0.12 + Math.sin(a) * ry * 0.95, rx * 0.34, 0, 7); g.fill(); } }
      else { g.beginPath(); g.ellipse(cx, hy - ry * 0.28, rx * 1.06, ry * 0.8, 0, Math.PI * 1.02, Math.PI * 1.98); g.fill();
        g.beginPath(); g.moveTo(cx - rx * 1.04, hy - ry * 0.3); g.quadraticCurveTo(cx - rx * 0.2, hy - ry * 0.15, cx + rx * 0.5, hy - ry * 0.62); g.lineTo(cx + rx * 1.04, hy - ry * 0.3); g.lineTo(cx, hy - ry * 1.08); g.closePath(); g.fill(); }
      if (style === 3) { g.beginPath(); g.arc(cx, hy - ry * 1.12, rx * 0.42, 0, 7); g.fill(); }
      // A face kept simple: eyes, a smile, a little warmth in the cheeks.
      g.fillStyle = "#2a1c18"; for (const d of [-1, 1]) { g.beginPath(); g.ellipse(cx + d * rx * 0.36, hy + ry * 0.12, rx * 0.07, ry * 0.07, 0, 0, 7); g.fill(); }
      g.fillStyle = "rgba(240,110,110,0.22)"; for (const d of [-1, 1]) { g.beginPath(); g.arc(cx + d * rx * 0.55, hy + ry * 0.38, rx * 0.16, 0, 7); g.fill(); }
      g.strokeStyle = "#7a3b2e"; g.lineWidth = w * 0.012; g.lineCap = "round"; g.beginPath(); g.arc(cx, hy + ry * 0.36, rx * 0.26, Math.PI * 0.2, Math.PI * 0.8); g.stroke();
    } else if (scene === "sheet") {
      // A spreadsheet's first page: a title, a table and a small chart.
      g.fillStyle = "#ffffff"; g.fillRect(0, 0, w, h);
      g.fillStyle = "#1f2430"; g.fillRect(w * 0.08, h * 0.06, w * 0.45, h * 0.035);
      const top = h * 0.14, rows = 12, rh = h * 0.032;
      g.fillStyle = "#34a853"; g.fillRect(w * 0.08, top, w * 0.84, rh);
      for (let i = 1; i <= rows; i++) { g.fillStyle = i % 2 ? "#f3f6f4" : "#ffffff"; g.fillRect(w * 0.08, top + i * rh, w * 0.84, rh);
        g.fillStyle = "#9aa0a6"; g.fillRect(w * 0.1, top + i * rh + rh * 0.35, w * (0.18 + r() * 0.1), rh * 0.3);
        g.fillStyle = "#5f6368"; g.fillRect(w * 0.62, top + i * rh + rh * 0.35, w * (0.08 + r() * 0.06), rh * 0.3); g.fillRect(w * 0.8, top + i * rh + rh * 0.35, w * 0.08, rh * 0.3); }
      const cb = top + (rows + 2) * rh;
      for (let i = 0; i < 6; i++) { const bh = h * (0.08 + r() * 0.2); g.fillStyle = ["#34a853", "#4285f4", "#fbbc04"][i % 3]; g.fillRect(w * (0.14 + i * 0.13), cb + h * 0.3 - bh, w * 0.08, bh); }
      g.fillStyle = "#dadce0"; g.fillRect(w * 0.1, cb + h * 0.3, w * 0.8, 2);
    } else if (scene === "slides") {
      // A presentation's title slide.
      const s = g.createLinearGradient(0, 0, w, h); s.addColorStop(0, "#1b2a6b"); s.addColorStop(1, "#4b2d84"); g.fillStyle = s; g.fillRect(0, 0, w, h);
      glow(w * 0.8, h * 0.2, h * 0.7, "#ff8fb1", 0.35);
      g.fillStyle = "#ffffff"; g.fillRect(w * 0.08, h * 0.38, w * 0.56, h * 0.09);
      g.fillStyle = "rgba(255,255,255,0.6)"; g.fillRect(w * 0.08, h * 0.53, w * 0.36, h * 0.04);
      g.fillStyle = "rgba(255,255,255,0.35)"; g.fillRect(w * 0.08, h * 0.82, w * 0.18, h * 0.03);
    }
    out[spec.name] = c.toDataURL("image/jpeg", 0.86).split(",")[1];
  }
  return out;
}
