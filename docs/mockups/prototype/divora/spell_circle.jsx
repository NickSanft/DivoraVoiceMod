// spell_circle.jsx — the signature element: voice core + orbiting effect sigils
const { useMemo: useMemoSC } = React;

function SpellCircle({ chain, status, motion = 1, mystical = 1, onToggle, selected, onSelect }) {
  const S = 440, C = S / 2;
  const Rnode = 158, coreR = 66;
  const active = status === "modulated";
  const muted = status === "muted";

  // node positions
  const nodes = useMemoSC(() => chain.map((c, i) => {
    const ang = (-90 + i * (360 / chain.length)) * Math.PI / 180;
    return { ...c, x: C + Rnode * Math.cos(ang), y: C + Rnode * Math.sin(ang), ang };
  }), [chain]);

  const coreColor = muted ? "#F2567A" : active ? "#7C5CF6" : "#6E6590";

  // mystical level gates how much arcane decoration is drawn
  const showOuterRing = mystical >= 0.5;
  const tickMode = mystical >= 0.9 ? "full" : mystical >= 0.5 ? "sparse" : "none";
  const showConstellation = mystical >= 0.9;
  const ticks = [];
  if (tickMode !== "none") {
    for (let i = 0; i < 48; i++) {
      const major = i % 4 === 0;
      if (tickMode === "sparse" && !major) continue;
      const a = (i * 360 / 48) * Math.PI / 180;
      const r1 = 206, r2 = major ? 195 : 201;
      ticks.push(<line key={i} x1={C + r1 * Math.cos(a)} y1={C + r1 * Math.sin(a)} x2={C + r2 * Math.cos(a)} y2={C + r2 * Math.sin(a)}
        stroke="currentColor" strokeWidth={major ? 1.4 : 0.8} />);
    }
  }
  const dur = (base) => `${(base / (motion || 1)).toFixed(1)}s`;

  return (
    <div style={{ position: "relative", width: S, height: S, flex: "none" }}>
      <svg width={S} height={S} viewBox={`0 0 ${S} ${S}`} style={{ position: "absolute", inset: 0, overflow: "visible" }}>
        <defs>
          <radialGradient id="coreGlow" cx="50%" cy="50%" r="50%">
            <stop offset="0%" stopColor={active ? "#9F7CFF" : coreColor} stopOpacity={active ? 0.9 : 0.5} />
            <stop offset="55%" stopColor={active ? "#5B3FD6" : coreColor} stopOpacity={active ? 0.45 : 0.18} />
            <stop offset="100%" stopColor="#000" stopOpacity="0" />
          </radialGradient>
          <linearGradient id="threadGrad" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0%" stopColor="#DB2777" /><stop offset="100%" stopColor="#6D5BF0" />
          </linearGradient>
          <linearGradient id="coreFill" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="#4F46E5" /><stop offset="55%" stopColor="#7C5CF6" /><stop offset="100%" stopColor="#DB2777" />
          </linearGradient>
        </defs>

        {/* ambient glow behind everything — pulses with motion */}
        <circle cx={C} cy={C} r={170} fill="url(#coreGlow)"
          style={{ opacity: active ? 0.9 : 0.4, transition: "opacity .6s", transformBox: "fill-box", transformOrigin: "center",
            animation: motion > 0 ? `breathe ${active ? 3.2 : 5}s ease-in-out infinite` : "none" }} />

        {/* emanating pulse rings — clearly visible motion cue */}
        {motion > 0 && [0, 1].map((i) => (
          <circle key={"p" + i} cx={C} cy={C} r={coreR + 6} fill="none"
            stroke={active ? "rgba(124,92,246,0.55)" : "rgba(168,150,220,0.32)"} strokeWidth="1.4"
            style={{ transformBox: "fill-box", transformOrigin: "center", animation: `pulse-ring ${dur(3.4)} ${(i * 1.7).toFixed(1)}s ease-out infinite` }} />
        ))}

        {/* outer decorative ring — counter-rotating, gated by mystical level */}
        {showOuterRing && (
          <g style={{ color: muted ? "rgba(242,86,122,0.35)" : `rgba(168,150,220,${0.18 + 0.14 * mystical})`,
            transformBox: "fill-box", transformOrigin: "center",
            animation: motion > 0 ? `spin-rev ${dur(60)} linear infinite` : "none" }}>
            <circle cx={C} cy={C} r={201} fill="none" stroke="currentColor" strokeWidth="1" opacity="0.5" />
            {ticks}
          </g>
        )}

        {/* rich-only constellation dots */}
        {showConstellation && (
          <g style={{ transformBox: "fill-box", transformOrigin: "center", animation: motion > 0 ? `spin-slow ${dur(120)} linear infinite` : "none" }}>
            {[...Array(12)].map((_, i) => {
              const a = (i * 30 + 15) * Math.PI / 180, r = 178;
              return <circle key={"c" + i} cx={C + r * Math.cos(a)} cy={C + r * Math.sin(a)} r={i % 3 === 0 ? 1.6 : 0.9} fill="rgba(201,184,255,0.7)" />;
            })}
          </g>
        )}

        {/* mid ring — node orbit, rotating dashes */}
        <circle cx={C} cy={C} r={Rnode} fill="none" stroke="rgba(168,150,220,0.16)" strokeWidth="1"
          strokeDasharray="1 7" strokeLinecap="round"
          style={{ transformBox: "fill-box", transformOrigin: "center", animation: motion > 0 ? `spin-slow ${dur(40)} linear infinite` : "none" }} />

        {/* orbiting spark — unmistakable motion cue */}
        {motion > 0 && (
          <g style={{ transformBox: "fill-box", transformOrigin: "center", animation: `spin-slow ${dur(7)} linear infinite` }}>
            <circle cx={C} cy={C - Rnode} r={3} fill={active ? "#EC4899" : "#9F7CFF"}
              style={{ filter: "drop-shadow(0 0 6px currentColor)", color: active ? "#EC4899" : "#9F7CFF" }} />
          </g>
        )}

        {/* threads from core to enabled nodes */}
        {nodes.map((n) => {
          const ix = C + (coreR + 8) * Math.cos(n.ang), iy = C + (coreR + 8) * Math.sin(n.ang);
          const ox = C + (Rnode - 30) * Math.cos(n.ang), oy = C + (Rnode - 30) * Math.sin(n.ang);
          if (!n.enabled) return <line key={n.id} x1={ix} y1={iy} x2={ox} y2={oy} stroke="rgba(168,150,220,0.10)" strokeWidth="1" strokeDasharray="2 4" />;
          return <line key={n.id} x1={ix} y1={iy} x2={ox} y2={oy} stroke="url(#threadGrad)" strokeWidth={active ? 2 : 1.4}
            strokeDasharray="5 6" strokeLinecap="round" opacity={active ? 0.95 : 0.5}
            style={{ animation: active && motion > 0 ? "dash-flow 1.1s linear infinite" : "none" }} />;
        })}

        {/* core rings */}
        <circle cx={C} cy={C} r={coreR + 14} fill="none" stroke={active ? "rgba(124,92,246,0.4)" : "rgba(168,150,220,0.2)"} strokeWidth="1"
          strokeDasharray={active ? "2 5" : "none"}
          style={{ transformBox: "fill-box", transformOrigin: "center", animation: active && motion > 0 ? "spin-slow 22s linear infinite" : "none" }} />
        <circle cx={C} cy={C} r={coreR} fill="rgba(13,10,22,0.7)" stroke={coreColor} strokeWidth="1.5"
          style={{ filter: active ? "drop-shadow(0 0 24px rgba(124,92,246,0.6))" : "none", transition: "all .5s" }} />
      </svg>

      {/* particles */}
      {active && motion > 0 && (
        <div style={{ position: "absolute", inset: 0, pointerEvents: "none" }}>
          {[0, 1, 2, 3, 4, 5].map((i) => (
            <span key={i} style={{
              position: "absolute", left: `${42 + (i * 13) % 18}%`, top: `${48 + (i % 3) * 4}%`,
              width: 4, height: 4, borderRadius: "50%",
              background: i % 2 ? "#EC4899" : "#7C5CF6",
              boxShadow: "0 0 8px currentColor", color: i % 2 ? "#EC4899" : "#7C5CF6",
              animation: `float-up ${2.4 + i * 0.4}s ${i * 0.35}s ease-out infinite`,
            }} />
          ))}
        </div>
      )}

      {/* CORE label (voice sigil + state) */}
      <div style={{
        position: "absolute", left: C, top: C, transform: "translate(-50%,-50%)",
        width: coreR * 2 - 16, height: coreR * 2 - 16,
      }}>
        <div style={{
          width: "100%", height: "100%", borderRadius: "50%",
          display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 4,
          animation: active && motion > 0 ? "breathe 3.2s ease-in-out infinite" : "none",
        }}>
          <div style={{ color: muted ? "#F2567A" : active ? "#fff" : "var(--text-mid)", filter: active ? "drop-shadow(0 0 10px rgba(255,255,255,0.5))" : "none" }}>
            <Sigil name={muted ? "muted" : active ? "modulated" : "clean"} size={40} />
          </div>
          <div className="display" style={{ fontSize: 15, color: muted ? "#F2567A" : active ? "#fff" : "var(--text-mid)", letterSpacing: "0.02em" }}>
            {muted ? "Muted" : active ? "Modulated" : "Clean"}
          </div>
        </div>
      </div>

      {/* NODES */}
      {nodes.map((n) => {
        const eff = EFFECTS[n.id];
        const isSel = selected === n.id;
        return (
          <button key={n.id} onClick={() => onSelect && onSelect(n.id)} onDoubleClick={() => onToggle && onToggle(n.id)}
            title={`${eff.name} — click to inspect, double-click to ${n.enabled ? "disable" : "enable"}`}
            style={{
              position: "absolute", left: n.x, top: n.y, transform: "translate(-50%,-50%)",
              width: 62, display: "flex", flexDirection: "column", alignItems: "center", gap: 5,
              background: "none", padding: 0,
            }}>
            <span style={{
              width: 50, height: 50, borderRadius: "50%", display: "grid", placeItems: "center",
              background: n.enabled ? "var(--surface-2)" : "var(--surface-1)",
              border: isSel ? "1.5px solid var(--indigo)" : n.enabled ? "1px solid var(--line-glow)" : "1px dashed var(--line-strong)",
              color: n.enabled ? (active ? "#C9B8FF" : "var(--indigo)") : "var(--text-dim)",
              boxShadow: n.enabled && active ? "0 0 16px rgba(124,92,246,0.4)" : isSel ? "0 0 0 4px rgba(124,92,246,0.2)" : "none",
              transition: "all .25s",
            }}>
              <Sigil name={eff.sigil} size={24} />
            </span>
            <span style={{ fontFamily: "var(--font-mono)", fontSize: 9, letterSpacing: "0.04em", textTransform: "uppercase",
              color: n.enabled ? "var(--text-mid)" : "var(--text-dim)", whiteSpace: "nowrap" }}>
              {eff.name}
            </span>
            {n.enabled && <span style={{ fontFamily: "var(--font-mono)", fontSize: 9.5, color: active ? "var(--pink)" : "var(--text-lo)", whiteSpace: "nowrap" }}>{eff.readout(n.vals)}</span>}
          </button>
        );
      })}
    </div>
  );
}
window.SpellCircle = SpellCircle;
