// Mixer — Phase 3 lights up the spell circle and the selected-rune
// inspector. Effects orbit a glowing voice core; threads of light
// connect enabled effects to the core; the inspector lets the user
// tweak the focused effect live.
//
// v0.11.3: the explicit Cast button is gone. The user drags anywhere
// on the Mixer (except on UI controls) to draw a glyph; the always-
// mounted SparkLayer renders the spark trail + canvas-based "omen"
// (shape outline + preset name + "◆ SPELL CAST ◆"). Matches the
// prototype's headline gesture: the Mixer *is* the casting surface.

import {
  createMemo,
  createSignal,
  createUniqueId,
  onCleanup,
  onMount,
  Show,
  type JSX,
} from "solid-js";
import { Badge } from "../components/Badge";
import { blurOnPointer } from "../components/blurOnPointer";
import { Button } from "../components/Button";
import { Inspector } from "../components/Inspector";
import { Kbd } from "../components/Kbd";
import { VMeter } from "../components/Meters";
import { Segmented } from "../components/Segmented";
import { Select } from "../components/Select";
import { Sigil, type SigilName } from "../components/Sigil";
import { SparkLayer } from "../components/SparkLayer";
import { SpellCircle } from "../components/SpellCircle";
import { Toggle } from "../components/Toggle";
import { statusMeta } from "../shell/statusMeta";
import {
  REACTIVE_CEIL_DB,
  REACTIVE_FLOOR_DB,
} from "../data/reactive";
import { useApp } from "../stores/app";
import { acceleratorToKeys } from "../data/hotkeys";
import {
  doubledHint,
  formatClock,
  monitorLabel,
  pickDesktopSource,
  pickMonitor,
  quietWarning,
  screenBusy,
  screenRecordSubtitle,
} from "../data/screenRecording";
import {
  desktopSettingWords,
  explainerNeeded,
  heldCopy,
  lengthTitle,
  lockedLine,
  memoryLabel,
  REPLAY_LENGTHS,
  replayOn,
  sanitizeReplaySeconds,
  saveLabel,
} from "../data/replay";
import type { DesktopAudioSource } from "../audio/api";
import {
  resamplingDetail,
  resamplingSummary,
  type ReadingMetrics,
  type VoiceReadingUpdate,
} from "../audio/api";
import { ptmButtonLabel, ptmButtonTitle, ptmCardLine } from "../data/ptm";
import type { EffectId, GlyphId, Preset, PtmHold } from "../types";

export function MixerScreen(): JSX.Element {
  const app = useApp();
  const activeCount = () => app.chain().filter((c) => c.enabled).length;
  const totalCount = () => app.chain().length;
  const [castMessage, setCastMessage] = createSignal<string | null>(null);
  let messageTimeout: number | undefined;

  const flashMessage = (text: string): void => {
    setCastMessage(text);
    if (messageTimeout !== undefined) {
      window.clearTimeout(messageTimeout);
    }
    messageTimeout = window.setTimeout(() => {
      setCastMessage(null);
      messageTimeout = undefined;
    }, 2400);
  };

  /** Resolve a recognised glyph id (built-in or custom) to its cast
   *  outcome — colour + label + the bound action to run. Null when
   *  unbound; the SparkLayer then surfaces a flash. */
  const resolveGlyph = (
    glyphId: string,
  ): { color: string; label: string; run: () => void } | null => {
    const outcome = app.glyphOutcome(glyphId);
    if (!outcome) return null;
    return {
      color: outcome.color,
      label: outcome.label,
      run: () => app.dispatchGlyphAction(outcome.action),
    };
  };

  onCleanup(() => {
    if (messageTimeout !== undefined) {
      window.clearTimeout(messageTimeout);
    }
  });

  return (
    <div
      style={{
        height: "100%",
        display: "flex",
        "flex-direction": "column",
        padding: "20px 24px",
        gap: "var(--s5)",
        position: "relative",
      }}
    >
      <PresetHeader activeCount={activeCount()} totalCount={totalCount()} />
      <SparkLayer
        recognizeCustom={(path) => app.recognizeCustomGlyph(path)}
        resolveGlyph={resolveGlyph}
        onMessage={flashMessage}
      />
      <Show when={castMessage()} keyed>
        {(text) => <CastFlash text={text} />}
      </Show>
      <div
        style={{
          flex: 1,
          display: "flex",
          gap: "var(--s7)",
          "min-height": 0,
          "align-items": "stretch",
        }}
      >
        <div
          style={{
            display: "flex",
            "flex-direction": "column",
            "align-items": "center",
            "justify-content": "center",
            gap: "var(--s3)",
          }}
        >
          <VMeter
            level={app.inputLevels().rms}
            peak={app.inputLevels().peak}
            height={320}
            label="In"
          />
          <DbReadout levels={app.inputLevels()} />
        </div>
        <div
          style={{
            flex: 1,
            display: "flex",
            "align-items": "center",
            "justify-content": "center",
          }}
        >
          <SpellCircle
            chain={app.chain()}
            status={app.status()}
            motion={app.tweaks.motion}
            mystical={app.tweaks.mystical}
            selected={app.selectedEffect()}
            onSelect={(id) => app.setSelectedEffect(id)}
            onToggle={(id) => app.toggleEffectById(id)}
          />
        </div>
        <div
          style={{
            display: "flex",
            "flex-direction": "column",
            "align-items": "center",
            "justify-content": "center",
            gap: "var(--s3)",
          }}
        >
          <VMeter
            level={app.outputLevels().rms}
            peak={app.outputLevels().peak}
            height={320}
            label="Out"
          />
          <DbReadout levels={app.outputLevels()} />
        </div>
        <RightRail />
      </div>
    </div>
  );
}

interface PresetHeaderProps {
  activeCount: number;
  totalCount: number;
}

function CastFlash(props: { text: string }): JSX.Element {
  return (
    <div
      style={{
        position: "absolute",
        bottom: "var(--s7)",
        left: "50%",
        transform: "translateX(-50%)",
        "z-index": 80,
        padding: "var(--s3) var(--s5)",
        "border-radius": "var(--r-pill)",
        background: "var(--surface-2)",
        border: "1px solid var(--line-glow)",
        "box-shadow": "var(--shadow-2)",
        "font-size": "var(--t-sm)",
        color: "var(--text-hi)",
        "pointer-events": "none",
      }}
    >
      {props.text}
    </div>
  );
}

function PresetHeader(props: PresetHeaderProps): JSX.Element {
  const app = useApp();
  return (
    <div
      style={{
        display: "flex",
        "align-items": "center",
        gap: "var(--s4)",
        flex: "none",
      }}
    >
      <div
        style={{
          width: "38px",
          height: "38px",
          "border-radius": "var(--r-md)",
          display: "grid",
          "place-items": "center",
          background: app.preset().color + "26",
          border: `1px solid ${app.preset().color}55`,
          color: app.preset().color,
        }}
      >
        <Sigil name={app.preset().glyph as SigilName} size={22} />
      </div>
      <div style={{ display: "flex", "flex-direction": "column", gap: "2px" }}>
        <div style={{ display: "flex", "align-items": "center", gap: "var(--s2)" }}>
          <h2 class="display" style={{ "font-size": "26px", "font-weight": 700 }}>
            {app.preset().name}
          </h2>
          <Badge tone={app.preset().tag === "Bundled" ? "accent" : "info"}>
            {app.preset().tag}
          </Badge>
        </div>
        <div style={{ "font-size": "var(--t-sm)", color: "var(--text-lo)" }}>
          {props.activeCount} of {props.totalCount} runes active
          <Show when={app.streamInfo()}>
            {(info) => (
              <>
                <span> · routed via {info().outputName}</span>
                {/* v1.51.2: say so when a device disagrees on a rate. Nothing
                    used to, because StreamInfo carried one rate — and the code
                    that bridged the gap was destroying the audio. The summary
                    names the legs that actually differ: the monitor converts
                    from the engine rate independently of the main output, so
                    naming the engine→output pair here printed two identical
                    numbers whenever only the headphones were mismatched. */}
                <Show when={resamplingSummary(info())}>
                  {(summary) => (
                    <span title={resamplingDetail(info())}>
                      {" · resampling "}
                      {summary()}
                    </span>
                  )}
                </Show>
              </>
            )}
          </Show>
          <Show when={app.engineRunning() && app.dspLatencyMs() >= 0.5}>
            <span title="Latency added by the active effects (e.g. Voice Convert ≈ 18 ms with the bundled voice, Denoiser ≈ 10 ms)">
              {" "}
              · +{Math.round(app.dspLatencyMs())} ms latency
            </span>
          </Show>
        </div>
      </div>
      <div style={{ flex: 1 }} />
      <div style={{ display: "flex", "align-items": "center", gap: "var(--s2)" }}>
        <span class="eyebrow">Compare</span>
        <Segmented
          options={["A", "B"]}
          value={app.ui.ab}
          onChange={(v) => app.setAbSlot(v as "A" | "B")}
          accent
        />
      </div>
    </div>
  );
}

function RightRail(): JSX.Element {
  const app = useApp();
  return (
    <div
      style={{
        width: "290px",
        flex: "none",
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
        overflow: "auto",
      }}
    >
      <VoiceStatusCard />
      <PushToModulateCard />
      <MonitorCard />
      <ReactiveCard />
      <VoiceReadingCard />
      <LoudnessCard />
      <RecordCard />
      <ScreenRecordCard />
      <Inspector />
      <Show when={app.engineError()}>
        {(err) => (
          <div
            class="card"
            style={{
              padding: "var(--s4)",
              "border-color": "rgba(242, 86, 122, 0.4)",
              background: "var(--danger-bg)",
              color: "var(--danger)",
              "font-size": "var(--t-xs)",
              "line-height": 1.5,
            }}
          >
            <div
              class="eyebrow"
              style={{ "margin-bottom": "4px", color: "var(--danger)" }}
            >
              Engine error
            </div>
            {err()}
          </div>
        )}
      </Show>
      <Show when={!app.engineRunning() && !app.engineError()}>
        <div
          class="card"
          style={{
            padding: "var(--s4)",
            "font-size": "var(--t-xs)",
            color: "var(--text-lo)",
            "line-height": 1.5,
          }}
        >
          <div class="eyebrow" style={{ "margin-bottom": "4px" }}>
            Engine offline
          </div>
          Go to{" "}
          <button
            type="button"
            class="btn btn-ghost btn-sm"
            style={{ padding: "0 4px", height: "auto", display: "inline" }}
            onClick={() => app.setNav("settings")}
          >
            Settings
          </button>{" "}
          to pick devices and start passthrough.
        </div>
      </Show>
    </div>
  );
}

function VoiceStatusCard(): JSX.Element {
  const app = useApp();
  const meta = () => statusMeta(app.status());
  return (
    <div
      class="card"
      style={{
        padding: "var(--s4)",
        display: "flex",
        "align-items": "center",
        gap: "var(--s3)",
        background: meta().bg,
        "border-color": meta().line,
      }}
    >
      <div
        style={{
          width: "38px",
          height: "38px",
          "border-radius": "var(--r-md)",
          display: "grid",
          "place-items": "center",
          color: meta().color,
          background: "var(--surface-2)",
        }}
      >
        <Sigil name={meta().sigil} size={22} />
      </div>
      <div>
        <div
          style={{
            "font-family": "var(--font-display)",
            "font-size": "var(--t-h3)",
            "font-weight": 700,
            color: meta().color,
          }}
        >
          {meta().label}
        </div>
        <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
          {meta().sub}
        </div>
      </div>
      {/* v1.56.0: sends the state its label names, never a toggle: a stale
          label (the mirror lags a hotkey mute by a tick) is then a no-op whose
          answer corrects it, not the opposite of what it says. Never
          disabled, and works with the engine stopped.
          A mouse or touch click (detail > 0) hands focus back to the page: a
          focused button takes Space, which is the in-app push-to-modulate
          key, so the next PTM press would unmute. Tab, then Space or Enter
          (detail 0), keeps focus on it. */}
      <Button
        variant={app.muted() ? "danger" : "secondary"}
        size="sm"
        data-testid="mute-button"
        title={
          app.muted()
            ? "Unmute your voice"
            : "Mute your voice for the call, your headphones, WAV and screen recordings and Replay. Clips and Speak still play."
        }
        style={{ "margin-left": "auto", "flex-shrink": 0 }}
        onClick={(e) => {
          blurOnPointer(e);
          void app.setMuted(!app.muted());
        }}
      >
        {app.muted() ? "Unmute" : "Mute"}
      </Button>
    </div>
  );
}

function PushToModulateCard(): JSX.Element {
  const app = useApp();
  // Show the key the way Settings does: "Win", not the accelerator's
  // "Super" (v1.51.4 stores the backend form).
  const ptmLabel = (): string => acceleratorToKeys(app.ui.ptmKey).join("+") || "Space";
  // v1.58.0: the card drives the engine. It renders from `divora.ptm` (on or
  // off, and which way the hold goes), which moves only when the engine's
  // answer lands. Setting either from here, even to what it already is,
  // lets go of every hold: the way out of a stuck one (D13).
  const on = (): boolean => app.ptmSetting().on;
  // Something that still works while a call has the focus: a global key, or
  // a `ptm` MIDI mapping that has learned its pad or knob (one that hasn't
  // can never fire) while a MIDI input is picked (Settings' "None —
  // disconnected" closes the port and keeps the mappings, so nothing can
  // arrive). With only the mapping, the key the card names still works only
  // in Divora's window, so the line says so and names the MIDI control.
  // Without either, it says where to set a key.
  const inCall = (): "key" | "midi" | null =>
    app.hotkeyBindings.ptm !== ""
      ? "key"
      : !!app.selectedMidiInput() &&
          app.midiMappings().some((m) => m.action === "ptm" && m.trigger !== null)
        ? "midi"
        : null;
  const lineId = `ptm-line-${createUniqueId()}`;

  // Hold to switch holds the "button" source while it is down, by the
  // pointer or by Enter or Space while it has the focus. `down` is only its
  // look: the store sends each edge once, so letting go twice sends one
  // release. Everything that can end a hold with no pointerup or keyup to
  // follow lets go as well: the pointer leaving the button (no pointer
  // capture, so it can't stick) or being cancelled, the button losing the
  // focus, the window losing it or being hidden, and the card going away.
  const [down, setDown] = createSignal(false);
  // A press that began with the pointer, so only its own end hands focus
  // back to the page (the Mute button's rule), and a pointer that merely
  // passes over the button can't end a hold made from the keyboard.
  let byPointer = false;
  const hold = (): void => {
    if (!on()) return;
    setDown(true);
    app.ptmDown("button");
  };
  const letGo = (): void => {
    setDown(false);
    app.ptmUp("button");
  };
  const pointerEnd = (e: PointerEvent & { currentTarget: HTMLButtonElement }): void => {
    if (!byPointer) return;
    byPointer = false;
    e.currentTarget.blur();
    letGo();
  };
  const holdKey = (e: KeyboardEvent): boolean => e.key === "Enter" || e.key === " ";
  onMount(() => {
    const hidden = (): void => {
      if (document.visibilityState === "hidden") letGo();
    };
    window.addEventListener("blur", letGo);
    document.addEventListener("visibilitychange", hidden);
    onCleanup(() => {
      window.removeEventListener("blur", letGo);
      document.removeEventListener("visibilitychange", hidden);
    });
  });
  onCleanup(() => app.ptmUp("button"));

  return (
    <div class="card" style={{ padding: "var(--s4)" }}>
      <div
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          gap: "var(--s2)",
          "margin-bottom": "var(--s2)",
        }}
      >
        <span class="eyebrow">Push to modulate</span>
        <div
          style={{
            display: "flex",
            "align-items": "center",
            gap: "var(--s2)",
            "flex-shrink": 0,
          }}
        >
          <Kbd>{ptmLabel()}</Kbd>
          <Toggle
            on={on()}
            ariaLabel="Push to modulate"
            ariaDescribedBy={lineId}
            blurOnPointer
            onChange={(next) => void app.setPtmOn(next)}
          />
        </div>
      </div>
      <div
        id={lineId}
        style={{
          "font-size": "var(--t-xs)",
          color: "var(--text-lo)",
          "line-height": 1.5,
          "margin-bottom": "var(--s3)",
        }}
      >
        {ptmCardLine(app.ptmSetting(), ptmLabel(), inCall())}
      </div>
      <Segmented<PtmHold>
        options={[
          { value: "apply", label: "Hold to apply" },
          { value: "bypass", label: "Hold to bypass" },
        ]}
        value={app.ptmSetting().hold}
        onChange={(v) => void app.setPtmHold(v)}
        disabled={!on()}
        blurOnPointer
      />
      <button
        type="button"
        class="btn btn-secondary btn-block"
        data-testid="ptm-hold"
        aria-pressed={down()}
        disabled={!on()}
        title={ptmButtonTitle(on())}
        style={{
          "margin-top": "var(--s3)",
          height: "44px",
          background: down() ? "var(--accent-bg)" : undefined,
          "border-color": down() ? "var(--line-glow)" : undefined,
          color: down() ? "var(--indigo)" : undefined,
        }}
        onPointerDown={() => {
          byPointer = true;
          hold();
        }}
        onPointerUp={pointerEnd}
        onPointerLeave={pointerEnd}
        onPointerCancel={pointerEnd}
        onKeyDown={(e) => {
          if (holdKey(e) && !e.repeat) hold();
        }}
        onKeyUp={(e) => {
          if (holdKey(e)) letGo();
        }}
        onBlur={letGo}
      >
        {ptmButtonLabel(down(), app.ptmBypassed(), ptmLabel())}
      </button>
    </div>
  );
}

function MonitorCard(): JSX.Element {
  const app = useApp();
  const pct = () => Math.round(app.monitorGain() * 100);
  return (
    <div
      class="card"
      style={{
        padding: "var(--s4)",
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
      }}
    >
      <div
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          gap: "var(--s3)",
        }}
      >
        <div
          style={{
            display: "flex",
            "align-items": "center",
            gap: "var(--s3)",
            color: "var(--text-mid)",
          }}
        >
          <Sigil name="monitor" size={20} style={{ color: "var(--indigo)" }} />
          <div>
            <div style={{ "font-size": "var(--t-sm)", "font-weight": 600 }}>
              Monitor
            </div>
            <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
              Hear yourself in headphones
            </div>
          </div>
        </div>
        <Toggle
          on={app.engineMonitoring()}
          onChange={(next) => void app.setMonitor(next)}
          ariaLabel="Sidetone monitor"
        />
      </div>

      {/* v1.6.0: monitor volume — boost/cut how loud you hear yourself
          (applies to the separate monitor device). */}
      <div style={{ display: "flex", "align-items": "center", gap: "var(--s3)" }}>
        <span
          class="eyebrow"
          style={{ color: "var(--text-lo)", "flex-shrink": 0 }}
        >
          Volume
        </span>
        <input
          type="range"
          min="0"
          max="2"
          step="0.05"
          value={app.monitorGain()}
          disabled={!app.engineMonitoring()}
          onInput={(e) => app.setMonitorGain(parseFloat(e.currentTarget.value))}
          aria-label="Monitor volume"
          style={{ flex: 1, opacity: app.engineMonitoring() ? 1 : 0.5 }}
        />
        <span
          class="tnum"
          style={{
            "font-size": "var(--t-xs)",
            color: "var(--text-lo)",
            width: "40px",
            "text-align": "right",
          }}
        >
          {pct()}%
        </span>
      </div>
    </div>
  );
}

// v1.46.0: reactive effects — the dry voice envelope drives the chain, so
// raising your voice hardens the character ("Rage": drive, plus a little
// reverb). Global and post-preset like LoudnessCard, not a per-preset effect.
//
// The live envelope strip is the important control here, not decoration: the
// feature's real failure mode is not being too weak, it's reading as "my voice
// is drifting on its own". Showing exactly what the follower hears, next to
// the window it maps onto, makes every parameter move attributable.
function ReactiveCard(): JSX.Element {
  const app = useApp();
  const on = (): boolean => app.reactiveEnabled();
  const depth = (): number => (on() ? app.modEnv() : 0);
  const pct = (): number => Math.round(Math.min(1, Math.max(0, depth())) * 100);

  return (
    <div
      class="card"
      style={{
        padding: "var(--s4)",
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
      }}
    >
      <div
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          gap: "var(--s3)",
        }}
      >
        <div
          style={{
            display: "flex",
            "align-items": "center",
            gap: "var(--s3)",
            color: "var(--text-mid)",
          }}
        >
          <Sigil name="bolt" size={20} style={{ color: "var(--indigo)" }} />
          <div>
            <div style={{ "font-size": "var(--t-sm)", "font-weight": 600 }}>
              Reactive
            </div>
            <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
              Raise your voice, harden the character
            </div>
          </div>
        </div>
        <Toggle
          on={on()}
          onChange={(next) => app.setReactiveEnabled(next)}
          ariaLabel="Reactive effects"
        />
      </div>

      {/* The guardrail: enabling this with nothing to drive would look broken.
          Say so instead of silently doing nothing. */}
      {/* The live region itself is always mounted and only its CONTENT is
          toggled — a region inserted together with its text is commonly missed
          by screen readers. */}
      <div role="status" aria-live="polite">
        <Show when={on() && !app.reactiveHasTarget()}>
          <div
            style={{
              display: "flex",
              "align-items": "center",
              gap: "var(--s2)",
              "font-size": "var(--t-xs)",
              color: "var(--text-mid)",
              background: "var(--surface-2)",
              border: "1px solid var(--line)",
              "border-radius": "var(--r-sm)",
              padding: "var(--s2) var(--s3)",
            }}
          >
            <Sigil name="info" size={14} />
            <span>
              This preset has no Distortion to drive — add one in Presets, or
              switch to a voice that uses it.
            </span>
          </div>
        </Show>
      </div>

      {/* Intensity: the single control. Scales every route's depth. */}
      <div style={{ display: "flex", "align-items": "center", gap: "var(--s3)" }}>
        <span class="eyebrow" style={{ color: "var(--text-lo)", "flex-shrink": 0 }}>
          Intensity
        </span>
        <input
          type="range"
          min="0"
          max="100"
          step="5"
          value={app.reactiveIntensity()}
          disabled={!on()}
          onInput={(e) =>
            app.setReactiveIntensity(parseFloat(e.currentTarget.value))
          }
          aria-label="Reactive intensity"
          style={{ flex: 1, opacity: on() ? 1 : 0.5 }}
        />
        <span
          class="tnum"
          style={{
            "font-size": "var(--t-xs)",
            color: "var(--text-lo)",
            width: "52px",
            "text-align": "right",
          }}
        >
          {app.reactiveIntensity()}%
        </span>
      </div>

      {/* Live envelope strip. The fill is what the follower currently hears,
          mapped across the response window — so a user calibrates by talking
          rather than by reasoning about dBFS. */}
      <Show when={on() && app.engineRunning()}>
        <div style={{ display: "flex", "flex-direction": "column", gap: "var(--s1)" }}>
          <div
            style={{
              display: "flex",
              "align-items": "center",
              "justify-content": "space-between",
              "font-size": "var(--t-xs)",
              color: "var(--text-lo)",
            }}
          >
            <span class="eyebrow">Envelope</span>
            <span class="tnum" style={{ color: "var(--indigo)" }}>
              {pct()}%
            </span>
          </div>
          <div
            role="meter"
            aria-label="Reactive envelope"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct()}
            style={{
              position: "relative",
              height: "8px",
              "border-radius": "999px",
              background: "var(--surface-2)",
              overflow: "hidden",
            }}
          >
            <div
              style={{
                position: "absolute",
                inset: "0 auto 0 0",
                width: `${pct()}%`,
                background: "var(--indigo)",
                "border-radius": "999px",
                // Cosmetic only — the follower already smooths the value; this
                // just keeps the 30 Hz event cadence from looking steppy.
                transition: "width 90ms linear",
              }}
            />
          </div>
          <div
            style={{
              display: "flex",
              "justify-content": "space-between",
              "font-size": "var(--t-xs)",
              color: "var(--text-lo)",
            }}
          >
            <span class="tnum">{REACTIVE_FLOOR_DB} dB</span>
            <span>quiet → shout</span>
            <span class="tnum">{REACTIVE_CEIL_DB} dB</span>
          </div>
        </div>
      </Show>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Voice reading — a live readout of the MEASURED ACOUSTIC PROPERTIES of the
// signal: level, level movement, pitch height, pitch range, pace, brightness.
//
// It is not emotion recognition, and that is a product decision rather than
// squeamishness. Voice alone carries intensity, not pleasantness — strip the
// words and an arousal score survives while a valence score does not — and
// this app pitch-shifts, bitcrushes and ring-modulates on top of that. So
// every word on screen describes a SIGNAL. The vocabulary lives in
// `divora-core/src/dsp/reading.rs` behind a test, and the backend sends the
// resolved words: this file renders what it is handed and owns no list of its
// own, so there is exactly one place a word can enter the product.
//
// In-app only. Deliberately not on the stream overlay: an audience cannot
// check a reading, and a guest on the mic would be read too.
// ---------------------------------------------------------------------------

/** What the card is showing, and why. */
export interface ReadingView {
  /** The headline. Also what (coarsely) gets announced. `divoraMuted`
   *  (v1.56.0): muted by Divora's own mute, not the device. */
  kind: "stopped" | "muted" | "divoraMuted" | "quiet" | "listening" | "held" | "live";
  /** Whether there are numbers worth rendering at all. */
  hasNumbers: boolean;
  /** Those numbers were measured earlier and are being held, not measured now. */
  stale: boolean;
}

/**
 * Resolve the payload into what the card shows.
 *
 * The four backend states are kept distinct on purpose. Most of a session is
 * not speech, and a panel that let every pause read as "quiet, flat, narrow"
 * would be delivering a verdict on the speaker several times a minute without
 * printing a word. A pause holds the last measured window and says it is held.
 */
export function readingView(
  reading: VoiceReadingUpdate | null,
  engineRunning: boolean,
): ReadingView {
  if (!reading) {
    return {
      kind: engineRunning ? "listening" : "stopped",
      hasNumbers: false,
      stale: false,
    };
  }
  const stale = reading.stale;
  // Held numbers still deserve showing — visibly stale — because "your last
  // measured window" is useful and "nothing" is not. What they must never do
  // is look live.
  const hasNumbers = reading.state === "speaking" || stale;
  let kind: ReadingView["kind"];
  if (reading.state === "stopped") {
    kind = "stopped";
  } else if (reading.state === "muted") {
    // v1.56.0: the cause rides on the same snapshot as the state, so the two
    // can't disagree. Never joined with the store's mirror: that travels at
    // a different rate and flashed "Input silent" after every unmute.
    kind = reading.mutedInDivora === true ? "divoraMuted" : "muted";
  } else if (reading.state === "speaking") {
    // Raw measurements are valid immediately; only the descriptors need a
    // baseline to be relative to, so say which one is missing.
    kind = reading.calibrated ? "live" : "listening";
  } else if (stale) {
    kind = "held";
  } else {
    kind = reading.calibrated ? "quiet" : "listening";
  }
  return { kind, hasNumbers, stale };
}

/**
 * The single sentence the card announces.
 *
 * Coarse on purpose. `live`, `quiet` and `held` collapse into one line
 * because they alternate every second or two while somebody talks, and a
 * screen reader repeating "live… paused… live" through a conversation is the
 * same defect as putting a number in a live region.
 */
export function readingAnnouncement(view: ReadingView): string {
  switch (view.kind) {
    case "stopped":
      return "Engine stopped. Nothing to read.";
    case "muted":
      return "Input is silent.";
    // Not the titlebar's sentence again: that one already said it.
    case "divoraMuted":
      return "Reading paused while your voice is muted.";
    case "listening":
      return "Still listening. Not enough speech yet to compare against.";
    default:
      return "Reading the microphone.";
  }
}

/** The visible headline. Changes freely; never announced. */
function readingHeadline(view: ReadingView): string {
  switch (view.kind) {
    case "stopped":
      return "Engine stopped";
    case "muted":
      return "Input silent";
    case "divoraMuted":
      return "Muted in Divora";
    case "quiet":
      return "Too quiet to measure";
    case "listening":
      return "Still listening";
    case "held":
      return "Paused";
    default:
      return "Live";
  }
}

/** "measured 8 s ago" / "measured 1.2 s ago" for a held window. */
function agoLabel(ms: number): string {
  const secs = ms / 1000;
  return secs < 10
    ? `measured ${secs.toFixed(1)} s ago`
    : `measured ${Math.round(secs)} s ago`;
}

/**
 * One measured number.
 *
 * `role="meter"`, and emphatically NOT inside a live region. These update
 * several times a second; a live region would make a screen reader talk
 * continuously and render the app unusable. A meter is read when the user asks
 * for it and never interrupts. The one live region on this card holds the
 * state sentence, which changes rarely by construction.
 */
function Readout(props: {
  /** Short visible label. */
  label: string;
  /** Full label for assistive tech, e.g. "Pitch, after effects". */
  ariaLabel: string;
  shown: string;
  spoken: string;
  value: number;
  min: number;
  max: number;
  dim?: boolean;
}): JSX.Element {
  return (
    <div
      style={{
        display: "flex",
        "align-items": "baseline",
        "justify-content": "space-between",
        gap: "var(--s2)",
      }}
    >
      <span class="eyebrow" style={{ color: "var(--text-lo)" }} aria-hidden="true">
        {props.label}
      </span>
      <span
        class="tnum"
        role="meter"
        aria-label={props.ariaLabel}
        aria-valuemin={props.min}
        aria-valuemax={props.max}
        aria-valuenow={props.value}
        aria-valuetext={props.spoken}
        style={{
          "font-size": "var(--t-xs)",
          color: props.dim ? "var(--text-lo)" : "var(--text-mid)",
        }}
      >
        {props.shown}
      </span>
    </div>
  );
}

/** The four numbers, for one half of the reading. */
function ReadoutGroup(props: {
  /** "your voice" / "after effects" — suffixed onto each accessible label. */
  half: string;
  metrics: ReadingMetrics;
  dim?: boolean;
}): JSX.Element {
  const m = () => props.metrics;
  return (
    <div
      style={{ display: "flex", "flex-direction": "column", gap: "var(--s1)" }}
    >
      <Readout
        label="Pitch"
        ariaLabel={`Pitch, ${props.half}`}
        shown={`${Math.round(m().f0Hz)} Hz`}
        spoken={`${Math.round(m().f0Hz)} hertz`}
        value={m().f0Hz}
        min={50}
        // The detector reaches 1 kHz since v1.51.0, because a voice
        // pitched up an octave is the after-effects half's whole point. A
        // 500 ceiling here left a screen reader computing the percentage
        // against the wrong range for exactly those readings.
        max={1000}
        dim={props.dim}
      />
      <Readout
        label="Range"
        ariaLabel={`Pitch range, ${props.half}`}
        shown={`${m().f0RangeSt.toFixed(1)} st`}
        spoken={`${m().f0RangeSt.toFixed(1)} semitones`}
        value={m().f0RangeSt}
        min={0}
        max={24}
        dim={props.dim}
      />
      <Readout
        label="Level"
        ariaLabel={`Level, ${props.half}`}
        shown={`${m().energyDbfs.toFixed(1)} dB`}
        spoken={`${m().energyDbfs.toFixed(1)} decibels`}
        value={m().energyDbfs}
        min={-60}
        max={0}
        dim={props.dim}
      />
      <Readout
        label="Pace"
        ariaLabel={`Pace, ${props.half}`}
        // Whole onsets only. The describer refuses to call a pace change
        // under 1.5/s anything at all, on the grounds that a 2.5 s window
        // holds ~10 onsets so the count's own scatter is ~±1.3/s — printing
        // a tenth would show that scatter as if it were a measurement of the
        // speaker. And the unit is on screen: "4/s" alone reads as syllables,
        // which is exactly what this is not.
        shown={`${Math.round(m().paceOps)} onsets/s`}
        spoken={`${Math.round(m().paceOps)} onsets per second`}
        value={m().paceOps}
        min={0}
        max={12}
        dim={props.dim}
      />
      <Readout
        label="Tone"
        ariaLabel={`Tone, ${props.half}`}
        // Brightness earns a row because of what this app's presets DO. A
        // bitcrusher, an EQ, a distortion or a reverb moves tone and level
        // movement while barely touching pitch, range, level or pace — so
        // without this the after-effects half showed four numbers nearly
        // identical to the microphone half on exactly those presets, and the
        // only conclusion available to the user was "the preset stopped
        // working". That is the misread the passing-through banner exists to
        // prevent, arriving through a different door.
        shown={`${Math.round(m().brightnessHz)} Hz`}
        spoken={`${Math.round(m().brightnessHz)} hertz centre`}
        value={m().brightnessHz}
        min={0}
        max={6000}
        dim={props.dim}
      />
    </div>
  );
}

function VoiceReadingCard(): JSX.Element {
  const app = useApp();
  const on = (): boolean => app.voiceReadingOn();
  const reading = (): VoiceReadingUpdate | null => app.voiceReading();
  const view = (): ReadingView => readingView(reading(), app.engineRunning());
  const phrase = (): string => (reading()?.words ?? []).join(", ");
  const passthrough = (): boolean => {
    const r = reading();
    // The engine measures this by correlating the two taps and puts the
    // answer on the wire. Re-deriving it here from the summary numbers was
    // wrong whenever loudness normalization was on, because that comparison
    // required the levels to match and the loudness stage sits between the
    // chain and the wet tap.
    return !!r && view().hasNumbers && r.wetBypassed;
  };

  return (
    <div
      class="card"
      style={{
        padding: "var(--s4)",
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
      }}
    >
      <div
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          gap: "var(--s3)",
        }}
      >
        <div
          style={{
            display: "flex",
            "align-items": "center",
            gap: "var(--s3)",
            color: "var(--text-mid)",
          }}
        >
          <Sigil name="eye" size={20} style={{ color: "var(--indigo)" }} />
          <div>
            <div style={{ "font-size": "var(--t-sm)", "font-weight": 600 }}>
              Voice reading
            </div>
            <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
              What the signal measures
            </div>
          </div>
        </div>
        <Toggle
          on={on()}
          onChange={(next) => app.setVoiceReadingOn(next)}
          ariaLabel="Voice reading"
        />
      </div>

      {/* The card's ONLY live region. It is always mounted (a region inserted
          together with its text is commonly missed) and holds a sentence that
          changes rarely — never a number, never the phrase. */}
      <div role="status" aria-live="polite">
        <Show when={on()}>
          <span style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
            {readingAnnouncement(view())}
          </span>
        </Show>
      </div>

      <Show when={on()}>
        {/* Visible state chip. Not announced: live/paused alternate every
            second or two while somebody talks. */}
        <div
          data-testid="reading-state"
          style={{
            display: "flex",
            "align-items": "center",
            "justify-content": "space-between",
            gap: "var(--s2)",
            "font-size": "var(--t-xs)",
          }}
        >
          <span
            class="eyebrow"
            style={{
              color: view().kind === "live" ? "var(--indigo)" : "var(--text-lo)",
            }}
          >
            {readingHeadline(view())}
          </span>
          <Show when={view().stale && reading()}>
            {(r) => (
              <span class="tnum" style={{ color: "var(--text-lo)" }}>
                {agoLabel(r().ageMs)}
              </span>
            )}
          </Show>
        </div>

        {/* ---- Your voice: the mic, before any effect. ---- */}
        <div
          data-testid="reading-dry"
          style={{
            display: "flex",
            "flex-direction": "column",
            gap: "var(--s2)",
            padding: "var(--s3)",
            background: "var(--surface-2)",
            border: "1px solid var(--line)",
            "border-radius": "var(--r-sm)",
            // Held numbers must not look live.
            opacity: view().stale ? 0.6 : 1,
          }}
        >
          <span class="eyebrow" style={{ color: "var(--text-mid)" }}>
            Your voice
          </span>
          <Show
            when={view().hasNumbers && reading()}
            fallback={
              <span
                style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}
              >
                Nothing measured yet.
              </span>
            }
          >
            {(r) => (
              <>
                <Show when={phrase()}>
                  <div
                    data-testid="reading-phrase"
                    style={{
                      "font-size": "var(--t-sm)",
                      color: "var(--text-hi)",
                    }}
                  >
                    {phrase()}
                  </div>
                </Show>
                <ReadoutGroup
                  half="your voice"
                  metrics={r().dry}
                  dim={view().stale}
                />
              </>
            )}
          </Show>
        </div>

        {/* ---- After effects: what the call hears (v1.58.0: the dry voice while push-to-modulate has the effects off). ---- */}
        <div
          data-testid="reading-wet"
          style={{
            display: "flex",
            "flex-direction": "column",
            gap: "var(--s2)",
            padding: "var(--s3)",
            background: "var(--surface-2)",
            border: "1px solid var(--line)",
            "border-radius": "var(--r-sm)",
            opacity: view().stale ? 0.6 : 1,
          }}
        >
          <div
            style={{
              display: "flex",
              "align-items": "baseline",
              "justify-content": "space-between",
              gap: "var(--s2)",
            }}
          >
            <span class="eyebrow" style={{ color: "var(--text-mid)" }}>
              After effects
            </span>
            <span
              style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}
            >
              {app.preset().name}
            </span>
          </div>
          <span style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
            What the call hears: your voice after your effects, or without
            them while Push to modulate has them off (Loudness, when it's on,
            still applies). While your effects are on, a preset change moves
            these numbers on its own; the microphone half is unaffected.
          </span>
          <Show
            when={view().hasNumbers && reading()}
            fallback={
              <span
                style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}
              >
                Nothing measured yet.
              </span>
            }
          >
            {(r) => (
              <>
                <Show when={passthrough()}>
                  <div
                    data-testid="reading-passthrough"
                    style={{
                      "font-size": "var(--t-xs)",
                      color: "var(--text-mid)",
                    }}
                  >
                    No measured difference from the microphone half — your
                    effects are off, or they are passing the signal through.
                  </div>
                </Show>
                <ReadoutGroup
                  half="after effects"
                  metrics={r().wet}
                  dim={view().stale}
                />
              </>
            )}
          </Show>
        </div>

        <div
          data-testid="reading-disclaimer"
          style={{
            "font-size": "var(--t-xs)",
            color: "var(--text-lo)",
            "line-height": 1.5,
          }}
        >
          Measured properties of the sound — pitch, range, level, pace, tone.
          Not a reading of the person speaking. Stays in the app; never on the
          stream overlay.
        </div>
      </Show>
    </div>
  );
}

// v1.7.0: output loudness normalization (auto-gain + safety limiter).
// A global, post-chain stage (NOT a per-preset effect) so the perceived
// level stays steady when you switch between a whisper and a roar, and
// never clips. The live "auto gain" readout shows it working.
function LoudnessCard(): JSX.Element {
  const app = useApp();
  const on = () => app.loudnessEnabled();
  const target = () => app.loudnessTarget();
  // Show the live makeup gain only while it's actually doing something.
  const gainDb = () => app.loudnessGainDb();
  const gainLabel = () => {
    const g = gainDb();
    const sign = g >= 0 ? "+" : "−"; // unicode minus
    return `${sign}${Math.abs(g).toFixed(1)} dB`;
  };
  return (
    <div
      class="card"
      style={{
        padding: "var(--s4)",
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
      }}
    >
      <div
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          gap: "var(--s3)",
        }}
      >
        <div
          style={{
            display: "flex",
            "align-items": "center",
            gap: "var(--s3)",
            color: "var(--text-mid)",
          }}
        >
          <Sigil name="wave" size={20} style={{ color: "var(--indigo)" }} />
          <div>
            <div style={{ "font-size": "var(--t-sm)", "font-weight": 600 }}>
              Loudness
            </div>
            <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
              Even out levels across voices
            </div>
          </div>
        </div>
        <Toggle
          on={on()}
          onChange={(next) => app.setLoudnessEnabled(next)}
          ariaLabel="Loudness normalization"
        />
      </div>

      {/* Target level: left = quieter (−30 dB), right = louder (−6 dB). */}
      <div style={{ display: "flex", "align-items": "center", gap: "var(--s3)" }}>
        <span class="eyebrow" style={{ color: "var(--text-lo)", "flex-shrink": 0 }}>
          Target
        </span>
        <input
          type="range"
          min="-30"
          max="-6"
          step="1"
          value={target()}
          disabled={!on()}
          onInput={(e) => app.setLoudnessTarget(parseFloat(e.currentTarget.value))}
          aria-label="Loudness target level"
          style={{ flex: 1, opacity: on() ? 1 : 0.5 }}
        />
        <span
          class="tnum"
          style={{
            "font-size": "var(--t-xs)",
            color: "var(--text-lo)",
            width: "52px",
            "text-align": "right",
          }}
        >
          {target()} dB
        </span>
      </div>

      {/* Live "it's working" readout: the makeup gain currently applied. */}
      <Show when={on() && app.engineRunning()}>
        <div
          style={{
            display: "flex",
            "align-items": "center",
            "justify-content": "space-between",
            "font-size": "var(--t-xs)",
            color: "var(--text-lo)",
          }}
        >
          <span class="eyebrow">Auto gain</span>
          <span class="tnum" style={{ color: "var(--indigo)" }}>
            {gainLabel()}
          </span>
        </div>
      </Show>
    </div>
  );
}

// Phase 16: one-click capture of the post-chain output to a WAV file.
// Recording rides the same processed mono the main output sends, so the
// file is exactly what call participants hear. Only available while the
// engine is running (the writer thread lives for the session).
function RecordCard(): JSX.Element {
  const app = useApp();
  const recording = () => app.isRecording();
  const canRecord = () => app.engineRunning();
  return (
    <div
      class="card"
      style={{
        padding: "var(--s4)",
        display: "flex",
        "align-items": "center",
        "justify-content": "space-between",
        gap: "var(--s3)",
      }}
    >
      <div
        style={{
          display: "flex",
          "align-items": "center",
          gap: "var(--s3)",
          color: "var(--text-mid)",
        }}
      >
        <span
          aria-hidden="true"
          style={{
            width: "14px",
            height: "14px",
            "border-radius": "var(--r-pill)",
            flex: "none",
            background: recording() ? "var(--danger)" : "var(--surface-3)",
            border: recording() ? "none" : "1px solid var(--line)",
            "box-shadow": recording() ? "0 0 10px var(--danger)" : "none",
            animation: recording() ? "breathe 1.4s ease-in-out infinite" : "none",
          }}
        />
        <div>
          <div style={{ "font-size": "var(--t-sm)", "font-weight": 600 }}>
            {recording() ? "Recording…" : "Record"}
          </div>
          <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
            {recording()
              ? "Capturing the modulated output"
              : "Save the modulated output to a WAV"}
          </div>
        </div>
      </div>
      <button
        type="button"
        class="btn btn-secondary"
        disabled={!canRecord() && !recording()}
        title={
          canRecord() || recording()
            ? "Toggle recording the modulated output"
            : "Start the engine to record"
        }
        style={{
          height: "38px",
          ...(recording()
            ? {
                background: "var(--danger-bg)",
                "border-color": "rgba(242, 86, 122, 0.5)",
                color: "var(--danger)",
              }
            : {}),
        }}
        onClick={() => void app.toggleRecording()}
      >
        {recording() ? "Stop" : "Record"}
      </button>
    </div>
  );
}

/** The desktop audio picker's entries: off, then what can be recorded —
 *  and the saved choice even when it can't be, so the card says so rather
 *  than show something else. */
function desktopOptions(
  sources: readonly DesktopAudioSource[] | null,
  saved: string,
): { value: string; label: string; sub?: string }[] {
  const options: { value: string; label: string; sub?: string }[] = [
    { value: "off", label: "No desktop audio" },
  ];
  for (const s of sources ?? []) {
    options.push({
      value: s.id,
      label: s.label + (s.isDefault ? " (default)" : ""),
      sub:
        s.kind === "everything"
          ? "Games, videos and other apps — not Divora itself"
          : s.divoraPlays === "send"
            ? "Divora sends your voice here — can't be recorded"
            : s.divoraPlays === "monitor" || s.divoraPlays === "onlyOutput"
              ? "Divora can play here too"
              : s.divoraPlays === "unknown"
                ? "Everything this device plays — can't tell yet if Divora plays here"
                : "Everything this device plays",
    });
  }
  // The saved choice, even when it isn't offered (yet): the card says so
  // rather than show something else. Before the list is in, it may well be.
  if (saved !== "off" && !(sources ?? []).some((s) => s.id === saved)) {
    options.push({
      value: saved,
      label:
        saved === "everything"
          ? "Everything except Divora"
          : sources
            ? "A playback device chosen earlier"
            : "Your saved playback device",
      ...(sources ? { sub: "Not offered now — takes leave desktop audio out" } : {}),
    });
  }
  return options;
}

/** v1.52.0: record a screen with the modulated voice and the soundboard.
 *  v1.53.0: and what the PC plays, as picked on the card. */
function ScreenRecordCard(): JSX.Element {
  const app = useApp();
  const [aboutOpen, setAboutOpen] = createSignal(false);
  // The screens can change while the app runs; ask each time the card shows.
  onMount(() => void app.refreshCapture());
  const rec = () => app.screenRecording();
  const phase = () => rec().state;
  const busy = () => screenBusy(rec());
  const recording = () => phase() === "recording";
  const status = () => app.captureStatus();
  const unsupported = () => {
    const s = status();
    return s && !s.supported ? (s.reason ?? "Screen recording isn't available.") : null;
  };
  const canStart = () => app.engineRunning() && !unsupported() && app.captureMonitors().length > 0;
  const replayRunning = () => replayOn(app.replayStatus());
  const chosen = () => pickMonitor(app.captureMonitors(), app.screenSettings().monitorId);
  const desktop = () => pickDesktopSource(app.desktopSources() ?? [], app.screenSettings().desktopAudio);
  // While a take runs, what it actually records; before, what it would.
  const subtitle = () => {
    const r = rec();
    if (!busy() && replayRunning()) {
      // A take now would record Replay's desktop audio, not the picker's.
      const id = app.replayStatus().armed?.desktopAudio ?? null;
      if (!id) return screenRecordSubtitle(null);
      return screenRecordSubtitle(
        app.desktopSources()?.find((s) => s.id === id) ?? {
          id,
          label: "the device",
          kind: id === "everything" ? "everything" : "device",
          isDefault: false,
          divoraPlays: "nothing",
        },
      );
    }
    if (!busy()) {
      if (
        app.desktopSources() === null &&
        app.screenSettings().desktopAudio !== "off" &&
        !unsupported()
      ) {
        return "Your voice and soundboard — checking desktop audio…";
      }
      return screenRecordSubtitle(desktop());
    }
    // Left out, lost or stopped for good: the take is voice-only for now.
    if (!r.desktopSourceId || r.desktopError || r.desktopLostAt !== null) {
      return screenRecordSubtitle(null);
    }
    return screenRecordSubtitle({
      id: r.desktopSourceId,
      label: r.desktopSource ?? "the device",
      kind: r.desktopSourceId === "everything" ? "everything" : "device",
      isDefault: false,
      divoraPlays: "nothing",
    });
  };
  const title = (): string => {
    switch (phase()) {
      case "starting":
        return "Starting…";
      case "recording":
        return `Recording · ${formatClock(rec().elapsedMs)}`;
      case "stopping":
        return "Finishing the file…";
      default:
        return "Record screen";
    }
  };
  const savedName = () => {
    const p = app.screenRecordingPath();
    return !busy() && p && !rec().error ? p.split(/[\\/]/).pop() : null;
  };
  return (
    <div
      class="card"
      data-testid="screen-record-card"
      style={{
        padding: "var(--s4)",
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
      }}
    >
      <div
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          gap: "var(--s3)",
        }}
      >
        <div
          style={{
            display: "flex",
            "align-items": "center",
            gap: "var(--s3)",
            color: recording() ? "var(--danger)" : "var(--text-mid)",
            "min-width": 0,
          }}
        >
          <span
            aria-hidden="true"
            style={{
              animation: recording() ? "breathe 1.4s ease-in-out infinite" : "none",
              flex: "none",
            }}
          >
            <Sigil name="screen" size={20} />
          </span>
          <div style={{ "min-width": 0 }}>
            <div
              style={{ "font-size": "var(--t-sm)", "font-weight": 600, color: "var(--text-hi)" }}
              data-testid="screen-record-title"
            >
              {title()}
            </div>
            <div
              style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}
              data-testid="screen-record-subtitle"
            >
              {subtitle()}
            </div>
          </div>
        </div>
        <button
          type="button"
          class="btn btn-secondary"
          data-testid="screen-record-toggle"
          disabled={phase() === "stopping" || (!busy() && !canStart())}
          title={
            busy()
              ? "Stop recording the screen"
              : unsupported() ??
                (app.engineRunning() ? "Record the chosen screen" : "Start the engine to record")
          }
          style={{
            height: "38px",
            flex: "none",
            ...(busy()
              ? {
                  background: "var(--danger-bg)",
                  "border-color": "rgba(242, 86, 122, 0.5)",
                  color: "var(--danger)",
                }
              : {}),
          }}
          onClick={() => void app.toggleScreenRecording()}
        >
          {busy() ? "Stop" : "Record"}
        </button>
      </div>

      {/* While Replay is on, a take shares its capture: the screen and the
          desktop audio are Replay's, so the pickers give way to a line
          saying so. */}
      <Show
        when={!busy() && !unsupported() && !replayRunning() && app.captureMonitors().length > 1}
      >
        <Select
          icon="screen"
          value={chosen()?.id ?? ""}
          options={app.captureMonitors().map((m) => ({ value: m.id, label: monitorLabel(m) }))}
          onChange={(id) => app.setScreenSettings({ monitorId: id })}
        />
      </Show>

      <Show when={!busy() && !unsupported() && !replayRunning()}>
        <div data-testid="screen-record-desktop">
          <Select
            icon="output"
            value={app.screenSettings().desktopAudio}
            options={desktopOptions(app.desktopSources(), app.screenSettings().desktopAudio)}
            onChange={(id) => app.setScreenSettings({ desktopAudio: id })}
          />
        </div>
        <Show when={doubledHint(desktop())}>
          {(hint) => (
            <div style={{ "font-size": "var(--t-xs)", color: "var(--warning)", "line-height": 1.5 }}>
              {hint()}
            </div>
          )}
        </Show>
      </Show>

      <Show when={!unsupported() && replayRunning()}>
        <div
          data-testid="replay-locked"
          style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)", "line-height": 1.5 }}
        >
          {lockedLine(
            app.replayStatus(),
            app.captureMonitors().find((m) => m.id === app.replayStatus().monitorId)?.number ??
              null,
          )}
        </div>
      </Show>

      <Show when={!unsupported()}>
        <ReplayRow desktop={desktop()} />
      </Show>

      <Show when={unsupported()}>
        {(why) => (
          <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)", "line-height": 1.5 }}>
            {why()}
          </div>
        )}
      </Show>
      <Show when={rec().error}>
        {(err) => (
          <div
            data-testid="screen-record-error"
            style={{ "font-size": "var(--t-xs)", color: "var(--danger)", "line-height": 1.5 }}
          >
            {err()}
          </div>
        )}
      </Show>
      <Show when={busy() && rec().audioLostAt !== null}>
        <div
          data-testid="screen-record-audio-lost"
          style={{ "font-size": "var(--t-xs)", color: "var(--warning)", "line-height": 1.5 }}
        >
          {rec().desktopSourceId !== null && rec().desktopLostAt === null && !rec().desktopError
            ? `Your voice and soundboard stopped at ${formatClock((rec().audioLostAt ?? 0) * 1000)} — start the engine to bring them back.`
            : `Audio lost at ${formatClock((rec().audioLostAt ?? 0) * 1000)} — start the engine to bring it back.`}
        </div>
      </Show>
      <Show when={busy() && rec().desktopError}>
        {(why) => (
          <div
            data-testid="screen-record-desktop-error"
            style={{ "font-size": "var(--t-xs)", color: "var(--warning)", "line-height": 1.5 }}
          >
            Recording without desktop audio: {why()}
          </div>
        )}
      </Show>
      <Show when={busy() && rec().desktopLostAt !== null}>
        <div
          data-testid="screen-record-desktop-lost"
          style={{ "font-size": "var(--t-xs)", color: "var(--warning)", "line-height": 1.5 }}
        >
          Desktop audio lost at {formatClock((rec().desktopLostAt ?? 0) * 1000)} — trying again.
        </div>
      </Show>
      <Show when={busy() && rec().desktopDoubled}>
        <div
          data-testid="screen-record-doubled"
          style={{ "font-size": "var(--t-xs)", color: "var(--warning)", "line-height": 1.5 }}
        >
          Divora is set to play its sound on {rec().desktopSource ?? "that device"}, so your
          voice or soundboard is recorded twice whenever it plays there.{" "}
          {doubledHint(desktop()) ?? ""}
        </div>
      </Show>
      <Show when={recording() && quietWarning(rec())}>
        {(warning) => (
          <div
            data-testid="screen-record-desktop-quiet"
            style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)", "line-height": 1.5 }}
          >
            {warning()}
          </div>
        )}
      </Show>
      <Show when={rec().stemsError}>
        {(why) => (
          <div style={{ "font-size": "var(--t-xs)", color: "var(--warning)", "line-height": 1.5 }}>
            The separate WAV files weren't saved in full: {why()}
          </div>
        )}
      </Show>
      <Show when={recording() && rec().droppedFrames > 0}>
        <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
          {rec().droppedFrames} frame{rec().droppedFrames === 1 ? "" : "s"} skipped to keep up
        </div>
      </Show>
      <Show when={savedName()}>
        {(name) => (
          <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
            Saved <span class="mono">{name()}</span>
          </div>
        )}
      </Show>

      <Show when={!busy() && !unsupported()}>
        <button
          type="button"
          class="btn btn-ghost btn-sm"
          style={{ "align-self": "flex-start", padding: "0 4px", height: "auto" }}
          aria-expanded={aboutOpen()}
          onClick={() => setAboutOpen(!aboutOpen())}
        >
          About screen recording
        </button>
        <Show when={aboutOpen()}>
          <ul
            style={{
              margin: 0,
              "padding-left": "18px",
              "font-size": "var(--t-xs)",
              color: "var(--text-lo)",
              "line-height": 1.5,
            }}
          >
            <li>Windows draws a yellow border around the screen while it records.</li>
            <li>Recording the screen Divora is on records Divora's own window too.</li>
            <li>Recordings are SDR for now: an HDR screen may record washed-out.</li>
            <li>
              "Everything except Divora" leaves out Divora's own sound, so your voice isn't
              recorded twice. It also leaves out anything Divora starts itself. Links and folders
              Divora opens are started by the Windows desktop instead, so a browser opened from
              Divora is recorded like any other app. If the desktop can't open one (another
              shell, or Explorer restarting), Divora opens it itself, and a browser it starts
              that way is left out until you close it.
            </li>
            <li>
              Apps that play your voice back themselves, such as a mic test, can be recorded
              too, and Windows' "Listen to this device" may be.
            </li>
            <li>
              Some protected playback, and sound a device's own audio software keeps to itself,
              can record as silence.
            </li>
          </ul>
        </Show>
      </Show>
    </div>
  );
}

/** v1.54.0: Replay on the Record screen card — the last 15–120 s of the
 *  chosen screen kept in memory, saved when something happens.
 *
 *  Until its explainer has been read (and read again once desktop audio is
 *  on, since that can carry a call) the row offers "Set up Replay…" in place
 *  of the switch: a switch that opened text instead of switching would
 *  announce "off" again and leave a screen-reader user unaware anything had
 *  appeared. The explainer is a disclosure, like "About screen recording":
 *  opening it moves focus to its heading, "Not now" returns focus to the
 *  button, and "Turn Replay on" is the one way to pass the gate. */
function ReplayRow(props: { desktop: DesktopAudioSource | null }): JSX.Element {
  const app = useApp();
  const [explainerOpen, setExplainerOpen] = createSignal(false);
  let setupButton: HTMLButtonElement | undefined;
  let heading: HTMLHeadingElement | undefined;
  let toggleWrap: HTMLSpanElement | undefined;
  onMount(() => void app.refreshReplayMemory());

  const ring = () => app.replayStatus();
  const on = () => replayOn(ring());
  const desktopOn = () => app.screenSettings().desktopAudio !== "off";
  // While on, the ring is the authority; the gate only stands in front of
  // turning it on.
  const needsExplainer = () => !on() && explainerNeeded(app.replaySettings(), desktopOn());
  const keptSeconds = () => (on() ? ring().seconds : app.replaySettings().seconds);
  const lengthMemory = (s: number) =>
    app.replayMemory()?.lengths.find((l) => l.seconds === s) ?? null;
  const keptCap = () =>
    on() && ring().capBytes > 0 ? ring().capBytes : (lengthMemory(keptSeconds())?.capBytes ?? null);
  const unsupportedReason = () =>
    ring().supported ? null : (ring().reason ?? "Replay isn't available on this PC.");
  const sub = () => {
    const cap = keptCap();
    return `Keeps the last ${keptSeconds()} s${cap ? ` · uses ${memoryLabel(cap)} of memory` : ""}`;
  };
  // Only the memory figures shape the options, so the buttons aren't
  // rebuilt (and focus lost) by the once-a-second status.
  const lengthOptions = createMemo(() => {
    const memory = app.replayMemory();
    return REPLAY_LENGTHS.map((s) => {
      const len = memory?.lengths.find((l) => l.seconds === s);
      const why = len && memory ? lengthTitle(len, memory.ceilingBytes) : null;
      return { value: String(s), label: `${s} s`, disabled: why !== null, title: why ?? undefined };
    });
  });

  const openExplainer = (): void => {
    const next = !explainerOpen();
    setExplainerOpen(next);
    if (next) queueMicrotask(() => heading?.focus());
  };
  const notNow = (): void => {
    setExplainerOpen(false);
    setupButton?.focus();
  };
  const turnOn = (): void => {
    app.markReplayExplained(desktopOn());
    setExplainerOpen(false);
    // The switch has taken the button's place: keep focus in the row.
    queueMicrotask(() => toggleWrap?.querySelector<HTMLButtonElement>("button")?.focus());
    void app.armReplay();
  };

  const line = { "font-size": "var(--t-xs)", "line-height": 1.5 } as const;

  return (
    <div
      data-testid="replay-row"
      style={{
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
        "border-top": "1px solid var(--line)",
        "padding-top": "var(--s3)",
      }}
    >
      <div
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          gap: "var(--s3)",
        }}
      >
        <div
          style={{
            display: "flex",
            "align-items": "center",
            gap: "var(--s3)",
            color: on() ? "var(--danger)" : "var(--text-mid)",
            "min-width": 0,
          }}
        >
          <span aria-hidden="true" style={{ flex: "none" }}>
            <Sigil name="replay" size={20} />
          </span>
          <div style={{ "min-width": 0 }}>
            <div style={{ "font-size": "var(--t-sm)", "font-weight": 600, color: "var(--text-hi)" }}>
              Replay
            </div>
            <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>{sub()}</div>
          </div>
        </div>
        <Show when={!needsExplainer()}>
          <span data-testid="replay-toggle" ref={toggleWrap} style={{ flex: "none" }}>
            <Toggle
              on={on()}
              ariaLabel="Replay"
              disabled={!!unsupportedReason() || (!on() && !app.engineRunning())}
              title={
                unsupportedReason() ??
                (on() || app.engineRunning() ? undefined : "Start the engine to turn Replay on")
              }
              onChange={(next) => void (next ? app.armReplay() : app.disarmReplay())}
            />
          </span>
        </Show>
      </div>

      <Show when={needsExplainer()}>
        <button
          ref={setupButton}
          type="button"
          class="btn btn-secondary btn-sm"
          data-testid="replay-setup"
          style={{ "align-self": "flex-start" }}
          aria-expanded={explainerOpen()}
          aria-controls="replay-explainer"
          onClick={openExplainer}
        >
          Set up Replay…
        </button>
        <Show when={explainerOpen()}>
          <div
            id="replay-explainer"
            data-testid="replay-explainer"
            role="region"
            aria-labelledby="replay-explainer-heading"
            style={{
              display: "flex",
              "flex-direction": "column",
              gap: "var(--s2)",
              padding: "var(--s3)",
              background: "var(--surface-2)",
              border: "1px solid var(--line)",
              "border-radius": "var(--r-sm)",
            }}
          >
            <h3
              id="replay-explainer-heading"
              ref={heading}
              tabindex="-1"
              style={{ margin: 0, "font-size": "var(--t-sm)", "font-weight": 600, color: "var(--text-hi)" }}
            >
              What Replay keeps
            </h3>
            <ul
              style={{
                margin: 0,
                "padding-left": "18px",
                ...line,
                color: "var(--text-mid)",
                display: "flex",
                "flex-direction": "column",
                gap: "var(--s1)",
              }}
            >
              <li>
                The last {app.replaySettings().seconds} s of this screen, in memory: the picture,
                your voice and soundboard as your call hears them, and — whenever this card records
                desktop audio — sound from other apps, calls included.{" "}
                {desktopSettingWords(
                  app.screenSettings().desktopAudio,
                  props.desktop,
                  app.desktopSources(),
                )}
              </li>
              <Show when={lengthMemory(app.replaySettings().seconds)}>
                {(len) => <li>It uses {memoryLabel(len().capBytes)} of memory.</li>}
              </Show>
              <li>
                <strong style={{ color: "var(--text-hi)" }}>
                  Divora writes no picture or sound to disk until you press Save.
                </strong>
              </li>
              <li>
                Windows draws a yellow border around this screen while Replay is on. Divora leaves
                it on, so you can always tell your screen is being kept. The tray icon changes too.
              </li>
              <li>It needs a hardware video encoder, such as a graphics card's.</li>
            </ul>
            <div style={{ display: "flex", gap: "var(--s2)" }}>
              <button type="button" class="btn btn-primary btn-sm" onClick={turnOn}>
                Turn Replay on
              </button>
              <button type="button" class="btn btn-ghost btn-sm" onClick={notNow}>
                Not now
              </button>
            </div>
          </div>
        </Show>
      </Show>

      <div data-testid="replay-length">
        <Segmented<string>
          options={lengthOptions()}
          value={String(app.replaySettings().seconds)}
          onChange={(v) => void app.setReplaySeconds(sanitizeReplaySeconds(Number(v)))}
        />
      </div>

      <Show when={on()}>
        <button
          type="button"
          class="btn btn-secondary"
          data-testid="replay-save"
          disabled={ring().saving}
          style={{ height: "34px" }}
          onClick={() => void app.saveReplay()}
        >
          {saveLabel(ring())}
        </button>
      </Show>

      <Show when={unsupportedReason()}>
        {(why) => (
          <div data-testid="replay-reason" style={{ ...line, color: "var(--text-lo)" }}>
            {why()}
          </div>
        )}
      </Show>
      <Show when={ring().state === "held"}>
        <div data-testid="replay-held" style={{ ...line, color: "var(--warning)" }}>
          {heldCopy(ring().heldReason)}
        </div>
      </Show>
    </div>
  );
}

interface DbReadoutProps {
  levels: { rms: number; peak: number };
}

function DbReadout(props: DbReadoutProps): JSX.Element {
  const fmt = (v: number) => {
    if (v <= 1e-6) return "−∞";
    const db = 20 * Math.log10(v);
    return `${db.toFixed(0)} dB`;
  };
  return (
    <div
      style={{
        "font-family": "var(--font-mono)",
        "font-size": "var(--t-xs)",
        color: "var(--text-lo)",
      }}
      class="tnum"
    >
      {fmt(props.levels.peak)}
    </div>
  );
}

// Silence unused-import lint when Phase 3 hasn't wired ID inference yet.
void (null as EffectId | null);
