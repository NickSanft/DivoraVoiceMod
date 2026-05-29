// Mixer — Phase 3 lights up the spell circle and the selected-rune
// inspector. Effects orbit a glowing voice core; threads of light
// connect enabled effects to the core; the inspector lets the user
// tweak the focused effect live.

import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import { GlyphCastOverlay } from "../components/GlyphCastOverlay";
import { Inspector } from "../components/Inspector";
import { Kbd } from "../components/Kbd";
import { VMeter } from "../components/Meters";
import { Segmented } from "../components/Segmented";
import { Sigil, type SigilName } from "../components/Sigil";
import { SpellCastReveal } from "../components/SpellCastReveal";
import { SpellCircle } from "../components/SpellCircle";
import { Toggle } from "../components/Toggle";
import { statusMeta } from "../shell/statusMeta";
import { useApp } from "../stores/app";
import type { EffectId, GlyphId, Preset, PtmMode } from "../types";

/**
 * True when a pointerdown's target should NOT trigger a cast gesture.
 *
 * The mockup says drag on EMPTY space starts the cast. We walk up from
 * the event target, stopping when we hit either the cast root or any
 * interactive ancestor (button, input, slider, contenteditable, card,
 * or anything explicitly tagged `data-cast-block`). If we find one,
 * the click belongs to that control — leave it alone.
 *
 * Exported so the empty-space-cast unit test can drive it directly
 * without spinning up a full app store.
 */
export function isInteractiveAncestor(
  start: Element | null,
  stopAt: Element,
): boolean {
  let cur: Element | null = start;
  while (cur && cur !== stopAt) {
    const tag = cur.tagName;
    if (
      tag === "BUTTON" ||
      tag === "INPUT" ||
      tag === "SELECT" ||
      tag === "TEXTAREA" ||
      tag === "A"
    ) {
      return true;
    }
    const role = cur.getAttribute("role");
    if (role === "button" || role === "slider" || role === "switch") {
      return true;
    }
    // Read both the live `isContentEditable` getter (browsers) and the
    // raw attribute (jsdom + defensive coverage when the host element
    // is missing the HTMLElement prototype methods).
    if (cur instanceof HTMLElement && cur.isContentEditable) return true;
    const ce = cur.getAttribute("contenteditable");
    if (ce !== null && ce !== "false") return true;
    if (cur.classList?.contains("card")) return true;
    if (cur.hasAttribute("data-cast-block")) return true;
    cur = cur.parentElement;
  }
  return false;
}

export function MixerScreen(): JSX.Element {
  const app = useApp();
  const activeCount = () => app.chain().filter((c) => c.enabled).length;
  const totalCount = () => app.chain().length;
  const [castOpen, setCastOpen] = createSignal(false);
  const [castMessage, setCastMessage] = createSignal<string | null>(null);
  /** Pointer seed forwarded to the overlay when the user starts the
   *  cast by dragging on the Mixer's empty space rather than pressing
   *  the explicit Cast button / G hotkey. Cleared on classify/cancel. */
  const [seedPointer, setSeedPointer] = createSignal<
    { pointerId: number; clientX: number; clientY: number } | null
  >(null);
  /** Active SPELL CAST reveal — populated when a glyph matches a bound
   *  preset; cleared once the reveal animation finishes. */
  const [reveal, setReveal] = createSignal<Preset | null>(null);
  let messageTimeout: number | undefined;
  let castRootRef: HTMLDivElement | undefined;

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

  const onClassified = (glyph: GlyphId | null): void => {
    setCastOpen(false);
    setSeedPointer(null);
    if (!glyph) {
      flashMessage("Glyph not recognised — try again");
      return;
    }
    const presetId = app.glyphs[glyph];
    const target = app.presets().find((p) => p.id === presetId);
    if (!target) {
      flashMessage(`No preset bound to ${glyph}`);
      return;
    }
    // Switch the preset immediately (so the chain is correct by the
    // time the reveal animation finishes) and trigger the SPELL CAST
    // ceremony over the Mixer.
    app.usePreset(target.id);
    setReveal(target);
  };

  const onCancelCast = (): void => {
    setCastOpen(false);
    setSeedPointer(null);
  };

  /**
   * Drag-from-empty-space cast trigger. Matches the prototype: any
   * left-button pointerdown that doesn't land on a UI control opens
   * the cast overlay and immediately seeds it with the originating
   * pointer event so the user's drag continues without a second
   * mouse press.
   *
   * No-ops if the overlay is already open (the cast button / G hotkey
   * just opened it) or the user is clicking inside an interactive
   * element identified by `isInteractiveAncestor`.
   */
  const onMixerPointerDown = (e: PointerEvent): void => {
    if (castOpen()) return;
    if (e.button !== 0) return;
    if (!castRootRef) return;
    const target = e.target;
    if (!(target instanceof Element)) return;
    if (isInteractiveAncestor(target, castRootRef)) return;
    e.preventDefault();
    setSeedPointer({
      pointerId: e.pointerId,
      clientX: e.clientX,
      clientY: e.clientY,
    });
    setCastOpen(true);
  };

  // Keyboard shortcut "G" enters cast mode unless a field is focused.
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "g" && e.key !== "G") return;
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable)
      ) {
        return;
      }
      e.preventDefault();
      setCastOpen(true);
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => {
      window.removeEventListener("keydown", onKey);
      if (messageTimeout !== undefined) {
        window.clearTimeout(messageTimeout);
      }
    });
  });

  return (
    <div
      ref={castRootRef}
      style={{
        height: "100%",
        display: "flex",
        "flex-direction": "column",
        padding: "20px 24px",
        gap: "var(--s5)",
        // Drag on empty space to cast — the pointerdown handler walks
        // up from `e.target` to filter out controls before firing.
        "touch-action": "none",
      }}
      onPointerDown={onMixerPointerDown}
    >
      <PresetHeader
        activeCount={activeCount()}
        totalCount={totalCount()}
        onCast={() => setCastOpen(true)}
      />
      <Show when={castOpen()}>
        <GlyphCastOverlay
          onClassified={onClassified}
          onCancel={onCancelCast}
          seedPointer={seedPointer() ?? undefined}
        />
      </Show>
      <Show when={reveal()} keyed>
        {(target) => (
          <SpellCastReveal
            glyph={target.glyph as SigilName}
            name={target.name}
            color={target.color}
            tag={target.tag}
            onDone={() => setReveal(null)}
          />
        )}
      </Show>
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
  onCast: () => void;
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
            {(info) => <span> · routed via {info().outputName}</span>}
          </Show>
        </div>
      </div>
      <div style={{ flex: 1 }} />
      <Button
        variant="ghost"
        size="sm"
        icon="bolt"
        onClick={props.onCast}
        title="Cast a glyph (G)"
      >
        Cast
      </Button>
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
    </div>
  );
}

function PushToModulateCard(): JSX.Element {
  const app = useApp();
  const pressed = () => app.ui.pressed;
  return (
    <div class="card" style={{ padding: "var(--s4)" }}>
      <div
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          gap: "var(--s2)",
          "margin-bottom": "var(--s3)",
        }}
      >
        <span class="eyebrow">Push to modulate</span>
        <Kbd>{app.ui.ptmKey}</Kbd>
      </div>
      <Segmented<PtmMode>
        options={[
          { value: "apply", label: "Hold to apply" },
          { value: "bypass", label: "Hold to bypass" },
        ]}
        value={app.ui.ptmMode}
        onChange={(v) => app.setUi("ptmMode", v)}
      />
      <button
        type="button"
        class="btn btn-secondary btn-block"
        style={{
          "margin-top": "var(--s3)",
          height: "44px",
          background: pressed() ? "var(--accent-bg)" : undefined,
          "border-color": pressed() ? "var(--line-glow)" : undefined,
          color: pressed() ? "var(--indigo)" : undefined,
        }}
        onPointerDown={() => app.setUi("pressed", true)}
        onPointerUp={() => app.setUi("pressed", false)}
        onPointerLeave={() => app.setUi("pressed", false)}
      >
        {pressed()
          ? app.ui.ptmMode === "apply"
            ? "APPLYING"
            : "BYPASSED"
          : `Hold to test · press ${app.ui.ptmKey}`}
      </button>
    </div>
  );
}

function MonitorCard(): JSX.Element {
  const app = useApp();
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
