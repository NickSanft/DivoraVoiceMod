// app.jsx — Divora shell: titlebar, sidebar nav, state, routing
const { useState: uS, useEffect: uE, useMemo, useCallback: uC } = React;

const NAV = [
  { id: "mixer", label: "Mixer", sigil: "mixer" },
  { id: "soundboard", label: "Board", sigil: "soundboard" },
  { id: "presets", label: "Presets", sigil: "presets" },
  { id: "settings", label: "Settings", sigil: "settings" },
];

function statusMeta(status) {
  if (status === "muted") return { label: "Muted", sub: "Nothing is being sent", sigil: "muted", color: "var(--danger)", line: "rgba(242,86,122,0.3)", bg: "var(--danger-bg)" };
  if (status === "modulated") return { label: "Modulated", sub: "Spell active · voice transformed", sigil: "modulated", color: "var(--indigo)", line: "var(--line-glow)", bg: "var(--accent-bg)" };
  return { label: "Clean", sub: "Your true voice, passing through", sigil: "clean", color: "var(--text-mid)", line: "var(--line)", bg: "transparent" };
}

function Titlebar({ status, sMeta }) {
  return (
    <div style={{ height: 40, flex: "none", display: "flex", alignItems: "center", gap: 12, padding: "0 14px",
      background: "var(--surface-1)", borderBottom: "1px solid var(--line)", WebkitAppRegion: "drag", position: "relative", zIndex: 30 }}>
      <DMark size={20} radius={6} />
      <span className="display" style={{ fontSize: 15, letterSpacing: "0.04em" }}>DivoraVoice</span>
      <span style={{ width: 1, height: 16, background: "var(--line-strong)" }} />
      <span style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 11.5, color: "var(--text-lo)" }}>
        <span style={{ width: 7, height: 7, borderRadius: "50%", background: sMeta.color, boxShadow: `0 0 7px ${sMeta.color}`,
          animation: status === "modulated" ? "shimmer 1.4s infinite" : "none" }} />
        {sMeta.label}
      </span>
      <div style={{ flex: 1 }} />
      <span style={{ display: "flex", alignItems: "center", gap: 5, fontSize: 10.5, color: "var(--text-dim)", fontFamily: "var(--font-mono)", letterSpacing: "0.05em" }}>
        <Sigil name="lock" size={13} /> LOCAL · NO ACCOUNT
      </span>
      <div style={{ display: "flex", gap: 4, marginLeft: 6 }}>
        {["min", "max", "close"].map((b) => (
          <span key={b} style={{ width: 26, height: 22, borderRadius: 6, display: "grid", placeItems: "center", color: "var(--text-lo)" }}
            className="winbtn">
            {b === "min" && <div style={{ width: 9, height: 1.5, background: "currentColor" }} />}
            {b === "max" && <div style={{ width: 9, height: 9, border: "1.5px solid currentColor", borderRadius: 2 }} />}
            {b === "close" && <Sigil name="x" size={13} />}
          </span>
        ))}
      </div>
    </div>
  );
}

function Sidebar({ nav, setNav, status, sMeta, onWizard }) {
  return (
    <div style={{ width: 80, flex: "none", background: "var(--surface-1)", borderRight: "1px solid var(--line)",
      display: "flex", flexDirection: "column", alignItems: "center", padding: "14px 0", gap: 6, position: "relative", zIndex: 20 }}>
      {NAV.map((n) => {
        const on = nav === n.id;
        return (
          <button key={n.id} onClick={() => setNav(n.id)}
            style={{ width: 60, padding: "9px 0 7px", borderRadius: 12, display: "flex", flexDirection: "column",
              alignItems: "center", gap: 5, position: "relative", transition: "all .15s",
              color: on ? "var(--text-hi)" : "var(--text-lo)", background: on ? "var(--surface-3)" : "transparent" }}
            onMouseEnter={(e) => { if (!on) e.currentTarget.style.background = "var(--surface-2)"; }}
            onMouseLeave={(e) => { if (!on) e.currentTarget.style.background = "transparent"; }}>
            {on && <span style={{ position: "absolute", left: -1, top: "50%", transform: "translateY(-50%)", width: 3, height: 22, borderRadius: 3, background: "var(--grad)", boxShadow: "0 0 10px rgba(124,92,246,0.6)" }} />}
            <span style={{ color: on ? "var(--indigo)" : "inherit", filter: on ? "drop-shadow(0 0 5px rgba(124,92,246,0.5))" : "none" }}>
              <Sigil name={n.sigil} size={23} />
            </span>
            <span style={{ fontSize: 9.5, fontFamily: "var(--font-mono)", letterSpacing: "0.04em", textTransform: "uppercase", fontWeight: 700 }}>{n.label}</span>
          </button>
        );
      })}
      <div style={{ flex: 1 }} />
      <Tip label="Replay first-run setup">
        <button onClick={onWizard} className="icon-btn"><Sigil name="bolt" size={18} /></button>
      </Tip>
      <div style={{ width: 34, height: 34, borderRadius: 10, display: "grid", placeItems: "center",
        background: "var(--success-bg)", border: "1px solid rgba(52,217,160,0.25)", color: "var(--success)" }}
        title="Local-first · no telemetry">
        <Sigil name="shield" size={18} />
      </div>
    </div>
  );
}

function Stub({ title, sigil }) {
  return <div style={{ height: "100%", display: "grid", placeItems: "center" }}>
    <EmptyState icon={sigil} title={title}>Coming together…</EmptyState>
  </div>;
}

function App() {
  const [nav, setNav] = uS("mixer");
  const [presetId, setPresetId] = uS(PRESETS[0].id);
  const preset = PRESETS.find((p) => p.id === presetId);
  const [chains, setChains] = uS(() => Object.fromEntries(PRESETS.map((p) => [p.id, p.chain.map((c) => ({ ...c, vals: { ...c.vals } }))])));
  const chain = chains[presetId];
  const setChain = uC((updater) => setChains((all) => ({ ...all, [presetId]: typeof updater === "function" ? updater(all[presetId]) : updater })), [presetId]);

  const [ui, setUi] = uS({ muted: false, monitor: true, ab: "A", ptmMode: "apply", ptmKey: "Space", pressed: false });
  const [wizard, setWizard] = uS(true);
  const [tweaks, setTweaks] = uS(null);
  const [glyphs, setGlyphs] = uS({ triangle: "velvet-demon", invtriangle: "glass-oracle", square: "hollow-king", circle: "clean" });
  const castGlyph = uC((id) => { setPresetId(id); setNav("mixer"); }, []);

  const hasEnabled = chain.some((c) => c.enabled);
  const effectiveModulated = ui.ptmMode === "apply" ? ui.pressed : !ui.pressed;
  const status = ui.muted ? "muted" : (hasEnabled && effectiveModulated ? "modulated" : "clean");
  const sMeta = statusMeta(status);

  // bind Space as push-to-modulate
  uE(() => {
    const down = (e) => { if (e.code === "Space" && e.target === document.body) { e.preventDefault(); setUi((u) => u.pressed ? u : { ...u, pressed: true }); } };
    const up = (e) => { if (e.code === "Space") { e.preventDefault(); setUi((u) => ({ ...u, pressed: false })); } };
    window.addEventListener("keydown", down); window.addEventListener("keyup", up);
    return () => { window.removeEventListener("keydown", down); window.removeEventListener("keyup", up); };
  }, []);

  const motion = tweaks ? tweaks.motion : 1;
  const mystical = tweaks ? tweaks.mystical : 1;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", background: "var(--surface-0)" }}>
      <Titlebar status={status} sMeta={sMeta} />
      <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
        <Sidebar nav={nav} setNav={setNav} status={status} sMeta={sMeta} onWizard={() => setWizard(true)} />
        <div style={{ flex: 1, minWidth: 0, position: "relative", background: "var(--surface-0)", overflow: "hidden" }}>
          {nav === "mixer" && <MixerScreen preset={preset} chain={chain} setChain={setChain} status={status} statusInfo={sMeta} ui={ui} setUi={setUi} motion={motion} mystical={mystical} />}
          {nav === "soundboard" && (window.SoundboardScreen ? <SoundboardScreen /> : <Stub title="Soundboard" sigil="soundboard" />)}
          {nav === "presets" && (window.PresetsScreen ? <PresetsScreen presetId={presetId} setPresetId={setPresetId} chains={chains} setChains={setChains} onUse={(id) => { setPresetId(id); setNav("mixer"); }} /> : <Stub title="Presets" sigil="presets" />)}
          {nav === "settings" && (window.SettingsScreen ? <SettingsScreen onWizard={() => setWizard(true)} glyphs={glyphs} setGlyphs={setGlyphs} /> : <Stub title="Settings" sigil="settings" />)}
        </div>
      </div>
      {wizard && window.WizardScreen && <WizardScreen onClose={() => setWizard(false)} />}
      {window.SparkLayer && <SparkLayer bindings={glyphs} onCast={castGlyph} />}
      {window.DivoraTweaks && <DivoraTweaks onChange={setTweaks} />}
    </div>
  );
}

function mountDivora() {
  const scaleFrame = () => {
    const stage = document.getElementById("stage"), frame = document.getElementById("frame");
    if (!stage || !frame) return;
    const s = Math.min(stage.clientWidth / 1100, stage.clientHeight / 720, 1);
    frame.style.transform = `scale(${s})`;
  };
  window.addEventListener("resize", scaleFrame); scaleFrame();
  ReactDOM.createRoot(document.getElementById("frame")).render(<App />);
}
window.mountDivora = mountDivora;
