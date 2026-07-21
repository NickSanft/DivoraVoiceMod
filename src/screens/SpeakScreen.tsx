// Speak (v1.17.0) — type text, pick a preset voice, and synthesize speech
// that plays through the output (mixed with the live mic via the soundboard
// seam, so a Discord/stream listener hears it too).
//
// This is the **scaffolding**: the workspace, voice picker, and command
// surface are live, but synthesis is gated behind "voice not installed"
// until the Kokoro model + voice pack + espeak-ng are bundled. Pressing
// Speak before then surfaces a graceful notice rather than failing silently.

import { For, Show, createMemo, createSignal, onMount, type JSX } from "solid-js";
import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import { Sigil } from "../components/Sigil";
import { openSpeakClipsFolder } from "../audio/api";
import { useApp } from "../stores/app";

export function SpeakScreen(): JSX.Element {
  const app = useApp();
  const [cloneName, setCloneName] = createSignal("");

  // v1.43.0: inline rename of a cloned voice. One card edits at a time. Enter
  // or ✓ commits, Escape or ✕ abandons — deliberately no save-on-blur, so
  // clicking cancel can't race a blur handler and save what you just rejected.
  const [renamingId, setRenamingId] = createSignal<string | null>(null);
  const [renameDraft, setRenameDraft] = createSignal("");
  const MAX_VOICE_NAME = 64; // matches MAX_CLONE_NAME_LEN in the backend

  const beginRename = (id: string, current: string): void => {
    setRenameDraft(current);
    setRenamingId(id);
  };
  // Closing the editor unmounts the focused input, which would drop focus to
  // <body> and strand a keyboard user at the top of the document — so hand it
  // back to the pencil that opened it. Re-queried rather than held as a ref
  // because the row is rebuilt whenever the voice list refreshes.
  const focusRenameTrigger = (id: string): void => {
    queueMicrotask(() => {
      document
        .querySelector<HTMLElement>(`[data-rename-for="${id}"]`)
        ?.focus();
    });
  };
  const cancelRename = (id?: string): void => {
    setRenamingId(null);
    setRenameDraft("");
    if (id) focusRenameTrigger(id);
  };
  const commitRename = (id: string): void => {
    const next = renameDraft().trim();
    if (!next) return; // keep the editor open rather than saving an empty name
    void (async () => {
      // Only close the editor if the backend accepted it; on failure the
      // reason renders inline in the editor and the editor stays open.
      if (await app.renameClonedVoice(id, next)) cancelRename(id);
    })();
  };

  onMount(() => {
    // Load cloned voices first so a persisted cloned selection isn't reset.
    void (async () => {
      await app.refreshClonedVoices();
      await app.refreshTtsVoices();
      await app.refreshCloneModelsStatus();
      await app.refreshVoxcpmStatus();
      await app.refreshVoxcpmGpuStatus();
      await app.refreshSpeakClips();
    })();
  });

  const openClipsFolder = (): void => {
    void openSpeakClipsFolder();
  };
  const clipDate = (ms: number): string => {
    try {
      return new Date(ms).toLocaleString();
    } catch {
      return "";
    }
  };

  const anyInstalled = (): boolean => app.ttsVoices().some((v) => v.installed);
  const hasText = (): boolean => app.ttsText().trim().length > 0;

  // Best-of-N tiers for VoxCPM cloned voices (~7 s/take on CPU). The backend
  // generates N takes and a speaker-verification reranker keeps the one closest
  // to your reference; more takes = more faithful but slower (v1.25.0).
  const QUALITY_TIERS = [
    { label: "Fast", n: 1, hint: "1 take · quickest" },
    { label: "Balanced", n: 3, hint: "3 takes · recommended" },
    { label: "Best", n: 6, hint: "6 takes · most faithful" },
  ] as const;
  const selectedClone = createMemo(() =>
    app.clonedVoices().find((v) => v.id === app.selectedTtsVoice()),
  );
  const isVoxcpmSelected = (): boolean => selectedClone()?.engine === "voxcpm";

  // Speak button label: shows best-of-N take progress while a multi-take VoxCPM
  // clone generates ("Take 2 of 6…"), so the wait reads as work, not a freeze.
  const speakLabel = (): string => {
    if (!app.synthesizing()) return "Speak";
    const p = app.ttsProgress();
    return p && p.total > 1 ? `Take ${p.done} of ${p.total}…` : "Synthesizing…";
  };

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
              fallback={
                <>
                  <button
                    type="button"
                    class="btn btn-primary btn-sm"
                    onClick={() => void app.startEngine()}
                  >
                    Start engine
                  </button>{" "}
                  to route it into a call or stream.
                </>
              }
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

        {/* Your voices (cloned) — v1.20.0 */}
        <Show when={anyInstalled()}>
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
              Your voices
            </span>

            <Show when={app.clonedVoices().length > 0}>
              <div
                role="radiogroup"
                aria-label="Your voices"
                style={{
                  display: "grid",
                  "grid-template-columns": "repeat(auto-fill, minmax(220px, 1fr))",
                  gap: "var(--s3)",
                }}
              >
                <For each={app.clonedVoices()}>
                  {(voice) => {
                    const selected = (): boolean =>
                      app.selectedTtsVoice() === voice.id;
                    return (
                      <div
                        style={{
                          display: "flex",
                          "align-items": "stretch",
                          gap: "var(--s1)",
                          "border-radius": "var(--r-md)",
                          border: `1px solid ${selected() ? "var(--accent)" : "var(--line)"}`,
                          background: selected()
                            ? "var(--accent-soft, var(--surface-2))"
                            : "var(--surface-1)",
                          overflow: "hidden",
                        }}
                      >
                        <Show
                          when={renamingId() !== voice.id}
                          fallback={
                            <div
                              style={{
                                display: "flex",
                                "flex-direction": "column",
                                gap: "var(--s1)",
                                flex: 1,
                                "min-width": 0,
                                padding: "var(--s2) var(--s3)",
                              }}
                            >
                              <div
                                style={{
                                  display: "flex",
                                  "align-items": "center",
                                  gap: "var(--s2)",
                                  "min-width": 0,
                                }}
                              >
                                <input
                                  type="text"
                                  value={renameDraft()}
                                  onInput={(e) =>
                                    setRenameDraft(e.currentTarget.value)
                                  }
                                  onKeyDown={(e) => {
                                    if (e.key === "Enter") {
                                      e.preventDefault();
                                      commitRename(voice.id);
                                    } else if (e.key === "Escape") {
                                      e.preventDefault();
                                      cancelRename(voice.id);
                                    }
                                  }}
                                  ref={(el) => {
                                    // Focus + select so typing replaces the old
                                    // name immediately.
                                    queueMicrotask(() => {
                                      el.focus();
                                      el.select();
                                    });
                                  }}
                                  maxlength={MAX_VOICE_NAME}
                                  aria-label={`Rename ${voice.name}`}
                                  style={{
                                    flex: 1,
                                    "min-width": 0,
                                    padding: "var(--s1) var(--s2)",
                                    "border-radius": "var(--r-sm)",
                                    border: "1px solid var(--accent)",
                                    background: "var(--surface-1)",
                                    color: "var(--text-high)",
                                    "font-family": "inherit",
                                    "font-size": "var(--t-sm)",
                                  }}
                                />
                                <button
                                  type="button"
                                  aria-label="Save name"
                                  title="Save"
                                  disabled={renameDraft().trim().length === 0}
                                  onClick={() => commitRename(voice.id)}
                                  style={{
                                    display: "flex",
                                    background: "transparent",
                                    border: "none",
                                    color:
                                      renameDraft().trim().length === 0
                                        ? "var(--text-low)"
                                        : "var(--accent)",
                                    cursor:
                                      renameDraft().trim().length === 0
                                        ? "default"
                                        : "pointer",
                                  }}
                                >
                                  <Sigil name="check" size={15} />
                                </button>
                                <button
                                  type="button"
                                  aria-label="Cancel rename"
                                  title="Cancel"
                                  onClick={() => cancelRename(voice.id)}
                                  style={{
                                    display: "flex",
                                    background: "transparent",
                                    border: "none",
                                    color: "var(--text-low)",
                                    cursor: "pointer",
                                  }}
                                >
                                  <Sigil name="x" size={15} />
                                </button>
                              </div>
                              {/* Why a rejected rename failed, right where the
                                  correction happens — the shared clone-error
                                  line sits below the whole grid. */}
                              <Show when={app.cloneError()}>
                                <span
                                  role="alert"
                                  style={{
                                    "font-size": "0.72rem",
                                    color: "var(--text-mid)",
                                  }}
                                >
                                  {app.cloneError()}
                                </span>
                              </Show>
                            </div>
                          }
                        >
                          <button
                            type="button"
                            role="radio"
                            aria-checked={selected()}
                            onClick={() => app.setSelectedTtsVoice(voice.id)}
                            style={{
                              display: "flex",
                              "align-items": "center",
                              gap: "var(--s2)",
                              flex: 1,
                              padding: "var(--s3) var(--s4)",
                              background: "transparent",
                              border: "none",
                              color: "var(--text-high)",
                              cursor: "pointer",
                              "text-align": "left",
                            }}
                          >
                            <span
                              style={{
                                color: selected() ? "var(--accent)" : "var(--text-low)",
                                display: "flex",
                              }}
                            >
                              <Sigil name="mic" size={16} />
                            </span>
                            <span
                              style={{
                                display: "flex",
                                "flex-direction": "column",
                                "min-width": 0,
                              }}
                            >
                              <span
                                style={{
                                  display: "flex",
                                  "align-items": "center",
                                  gap: "var(--s2)",
                                  "min-width": 0,
                                }}
                              >
                                <span
                                  style={{
                                    overflow: "hidden",
                                    "text-overflow": "ellipsis",
                                    "white-space": "nowrap",
                                  }}
                                >
                                  {voice.name}
                                </span>
                                {/* v1.25.0: which engine cloned this voice —
                                    accent (VoxCPM) supports the Clone quality
                                    control; timbre (OpenVoice) does not. */}
                                <Show when={voice.engine === "voxcpm"} fallback={
                                  <Badge tone="info">timbre</Badge>
                                }>
                                  <Badge tone="accent">accent</Badge>
                                </Show>
                              </span>
                              <Show when={voice.baseName}>
                                <span
                                  style={{
                                    "font-size": "0.72rem",
                                    color: "var(--text-low)",
                                  }}
                                >
                                  based on {voice.baseName}
                                </span>
                              </Show>
                            </span>
                          </button>
                          {/* v1.43.0: rename — swaps this card into an inline
                              editor. Only the label changes; the voice keeps its
                              id, so the selection and saved clips are unaffected. */}
                          <button
                            type="button"
                            aria-label={`Rename ${voice.name}`}
                            title="Rename this voice"
                            data-rename-for={voice.id}
                            onClick={() => beginRename(voice.id, voice.name)}
                            style={{
                              display: "flex",
                              "align-items": "center",
                              padding: "0 var(--s3)",
                              background: "transparent",
                              border: "none",
                              "border-left": "1px solid var(--line)",
                              color: "var(--text-low)",
                              cursor: "pointer",
                            }}
                          >
                            <Sigil name="pencil" size={14} />
                          </button>
                          <button
                            type="button"
                            aria-label={`Delete ${voice.name}`}
                            title="Delete this voice"
                            onClick={() => void app.removeClonedVoice(voice.id)}
                            style={{
                              display: "flex",
                              "align-items": "center",
                              padding: "0 var(--s3)",
                              background: "transparent",
                              border: "none",
                              "border-left": "1px solid var(--line)",
                              color: "var(--text-low)",
                              cursor: "pointer",
                            }}
                          >
                            <Sigil name="trash" size={14} />
                          </button>
                        </Show>
                      </div>
                    );
                  }}
                </For>
              </div>
            </Show>

            {/* Add your voice — gated on the cloning models being downloaded */}
            <Show
              when={app.cloneModelsReady()}
              fallback={
                <div
                  style={{
                    display: "flex",
                    "align-items": "center",
                    gap: "var(--s3)",
                    "flex-wrap": "wrap",
                  }}
                >
                  <Button
                    variant="secondary"
                    icon="download"
                    disabled={app.cloneDownloading()}
                    onClick={() => void app.downloadCloneModels()}
                  >
                    {app.cloneDownloading()
                      ? `Downloading… ${app.cloneDownloadPct()}%`
                      : "Download voice-cloning models (~157 MB)"}
                  </Button>
                  <span
                    style={{ "font-size": "var(--t-xs)", color: "var(--text-low)" }}
                  >
                    One-time, on-device — needed to add your own voice.
                  </span>
                </div>
              }
            >
              <div
                style={{
                  display: "flex",
                  "align-items": "center",
                  gap: "var(--s2)",
                  "flex-wrap": "wrap",
                }}
              >
                <input
                  type="text"
                  value={cloneName()}
                  onInput={(e) => setCloneName(e.currentTarget.value)}
                  placeholder="Name your voice, then record or pick a 20–30s clip"
                  maxlength={MAX_VOICE_NAME}
                  disabled={app.cloning() || app.recordingVoice()}
                  style={{
                    flex: "1 1 240px",
                    padding: "var(--s2) var(--s3)",
                    "border-radius": "var(--r-md)",
                    border: "1px solid var(--line)",
                    background: "var(--surface-1)",
                    color: "var(--text-high)",
                    "font-family": "inherit",
                    "font-size": "var(--t-sm)",
                  }}
                />
                <Button
                  variant={app.recordingVoice() ? "primary" : "secondary"}
                  icon={app.recordingVoice() ? "stop" : "mic"}
                  disabled={
                    app.cloning() ||
                    (!app.recordingVoice() && cloneName().trim().length === 0)
                  }
                  onClick={() => {
                    if (app.recordingVoice()) {
                      void app
                        .stopRecordingVoice(cloneName())
                        .then(() => setCloneName(""));
                    } else {
                      void app.startRecordingVoice();
                    }
                  }}
                >
                  {app.recordingVoice()
                    ? "Stop & save"
                    : app.cloning()
                      ? "Saving…"
                      : "Record"}
                </Button>
                <Button
                  variant="secondary"
                  icon="folder"
                  disabled={
                    app.cloning() ||
                    app.recordingVoice() ||
                    cloneName().trim().length === 0
                  }
                  onClick={() => {
                    void app.addClonedVoice(cloneName()).then(() => setCloneName(""));
                  }}
                >
                  Pick a clip
                </Button>
              </div>
              <Show when={app.recordingVoice()}>
                <Show
                  when={app.voxcpmAvailable() && app.voxcpmReadPrompt()}
                  fallback={
                    <span
                      role="status"
                      style={{ "font-size": "var(--t-xs)", color: "var(--text-low)" }}
                    >
                      Recording… speak naturally for 20–30 seconds, then Stop &amp; save.
                    </span>
                  }
                >
                  {/* VoxCPM (accent-preserving) needs a known transcript, so the
                      user reads this fixed sentence. */}
                  <span
                    role="status"
                    style={{ "font-size": "var(--t-xs)", color: "var(--text-mid)" }}
                  >
                    Recording… read this aloud, then Stop &amp; save:
                    <span
                      style={{
                        display: "block",
                        "margin-top": "var(--s1)",
                        color: "var(--text-high)",
                        "font-style": "italic",
                      }}
                    >
                      “{app.voxcpmReadPrompt()}”
                    </span>
                  </span>
                </Show>
              </Show>
              {/* Accent-preserving upgrade: offer the one-time VoxCPM download
                  when cloning is available but the accent models aren't. */}
              <Show when={!app.voxcpmAvailable()}>
                <div
                  style={{
                    display: "flex",
                    "align-items": "center",
                    gap: "var(--s3)",
                    "flex-wrap": "wrap",
                  }}
                >
                  <Button
                    variant="secondary"
                    icon="download"
                    disabled={app.cloneDownloading()}
                    onClick={() => void app.downloadVoxcpmModels()}
                  >
                    {app.cloneDownloading()
                      ? `Downloading… ${app.cloneDownloadPct()}%`
                      : "Accent-preserving clones (~1.6 GB)"}
                  </Button>
                  <span
                    style={{ "font-size": "var(--t-xs)", color: "var(--text-low)" }}
                  >
                    One-time download — recorded voices then keep your accent
                    (without it, clones are timbre-only).
                  </span>
                </div>
              </Show>
            </Show>

            <Show when={app.cloneError()}>
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
                {app.cloneError()}
              </span>
            </Show>
          </div>
        </Show>

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

          {/* v1.29.0: hear Speak in the monitor, independent of the Mixer's
              mic monitor — disabling that no longer silences Speak previews. */}
          <label
            style={{
              display: "flex",
              "align-items": "center",
              gap: "var(--s2)",
              cursor: "pointer",
              "user-select": "none",
            }}
            title="Hear Speak in your monitor — independent of the Mixer's mic monitor"
          >
            <input
              type="checkbox"
              checked={app.speakMonitor()}
              onChange={(e) => app.setSpeakMonitor(e.currentTarget.checked)}
              style={{ "accent-color": "var(--accent)" }}
            />
            <span style={{ "font-size": "var(--t-sm)", color: "var(--text-high)" }}>
              Monitor
            </span>
          </label>
        </div>

        {/* Clone quality (best-of-N) — VoxCPM cloned voices only, v1.25.0 */}
        <Show when={isVoxcpmSelected()}>
          <div
            style={{
              display: "flex",
              "align-items": "center",
              gap: "var(--s3)",
              "flex-wrap": "wrap",
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
              Clone quality
            </span>
            <div
              role="radiogroup"
              aria-label="Clone quality"
              style={{
                display: "inline-flex",
                border: "1px solid var(--line)",
                "border-radius": "var(--r-md)",
                overflow: "hidden",
              }}
            >
              <For each={QUALITY_TIERS}>
                {(tier) => {
                  const active = (): boolean => app.cloneQuality() === tier.n;
                  return (
                    <button
                      type="button"
                      role="radio"
                      aria-checked={active()}
                      title={tier.hint}
                      onClick={() => app.setCloneQuality(tier.n)}
                      style={{
                        padding: "var(--s2) var(--s4)",
                        "font-size": "var(--t-sm)",
                        border: "none",
                        cursor: "pointer",
                        background: active() ? "var(--accent)" : "transparent",
                        color: active() ? "var(--on-accent, #fff)" : "var(--text-mid)",
                      }}
                    >
                      {tier.label}
                    </button>
                  );
                }}
              </For>
            </div>
            <span style={{ "font-size": "var(--t-xs)", color: "var(--text-low)" }}>
              more takes sound more like you, but take longer
            </span>
          </div>
        </Show>

        {/* Experimental DirectML GPU — VoxCPM cloned voices only, v1.30.0. Opt-in:
            needs the ~1.2 GB fp16 GPU model; falls back to CPU if DirectML fails. */}
        <Show when={isVoxcpmSelected() && app.voxcpmGpuSupported()}>
          <div
            style={{
              display: "flex",
              "align-items": "center",
              gap: "var(--s3)",
              "flex-wrap": "wrap",
            }}
          >
            <Show
              when={app.voxcpmGpuModelPresent()}
              fallback={
                <button
                  type="button"
                  disabled={app.cloneDownloading()}
                  onClick={() => void app.downloadVoxcpmGpuModel()}
                  style={{
                    padding: "var(--s2) var(--s4)",
                    "font-size": "var(--t-sm)",
                    border: "1px solid var(--line)",
                    "border-radius": "var(--r-md)",
                    background: "transparent",
                    color: "var(--text-mid)",
                    cursor: app.cloneDownloading() ? "default" : "pointer",
                  }}
                >
                  {app.cloneDownloading()
                    ? `Downloading… ${app.cloneDownloadPct()}%`
                    : "Enable GPU (download ~1.2 GB)"}
                </button>
              }
            >
              <label
                style={{
                  display: "flex",
                  "align-items": "center",
                  gap: "var(--s2)",
                  cursor: "pointer",
                  "user-select": "none",
                }}
                title="Run the decode step on your GPU via DirectML — falls back to CPU if unavailable"
              >
                <input
                  type="checkbox"
                  checked={app.cloneGpu()}
                  onChange={(e) => app.setCloneGpu(e.currentTarget.checked)}
                  style={{ "accent-color": "var(--accent)" }}
                />
                <span style={{ "font-size": "var(--t-sm)", color: "var(--text-high)" }}>
                  Use GPU (experimental)
                </span>
              </label>
              <span style={{ "font-size": "var(--t-xs)", color: "var(--text-low)" }}>
                ~1.3× faster decode on a DX12 GPU
              </span>
            </Show>
          </div>
        </Show>

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
            {speakLabel()}
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

        {/* Saved clips (v1.28.0): every generated TTS clip, replayable. Shown
            whenever Speak is usable so "Open folder" is always available; the
            list itself appears once there are clips. */}
        <Show when={anyInstalled()}>
          <div
            style={{
              display: "flex",
              "flex-direction": "column",
              gap: "var(--s2)",
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
              <span
                style={{
                  "font-size": "var(--t-xs)",
                  "text-transform": "uppercase",
                  "letter-spacing": "0.08em",
                  color: "var(--text-low)",
                }}
              >
                Saved clips
              </span>
              <button
                type="button"
                onClick={openClipsFolder}
                title="Open the saved-clips folder"
                style={{
                  display: "flex",
                  "align-items": "center",
                  gap: "var(--s2)",
                  background: "transparent",
                  border: "1px solid var(--line)",
                  "border-radius": "var(--r-md)",
                  color: "var(--text-mid)",
                  cursor: "pointer",
                  padding: "var(--s1) var(--s3)",
                  "font-size": "var(--t-sm)",
                }}
              >
                <Sigil name="folder" size={14} />
                Open folder
              </button>
            </div>
            <Show
              when={app.speakClips().length > 0}
              fallback={
                <span
                  style={{ "font-size": "var(--t-sm)", color: "var(--text-low)" }}
                >
                  No saved clips yet — generate one above and it appears here.
                </span>
              }
            >
            <div
              style={{
                display: "flex",
                "flex-direction": "column",
                gap: "var(--s1)",
                "max-height": "220px",
                "overflow-y": "auto",
              }}
            >
              <For each={app.speakClips()}>
                {(clip) => (
                  <div
                    style={{
                      display: "flex",
                      "align-items": "center",
                      gap: "var(--s2)",
                      padding: "var(--s2) var(--s3)",
                      "border-radius": "var(--r-md)",
                      border: "1px solid var(--line)",
                      background: "var(--surface-1)",
                    }}
                  >
                    <button
                      type="button"
                      aria-label="Play clip"
                      title="Play"
                      onClick={() => void app.playSpeakClip(clip)}
                      style={{
                        display: "flex",
                        background: "transparent",
                        border: "none",
                        color: "var(--accent)",
                        cursor: "pointer",
                        padding: 0,
                      }}
                    >
                      <Sigil name="play" size={16} />
                    </button>
                    <span
                      style={{
                        display: "flex",
                        "flex-direction": "column",
                        flex: 1,
                        "min-width": 0,
                      }}
                    >
                      <span
                        style={{
                          overflow: "hidden",
                          "text-overflow": "ellipsis",
                          "white-space": "nowrap",
                          "font-size": "var(--t-sm)",
                          color: "var(--text-high)",
                        }}
                      >
                        {clip.text}
                      </span>
                      <span
                        style={{
                          "font-size": "0.72rem",
                          color: "var(--text-low)",
                          "font-variant-numeric": "tabular-nums",
                        }}
                      >
                        {clipDate(clip.createdAt)} · {clip.durationSecs.toFixed(1)}s
                      </span>
                    </span>
                    <button
                      type="button"
                      aria-label="Load into editor"
                      title="Load this text + voice back into the editor"
                      onClick={() => {
                        app.setTtsText(clip.text);
                        app.setSelectedTtsVoice(clip.voice);
                      }}
                      style={{
                        display: "flex",
                        background: "transparent",
                        border: "none",
                        color: "var(--text-mid)",
                        cursor: "pointer",
                        padding: 0,
                      }}
                    >
                      <Sigil name="copy" size={16} />
                    </button>
                    <button
                      type="button"
                      aria-label="Delete clip"
                      title="Delete"
                      onClick={() => void app.removeSpeakClip(clip.id)}
                      style={{
                        display: "flex",
                        background: "transparent",
                        border: "none",
                        color: "var(--text-low)",
                        cursor: "pointer",
                        padding: 0,
                      }}
                    >
                      <Sigil name="trash" size={14} />
                    </button>
                  </div>
                )}
              </For>
            </div>
            </Show>
          </div>
        </Show>
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
