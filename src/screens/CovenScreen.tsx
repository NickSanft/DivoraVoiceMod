// The Coven (v1.1.0) — a curated gallery of Divora's character voices.
//
// Each card is a cast member from `src/data/coven.ts`, drawing its
// name / color / glyph / chain from the matching bundled preset
// (`app.presets()`). "Summon" applies that preset's chain live and, for
// model-backed members (the narrator), loads its conversion model;
// summoning a DSP character clears any active model. The active member
// is derived from `app.presetId()` — no extra state.

import { For, Show, type JSX } from "solid-js";
import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import { Sigil, type SigilName } from "../components/Sigil";
import { COVEN, type CastMember } from "../data/coven";
import { useApp } from "../stores/app";
import type { Preset } from "../types";

interface CastEntry {
  member: CastMember;
  preset: Preset;
}

export function CovenScreen(): JSX.Element {
  const app = useApp();

  // Resolve each cast member to its preset (the source of truth for its
  // display + recipe). Members whose preset isn't loaded are skipped.
  const cast = (): CastEntry[] =>
    COVEN.flatMap((member) => {
      const preset = app.presets().find((p) => p.id === member.presetId);
      return preset ? [{ member, preset }] : [];
    });

  // Is a model-backed member's AI voice actually installed + runnable?
  // (DSP members are always "ready".)
  const modelReady = (m: CastMember): boolean =>
    !!m.modelId &&
    (app.onnxStatus()?.runtimeAvailable ?? false) &&
    app.voices().some((v) => v.id === m.modelId);

  const summon = (m: CastMember): void => {
    app.summon(m.presetId, m.kind === "model" ? m.modelId ?? null : null);
  };

  return (
    <div style={{ height: "100%", overflow: "auto", padding: "var(--s7)" }}>
      <div
        style={{
          "max-width": "960px",
          margin: "0 auto",
          display: "flex",
          "flex-direction": "column",
          gap: "var(--s6)",
        }}
      >
        <div>
          <h2
            class="display"
            style={{ "font-size": "var(--t-h1)", "margin-bottom": "var(--s2)" }}
          >
            The Coven
          </h2>
          <p style={{ color: "var(--text-mid)", "font-size": "var(--t-sm)" }}>
            Summon a voice — each one transmutes you live.{" "}
            <Show
              when={app.engineRunning()}
              fallback={<>Start the engine on the Mixer to hear them.</>}
            >
              They take effect the moment you speak.
            </Show>
          </p>
        </div>

        <div
          style={{
            display: "grid",
            "grid-template-columns": "repeat(auto-fill, minmax(264px, 1fr))",
            gap: "var(--s4)",
          }}
        >
          <For each={cast()}>
            {(entry) => (
              <CastCard
                preset={entry.preset}
                member={entry.member}
                active={app.presetId() === entry.member.presetId}
                modelReady={
                  entry.member.kind === "model"
                    ? modelReady(entry.member)
                    : true
                }
                onSummon={() => summon(entry.member)}
              />
            )}
          </For>
        </div>
      </div>
    </div>
  );
}

interface CastCardProps {
  preset: Preset;
  member: CastMember;
  active: boolean;
  modelReady: boolean;
  onSummon: () => void;
}

function CastCard(props: CastCardProps): JSX.Element {
  const color = (): string => props.preset.color;
  return (
    <div
      class="card"
      style={{
        padding: "var(--s5)",
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
        position: "relative",
        "border-color": props.active ? `${color()}88` : undefined,
        "box-shadow": props.active
          ? `0 0 0 1px ${color()}55, 0 0 28px ${color()}33`
          : undefined,
      }}
    >
      <Show when={props.active}>
        <div style={{ position: "absolute", top: "var(--s4)", right: "var(--s4)" }}>
          <Badge tone="accent">Active</Badge>
        </div>
      </Show>

      <div
        style={{
          width: "56px",
          height: "56px",
          "border-radius": "var(--r-lg)",
          display: "grid",
          "place-items": "center",
          background: `${color()}22`,
          border: `1px solid ${color()}55`,
          color: color(),
        }}
      >
        <Sigil name={props.preset.glyph as SigilName} size={30} />
      </div>

      <div>
        <div style={{ display: "flex", "align-items": "center", gap: "var(--s2)" }}>
          <h3 class="display" style={{ "font-size": "var(--t-h3)", "font-weight": 700 }}>
            {props.preset.name}
          </h3>
          <Badge tone={props.member.kind === "model" ? "info" : ""}>
            {props.member.kind === "model" ? "AI Voice" : "DSP"}
          </Badge>
        </div>
        <p
          style={{
            "font-size": "var(--t-sm)",
            color: "var(--text-mid)",
            "line-height": 1.5,
            "margin-top": "4px",
          }}
        >
          {props.member.lore}
        </p>
      </div>

      <Show when={props.member.kind === "model" && !props.modelReady}>
        <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)", "line-height": 1.45 }}>
          AI model not installed — Summon uses the DSP voice. Add it in{" "}
          <strong>Settings → Voice library</strong>.
        </div>
      </Show>

      <div style={{ flex: 1 }} />

      <Button
        variant={props.active ? "secondary" : "primary"}
        block
        onClick={props.onSummon}
      >
        {props.active ? "Summoned" : "Summon"}
      </Button>
    </div>
  );
}
