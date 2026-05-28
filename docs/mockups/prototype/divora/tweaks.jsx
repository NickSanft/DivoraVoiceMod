// tweaks.jsx — visual-style variations (the user's requested exploration axis)
const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "mood": "dusk",
  "mystical": "balanced",
  "motion": "rich",
  "accent": "brand",
  "grain": false,
  "vignette": true
}/*EDITMODE-END*/;

const MOOD_LABELS = { dusk: "Dusk Violet", ink: "Ink + Candle", midnight: "Midnight" };
const MYST = { subtle: 0.3, balanced: 0.7, rich: 1 };
const MOT = { functional: 0, ambient: 0.6, rich: 1 };
const ACCENTS = {
  brand: "linear-gradient(120deg,#4F46E5 0%,#7C5CF6 42%,#DB2777 100%)",
  abyssal: "linear-gradient(120deg,#4F46E5 0%,#5B7CF0 50%,#58C6F2 100%)",
  ember: "linear-gradient(120deg,#7C5CF6 0%,#E9B14C 60%,#F2567A 100%)",
};

function DivoraTweaks({ onChange }) {
  const [t, setTweak] = useTweaks(TWEAK_DEFAULTS);

  // report derived numeric knobs up to the app
  React.useEffect(() => {
    onChange && onChange({ mystical: MYST[t.mystical] ?? 0.7, motion: MOT[t.motion] ?? 1 });
  }, [t.mystical, t.motion]);

  // apply DOM-level visual changes
  React.useEffect(() => {
    const root = document.documentElement;
    root.dataset.mood = t.mood;
    root.style.setProperty("--motion", String(MOT[t.motion] ?? 1));
    root.style.setProperty("--grad", ACCENTS[t.accent] || ACCENTS.brand);
    const f = document.getElementById("frame");
    if (f) { f.classList.toggle("grain", t.grain); f.classList.toggle("vignette", t.vignette); }
  }, [t.mood, t.motion, t.accent, t.grain, t.vignette]);

  return (
    <TweaksPanel title="Tweaks">
      <TweakSection label="Atmosphere" />
      <TweakRadio label="Mystical level" value={t.mystical} options={["subtle", "balanced", "rich"]} onChange={(v) => setTweak("mystical", v)} />
      <TweakRadio label="Motion" value={t.motion} options={["functional", "ambient", "rich"]} onChange={(v) => setTweak("motion", v)} />

      <TweakSection label="Color treatment" />
      <TweakRadio label="Mood" value={t.mood} options={[
        { value: "dusk", label: "Dusk" }, { value: "ink", label: "Ink" }, { value: "midnight", label: "Night" }]}
        onChange={(v) => setTweak("mood", v)} />
      <TweakRadio label="Accent" value={t.accent} options={[
        { value: "brand", label: "Brand" }, { value: "abyssal", label: "Abyssal" }, { value: "ember", label: "Ember" }]}
        onChange={(v) => setTweak("accent", v)} />

      <TweakSection label="Texture" />
      <TweakToggle label="Parchment grain" value={t.grain} onChange={(v) => setTweak("grain", v)} />
      <TweakToggle label="Vignette" value={t.vignette} onChange={(v) => setTweak("vignette", v)} />
    </TweaksPanel>
  );
}
window.DivoraTweaks = DivoraTweaks;
