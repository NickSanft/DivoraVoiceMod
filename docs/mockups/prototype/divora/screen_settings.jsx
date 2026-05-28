// screen_settings.jsx — devices, virtual mic, hotkeys, appearance, about
function SettingsSection({ icon, title, desc, children }) {
  return (
    <div style={{ marginBottom: 26 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 14 }}>
        <span style={{ color: "var(--indigo)" }}><Sigil name={icon} size={19} /></span>
        <div>
          <div className="display" style={{ fontSize: 16 }}>{title}</div>
          {desc && <div style={{ fontSize: 11.5, color: "var(--text-lo)", marginTop: 1 }}>{desc}</div>}
        </div>
      </div>
      <div className="panel" style={{ padding: 18 }}>{children}</div>
    </div>
  );
}
function Row({ label, sub, children, last }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 16, padding: "12px 0", borderBottom: last ? "none" : "1px solid var(--line)" }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: "var(--text-hi)" }}>{label}</div>
        {sub && <div style={{ fontSize: 11.5, color: "var(--text-lo)", marginTop: 2, lineHeight: 1.4 }}>{sub}</div>}
      </div>
      <div style={{ flex: "none" }}>{children}</div>
    </div>
  );
}

function ShapeGlyph({ type, size = 26, color = "currentColor" }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" style={{ display: "block" }} fill="none" stroke={color} strokeWidth="1.6" strokeLinejoin="round">
      {type === "triangle" && <path d="M12 4 L20 19 L4 19 Z" />}
      {type === "invtriangle" && <path d="M4 5 L20 5 L12 20 Z" />}
      {type === "square" && <rect x="5" y="5" width="14" height="14" rx="2.5" />}
      {type === "circle" && <circle cx="12" cy="12" r="8" />}
    </svg>
  );
}
const GLYPH_DEFS = [
  { key: "triangle", name: "Triangle", sub: "apex up" },
  { key: "invtriangle", name: "Inverted triangle", sub: "apex down" },
  { key: "square", name: "Square", sub: "four corners" },
  { key: "circle", name: "Circle", sub: "closed loop" },
];

function SettingsScreen({ onWizard, glyphs = {}, setGlyphs }) {
  const [inDev, setInDev] = useState("blue-yeti");
  const [outDev, setOutDev] = useState("vb-cable");
  const [cableOk, setCableOk] = useState(true);
  const [scanning, setScanning] = useState(false);
  const [grain, setGrain] = useState(false);
  const inLvl = useLevel(true, { base: 0.3, swing: 0.55 });
  const [keys, setKeys] = useState({ ptm: ["Space"], panic: ["Ctrl", "Shift", "X"], monitor: ["Ctrl", "M"] });
  const [theme, setTheme] = useState("dark");

  const rescan = () => { setScanning(true); setTimeout(() => { setScanning(false); setCableOk(true); }, 1400); };
  useEffect(() => {
    const f = document.getElementById("frame");
    if (f) f.classList.toggle("grain", grain);
  }, [grain]);

  return (
    <div style={{ height: "100%", overflowY: "auto", overflowX: "hidden", padding: "22px 0" }}>
      <div style={{ maxWidth: 680, margin: "0 auto", padding: "0 24px" }}>
        <h1 className="display" style={{ fontSize: 24, marginBottom: 4 }}>Settings</h1>
        <p style={{ fontSize: 12.5, color: "var(--text-lo)", marginBottom: 24 }}>Everything lives on this machine. Nothing leaves it.</p>

        {/* Devices */}
        <SettingsSection icon="mic" title="Audio devices" desc="Where DivoraVoice listens, and where it sends the result.">
          <Row label="Input device" sub="Your real microphone">
            <div style={{ width: 300 }}><Select icon="mic" value={inDev} options={DEVICES_IN} onChange={setInDev} /></div>
          </Row>
          <Row label="Input confirmation" sub="Speak — you should see this move.">
            <div style={{ width: 300 }}><HMeter level={inLvl.lvl} peak={inLvl.peak} /></div>
          </Row>
          <Row label="Output device" sub="Send modulated voice here — usually the virtual cable" last>
            <div style={{ width: 300 }}><Select icon="output" value={outDev} options={DEVICES_OUT} onChange={setOutDev} /></div>
          </Row>
        </SettingsSection>

        {/* Virtual mic */}
        <SettingsSection icon="output" title="Virtual microphone" desc="The bridge that carries your voice into Discord, Zoom, OBS & games.">
          <div style={{ display: "flex", alignItems: "center", gap: 14, padding: "4px 0 14px", borderBottom: "1px solid var(--line)" }}>
            <span style={{ width: 42, height: 42, borderRadius: 11, display: "grid", placeItems: "center", flex: "none",
              background: cableOk ? "var(--success-bg)" : "var(--warning-bg)", color: cableOk ? "var(--success)" : "var(--warning)",
              border: `1px solid ${cableOk ? "rgba(52,217,160,0.3)" : "rgba(233,177,76,0.3)"}` }}>
              <Sigil name={cableOk ? "check" : "warning"} size={22} />
            </span>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 14, fontWeight: 600 }}>{cableOk ? "VB-Cable detected" : "VB-Cable not found"}</div>
              <div style={{ fontSize: 11.5, color: "var(--text-lo)", marginTop: 2 }}>
                {cableOk ? "CABLE Input / CABLE Output · v1.0.3.8 · ready to route" : "Install the free virtual audio cable to route DivoraVoice into other apps."}
              </div>
            </div>
            {cableOk
              ? <Btn variant="secondary" size="sm" icon={scanning ? null : "refresh"} onClick={rescan}>{scanning ? <><span className="spinner" />Scanning</> : "Re-scan"}</Btn>
              : <Btn variant="primary" size="sm" iconR="external">Download</Btn>}
          </div>
          <button onClick={() => setCableOk(!cableOk)} style={{ fontSize: 10.5, color: "var(--text-dim)", fontFamily: "var(--font-mono)", padding: "10px 0 4px" }}>
            ⌁ preview {cableOk ? "missing" : "detected"} state
          </button>
          <div style={{ marginTop: 6 }}>
            <div className="eyebrow" style={{ marginBottom: 10 }}>Set your virtual mic inside each app</div>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 10 }}>
              {[{ a: "Discord", s: "Settings → Voice → Input" }, { a: "Zoom", s: "Audio → Microphone" }, { a: "OBS", s: "Sources → Audio Input" }].map((app) => (
                <div key={app.a} className="card" style={{ padding: 0, overflow: "hidden" }}>
                  <div style={{ height: 78, background: "linear-gradient(135deg, var(--surface-3), var(--surface-1))", display: "grid", placeItems: "center", position: "relative", borderBottom: "1px solid var(--line)" }}>
                    <div style={{ color: "var(--text-dim)" }}>
                      <div style={{ textAlign: "center" }}>
                        <div style={{ width: 26, height: 18, border: "1px solid var(--line-strong)", borderRadius: 3, margin: "0 auto 6px", position: "relative" }}>
                          <div style={{ position: "absolute", left: 3, top: 3, right: 9, height: 2, background: "var(--line-strong)", borderRadius: 2 }} />
                          <div style={{ position: "absolute", left: 3, top: 8, right: 6, height: 2, background: "var(--line-strong)", borderRadius: 2 }} />
                        </div>
                        <span style={{ fontSize: 9, fontFamily: "var(--font-mono)" }}>screenshot</span>
                      </div>
                    </div>
                  </div>
                  <div style={{ padding: "9px 11px" }}>
                    <div style={{ fontSize: 12.5, fontWeight: 600 }}>{app.a}</div>
                    <div style={{ fontSize: 10.5, color: "var(--text-lo)", marginTop: 1 }}>{app.s}</div>
                  </div>
                </div>
              ))}
            </div>
            <div style={{ marginTop: 10, fontSize: 11.5, color: "var(--text-mid)", display: "flex", alignItems: "center", gap: 8 }}>
              <Sigil name="info" size={15} style={{ color: "var(--info)", flex: "none" }} />
              In each app, choose <span className="kbd" style={{ height: 18, fontSize: 10 }}>CABLE Output</span> as the microphone.
            </div>
          </div>
        </SettingsSection>

        {/* Hotkeys */}
        <SettingsSection icon="keyboard" title="Hotkeys" desc="Global shortcuts — work even when DivoraVoice is in the background.">
          <Row label="Push to modulate" sub="Hold to apply (or bypass) the active spell">
            <HotkeyCapture value={keys.ptm} onChange={(v) => setKeys((k) => ({ ...k, ptm: v }))} />
          </Row>
          <Row label="Panic — stop everything" sub="Instantly mute output and stop all clips">
            <HotkeyCapture value={keys.panic} onChange={(v) => setKeys((k) => ({ ...k, panic: v }))} />
          </Row>
          <Row label="Toggle monitor" sub="Hear yourself on / off" last>
            <HotkeyCapture value={keys.monitor} onChange={(v) => setKeys((k) => ({ ...k, monitor: v }))} />
          </Row>
        </SettingsSection>

        {/* Glyph casting */}
        <SettingsSection icon="bolt" title="Glyph casting" desc="Draw a shape anywhere on the background — it conjures the bound preset and switches to it.">
          {GLYPH_DEFS.map((g, i) => {
            const preset = PRESETS.find((p) => p.id === glyphs[g.key]);
            const opts = PRESETS.map((p) => ({ value: p.id, label: p.name, sub: p.tag }));
            return (
              <div key={g.key} style={{ display: "flex", alignItems: "center", gap: 14, padding: "12px 0", borderBottom: i === GLYPH_DEFS.length - 1 ? "none" : "1px solid var(--line)" }}>
                <span style={{ width: 42, height: 42, flex: "none", borderRadius: 11, display: "grid", placeItems: "center",
                  background: "var(--surface-2)", border: "1px solid var(--line-strong)", color: preset ? preset.color : "var(--text-mid)" }}>
                  <ShapeGlyph type={g.key} size={24} />
                </span>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 13, fontWeight: 600 }}>{g.name}</div>
                  <div style={{ fontSize: 11, color: "var(--text-lo)", marginTop: 1 }}>{g.sub} · casts</div>
                </div>
                <div style={{ width: 260, flex: "none" }}>
                  <Select icon={preset ? preset.glyph : "presets"} value={glyphs[g.key]} options={opts} onChange={(v) => setGlyphs((s) => ({ ...s, [g.key]: v }))} />
                </div>
              </div>
            );
          })}
          <div style={{ marginTop: 12, fontSize: 11.5, color: "var(--text-mid)", display: "flex", alignItems: "center", gap: 8 }}>
            <Sigil name="info" size={15} style={{ color: "var(--info)", flex: "none" }} />
            Hold left-click on any empty area and trace the shape. Release to cast.
          </div>
        </SettingsSection>

        {/* Appearance */}
        <SettingsSection icon="eye" title="Appearance">
          <Row label="Theme" sub="Dark is home. Light is on its way.">
            <Segmented options={[{ value: "dark", label: "Dark" }, { value: "light", label: "Light ·soon" }]} value={theme} onChange={(v) => v === "dark" && setTheme(v)} />
          </Row>
          <Row label="Parchment grain" sub="Subtle arcane texture over the whole window" last>
            <Toggle on={grain} onChange={setGrain} />
          </Row>
        </SettingsSection>

        {/* About */}
        <SettingsSection icon="info" title="About DivoraVoice">
          <div style={{ display: "flex", alignItems: "center", gap: 14, paddingBottom: 14, borderBottom: "1px solid var(--line)" }}>
            <DMark size={42} radius={11} />
            <div style={{ flex: 1 }}>
              <div className="display" style={{ fontSize: 17 }}>DivoraVoice <span style={{ color: "var(--text-lo)", fontWeight: 400 }}>v0.9.2</span></div>
              <div style={{ fontSize: 11.5, color: "var(--text-lo)" }}>Real-time voice modulator · MIT License · Tauri + SolidJS</div>
            </div>
            <Btn variant="secondary" size="sm" icon="github" iconR="external">GitHub</Btn>
          </div>
          <div style={{ display: "flex", gap: 10, marginTop: 14 }}>
            {[{ i: "shield", t: "No telemetry", s: "We never measure you." }, { i: "lock", t: "No account", s: "Nothing to sign into." }, { i: "bolt", t: "Free forever", s: "Open source, MIT." }].map((x) => (
              <div key={x.t} className="card" style={{ flex: 1, padding: 13, display: "flex", gap: 10, alignItems: "flex-start" }}>
                <span style={{ color: "var(--success)", flex: "none" }}><Sigil name={x.i} size={18} /></span>
                <div><div style={{ fontSize: 12.5, fontWeight: 600 }}>{x.t}</div><div style={{ fontSize: 10.5, color: "var(--text-lo)", marginTop: 1 }}>{x.s}</div></div>
              </div>
            ))}
          </div>
          <div style={{ marginTop: 14, display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontSize: 11.5, color: "var(--text-lo)" }}>Want to see the welcome flow again?</span>
            <Btn variant="ghost" size="sm" icon="bolt" onClick={onWizard}>Replay setup</Btn>
          </div>
        </SettingsSection>
      </div>
    </div>
  );
}
window.SettingsScreen = SettingsScreen;
