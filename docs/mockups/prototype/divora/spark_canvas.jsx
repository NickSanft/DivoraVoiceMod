// spark_canvas.jsx — background drag emits fading sparks; drawing a bound glyph
// (triangle / inverted triangle / square / circle) casts its assigned preset.
// Everything is drawn on one canvas via rAF, independent of CSS animation.
const { useRef: uR2, useEffect: uE2 } = React;

// ---- geometry helpers ----
function perpDist(p, a, b) {
  const dx = b.x - a.x, dy = b.y - a.y;
  const len = Math.hypot(dx, dy) || 1;
  return Math.abs((p.x - a.x) * dy - (p.y - a.y) * dx) / len;
}
function rdp(pts, eps) {
  if (pts.length < 3) return pts.slice();
  let dmax = 0, idx = 0;
  const a = pts[0], b = pts[pts.length - 1];
  for (let i = 1; i < pts.length - 1; i++) {
    const d = perpDist(pts[i], a, b);
    if (d > dmax) { dmax = d; idx = i; }
  }
  if (dmax > eps) {
    const l = rdp(pts.slice(0, idx + 1), eps);
    const r = rdp(pts.slice(idx), eps);
    return l.slice(0, -1).concat(r);
  }
  return [a, b];
}
function polyArea(p) {
  let s = 0;
  for (let i = 0; i < p.length; i++) { const q = p[(i + 1) % p.length]; s += p[i].x * q.y - q.x * p[i].y; }
  return Math.abs(s / 2);
}

// returns {type:'triangle'|'invtriangle'|'square'|'circle', cx, cy, size, corners?, r?}
function detectShape(path) {
  if (path.length < 12) return null;
  let pts = path.slice();
  while (pts.length > 3 && Math.hypot(pts[0].x - pts[pts.length - 1].x, pts[0].y - pts[pts.length - 1].y) < 3) pts.pop();
  const xs = pts.map(p => p.x), ys = pts.map(p => p.y);
  const minX = Math.min(...xs), maxX = Math.max(...xs), minY = Math.min(...ys), maxY = Math.max(...ys);
  const w = maxX - minX, h = maxY - minY, diag = Math.hypot(w, h);
  if (diag < 110) return null;
  const s = pts[0], e = pts[pts.length - 1];
  if (Math.hypot(e.x - s.x, e.y - s.y) > diag * 0.30) return null;        // must be closed
  const cx = xs.reduce((a, b) => a + b, 0) / xs.length;
  const cy = ys.reduce((a, b) => a + b, 0) / ys.length;
  // radial uniformity (circle test) from the full path
  const radii = pts.map(p => Math.hypot(p.x - cx, p.y - cy));
  const meanR = radii.reduce((a, b) => a + b, 0) / radii.length;
  const stdR = Math.sqrt(radii.reduce((a, b) => a + (b - meanR) ** 2, 0) / radii.length);
  const circ = meanR ? stdR / meanR : 1;
  const aspect = Math.min(w, h) / Math.max(w, h);
  // corner count
  let simp = rdp(pts, diag * 0.09);
  while (simp.length > 3 && Math.hypot(simp[0].x - simp[simp.length - 1].x, simp[0].y - simp[simp.length - 1].y) < diag * 0.20) simp.pop();
  const n = simp.length;

  if (n >= 5 && circ < 0.17 && aspect > 0.7) return { type: "circle", cx, cy, size: diag, r: meanR };
  if (n === 3) {
    if (polyArea(simp) < w * h * 0.16) return null;
    const tcy = (simp[0].y + simp[1].y + simp[2].y) / 3;
    const above = simp.filter(p => p.y < tcy).length;
    return { type: above === 1 ? "triangle" : "invtriangle", cx, cy, size: diag, corners: simp };
  }
  if (n === 4) {
    if (polyArea(simp) < w * h * 0.4) return null;
    return { type: "square", cx, cy, size: diag, corners: simp };
  }
  if (n >= 5 && circ < 0.24 && aspect > 0.6) return { type: "circle", cx, cy, size: diag, r: meanR };
  return null;
}

const TRAIL = ["#7C5CF6", "#EC4899", "#9F7CFF", "#C9B8FF"];
const OMEN_LIFE = 2500;

function SparkLayer({ bindings, onCast }) {
  const cvs = uR2(null);
  const st = uR2({ parts: [], drawing: false, path: [], last: null, raf: 0, running: false, omen: null });
  const props = uR2({ bindings, onCast });
  props.current = { bindings, onCast };

  uE2(() => {
    const canvas = cvs.current; if (!canvas) return;
    const ctx = canvas.getContext("2d");
    const S = st.current;

    const toLocal = (clientX, clientY) => {
      const r = canvas.getBoundingClientRect();
      return { x: (clientX - r.left) * (1100 / r.width), y: (clientY - r.top) * (720 / r.height) };
    };
    const isInteractive = (el) => !el || !!(el.closest && el.closest(
      'button,a,input,select,textarea,[role="switch"],[role="radiogroup"],.slider,.toggle,.seg,.select,.dropdown,.hotkey-capture,.tweaks-root,[data-tweaks]'));

    const spawn = (x, y, n, opts = {}) => {
      const pal = opts.colors || TRAIL;
      for (let i = 0; i < n; i++) {
        const ang = opts.burst != null ? Math.random() * Math.PI * 2 : (opts.dir || 0) + (Math.random() - 0.5) * 1.4;
        const spd = opts.burst != null ? (1.5 + Math.random() * opts.burst) : (0.3 + Math.random() * 0.9);
        S.parts.push({
          x, y, vx: Math.cos(ang) * spd, vy: Math.sin(ang) * spd - (opts.burst != null ? 0 : 0.3),
          life: 1, decay: 0.012 + Math.random() * 0.02,
          size: (opts.size || 2) + Math.random() * 2.4,
          color: pal[(Math.random() * pal.length) | 0],
        });
      }
      if (S.parts.length > 1600) S.parts.splice(0, S.parts.length - 1600);
      ensure();
    };

    const drawShapeOutline = (o, half) => {
      const x = o.x, y = o.y;
      ctx.beginPath();
      if (o.type === "triangle") {
        ctx.moveTo(x, y - half); ctx.lineTo(x + half * 0.92, y + half * 0.7); ctx.lineTo(x - half * 0.92, y + half * 0.7); ctx.closePath();
      } else if (o.type === "invtriangle") {
        ctx.moveTo(x - half * 0.92, y - half * 0.7); ctx.lineTo(x + half * 0.92, y - half * 0.7); ctx.lineTo(x, y + half); ctx.closePath();
      } else if (o.type === "square") {
        const s = half * 0.82, rad = 10;
        ctx.moveTo(x - s + rad, y - s); ctx.arcTo(x + s, y - s, x + s, y + s, rad);
        ctx.arcTo(x + s, y + s, x - s, y + s, rad); ctx.arcTo(x - s, y + s, x - s, y - s, rad);
        ctx.arcTo(x - s, y - s, x + s, y - s, rad); ctx.closePath();
      } else { // circle
        ctx.arc(x, y, half * 0.86, 0, Math.PI * 2);
      }
      ctx.stroke();
    };

    const drawOmen = (now) => {
      const o = S.omen; if (!o) return;
      const age = now - o.start;
      if (age > OMEN_LIFE) { S.omen = null; return; }
      let a = 1;
      if (age < 280) a = age / 280;
      else if (age > OMEN_LIFE - 650) a = (OMEN_LIFE - age) / 650;
      a = Math.max(0, Math.min(1, a));
      const pop = age < 280 ? 0.6 + 0.4 * (age / 280) : 1;
      const half = Math.min(150, Math.max(95, o.size * 0.5)) * pop;

      ctx.save();
      ctx.strokeStyle = o.color; ctx.shadowColor = o.color; ctx.shadowBlur = 22; ctx.lineWidth = 3.2; ctx.lineJoin = "round";
      ctx.globalAlpha = a * 0.32;
      ctx.beginPath(); ctx.arc(o.x, o.y, half * (1.12 + (age / OMEN_LIFE) * 0.55), 0, Math.PI * 2); ctx.stroke();
      ctx.globalAlpha = a;
      drawShapeOutline(o, half);
      // labels
      ctx.shadowBlur = 14; ctx.fillStyle = o.color; ctx.textAlign = "center"; ctx.textBaseline = "middle";
      ctx.font = '700 19px "Bricolage Grotesque", system-ui, sans-serif';
      ctx.fillText(o.name, o.x, o.y + half + 28);
      ctx.shadowBlur = 0; ctx.fillStyle = "rgba(233,231,248,0.65)";
      ctx.font = '700 10px "Space Mono", monospace';
      ctx.fillText("◆  S P E L L   C A S T  ◆", o.x, o.y + half + 47);
      ctx.restore();
    };

    const tick = (now) => {
      ctx.clearRect(0, 0, 1100, 720);
      const ps = S.parts;
      for (let i = ps.length - 1; i >= 0; i--) {
        const p = ps[i];
        p.x += p.vx; p.y += p.vy; p.vy += 0.012; p.vx *= 0.985; p.vy *= 0.985;
        p.life -= p.decay;
        if (p.life <= 0) { ps.splice(i, 1); continue; }
        ctx.globalAlpha = Math.max(0, p.life);
        ctx.fillStyle = p.color; ctx.shadowColor = p.color; ctx.shadowBlur = 10;
        ctx.beginPath(); ctx.arc(p.x, p.y, p.size * p.life, 0, Math.PI * 2); ctx.fill();
      }
      ctx.globalAlpha = 1; ctx.shadowBlur = 0;
      drawOmen(now);
      const omenAlive = S.omen && (now - S.omen.start) <= OMEN_LIFE;
      if (ps.length > 0 || S.drawing || omenAlive) S.raf = requestAnimationFrame(tick);
      else S.running = false;
    };
    const ensure = () => { if (!S.running) { S.running = true; S.raf = requestAnimationFrame(tick); } };

    const conjure = (shape) => {
      const { bindings, onCast } = props.current;
      const presetId = bindings && bindings[shape.type];
      const preset = (window.PRESETS || []).find(p => p.id === presetId);
      if (!preset) return;                                   // unbound glyph → no cast
      const colors = [preset.color, "#FFFFFF", preset.color, "#C9B8FF"];
      // trace the outline with sparks
      if (shape.corners) {
        const c = shape.corners;
        for (let e = 0; e < c.length; e++) {
          const a = c[e], b = c[(e + 1) % c.length];
          const steps = Math.max(6, Math.floor(Math.hypot(b.x - a.x, b.y - a.y) / 14));
          for (let s = 0; s <= steps; s++) spawn(a.x + (b.x - a.x) * s / steps, a.y + (b.y - a.y) * s / steps, 2, { colors, size: 2.2 });
        }
      } else if (shape.r) {
        for (let i = 0; i < 28; i++) { const ang = (i / 28) * Math.PI * 2; spawn(shape.cx + shape.r * Math.cos(ang), shape.cy + shape.r * Math.sin(ang), 2, { colors, size: 2.2 }); }
      }
      spawn(shape.cx, shape.cy, 110, { colors, burst: 6.5, size: 2.6 });
      S.omen = { type: shape.type, x: shape.cx, y: shape.cy, size: shape.size, color: preset.color, name: preset.name, start: performance.now() };
      ensure();
      if (onCast) onCast(presetId);
    };

    const onDown = (ev) => {
      if (ev.button !== 0) return;
      if (isInteractive(ev.target)) { S.drawing = false; return; }
      ev.preventDefault();
      S.drawing = true; S.path = [];
      const p = toLocal(ev.clientX, ev.clientY);
      S.last = p; S.path.push(p);
      spawn(p.x, p.y, 4);
    };
    const onMove = (ev) => {
      if (!S.drawing) return;
      const p = toLocal(ev.clientX, ev.clientY);
      const d = S.last ? Math.hypot(p.x - S.last.x, p.y - S.last.y) : 0;
      const dir = S.last ? Math.atan2(p.y - S.last.y, p.x - S.last.x) : 0;
      spawn(p.x, p.y, Math.min(6, 1 + Math.floor(d / 6)), { dir });
      S.last = p; S.path.push(p);
    };
    const onUp = () => {
      if (!S.drawing) return;
      S.drawing = false;
      const shape = detectShape(S.path);
      if (shape) conjure(shape);
      S.path = [];
    };

    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("pointermove", onMove, true);
    window.addEventListener("pointerup", onUp, true);
    window.addEventListener("pointercancel", onUp, true);
    return () => {
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("pointermove", onMove, true);
      window.removeEventListener("pointerup", onUp, true);
      window.removeEventListener("pointercancel", onUp, true);
      cancelAnimationFrame(S.raf);
    };
  }, []);

  return (
    <canvas ref={cvs} width={1100} height={720}
      style={{ position: "absolute", inset: 0, width: "100%", height: "100%", pointerEvents: "none", zIndex: 58 }} />
  );
}
window.SparkLayer = SparkLayer;
