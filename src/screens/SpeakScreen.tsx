// Speak (v1.17.0) — type text, pick a preset voice, and synthesize speech
// that plays through the output (mixed with the live mic via the soundboard
// seam, so a Discord/stream listener hears it too).
//
// This is the **scaffolding**: the workspace, voice picker, and command
// surface are live, but synthesis is gated behind "voice not installed"
// until the Kokoro model + voice pack + espeak-ng are bundled. Pressing
// Speak before then surfaces a graceful notice rather than failing silently.

import { For, Show, createMemo, onMount, type JSX } from "solid-js";
import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import { Sigil } from "../components/Sigil";
import { useApp } from "../stores/app";

export function SpeakScreen(): JSX.Element {
  const app = useApp();

  onMount(() => {
    void app.refreshTtsVoices();
  });

  const anyInstalled = (): boolean => app.ttsVoices().some((v) => v.installed);
  const hasText = (): boolean => app.ttsText().trim().length > 0;

  // The "tts" clip lives in the shared soundboard playback store; the global
  // ~30 Hz clock drives the progress ring and auto-expires the entry.
  const ttsClip = () => app.playingClips["tts"];
  const playing = (): boolean => !!ttsClip();
  const progress = createMemo<number>(() => {
    app.clockTick(); // re-evaluate each tick while playing
    const clip = ttsClip();
    if (!clip || clip.durationSecs <= 0) return 0;
    const elapsed = (performance.now() - clip.startedAt) / 1000;
    return Math.min(1, Math.max(0, elapsed / clip.durationSecs));
  });

  return (
    <div style={{ height: "100%", overflow: "auto", padding: "var(--s7)" }}>
      <div
        style={{
          "max-width": "760px",
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
            Speak
          </h2>
          <p style={{ color: "var(--text-mid)", "font-size": "var(--t-sm)" }}>
            Type something, choose a voice, and Divora speaks it aloud —{" "}
            <Show
              when={app.engineRunning()}
              fallback={<>start the engine on the Mixer to route it into a call.</>}
            >
              mixed into your output so a call or stream hears it too.
            </Show>
          </p>
        </div>

        <Show when={!anyInstalled()}>
          <div
            role="status"
            style={{
              display: "flex",
              gap: "var(--s3)",
              padding: "var(--s4)",
              "border-radius": "var(--r-md)",
              background: "var(--surface-2)",
              border: "1px solid var(--line)",
              color: "var(--text-mid)",
              "font-size": "var(--t-sm)",
            }}
          >
            <span style={{ color: "var(--text-low)", "flex-shrink": 0 }}>
              <Sigil name="info" size={18} />
            </span>
            <span>
              <strong style={{ color: "var(--text-high)" }}>
                Preset voices aren't installed yet.
              </strong>{" "}
              The Speak workspace is ready — synthesis activates automatically
              once the voice models ship in an update. Everything stays
              on-device; no text ever leaves your machine.
            </span>
          </div>
        </Show>

        {/* Text input */}
        <label
          style={{
            display: "flex",
            "flex-direction": "column",
            gap: "var(--s2)",
          }}
        >
          <span
            style={{
              "font-size": "var(--t-xs)",
              "text-transform": "uppercase",
              "letter-spacing": "0.08em",
              color: "var(--text-low)",
            }}
          >
            Text
          </span>
          <textarea
            value={app.ttsText()}
            onInput={(e) => app.setTtsText(e.currentTarget.value)}
            placeholder="Speak, and the coven echoes…"
            rows={5}
            spellcheck
            style={{
              width: "100%",
              resize: "vertical",
              "min-height": "120px",
              padding: "var(--s4)",
              "border-radius": "var(--r-md)",
              border: "1px solid var(--line)",
              background: "var(--surface-1)",
              color: "var(--text-high)",
              "font-family": "inherit",
              "font-size": "var(--t-base)",
              "line-height": "1.5",
            }}
          />
          <span
            style={{
              "align-self": "flex-end",
              "font-size": "var(--t-xs)",
              color: "var(--text-low)",
              "font-variant-numeric": "tabular-nums",
            }}
          >
            {app.ttsText().trim().length} chars
          </span>
        </label>

        {/* Voice picker */}
        <div
          style={{
            display: "flex",
            "flex-direction": "column",
            gap: "var(--s2)",
          }}
        >
          <span
            style={{
              "font-size": "var(--t-xs)",
              "text-transform": "uppercase",
              "letter-spacing": "0.08em",
              color: "var(--text-low)",
            }}
          >
            Voice
          </span>
          <div
            role="radiogroup"
            aria-label="Preset voice"
            style={{
              display: "grid",
              "grid-template-columns": "repeat(auto-fill, minmax(220px, 1fr))",
              gap: "var(--s3)",
            }}
          >
            <For each={app.ttsVoices()}>
              {(voice) => {
                const selected = (): boolean =>
                  app.selectedTtsVoice() === voice.id;
                return (
                  <button
                    type="button"
                    role="radio"
                    aria-checked={selected()}
                    onClick={() => app.setSelectedTtsVoice(voice.id)}
                    style={{
                      display: "flex",
                      "align-items": "center",
                      "justify-content": "space-between",
                      gap: "var(--s2)",
                      padding: "var(--s3) var(--s4)",
                      "border-radius": "var(--r-md)",
                      border: `1px solid ${
                        selected() ? "var(--accent)" : "var(--line)"
                      }`,
                      background: selected()
                        ? "var(--accent-soft, var(--surface-2))"
                        : "var(--surface-1)",
                      color: "var(--text-high)",
                      cursor: "pointer",
                      "text-align": "left",
                    }}
                  >
                    <span
                      style={{
                        display: "flex",
                        "align-items": "center",
                        gap: "var(--s2)",
                      }}
                    >
                      <span
                        style={{
                          color: selected()
                            ? "var(--accent)"
                            : "var(--text-low)",
                          display: "flex",
                        }}
                      >
                        <Sigil name="wave" size={16} />
                      </span>
                      {voice.name}
                    </span>
                    <Show when={!voice.installed}>
                      <Badge tone="warning">Soon</Badge>
                    </Show>
                  </button>
                );
              }}
            </For>
          </div>
        </div>

        {/* Volume + preview options */}
        <div
          style={{
            display: "flex",
            "align-items": "center",
            gap: "var(--s5)",
            "flex-wrap": "wrap",
          }}
        >
          <label
            style={{
              display: "flex",
              "align-items": "center",
              gap: "var(--s3)",
              flex: "1 1 240px",
            }}
          >
            <span
              style={{
                "font-size": "var(--t-xs)",
                "text-transform": "uppercase",
                "letter-spacing": "0.08em",
                color: "var(--text-low)",
                "white-space": "nowrap",
              }}
            >
              Volume
            </span>
            <input
              type="range"
              min="0"
              max="2"
              step="0.05"
              value={app.ttsVolume()}
              onInput={(e) => app.setTtsVolume(Number(e.currentTarget.value))}
              style={{ flex: 1, "accent-color": "var(--accent)" }}
              aria-label="Speak volume"
            />
            <span
              style={{
                "font-size": "var(--t-xs)",
                color: "var(--text-mid)",
                "font-variant-numeric": "tabular-nums",
                "min-width": "3ch",
                "text-align": "right",
              }}
            >
              {Math.round(app.ttsVolume() * 100)}%
            </span>
          </label>

          <label
            style={{
              display: "flex",
              "align-items": "center",
              gap: "var(--s2)",
              cursor: "pointer",
              "user-select": "none",
            }}
            title="Play only through your monitor — you hear it, the call doesn't"
          >
            <input
              type="checkbox"
              checked={app.ttsPreviewOnly()}
              onChange={(e) => app.setTtsPreviewOnly(e.currentTarget.checked)}
              style={{ "accent-color": "var(--accent)" }}
            />
            <span style={{ "font-size": "var(--t-sm)", color: "var(--text-high)" }}>
              Preview only
            </span>
            <span aria-hidden="true" style={{ color: "var(--text-low)", display: "flex" }}>
              <Sigil name="monitor" size={15} />
            </span>
          </label>
        </div>

        {/* Controls */}
        <div
          style={{
            display: "flex",
            "align-items": "center",
            gap: "var(--s3)",
          }}
        >
          <Button
            variant="primary"
            icon="wave"
            disabled={!hasText() || app.synthesizing()}
            onClick={() => void app.speakText()}
          >
            {app.synthesizing() ? "Synthesizing…" : "Speak"}
          </Button>
          <Button
            variant="secondary"
            icon="stop"
            disabled={!playing()}
            onClick={() => void app.stopSpeaking()}
          >
            Stop
          </Button>

          <Show when={playing()}>
            <span
              aria-hidden="true"
              style={{ display: "flex", "margin-left": "var(--s1)" }}
            >
              <ProgressRing progress={progress()} />
            </span>
          </Show>

          <div style={{ flex: 1 }} />

          <Show when={app.ttsError()}>
            <span
              role="alert"
              style={{
                display: "flex",
                "align-items": "center",
                gap: "var(--s2)",
                "font-size": "var(--t-sm)",
                color: "var(--text-mid)",
              }}
            >
              <span style={{ color: "var(--warning, var(--text-low))" }}>
                <Sigil name="warning" size={15} />
              </span>
              {app.ttsError()}
            </span>
          </Show>
        </div>
      </div>
    </div>
  );
}

interface ProgressRingProps {
  progress: number;
}

/** Small playback progress ring — mirrors the soundboard's. */
function ProgressRing(props: ProgressRingProps): JSX.Element {
  const RADIUS = 9;
  const CIRC = 2 * Math.PI * RADIUS;
  const dashoffset = (): number => CIRC * (1 - props.progress);
  return (
    <svg width="24" height="24" viewBox="0 0 24 24" aria-hidden="true">
      <circle
        cx="12"
        cy="12"
        r={RADIUS}
        fill="none"
        stroke="rgba(168,150,220,0.18)"
        stroke-width="2"
      />
      <circle
        cx="12"
        cy="12"
        r={RADIUS}
        fill="none"
        stroke="var(--accent)"
        stroke-width="2.5"
        stroke-linecap="round"
        stroke-dasharray={String(CIRC)}
        stroke-dashoffset={String(dashoffset())}
        transform="rotate(-90 12 12)"
        style={{ transition: "stroke-dashoffset 0.05s linear" }}
      />
    </svg>
  );
}
