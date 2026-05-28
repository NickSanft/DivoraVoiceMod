// Settings (Phase 1 placeholder).
//
// Real Settings (Audio devices, Virtual microphone, Hotkeys, Glyph
// casting, Appearance/Tweaks, About) lands in Phase 6. To prove the
// Tweaks foundation works today, this screen exposes the Color mood
// and Motion knobs early — they bind directly into the data-mood /
// data-motion attributes on the root via effects in App.tsx.

import type { JSX } from "solid-js";
import { Segmented } from "../components/Segmented";
import { useApp } from "../stores/app";
import type { TweaksState } from "../types";

export function SettingsScreen(): JSX.Element {
  const app = useApp();
  const motionLabel = (m: number) =>
    m === 0 ? "functional" : m < 0.8 ? "ambient" : "rich";
  return (
    <div
      style={{
        height: "100%",
        padding: "var(--s8) var(--s7)",
        overflow: "auto",
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s7)",
        "max-width": "680px",
        margin: "0 auto",
      }}
    >
      <div>
        <h2
          class="display"
          style={{ "font-size": "var(--t-h1)", "margin-bottom": "var(--s2)" }}
        >
          Settings
        </h2>
        <p style={{ color: "var(--text-mid)", "font-size": "var(--t-sm)" }}>
          Full Settings (devices, virtual mic, hotkeys, glyph casting, about)
          lands in Phase 6. Until then, the Tweaks foundation is exposed here so
          the design system is testable.
        </p>
      </div>

      <section>
        <div
          class="eyebrow"
          style={{ "margin-bottom": "var(--s3)" }}
        >
          Appearance
        </div>
        <div
          class="panel"
          style={{
            padding: "var(--s5)",
            display: "flex",
            "flex-direction": "column",
            gap: "var(--s5)",
          }}
        >
          <Row label="Color mood">
            <Segmented<TweaksState["mood"]>
              options={[
                { value: "violet", label: "Dusk Violet" },
                { value: "ink", label: "Ink + Candle" },
                { value: "midnight", label: "Midnight" },
              ]}
              value={app.tweaks.mood}
              onChange={(v) => app.setTweaks("mood", v)}
            />
          </Row>
          <Row label="Accent">
            <Segmented<TweaksState["accent"]>
              options={[
                { value: "brand", label: "Brand" },
                { value: "abyssal", label: "Abyssal" },
                { value: "ember", label: "Ember" },
              ]}
              value={app.tweaks.accent}
              onChange={(v) => app.setTweaks("accent", v)}
              accent
            />
          </Row>
          <Row label={`Motion · ${motionLabel(app.tweaks.motion)}`}>
            <Segmented<"functional" | "ambient" | "rich">
              options={[
                { value: "functional", label: "Functional" },
                { value: "ambient", label: "Ambient" },
                { value: "rich", label: "Rich" },
              ]}
              value={
                app.tweaks.motion === 0
                  ? "functional"
                  : app.tweaks.motion < 0.8
                    ? "ambient"
                    : "rich"
              }
              onChange={(v) =>
                app.setTweaks(
                  "motion",
                  v === "functional" ? 0 : v === "ambient" ? 0.6 : 1,
                )
              }
            />
          </Row>
        </div>
      </section>
    </div>
  );
}

interface RowProps {
  label: string;
  children: JSX.Element;
}

function Row(props: RowProps): JSX.Element {
  return (
    <div
      style={{
        display: "flex",
        "align-items": "center",
        "justify-content": "space-between",
        gap: "var(--s4)",
      }}
    >
      <span style={{ "font-size": "var(--t-sm)", color: "var(--text-mid)" }}>
        {props.label}
      </span>
      {props.children}
    </div>
  );
}
