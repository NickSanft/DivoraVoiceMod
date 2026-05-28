// screen_soundboard.jsx — grid of incantation tiles
function SoundboardScreen() {
  const [q, setQ] = useState("");
  const [playing, setPlaying] = useState({}); // id -> {start, dur}
  const [, force] = useState(0);
  const [folder] = useState("Grimoire / SFX");

  // progress ticker
  useEffect(() => {
    if (!Object.keys(playing).length) return;
    let raf;
    const tick = () => {
      const now = performance.now();
      let changed = false;
      const next = { ...playing };
      for (const id in next) { if (now - next[id].start > next[id].dur * 1000) { delete next[id]; changed = true; } }
      if (changed) setPlaying(next); else { force((n) => n + 1); raf = requestAnimationFrame(tick); }
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [playing]);

  const play = (clip) => setPlaying((p) => ({ ...p, [clip.id]: { start: performance.now(), dur: clip.dur } }));
  const stopAll = () => setPlaying({});
  const prog = (id, dur) => playing[id] ? Math.min(1, (performance.now() - playing[id].start) / (dur * 1000)) : 0;

  const list = SOUNDBOARD.filter((c) => c.label.toLowerCase().includes(q.toLowerCase()));
  const anyPlaying = Object.keys(playing).length > 0;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", padding: "20px 24px" }}>
      {/* header */}
      <div style={{ display: "flex", alignItems: "center", gap: 14, marginBottom: 16 }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="eyebrow">Soundboard</div>
          <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 3 }}>
            <Sigil name="folder" size={18} style={{ color: "var(--text-lo)" }} />
            <span className="display" style={{ fontSize: 19 }}>{folder}</span>
            <button className="btn btn-ghost sm">change folder</button>
          </div>
        </div>
        <div style={{ position: "relative", width: 220 }}>
          <span style={{ position: "absolute", left: 11, top: "50%", transform: "translateY(-50%)", color: "var(--text-lo)" }}><Sigil name="search" size={16} /></span>
          <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search clips…"
            style={{ width: "100%", height: 38, paddingLeft: 34, paddingRight: 12, borderRadius: 11, background: "var(--surface-2)",
              border: "1px solid var(--line-strong)", color: "var(--text-hi)", fontSize: 13, outline: "none" }}
            onFocus={(e) => e.target.style.borderColor = "var(--indigo)"} onBlur={(e) => e.target.style.borderColor = "var(--line-strong)"} />
        </div>
        <Btn variant="danger" icon="stop" disabled={!anyPlaying} onClick={stopAll}>Stop all{anyPlaying ? ` (${Object.keys(playing).length})` : ""}</Btn>
      </div>

      {/* grid */}
      {list.length === 0 ? (
        <div style={{ flex: 1, display: "grid", placeItems: "center" }}>
          <EmptyState icon="search" title="No clips match">Nothing in this grimoire matches “{q}”. Try another word, or clear the search.</EmptyState>
        </div>
      ) : (
        <div style={{ flex: 1, overflowY: "auto", overflowX: "hidden", display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 12, alignContent: "start", paddingRight: 4, paddingBottom: 4 }}>
          {list.map((clip) => {
            const p = prog(clip.id, clip.dur);
            const on = !!playing[clip.id];
            return (
              <button key={clip.id} onClick={() => play(clip)}
                style={{
                  position: "relative", height: 120, borderRadius: 14, padding: 13,
                  display: "flex", flexDirection: "column", justifyContent: "space-between", textAlign: "left",
                  background: on ? `color-mix(in srgb, ${clip.color} 16%, var(--surface-2))` : "var(--surface-2)",
                  border: `1px solid ${on ? clip.color : "var(--line)"}`,
                  boxShadow: on ? `0 0 22px color-mix(in srgb, ${clip.color} 35%, transparent)` : "none",
                  transition: "all .18s", overflow: "hidden",
                }}
                onMouseEnter={(e) => { if (!on) { e.currentTarget.style.borderColor = "var(--line-glow)"; e.currentTarget.style.transform = "translateY(-2px)"; } }}
                onMouseLeave={(e) => { if (!on) { e.currentTarget.style.borderColor = "var(--line)"; e.currentTarget.style.transform = "none"; } }}>
                {/* progress ring */}
                {on && (
                  <svg viewBox="0 0 36 36" style={{ position: "absolute", top: 10, right: 10, width: 30, height: 30, transform: "rotate(-90deg)" }}>
                    <circle cx="18" cy="18" r="15" fill="none" stroke="rgba(255,255,255,0.12)" strokeWidth="3" />
                    <circle cx="18" cy="18" r="15" fill="none" stroke={clip.color} strokeWidth="3" strokeLinecap="round"
                      strokeDasharray={94.2} strokeDashoffset={94.2 * (1 - p)} />
                  </svg>
                )}
                {/* hotkey badge */}
                {clip.key && !on && (
                  <span style={{ position: "absolute", top: 11, right: 11 }}><Kbd>{clip.key[0]}</Kbd></span>
                )}
                <div style={{ fontSize: 30, filter: on ? "drop-shadow(0 0 8px " + clip.color + ")" : "none",
                  transform: on ? "scale(1.05)" : "none", transition: "transform .2s" }}>{clip.emoji}</div>
                <div>
                  <div style={{ fontWeight: 600, fontSize: 13.5, color: "var(--text-hi)" }}>{clip.label}</div>
                  <div style={{ display: "flex", alignItems: "center", gap: 6, marginTop: 4 }}>
                    <span style={{ width: 7, height: 7, borderRadius: "50%", background: clip.color }} />
                    <span className="mono" style={{ fontSize: 10.5, color: "var(--text-lo)" }}>
                      {on ? `${(clip.dur * (1 - p)).toFixed(1)}s` : `${clip.dur.toFixed(1)}s`}
                    </span>
                    {on && <span style={{ marginLeft: "auto", fontSize: 9.5, fontFamily: "var(--font-mono)", color: clip.color, letterSpacing: "0.08em" }}>PLAYING</span>}
                  </div>
                </div>
              </button>
            );
          })}
          {/* add tile */}
          <button style={{ height: 120, borderRadius: 14, border: "1px dashed var(--line-strong)", background: "transparent",
            display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 8, color: "var(--text-lo)", transition: "all .15s" }}
            onMouseEnter={(e) => { e.currentTarget.style.borderColor = "var(--line-glow)"; e.currentTarget.style.color = "var(--text-mid)"; }}
            onMouseLeave={(e) => { e.currentTarget.style.borderColor = "var(--line-strong)"; e.currentTarget.style.color = "var(--text-lo)"; }}>
            <Sigil name="plus" size={22} /><span style={{ fontSize: 12, fontWeight: 600 }}>Add clip</span>
          </button>
        </div>
      )}
    </div>
  );
}
window.SoundboardScreen = SoundboardScreen;
