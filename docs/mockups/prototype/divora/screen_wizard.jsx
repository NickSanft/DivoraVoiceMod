// screen_wizard.jsx — first-run ceremony (4 steps)
function WizardScreen({ onClose }) {
  const [step, setStep] = useState(0);
  const [cableOk, setCableOk] = useState(true);
  const [inDev, setInDev] = useState("blue-yeti");
  const [outDev, setOutDev] = useState("vb-cable");
  const inLvl = useLevel(true, { base: 0.3, swing: 0.55 });
  const steps = ["Welcome", "Virtual cable", "Devices", "Ready"];

  const next = () => step < 3 ? setStep(step + 1) : onClose();
  const back = () => step > 0 && setStep(step - 1);

  return (
    <div style={{ position: "absolute", inset: 0, zIndex: 100, display: "flex", background: "var(--surface-0)" }}>
      {/* LEFT ceremonial panel */}
      <div style={{ width: 372, flex: "none", position: "relative", overflow: "hidden",
        background: "radial-gradient(120% 90% at 30% 10%, rgba(79,70,229,0.28), transparent 60%), radial-gradient(110% 80% at 90% 100%, rgba(219,39,119,0.22), transparent 55%), var(--surface-1)",
        borderRight: "1px solid var(--line)", display: "flex", flexDirection: "column", padding: 32 }}>
        {/* faint ring */}
        <svg viewBox="0 0 400 400" style={{ position: "absolute", left: "50%", top: "46%", width: 460, height: 460, transform: "translate(-50%,-50%)", opacity: 0.5 }}>
          <g style={{ transformBox: "fill-box", transformOrigin: "center", animation: "spin-slow 120s linear infinite" }}>
            <circle cx="200" cy="200" r="150" fill="none" stroke="rgba(168,150,220,0.25)" strokeWidth="1" />
            <circle cx="200" cy="200" r="150" fill="none" stroke="rgba(168,150,220,0.4)" strokeWidth="1.4" strokeDasharray="1 14" strokeLinecap="round" />
            <circle cx="200" cy="200" r="120" fill="none" stroke="rgba(124,92,246,0.3)" strokeWidth="1" strokeDasharray="3 9" />
          </g>
        </svg>
        <div style={{ position: "relative", zIndex: 2, display: "flex", alignItems: "center", gap: 11 }}>
          <DMark size={30} radius={8} />
          <span className="display" style={{ fontSize: 18 }}>DivoraVoice</span>
        </div>
        <div style={{ flex: 1, display: "grid", placeItems: "center", position: "relative", zIndex: 2 }}>
          <div style={{ width: 132, height: 132, borderRadius: "50%", display: "grid", placeItems: "center",
            background: "radial-gradient(circle, rgba(124,92,246,0.35), transparent 70%)",
            animation: "breathe 3.6s ease-in-out infinite" }}>
            <div style={{ width: 92, height: 92, borderRadius: "50%", background: "var(--surface-0)", border: "1.5px solid var(--line-glow)",
              display: "grid", placeItems: "center", color: "#fff", boxShadow: "0 0 40px rgba(124,92,246,0.5)" }}>
              <Sigil name={["clean", "output", "mic", "modulated"][step]} size={42} />
            </div>
          </div>
        </div>
        <div style={{ position: "relative", zIndex: 2, display: "flex", flexDirection: "column", gap: 7 }}>
          {steps.map((s, i) => (
            <div key={s} style={{ display: "flex", alignItems: "center", gap: 10, opacity: i === step ? 1 : 0.5 }}>
              <span style={{ width: 22, height: 22, borderRadius: "50%", flex: "none", display: "grid", placeItems: "center", fontSize: 11, fontWeight: 700,
                fontFamily: "var(--font-mono)", background: i < step ? "var(--success)" : i === step ? "var(--grad)" : "var(--surface-3)",
                color: i <= step ? "#fff" : "var(--text-lo)", border: i === step ? "none" : "1px solid var(--line)" }}>
                {i < step ? <Sigil name="check" size={13} /> : i + 1}
              </span>
              <span style={{ fontSize: 12.5, fontWeight: i === step ? 600 : 500, color: i === step ? "var(--text-hi)" : "var(--text-lo)" }}>{s}</span>
            </div>
          ))}
        </div>
      </div>

      {/* RIGHT content */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}>
        <div style={{ flex: 1, overflowY: "auto", overflowX: "hidden", padding: "44px 48px", display: "flex", flexDirection: "column", justifyContent: "center" }}>
          {step === 0 && (
            <div>
              <div className="eyebrow" style={{ marginBottom: 12 }}>First run · no account needed</div>
              <h1 className="display" style={{ fontSize: 34, lineHeight: 1.08, marginBottom: 14 }}>Your voice,<br />transmuted in real time.</h1>
              <p style={{ fontSize: 14, color: "var(--text-mid)", lineHeight: 1.6, maxWidth: 440, marginBottom: 26 }}>
                DivoraVoice bends your microphone through layered spells — pitch, formant, reverb and more — then routes the result into Discord, Zoom, OBS or any game. Three quick steps and you’re ready.
              </p>
              <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12, maxWidth: 460 }}>
                {[{ i: "shield", t: "Local-first", s: "All DSP runs on your machine." }, { i: "lock", t: "Private", s: "No telemetry. No tracking. Ever." }, { i: "bolt", t: "Free", s: "Open source under MIT." }, { i: "mixer", t: "Real-time", s: "Sub-20ms processing." }].map((x) => (
                  <div key={x.t} className="card" style={{ padding: 14, display: "flex", gap: 11, alignItems: "flex-start" }}>
                    <span style={{ color: "var(--indigo)", flex: "none" }}><Sigil name={x.i} size={19} /></span>
                    <div><div style={{ fontSize: 13, fontWeight: 600 }}>{x.t}</div><div style={{ fontSize: 11, color: "var(--text-lo)", marginTop: 2, lineHeight: 1.4 }}>{x.s}</div></div>
                  </div>
                ))}
              </div>
            </div>
          )}

          {step === 1 && (
            <div style={{ maxWidth: 480 }}>
              <div className="eyebrow" style={{ marginBottom: 12 }}>Step 2 · the bridge</div>
              <h1 className="display" style={{ fontSize: 28, marginBottom: 12 }}>A virtual cable carries your voice</h1>
              <p style={{ fontSize: 13.5, color: "var(--text-mid)", lineHeight: 1.6, marginBottom: 22 }}>
                Apps can’t read DivoraVoice directly. <strong style={{ color: "var(--text-hi)" }}>VB-Cable</strong> is a free virtual audio device that acts as the microphone other apps see — DivoraVoice pours your modulated voice into it.
              </p>
              <div className="card" style={{ padding: 16, display: "flex", alignItems: "center", gap: 14,
                borderColor: cableOk ? "rgba(52,217,160,0.3)" : "rgba(233,177,76,0.3)", background: cableOk ? "var(--success-bg)" : "var(--warning-bg)" }}>
                <span style={{ color: cableOk ? "var(--success)" : "var(--warning)" }}><Sigil name={cableOk ? "check" : "warning"} size={26} /></span>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 14, fontWeight: 600 }}>{cableOk ? "VB-Cable is installed" : "VB-Cable not found"}</div>
                  <div style={{ fontSize: 11.5, color: "var(--text-lo)", marginTop: 2 }}>{cableOk ? "Detected v1.0.3.8 — you’re good to go." : "Free download, ~1 MB. Install, then restart Divora."}</div>
                </div>
                {!cableOk && <Btn variant="primary" size="sm" iconR="external">Download</Btn>}
              </div>
              <button onClick={() => setCableOk(!cableOk)} style={{ fontSize: 10.5, color: "var(--text-dim)", fontFamily: "var(--font-mono)", marginTop: 12 }}>
                ⌁ preview {cableOk ? "missing" : "detected"} state
              </button>
            </div>
          )}

          {step === 2 && (
            <div style={{ maxWidth: 480 }}>
              <div className="eyebrow" style={{ marginBottom: 12 }}>Step 3 · channels</div>
              <h1 className="display" style={{ fontSize: 28, marginBottom: 20 }}>Pick your devices</h1>
              <label className="field-label">Microphone in</label>
              <Select icon="mic" value={inDev} options={DEVICES_IN} onChange={setInDev} />
              <div style={{ margin: "14px 0 4px", display: "flex", alignItems: "center", gap: 10 }}>
                <span className="eyebrow" style={{ flex: "none" }}>Hearing you</span>
                <div style={{ flex: 1 }}><HMeter level={inLvl.lvl} peak={inLvl.peak} /></div>
              </div>
              <label className="field-label" style={{ marginTop: 18 }}>Modulated out</label>
              <Select icon="output" value={outDev} options={DEVICES_OUT} onChange={setOutDev} />
              <div style={{ marginTop: 12, fontSize: 11.5, color: "var(--text-mid)", display: "flex", gap: 8, alignItems: "center" }}>
                <Sigil name="info" size={15} style={{ color: "var(--info)", flex: "none" }} /> Send to <span className="kbd" style={{ height: 18, fontSize: 10 }}>CABLE Input</span> so other apps can pick it up.
              </div>
            </div>
          )}

          {step === 3 && (
            <div style={{ maxWidth: 480 }}>
              <div style={{ width: 56, height: 56, borderRadius: "50%", display: "grid", placeItems: "center", marginBottom: 18,
                background: "var(--success-bg)", color: "var(--success)", border: "1px solid rgba(52,217,160,0.3)" }}>
                <Sigil name="check" size={30} />
              </div>
              <h1 className="display" style={{ fontSize: 30, marginBottom: 12 }}>You’re ready.</h1>
              <p style={{ fontSize: 13.5, color: "var(--text-mid)", lineHeight: 1.6, marginBottom: 22 }}>
                Pick a spell from the Mixer and hold <span className="kbd" style={{ height: 18, fontSize: 10 }}>Space</span> to modulate. One last thing — point your chat app at DivoraVoice:
              </p>
              <div className="card" style={{ padding: 18, background: "var(--surface-1)" }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 14 }}>
                  <Sigil name="output" size={17} style={{ color: "var(--indigo)" }} />
                  <span style={{ fontSize: 13, fontWeight: 600 }}>Route into Discord</span>
                </div>
                {["Open Discord → User Settings → Voice & Video", "Under Input Device, choose CABLE Output", "Speak — your modulated voice now travels through"].map((s, i) => (
                  <div key={i} style={{ display: "flex", gap: 11, alignItems: "flex-start", marginBottom: i < 2 ? 11 : 0 }}>
                    <span style={{ width: 20, height: 20, borderRadius: "50%", flex: "none", display: "grid", placeItems: "center", fontSize: 11, fontWeight: 700,
                      fontFamily: "var(--font-mono)", background: "var(--accent-bg)", color: "var(--indigo)", border: "1px solid var(--line-glow)" }}>{i + 1}</span>
                    <span style={{ fontSize: 12.5, color: "var(--text-mid)", lineHeight: 1.45, paddingTop: 1 }}>{s}</span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>

        {/* footer nav */}
        <div style={{ flex: "none", padding: "16px 48px", borderTop: "1px solid var(--line)", display: "flex", alignItems: "center", gap: 12 }}>
          <button onClick={onClose} style={{ fontSize: 12, color: "var(--text-lo)", fontWeight: 600 }}>Skip setup</button>
          <div style={{ flex: 1 }} />
          {step > 0 && <Btn variant="ghost" icon="chevronL" onClick={back}>Back</Btn>}
          <Btn variant="primary" iconR={step < 3 ? "chevronR" : "bolt"} onClick={next}>{step < 3 ? "Continue" : "Enter Divora"}</Btn>
        </div>
      </div>
    </div>
  );
}
window.WizardScreen = WizardScreen;
