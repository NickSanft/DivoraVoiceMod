// screen_mixer.jsx — HERO. Spell-circle mixer.
function MixerScreen({ preset, chain, setChain, status, statusInfo, ui, setUi, motion, mystical }) {
  const { muted, monitor, ab, ptmMode, ptmKey, pressed } = ui;
  const set = (k, v) => setUi((u) => ({ ...u, [k]: v }));

  const hasEnabled = chain.some((c) => c.enabled);
  const inLvl = useLevel(!muted, { base: 0.28, swing: 0.55 });
  const outLvl = useLevel(!muted, { base: status === "modulated" ? 0.34 : 0.26, swing: status === "modulated" ? 0.6 : 0.45, speed: 0.18 });

  const [selected, setSelected] = useState(chain[0] ? chain[0].id : null);
  useEffect(() => { if (chain[0] && !chain.find((c) => c.id === selected)) setSelected(chain[0].id); }, [chain]);
  const selEff = selected ? EFFECTS[selected] : null;
  const selNode = chain.find((c) => c.id === selected);

  const toggleEffect = (id) => setChain((c) => c.map((e) => e.id === id ? { ...e, enabled: !e.enabled } : e));
  const setParam = (id, key, val) => setChain((c) => c.map((e) => e.id === id ? { ...e, vals: { ...e.vals, [key]: val } } : e));

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", padding: "20px 24px 22px" }}>
      {/* ---- Header ---- */}
      <div style={{ display: "flex", alignItems: "center", gap: 18, marginBottom: 8 }}>
        <span style={{ width: 38, height: 38, borderRadius: 11, display: "grid", placeItems: "center",
          background: "var(--surface-2)", border: "1px solid var(--line-glow)", color: preset.color, flex: "none" }}>
          <Sigil name={preset.glyph} size={21} />
        </span>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <h1 className="display" style={{ fontSize: 26, lineHeight: 1, whiteSpace: "nowrap", flex: "none" }}>{preset.name}</h1>
            <Badge tone={preset.tag === "User" ? "" : "accent"}>{preset.tag}</Badge>
          </div>
          <div style={{ fontSize: 12, color: "var(--text-lo)", marginTop: 3 }}>{chain.filter(c => c.enabled).length} of {chain.length} runes active · routed to CABLE Input</div>
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span className="eyebrow" style={{ marginRight: 2 }}>Compare</span>
          <Segmented options={["A", "B"]} value={ab} onChange={(v) => set("ab", v)} accent />
          <Tip label={muted ? "Unmute voice" : "Mute voice"}>
            <IconBtn icon={muted ? "muted" : "mic"} active={muted} onClick={() => set("muted", !muted)} />
          </Tip>
        </div>
      </div>

      {/* ---- Main ---- */}
      <div style={{ display: "flex", flex: 1, minHeight: 0, gap: 16, alignItems: "stretch" }}>
        {/* IN meter */}
        <div style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 8, paddingTop: 8 }}>
          <span className="eyebrow">In</span>
          <VMeter level={inLvl.lvl} peak={inLvl.peak} height={320} />
          <span className="mono" style={{ fontSize: 10, color: "var(--text-lo)" }}>{(-48 + inLvl.lvl * 48).toFixed(0)}dB</span>
        </div>

        {/* Circle */}
        <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center", position: "relative" }}>
          <SpellCircle chain={chain} status={status} motion={motion} mystical={mystical}
            selected={selected} onSelect={setSelected} onToggle={toggleEffect} />
        </div>

        {/* OUT meter */}
        <div style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 8, paddingTop: 8 }}>
          <span className="eyebrow">Out</span>
          <VMeter level={muted ? 0 : outLvl.lvl} peak={muted ? 0 : outLvl.peak} height={320} />
          <span className="mono" style={{ fontSize: 10, color: "var(--text-lo)" }}>{muted ? "−∞" : (-48 + outLvl.lvl * 48).toFixed(0)}dB</span>
        </div>

        {/* Right rail */}
        <div style={{ width: 290, display: "flex", flexDirection: "column", gap: 12, flex: "none" }}>
          {/* Voice status */}
          <div className="card" style={{ padding: 14, display: "flex", alignItems: "center", gap: 12,
            borderColor: statusInfo.line, background: statusInfo.bg }}>
            <span style={{ color: statusInfo.color, filter: status === "modulated" ? "drop-shadow(0 0 6px currentColor)" : "none" }}>
              <Sigil name={statusInfo.sigil} size={26} />
            </span>
            <div style={{ flex: 1 }}>
              <div className="display" style={{ fontSize: 16, color: statusInfo.color }}>{statusInfo.label}</div>
              <div style={{ fontSize: 11, color: "var(--text-lo)" }}>{statusInfo.sub}</div>
            </div>
            <span style={{ width: 8, height: 8, borderRadius: "50%", background: statusInfo.color,
              boxShadow: `0 0 8px ${statusInfo.color}`, animation: status === "modulated" ? "shimmer 1.4s infinite" : "none" }} />
          </div>

          {/* Push to modulate */}
          <div className="card" style={{ padding: 14 }}>
            <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 10 }}>
              <span className="eyebrow">Push to modulate</span>
              <Kbd>{ptmKey}</Kbd>
            </div>
            <Segmented options={[{ value: "apply", label: "Hold to apply" }, { value: "bypass", label: "Hold to bypass" }]}
              value={ptmMode} onChange={(v) => set("ptmMode", v)} />
            <button
              onPointerDown={() => set("pressed", true)} onPointerUp={() => set("pressed", false)}
              onPointerLeave={() => set("pressed", false)}
              style={{
                marginTop: 11, width: "100%", height: 52, borderRadius: 12, userSelect: "none",
                border: `1px solid ${pressed ? "var(--indigo)" : "var(--line-strong)"}`,
                background: pressed ? "var(--accent-bg)" : "var(--surface-1)",
                color: pressed ? "var(--indigo)" : "var(--text-mid)",
                display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 2,
                boxShadow: pressed ? "0 0 0 3px rgba(124,92,246,0.2), inset 0 0 24px rgba(124,92,246,0.15)" : "none",
                transition: "all .1s",
              }}>
              <span style={{ fontWeight: 700, fontSize: 13 }}>
                {pressed ? (ptmMode === "apply" ? "APPLYING" : "BYPASSED") : "Hold to test"}
              </span>
              <span style={{ fontSize: 10.5, color: "var(--text-lo)", fontFamily: "var(--font-mono)" }}>
                {pressed ? `${ptmKey} held` : `press & hold, or tap ${ptmKey}`}
              </span>
            </button>
          </div>

          {/* Monitor */}
          <div className="card" style={{ padding: "12px 14px", display: "flex", alignItems: "center", gap: 12 }}>
            <span style={{ color: monitor ? "var(--indigo)" : "var(--text-lo)" }}><Sigil name="monitor" size={20} /></span>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 13, fontWeight: 600 }}>Monitor</div>
              <div style={{ fontSize: 11, color: "var(--text-lo)" }}>Hear yourself in headphones</div>
            </div>
            <Toggle on={monitor} onChange={(v) => set("monitor", v)} />
          </div>

          {/* Selected rune quick edit */}
          {selEff && selNode && (
            <div className="card" style={{ padding: 14, flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
              <div style={{ display: "flex", alignItems: "center", gap: 9, marginBottom: 4 }}>
                <span style={{ color: selNode.enabled ? "var(--indigo)" : "var(--text-lo)" }}><Sigil name={selEff.sigil} size={18} /></span>
                <span style={{ fontWeight: 600, fontSize: 13, flex: 1 }}>{selEff.name}</span>
                <Toggle on={selNode.enabled} onChange={() => toggleEffect(selEff.id)} />
              </div>
              <div style={{ fontSize: 11, color: "var(--text-lo)", lineHeight: 1.4, marginBottom: 10 }}>{selEff.desc}</div>
              <div style={{ display: "flex", flexDirection: "column", gap: 12, opacity: selNode.enabled ? 1 : 0.45, pointerEvents: selNode.enabled ? "auto" : "none" }}>
                {selEff.params.map((p) => (
                  <div key={p.key}>
                    <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 5 }}>
                      <span style={{ fontSize: 11, color: "var(--text-mid)" }}>{p.label}</span>
                      <span className="mono" style={{ fontSize: 11, color: "var(--text-hi)" }}>{selNode.vals[p.key]}{p.unit}</span>
                    </div>
                    <Slider value={selNode.vals[p.key]} min={p.min} max={p.max} step={p.step} bipolar={p.bipolar}
                      onChange={(v) => setParam(selEff.id, p.key, v)} />
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
window.MixerScreen = MixerScreen;
