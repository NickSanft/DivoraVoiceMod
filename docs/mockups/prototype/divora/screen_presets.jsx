// screen_presets.jsx — browse + chain editor
function PresetsScreen({ presetId, setPresetId, chains, setChains, onUse }) {
  const [sel, setSel] = useState(presetId);
  const preset = PRESETS.find((p) => p.id === sel);
  const chain = chains[sel];
  const [dragId, setDragId] = useState(null);
  const [overId, setOverId] = useState(null);
  const [showJSON, setShowJSON] = useState(false);
  const [addOpen, setAddOpen] = useState(false);

  const setChain = (updater) => setChains((all) => ({ ...all, [sel]: typeof updater === "function" ? updater(all[sel]) : updater }));
  const toggle = (id) => setChain((c) => c.map((e) => e.id === id ? { ...e, enabled: !e.enabled } : e));
  const setParam = (id, key, v) => setChain((c) => c.map((e) => e.id === id ? { ...e, vals: { ...e.vals, [key]: v } } : e));
  const remove = (id) => setChain((c) => c.filter((e) => e.id !== id));
  const add = (id) => { setChain((c) => [...c, fx(id, true)]); setAddOpen(false); };

  const onDrop = (targetId) => {
    if (!dragId || dragId === targetId) { setDragId(null); setOverId(null); return; }
    setChain((c) => {
      const arr = [...c];
      const from = arr.findIndex((e) => e.id === dragId);
      const to = arr.findIndex((e) => e.id === targetId);
      const [m] = arr.splice(from, 1); arr.splice(to, 0, m); return arr;
    });
    setDragId(null); setOverId(null);
  };

  const available = EFFECT_ORDER.filter((id) => !chain.find((c) => c.id === id));
  const grouped = { Bundled: PRESETS.filter((p) => p.tag === "Bundled"), User: PRESETS.filter((p) => p.tag === "User") };

  return (
    <div style={{ display: "flex", height: "100%" }}>
      {/* LEFT list */}
      <div style={{ width: 248, flex: "none", borderRight: "1px solid var(--line)", display: "flex", flexDirection: "column", background: "var(--surface-1)" }}>
        <div style={{ padding: "18px 16px 12px", display: "flex", alignItems: "center", justifyContent: "space-between" }}>
          <span className="display" style={{ fontSize: 18 }}>Presets</span>
          <Tip label="New preset"><IconBtn icon="plus" size={18} /></Tip>
        </div>
        <div style={{ flex: 1, overflowY: "auto", overflowX: "hidden", padding: "0 10px 12px" }}>
          {Object.entries(grouped).map(([group, items]) => (
            <div key={group} style={{ marginBottom: 10 }}>
              <div className="eyebrow" style={{ padding: "8px 8px 6px" }}>{group} · {items.length}</div>
              {items.map((p) => {
                const on = sel === p.id;
                const active = p.id === presetId;
                return (
                  <button key={p.id} onClick={() => setSel(p.id)}
                    style={{ width: "100%", display: "flex", alignItems: "center", gap: 11, padding: "9px 10px", borderRadius: 10, textAlign: "left",
                      background: on ? "var(--surface-3)" : "transparent", border: on ? "1px solid var(--line-glow)" : "1px solid transparent", marginBottom: 2, transition: "all .12s" }}
                    onMouseEnter={(e) => { if (!on) e.currentTarget.style.background = "var(--surface-2)"; }}
                    onMouseLeave={(e) => { if (!on) e.currentTarget.style.background = "transparent"; }}>
                    <span style={{ width: 30, height: 30, borderRadius: 8, flex: "none", display: "grid", placeItems: "center",
                      background: `color-mix(in srgb, ${p.color} 16%, var(--surface-1))`, color: p.color, border: `1px solid color-mix(in srgb, ${p.color} 30%, transparent)` }}>
                      <Sigil name={p.glyph} size={16} />
                    </span>
                    <span style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ fontSize: 13, fontWeight: 600, color: on ? "var(--text-hi)" : "var(--text-mid)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{p.name}</div>
                      <div className="mono" style={{ fontSize: 10, color: "var(--text-lo)" }}>{p.chain.length} runes</div>
                    </span>
                    {active && <span style={{ width: 7, height: 7, borderRadius: "50%", background: "var(--success)", boxShadow: "0 0 7px var(--success)", flex: "none" }} title="In use" />}
                  </button>
                );
              })}
            </div>
          ))}
        </div>
      </div>

      {/* RIGHT editor */}
      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
        <div style={{ padding: "18px 22px 14px", borderBottom: "1px solid var(--line)", display: "flex", alignItems: "flex-start", gap: 14 }}>
          <span style={{ width: 44, height: 44, borderRadius: 12, flex: "none", display: "grid", placeItems: "center",
            background: `color-mix(in srgb, ${preset.color} 18%, var(--surface-1))`, color: preset.color, border: `1px solid color-mix(in srgb, ${preset.color} 32%, transparent)` }}>
            <Sigil name={preset.glyph} size={24} />
          </span>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <h1 className="display" style={{ fontSize: 22, whiteSpace: "nowrap" }}>{preset.name}</h1>
              <Badge tone={preset.tag === "User" ? "" : "accent"}>{preset.tag}</Badge>
              {preset.id === presetId && <Badge tone="success" icon="check">In use</Badge>}
            </div>
            <p style={{ fontSize: 12.5, color: "var(--text-lo)", marginTop: 4, maxWidth: 520, lineHeight: 1.45 }}>{preset.desc}</p>
          </div>
          <div style={{ display: "flex", gap: 8, flex: "none" }}>
            <Btn variant="primary" icon="bolt" onClick={() => onUse(sel)}>Use</Btn>
          </div>
        </div>

        {/* action row */}
        <div style={{ padding: "12px 22px", display: "flex", alignItems: "center", gap: 8, borderBottom: "1px solid var(--line)" }}>
          <span className="eyebrow" style={{ flex: 1 }}>Effect chain · {chain.filter(c=>c.enabled).length}/{chain.length} active · drag to reorder</span>
          <Btn variant="ghost" size="sm" icon="copy">Duplicate</Btn>
          <Btn variant="ghost" size="sm" icon="download" onClick={() => setShowJSON(true)}>Export JSON</Btn>
          <Btn variant="ghost" size="sm" icon="check">Save as…</Btn>
          <Btn variant="danger" size="sm" icon="trash">Delete</Btn>
        </div>

        {/* chain */}
        <div style={{ flex: 1, overflowY: "auto", overflowX: "hidden", padding: "16px 22px 22px", display: "flex", flexDirection: "column", gap: 10 }}>
          {chain.map((c) => {
            const eff = EFFECTS[c.id];
            const isOver = overId === c.id && dragId !== c.id;
            return (
              <div key={c.id} draggable onDragStart={() => setDragId(c.id)} onDragEnd={() => { setDragId(null); setOverId(null); }}
                onDragOver={(e) => { e.preventDefault(); setOverId(c.id); }} onDrop={() => onDrop(c.id)}
                className="card" style={{ padding: 14, opacity: dragId === c.id ? 0.4 : 1,
                  borderColor: isOver ? "var(--indigo)" : c.enabled ? "var(--line-strong)" : "var(--line)",
                  boxShadow: isOver ? "0 0 0 3px rgba(124,92,246,0.2)" : "none", transition: "border-color .12s, box-shadow .12s" }}>
                <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                  <span style={{ color: "var(--text-dim)", cursor: "grab" }}><Sigil name="drag" size={18} /></span>
                  <span style={{ width: 34, height: 34, borderRadius: 9, flex: "none", display: "grid", placeItems: "center",
                    background: c.enabled ? "var(--accent-bg)" : "var(--surface-1)", color: c.enabled ? "var(--indigo)" : "var(--text-lo)",
                    border: `1px solid ${c.enabled ? "var(--line-glow)" : "var(--line)"}` }}>
                    <Sigil name={eff.sigil} size={19} />
                  </span>
                  <div style={{ flex: 1 }}>
                    <div style={{ fontSize: 14, fontWeight: 600, color: c.enabled ? "var(--text-hi)" : "var(--text-lo)" }}>{eff.name}</div>
                    <div className="mono" style={{ fontSize: 10.5, color: "var(--text-lo)" }}>{eff.readout(c.vals)}</div>
                  </div>
                  <Toggle on={c.enabled} onChange={() => toggle(c.id)} />
                  <Tip label="Remove rune"><IconBtn icon="x" size={16} onClick={() => remove(c.id)} /></Tip>
                </div>
                {c.enabled && (
                  <div style={{ display: "grid", gridTemplateColumns: eff.params.length > 1 ? "1fr 1fr" : "1fr", gap: "12px 22px", marginTop: 14, paddingTop: 13, borderTop: "1px solid var(--line)" }}>
                    {eff.params.map((p) => (
                      <div key={p.key}>
                        <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 6 }}>
                          <span style={{ fontSize: 11.5, color: "var(--text-mid)" }}>{p.label}</span>
                          <span className="mono" style={{ fontSize: 11.5, color: "var(--text-hi)" }}>{c.vals[p.key] > 0 && p.bipolar ? "+" : ""}{c.vals[p.key]}{p.unit}</span>
                        </div>
                        <Slider value={c.vals[p.key]} min={p.min} max={p.max} step={p.step} bipolar={p.bipolar} onChange={(v) => setParam(c.id, p.key, v)} />
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })}

          {/* add effect */}
          <div style={{ position: "relative" }}>
            <button onClick={() => setAddOpen(!addOpen)} disabled={!available.length}
              style={{ width: "100%", padding: "13px", borderRadius: 11, border: "1px dashed var(--line-strong)", background: "transparent",
                display: "flex", alignItems: "center", justifyContent: "center", gap: 8, color: available.length ? "var(--text-mid)" : "var(--text-dim)",
                fontWeight: 600, fontSize: 13, transition: "all .14s", cursor: available.length ? "pointer" : "not-allowed" }}
              onMouseEnter={(e) => { if (available.length) e.currentTarget.style.borderColor = "var(--line-glow)"; }}
              onMouseLeave={(e) => e.currentTarget.style.borderColor = "var(--line-strong)"}>
              <Sigil name="plus" size={17} /> {available.length ? "Add rune to chain" : "All runes in chain"}
            </button>
            {addOpen && available.length > 0 && (
              <div className="dropdown" style={{ position: "absolute", bottom: "calc(100% + 6px)", left: 0, right: 0, zIndex: 40, display: "grid", gridTemplateColumns: "1fr 1fr", gap: 4 }}>
                {available.map((id) => (
                  <div key={id} className="opt" onClick={() => add(id)}>
                    <Sigil name={EFFECTS[id].sigil} size={17} style={{ color: "var(--indigo)" }} />
                    <div style={{ flex: 1 }}><div style={{ fontSize: 13, fontWeight: 600 }}>{EFFECTS[id].name}</div></div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* JSON export modal */}
      {showJSON && (
        <div onClick={() => setShowJSON(false)} style={{ position: "absolute", inset: 0, background: "rgba(7,6,13,0.7)", backdropFilter: "blur(4px)", display: "grid", placeItems: "center", zIndex: 90 }}>
          <div onClick={(e) => e.stopPropagation()} className="panel" style={{ width: 460, maxHeight: 480, display: "flex", flexDirection: "column", background: "var(--surface-1)", boxShadow: "var(--shadow-3)" }}>
            <div style={{ padding: "16px 18px", borderBottom: "1px solid var(--line)", display: "flex", alignItems: "center", gap: 10 }}>
              <Sigil name="download" size={18} style={{ color: "var(--indigo)" }} />
              <span className="display" style={{ fontSize: 16, flex: 1 }}>Export · {preset.name}</span>
              <IconBtn icon="x" size={16} onClick={() => setShowJSON(false)} />
            </div>
            <pre className="mono" style={{ flex: 1, overflow: "auto", margin: 0, padding: 18, fontSize: 11.5, color: "var(--text-mid)", lineHeight: 1.6 }}>
{JSON.stringify({ name: preset.name, version: 1, chain: chain.map((c) => ({ effect: c.id, on: c.enabled, ...c.vals })) }, null, 2)}
            </pre>
            <div style={{ padding: 14, borderTop: "1px solid var(--line)", display: "flex", gap: 8, justifyContent: "flex-end" }}>
              <Btn variant="secondary" icon="copy">Copy</Btn>
              <Btn variant="primary" icon="download">Save .json</Btn>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
window.PresetsScreen = PresetsScreen;
