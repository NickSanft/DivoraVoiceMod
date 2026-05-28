// components.jsx — Divora shared UI library
const { useState, useEffect, useRef, useCallback } = React;

/* ---------------- Button ---------------- */
function Btn({ variant = "secondary", size, icon, iconR, children, className = "", ...p }) {
  const cls = `btn btn-${variant} ${size ? size : ""} ${className}`;
  return (
    <button className={cls} {...p}>
      {icon && <Sigil name={icon} size={size === "sm" ? 15 : 17} />}
      {children}
      {iconR && <Sigil name={iconR} size={size === "sm" ? 15 : 17} />}
    </button>
  );
}
function IconBtn({ icon, size = 19, active, className = "", tip, ...p }) {
  const btn = (
    <button className={`icon-btn ${active ? "active" : ""} ${className}`} {...p}>
      <Sigil name={icon} size={size} />
    </button>
  );
  if (!tip) return btn;
  return <span className="tip" style={{ display: "inline-flex" }}>{btn}<span className="tip-body">{tip}</span></span>;
}

/* ---------------- Toggle ---------------- */
function Toggle({ on, onChange, tone = "" }) {
  return (
    <button className={`toggle ${on ? "on" : ""} ${tone}`} onClick={() => onChange && onChange(!on)} role="switch" aria-checked={on}>
      <span className="knob" />
    </button>
  );
}

/* ---------------- Slider ---------------- */
function Slider({ value, min = 0, max = 100, step = 1, onChange, disabled, bipolar }) {
  const ref = useRef(null);
  const pct = ((value - min) / (max - min)) * 100;
  const setFromEvent = useCallback((clientX) => {
    const el = ref.current; if (!el) return;
    const r = el.getBoundingClientRect();
    let p = (clientX - r.left) / r.width; p = Math.max(0, Math.min(1, p));
    let v = min + p * (max - min);
    v = Math.round(v / step) * step;
    onChange && onChange(Math.max(min, Math.min(max, v)));
  }, [min, max, step, onChange]);
  const onDown = (e) => {
    if (disabled) return;
    setFromEvent(e.clientX);
    const move = (ev) => setFromEvent(ev.clientX);
    const up = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); };
    window.addEventListener("pointermove", move); window.addEventListener("pointerup", up);
  };
  return (
    <div ref={ref} className={`slider ${disabled ? "disabled" : ""}`} onPointerDown={onDown}>
      <div className="track" />
      {bipolar
        ? <div className="fill" style={{ left: `${Math.min(50, pct)}%`, width: `${Math.abs(pct - 50)}%` }} />
        : <div className="fill" style={{ width: `${pct}%` }} />}
      {bipolar && <div className="center-tick" style={{ left: "50%" }} />}
      <div className="thumb" style={{ left: `${pct}%` }} />
    </div>
  );
}

/* ---------------- Badge ---------------- */
function Badge({ tone = "", icon, children }) {
  return <span className={`badge ${tone}`}>{icon && <Sigil name={icon} size={11} />}{children}</span>;
}
function Kbd({ children }) { return <span className="kbd">{children}</span>; }

/* ---------------- Segmented ---------------- */
function Segmented({ options, value, onChange, accent }) {
  return (
    <div className="seg">
      {options.map((o) => {
        const val = typeof o === "string" ? o : o.value;
        const label = typeof o === "string" ? o : o.label;
        return <button key={val} className={`${value === val ? "on" : ""} ${accent ? "accent" : ""}`} onClick={() => onChange(val)}>{label}</button>;
      })}
    </div>
  );
}

/* ---------------- Level hook (smooth pseudo-audio) ---------------- */
function useLevel(active, opts = {}) {
  const { base = 0.32, swing = 0.5, speed = 0.14 } = opts;
  const [lvl, setLvl] = useState(0);
  const [peak, setPeak] = useState(0);
  const st = useRef({ lvl: 0, peak: 0, peakT: 0, t: Math.random() * 100 });
  useEffect(() => {
    let raf, last = performance.now();
    const tick = (now) => {
      const dt = Math.min(50, now - last); last = now;
      const s = st.current; s.t += dt / 1000;
      let target = 0;
      if (active) {
        const n = (Math.sin(s.t * 6) * 0.5 + Math.sin(s.t * 13.3 + 1) * 0.3 + Math.sin(s.t * 2.1) * 0.2);
        target = base + (n * 0.5 + 0.5) * swing * (0.7 + 0.3 * Math.random());
      }
      s.lvl += (target - s.lvl) * speed;
      s.lvl = Math.max(0, Math.min(1, s.lvl));
      if (s.lvl > s.peak) { s.peak = s.lvl; s.peakT = now; }
      else if (now - s.peakT > 900) { s.peak += (s.lvl - s.peak) * 0.04; }
      setLvl(s.lvl); setPeak(s.peak);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [active, base, swing, speed]);
  return { lvl, peak };
}

/* ---------------- Vertical meter ---------------- */
function VMeter({ level = 0, peak = 0, height = 200, label }) {
  const ticks = [0.25, 0.5, 0.7, 0.85];
  return (
    <div className="vmeter">
      <div className="col" style={{ height }}>
        <div className="ticks">{ticks.map((t) => <div key={t} className="tick" style={{ bottom: `${t * 100}%` }} />)}</div>
        <div className="level" style={{ height: `${level * 100}%` }} />
        {peak > 0.02 && <div className="peak" style={{ bottom: `calc(${peak * 100}% - 1px)` }} />}
      </div>
      {label && <div className="eyebrow" style={{ letterSpacing: "0.16em" }}>{label}</div>}
    </div>
  );
}

/* ---------------- Horizontal meter ---------------- */
function HMeter({ level = 0, peak = 0 }) {
  return (
    <div className="hmeter">
      <div className="level" style={{ width: `${level * 100}%` }} />
      {peak > 0.02 && <div className="peak" style={{ left: `calc(${peak * 100}% - 1px)` }} />}
    </div>
  );
}

/* ---------------- Select / Device picker ---------------- */
function Select({ icon = "mic", value, options, onChange, sub }) {
  const [open, setOpen] = useState(false);
  const ref = useRef(null);
  useEffect(() => {
    const h = (e) => { if (ref.current && !ref.current.contains(e.target)) setOpen(false); };
    document.addEventListener("pointerdown", h); return () => document.removeEventListener("pointerdown", h);
  }, []);
  const cur = options.find((o) => o.value === value) || options[0];
  return (
    <div ref={ref} style={{ position: "relative" }}>
      <button className={`select ${open ? "open" : ""}`} onClick={() => setOpen(!open)}>
        <span className="sicon"><Sigil name={icon} size={18} /></span>
        <span className="stext">
          <div className="main">{cur ? cur.label : "—"}</div>
          {(cur && cur.sub) || sub ? <div className="sub">{(cur && cur.sub) || sub}</div> : null}
        </span>
        <Sigil name="chevronD" size={16} style={{ color: "var(--text-lo)", transform: open ? "rotate(180deg)" : "none", transition: "transform .16s" }} />
      </button>
      {open && (
        <div className="dropdown" style={{ position: "absolute", top: "calc(100% + 6px)", left: 0, right: 0, zIndex: 50 }}>
          {options.map((o) => (
            <div key={o.value} className={`opt ${o.value === value ? "sel" : ""}`} onClick={() => { onChange(o.value); setOpen(false); }}>
              <Sigil name={icon} size={16} style={{ color: o.value === value ? "var(--indigo)" : "var(--text-lo)" }} />
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: "var(--t-sm)", fontWeight: 600 }}>{o.label}</div>
                {o.sub && <div style={{ fontSize: "var(--t-xs)", color: "var(--text-lo)" }}>{o.sub}</div>}
              </div>
              {o.value === value && <Sigil name="check" size={15} style={{ color: "var(--indigo)" }} />}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/* ---------------- Hotkey capture ---------------- */
function HotkeyCapture({ value, onChange }) {
  const [cap, setCap] = useState(false);
  useEffect(() => {
    if (!cap) return;
    const fmt = (e) => {
      const parts = [];
      if (e.ctrlKey) parts.push("Ctrl");
      if (e.altKey) parts.push("Alt");
      if (e.shiftKey) parts.push("Shift");
      if (e.metaKey) parts.push("Win");
      let k = e.key;
      if (k === " ") k = "Space";
      else if (k.length === 1) k = k.toUpperCase();
      if (!["Control", "Alt", "Shift", "Meta"].includes(e.key)) parts.push(k);
      return parts;
    };
    const h = (e) => {
      e.preventDefault();
      if (e.key === "Escape") { setCap(false); return; }
      if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) return;
      onChange(fmt(e)); setCap(false);
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [cap, onChange]);
  return (
    <button className={`hotkey-capture ${cap ? "capturing" : ""}`} onClick={() => setCap(!cap)}>
      {cap ? <span className="hint">Press a key… (Esc to cancel)</span>
        : (value && value.length ? value.map((k, i) => <Kbd key={i}>{k}</Kbd>) : <span className="hint">Click to set</span>)}
    </button>
  );
}

/* ---------------- Empty state ---------------- */
function EmptyState({ icon = "presets", title, children, action }) {
  return (
    <div className="empty">
      <div className="glyph"><Sigil name={icon} size={28} /></div>
      <div><h3>{title}</h3>{children && <p>{children}</p>}</div>
      {action}
    </div>
  );
}

/* ---------------- Tooltip ---------------- */
function Tip({ label, children }) {
  return <span className="tip" style={{ display: "inline-flex" }}>{children}<span className="tip-body">{label}</span></span>;
}

Object.assign(window, {
  Btn, IconBtn, Toggle, Slider, Badge, Kbd, Segmented,
  useLevel, VMeter, HMeter, Select, HotkeyCapture, EmptyState, Tip,
});
