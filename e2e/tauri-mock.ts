// Tauri-in-the-browser mock for Playwright E2E.
//
// The app talks to the backend only through `@tauri-apps/api`'s `invoke`
// (core) and `listen` (event), which both resolve through
// `window.__TAURI_INTERNALS__`. We inject a stub of that object before any
// page script runs, so the REAL `@tauri-apps/api` functions work against
// our mock — no source changes, and we can assert exactly which commands
// the UI calls (guarding the command-surface contract from the UI side).
//
// Boot only uses core.invoke + event.listen; window/dialog/shell plugins
// are lazy-imported and also route through invoke, so this one stub
// covers everything. Test-side helpers are exposed on `window`:
//   __E2E_INVOKES__ : Array<{cmd, args}>  — every non-plugin invoke
//   __E2E_PLUGIN_INVOKES__ : Array<{cmd, args}> — every other plugin call
//                     (v1.55.0: `plugin:event|emit`, `plugin:window|…`)
//   __E2E_EMIT__(event, payload)          — fire a backend event
//
// v1.55.0: each page also knows which window it is (`metadata`, as Tauri
// injects it): `overlay` on /overlay.html, `main` everywhere else, so
// `getCurrentWindow()` works and its calls are recorded with that label.

import { test as base, expect, type Page } from "@playwright/test";
import wire from "../src/overlay/wire.fixture.json" with { type: "json" };
import type { OverlayStatus } from "../src/overlay/status";

export const MOCK_INPUTS = [
  { name: "Mock Mic", isDefault: true, defaultSampleRate: 48000, channels: 1 },
  { name: "USB Condenser", isDefault: false, defaultSampleRate: 48000, channels: 1 },
];
export const MOCK_OUTPUTS = [
  { name: "Mock Speakers", isDefault: true, defaultSampleRate: 48000, channels: 2 },
  { name: "CABLE Input (VB-Audio Virtual Cable)", isDefault: false, defaultSampleRate: 48000, channels: 2 },
];

// A few real bundled presets in WIRE shape (mirrors divora-core bundled
// JSON → the `list_presets` command). The first is the default-active one.
export const MOCK_PRESETS = [
  {
    id: "hollow-king",
    version: 1,
    name: "Hollow King",
    color: "#7C5CF6",
    glyph: "reverb",
    tag: "Bundled",
    desc: "Cavernous, regal, distant.",
    chain: [
      { id: "gate", enabled: true, vals: { thresh: -48 } },
      { id: "pitch", enabled: true, vals: { shift: -5 } },
      { id: "formant", enabled: true, vals: { shift: -3 } },
      { id: "eq", enabled: true, vals: { low: 3, mid: -2, high: 1 } },
      { id: "reverb", enabled: true, vals: { size: 78, mix: 42 } },
    ],
  },
  {
    id: "static-wraith",
    version: 1,
    name: "Static Wraith",
    color: "#58C6F2",
    glyph: "distortion",
    tag: "Bundled",
    desc: "Broken-radio specter.",
    chain: [
      { id: "gate", enabled: true, vals: { thresh: -44 } },
      { id: "pitch", enabled: true, vals: { shift: -2 } },
      { id: "distortion", enabled: true, vals: { drive: 58 } },
      { id: "eq", enabled: true, vals: { low: -4, mid: 3, high: 5 } },
      { id: "echo", enabled: true, vals: { time: 180, fb: 48 } },
    ],
  },
  {
    id: "velvet-demon",
    version: 1,
    name: "Velvet Demon",
    color: "#F2567A",
    glyph: "robot",
    tag: "Bundled",
    desc: "Smooth, low, and wrong in the best way.",
    chain: [
      { id: "gate", enabled: true, vals: { thresh: -50 } },
      { id: "pitch", enabled: true, vals: { shift: -7 } },
      { id: "robot", enabled: true, vals: { freq: 90, mix: 32 } },
      { id: "reverb", enabled: true, vals: { size: 48, mix: 34 } },
    ],
  },
];

/** v1.55.0: the overlay's wire samples (src/overlay/wire.fixture.json), the
 *  same file both unit suites read, so a status built here can't drift
 *  from the backend's keys. */
export const OVERLAY_STATUS_SAMPLES = wire.statusSamples as Record<
  keyof typeof wire.statusSamples,
  OverlayStatus
>;
export const OVERLAY_STATUS_KEYS: readonly string[] = wire.statusKeys;
export const IMPORTED_ASSET_SAMPLE = wire.importedAssetSample;

export interface MockOptions {
  /** Seed `divora.wizardSeen=true` so the app lands on the Mixer, not the
   *  first-run wizard. Default true; the wizard spec passes false. */
  skipWizard?: boolean;
}

export async function installTauriMock(
  page: Page,
  opts: MockOptions = {},
): Promise<void> {
  const skipWizard = opts.skipWizard !== false;

  // 1) The Tauri internals stub. Runs before any page script.
  await page.addInitScript(
    (data: {
      inputs: unknown;
      outputs: unknown;
      presets: unknown;
      overlayClosed: object;
      importedAsset: Record<string, unknown>;
    }) => {
      const invokes: Array<{ cmd: string; args: unknown }> = [];
      (window as unknown as { __E2E_INVOKES__: unknown }).__E2E_INVOKES__ =
        invokes;
      const pluginInvokes: Array<{ cmd: string; args: unknown }> = [];
      (
        window as unknown as { __E2E_PLUGIN_INVOKES__: unknown }
      ).__E2E_PLUGIN_INVOKES__ = pluginInvokes;
      // The window this page is, as Tauri names it.
      const label = location.pathname.endsWith("/overlay.html") ? "overlay" : "main";

      const callbacks = new Map<number, (e: unknown) => void>();
      const listeners = new Map<string, number[]>();
      let nextId = 1;

      const engineStatus = {
        running: false,
        monitoring: true,
        input: { rms: 0, peak: 0 },
        output: { rms: 0, peak: 0 },
        dspLatencyMs: 0,
        recording: false,
        loudnessGainDb: 0,
      };

      const emit = (event: string, payload: unknown): void => {
        for (const id of listeners.get(event) ?? []) {
          const cb = callbacks.get(id);
          if (cb) cb({ event, id, payload });
        }
      };

      // v1.54.0: Replay. The ring's status lives here so the stubs and
      // emitReplay (test side) agree on it. Memory follows the backend's
      // formula at 1440p30 Balanced, under the ceiling of a PC with 2 GiB
      // (256 MiB), so 120 s is over it.
      const replay = {
        status: {
          supported: true,
          reason: null as string | null,
          state: "off",
          saving: false,
          seconds: 30,
          bufferedMs: 0,
          bytes: 0,
          capBytes: 0,
          monitorId: null as string | null,
          heldReason: null as string | null,
          armed: null as Record<string, unknown> | null,
        },
      };
      (window as unknown as { __E2E_REPLAY__: unknown }).__E2E_REPLAY__ = replay;
      const REPLAY_CEILING = 268_435_456;
      const replayCap = (s: number): number => {
        const estimate = ((s + 2) * 14_376_960) / 8 + (s + 4) * 192_000;
        return (estimate * 5) / 4;
      };
      const replayLengths = () =>
        [15, 30, 60, 120].map((s) => ({
          seconds: s,
          bytes: (replayCap(s) * 4) / 5,
          capBytes: replayCap(s),
          allowed: replayCap(s) <= REPLAY_CEILING,
        }));
      const setReplay = (patch: Record<string, unknown>): typeof replay.status => {
        replay.status = { ...replay.status, ...patch } as typeof replay.status;
        emit("replay-buffer", replay.status);
        return replay.status;
      };
      const replaySaved = (saved: Record<string, unknown>): void =>
        emit("replay-saved", { path: null, durationMs: 0, error: null, ...saved });

      // v1.55.0: the stream overlay, as the backend's core reports it. Every
      // status starts from the fixture's closed sample. The commands answer
      // as the backend does but never emit `overlay:status` themselves: the
      // specs send each status with emitOverlayStatus, so every step shows
      // the page reacting to the one they sent.
      const overlay = {
        status: { ...data.overlayClosed, hideAvailable: true } as Record<string, unknown>,
        windows: 0,
      };
      (window as unknown as { __E2E_OVERLAY__: unknown }).__E2E_OVERLAY__ = overlay;
      const setOverlay = (patch: Record<string, unknown>): Record<string, unknown> => {
        overlay.status = { ...overlay.status, ...patch };
        return overlay.status;
      };
      const NOT_OPEN = "The overlay isn't open.";
      const STARTING = "The overlay is still starting. Try again in a moment.";
      const overlayLive = (): boolean =>
        overlay.status.open === true && overlay.status.phase !== "closing";

      const responses: Record<string, (args: any) => unknown> = {
        list_audio_input_devices: () => data.inputs,
        list_audio_output_devices: () => data.outputs,
        list_presets: () => data.presets,
        detect_virtual_mic: () => ({
          detected: false,
          cableInputDevice: null,
          cableOutputDevice: null,
          downloadUrl: "https://vb-audio.com/Cable/",
        }),
        onnx_runtime_status: () => ({
          runtimeAvailable: false,
          voicesDir: "C:/mock/voices",
        }),
        list_voices: () => [],
        voices_dir: () => "C:/mock/voices",
        // v1.17.0: TTS scaffolding — preset voices exist but aren't
        // installed yet, and synthesis rejects with the graceful message
        // (mirrors the gated backend until the model assets are staged).
        // v1.50.0: the Critter Chatter voices follow, always installed, and
        // they are the only ids speak renders (as the real backend would in a
        // build without Kokoro).
        list_tts_voices: () => [
          { id: "af_heart", name: "Aria — warm (US)", lang: "en-us", installed: false, engine: "kokoro" },
          { id: "bm_george", name: "George — crisp (UK)", lang: "en-gb", installed: false, engine: "kokoro" },
          { id: "babble:bright", name: "Bright", lang: "en-us", installed: true, engine: "babble" },
          { id: "babble:mellow", name: "Mellow", lang: "en-us", installed: true, engine: "babble" },
          { id: "babble:gruff", name: "Gruff", lang: "en-us", installed: true, engine: "babble" },
        ],
        speak: (a: any) => {
          if (typeof a?.voiceId === "string" && a.voiceId.startsWith("babble:")) {
            return 1.4;
          }
          throw "text-to-speech voices are not installed";
        },
        stop_speak: () => null,
        preview_voice: () => 1.2,
        stop_preview_voice: () => null,
        list_speak_clips: () => [],
        speak_clips_dir: () => "C:/mock/speak-clips",
        open_speak_clips_folder: () => null,
        delete_speak_clip: () => null,
        set_reactive_config: () => null,
        set_voice_reading_enabled: () => null,
        voice_reading: () => ({
          state: "quiet",
          dry: {
            energyDbfs: -120,
            energyRangeDb: 0,
            f0Hz: 0,
            f0RangeSt: 0,
            voicedRatio: 0,
            paceOps: 0,
            brightnessHz: 0,
          },
          wet: {
            energyDbfs: -120,
            energyRangeDb: 0,
            f0Hz: 0,
            f0RangeSt: 0,
            voicedRatio: 0,
            paceOps: 0,
            brightnessHz: 0,
          },
          words: [],
          calibrated: false,
          stale: false,
          ageMs: 0,
        }),
        list_cloned_voices: () => [],
        clone_voice: () => {
          throw "text-to-speech voices are not installed";
        },
        start_voice_recording: () => null,
        stop_voice_recording: () => {
          throw "text-to-speech voices are not installed";
        },
        rename_cloned_voice: () => null,
        delete_cloned_voice: () => null,
        clone_models_status: () => ({ ready: false }),
        voxcpm_status: () => ({ available: false, readPrompt: "" }),
        download_voxcpm_models: () => null,
        download_clone_models: () => null,
        recordings_dir: () => "C:/mock/recordings",
        // v1.52.0: screen recording. Supported, two screens, nothing running;
        // takes are driven by firing `screen-recording` with __E2E_EMIT__.
        screen_capture_status: () => ({
          supported: true,
          reason: null,
          h264Encoder: "Mock H.264 Encoder",
          hardware: true,
        }),
        list_capture_monitors: () => [
          { id: "\\\\.\\DISPLAY1", number: 1, x: 0, y: 0, width: 2560, height: 1440, primary: true },
          { id: "\\\\.\\DISPLAY2", number: 2, x: 2560, y: 0, width: 1920, height: 1080, primary: false },
        ],
        // v1.53.0: desktop audio — this "PC" can record everything except
        // Divora, so that is the one source offered.
        list_desktop_audio_sources: () => [
          {
            id: "everything",
            label: "Everything except Divora",
            kind: "everything",
            isDefault: false,
            divoraPlays: "nothing",
          },
        ],
        start_screen_recording: () => "C:/mock/recordings/divora-screen-x.mp4",
        stop_screen_recording: () => "C:/mock/recordings/divora-screen-x.mp4",
        screen_recording_status: () => ({
          state: "idle",
          path: null,
          elapsedMs: 0,
          droppedFrames: 0,
          driftPpm: 0,
          audioLostAt: null,
          error: null,
          desktopSourceId: null,
          desktopSource: null,
          desktopQuietSeconds: null,
          desktopDoubled: false,
          desktopLostAt: null,
          desktopError: null,
          stemsError: null,
        }),
        // v1.54.0: Replay. Arming goes straight to armed; every save attempt
        // reports itself with `replay-saved`, refusals included, as the
        // backend does.
        replay_buffer_status: () => replay.status,
        replay_memory: () => ({ ceilingBytes: REPLAY_CEILING, lengths: replayLengths() }),
        arm_replay_buffer: (a: any) => {
          const seconds = [15, 30, 60, 120].includes(a?.seconds) ? a.seconds : 30;
          return setReplay({
            state: "armed",
            seconds,
            capBytes: replayCap(seconds),
            monitorId: a?.monitorId ?? null,
            heldReason: null,
            armed: {
              fps: a?.fps ?? 30,
              quality: a?.quality ?? "balanced",
              cursor: a?.cursor ?? true,
              desktopAudio: a?.desktopAudio ?? null,
              desktopGainDb: a?.desktopGainDb ?? -6,
              avOffsetMs: a?.avOffsetMs ?? 0,
            },
          });
        },
        disarm_replay_buffer: () =>
          setReplay({
            state: "off",
            saving: false,
            bufferedMs: 0,
            bytes: 0,
            capBytes: 0,
            monitorId: null,
            heldReason: null,
            armed: null,
          }),
        save_replay: () => {
          const s = replay.status;
          const refusal =
            s.state === "off"
              ? "Replay is off."
              : s.state === "arming"
                ? "Replay hasn't kept anything yet — try again in a moment."
                : s.saving
                  ? "Still saving the last replay."
                  : null;
          if (refusal) {
            replaySaved({ error: refusal });
            throw refusal;
          }
          const durationMs = Math.min(s.bufferedMs, s.seconds * 1000) || s.seconds * 1000;
          const path = "C:/mock/recordings/divora-replay-x.mp4";
          replaySaved({ path, durationMs });
          return { path, durationMs };
        },
        preset_store_path: () => "C:/mock/presets",
        list_midi_inputs: () => [],
        audio_engine_status: () => engineStatus,
        start_audio_engine: (a: any) => ({
          inputName: a?.inputName ?? "Mock Mic",
          outputName: a?.outputName ?? "Mock Speakers",
          monitorName: a?.monitorName ?? null,
          sampleRate: 48000,
          outputRate: 48000,
          monitorRate: a?.monitorName ? 48000 : null,
          inputChannels: 1,
          outputChannels: 2,
        }),
        export_preset_json: (a: any) =>
          JSON.stringify(a?.preset ?? {}, null, 2),
        scan_soundboard_folder: () => [],
        play_soundboard_clip: () => 1,
        // v1.55.0: the stream overlay (STABLE-SURFACE §7). Closed, and this
        // "PC" can leave it out of captures.
        overlay_status: () => overlay.status,
        set_overlay_hide_from_recordings: (a: any) => setOverlay({ hideWanted: a?.enabled === true }),
        // A new window starts hidden; already open → the current status.
        open_overlay: (a: any) => {
          if (overlay.status.open === true) return overlay.status;
          overlay.windows += 1;
          return setOverlay({
            open: true,
            phase: "starting",
            instance: overlay.windows,
            mode: "locked",
            canArrange: false,
            fit: a?.rect != null && a?.fit === true,
            error: null,
          });
        },
        close_overlay: () =>
          setOverlay({ open: false, phase: "closed", mode: "locked", canArrange: false, fit: false, error: null }),
        // Placed where asked; the window has painted by the time anyone places it.
        place_overlay: (a: any) => {
          if (!overlayLive()) throw NOT_OPEN;
          return setOverlay({ phase: "painted", canArrange: true, fit: a?.fit === true });
        },
        set_overlay_mode: (a: any) => {
          const m = a?.mode;
          if (m !== "locked" && m !== "arrange" && m !== "toggle") throw "Unknown overlay mode.";
          const want = m === "toggle" ? (overlay.status.mode === "arrange" ? "locked" : "arrange") : m;
          if (want === "locked") return setOverlay({ mode: "locked" });
          if (!overlayLive()) throw NOT_OPEN;
          if (overlay.status.canArrange !== true) throw STARTING;
          if (overlay.status.mode === "arrange") return overlay.status;
          return setOverlay({ mode: "arrange", arrangeGen: Number(overlay.status.arrangeGen) + 1, error: null });
        },
        // Only the overlay page calls these two: a status for its window.
        overlay_handshake: (a: any) => {
          if (a?.stage !== "ready" && a?.stage !== "painted") throw "Unknown overlay stage.";
          return setOverlay({
            open: true,
            instance: a.instance,
            phase: a.stage === "ready" ? "shown" : "painted",
            canArrange: a.stage === "painted",
          });
        },
        overlay_alive: () => null,
        import_overlay_asset: () => ({ ...data.importedAsset }),
        delete_overlay_asset: () => null,
      };
      // Plugin calls answer null, except where null would mean something:
      // the main window's toasts wait while `isVisible()` is false. A spec
      // can pick a file in the next dialog by setting __E2E_DIALOG_PICK__.
      const pluginResponses: Record<string, () => unknown> = {
        "plugin:window|is_visible": () => true,
        "plugin:dialog|open": () =>
          (window as unknown as { __E2E_DIALOG_PICK__?: unknown }).__E2E_DIALOG_PICK__ ?? null,
      };
      // v1.55.0: a broadcast also goes to the other window when a spec has
      // linked two pages (linkWindows), as Tauri's event bus would carry it.
      const relay = (args: unknown): void => {
        const bridge = (window as unknown as { __E2E_BRIDGE__?: unknown }).__E2E_BRIDGE__;
        if (typeof bridge !== "function") return;
        const a = args as { event: string; payload: unknown };
        Promise.resolve(bridge(a.event, a.payload)).catch(() => {
          /* the other page is gone */
        });
      };

      const internals = {
        invoke(cmd: string, args: unknown) {
          // Event plugin: record listeners so __E2E_EMIT__ can fire them.
          if (cmd === "plugin:event|listen") {
            const ev = (args as { event: string }).event;
            const handlerId = (args as { handler: number }).handler;
            if (!listeners.has(ev)) listeners.set(ev, []);
            listeners.get(ev)!.push(handlerId);
            return Promise.resolve(nextId++);
          }
          // Other plugin calls (window/dialog/shell/unlisten/emit): no-op,
          // recorded (v1.55.0) so `overlay:edit` emits and window drags can
          // be asserted.
          if (cmd.indexOf("plugin:") === 0) {
            pluginInvokes.push({ cmd, args });
            if (cmd === "plugin:event|emit") relay(args);
            const answer = pluginResponses[cmd];
            return Promise.resolve(answer ? answer() : null);
          }

          invokes.push({ cmd, args });
          const fn = responses[cmd];
          try {
            return Promise.resolve(fn ? fn(args) : null);
          } catch (err) {
            return Promise.reject(err);
          }
        },
        transformCallback(cb: (e: unknown) => void) {
          const id = nextId++;
          callbacks.set(id, cb);
          return id;
        },
        unregisterCallback(id: number) {
          callbacks.delete(id);
        },
        convertFileSrc(p: string) {
          return p;
        },
        metadata: {
          currentWindow: { label },
          currentWebview: { windowLabel: label, label },
        },
      };
      (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ =
        internals;

      (window as unknown as { __E2E_EMIT__: unknown }).__E2E_EMIT__ = emit;
    },
    {
      inputs: MOCK_INPUTS,
      outputs: MOCK_OUTPUTS,
      presets: MOCK_PRESETS,
      overlayClosed: OVERLAY_STATUS_SAMPLES.closed,
      importedAsset: IMPORTED_ASSET_SAMPLE,
    },
  );

  // 2) Seed localStorage MERGE-IF-ABSENT so we don't clobber app-persisted
  //    state across reloads (the theme-persistence test depends on this).
  //    Quiet motion for deterministic visuals; optionally skip the wizard.
  await page.addInitScript((skip: boolean) => {
    try {
      const raw = localStorage.getItem("divora.tweaks");
      const tweaks = raw ? JSON.parse(raw) : {};
      if (tweaks.motion === undefined) {
        tweaks.motion = 0;
        localStorage.setItem("divora.tweaks", JSON.stringify(tweaks));
      }
      if (skip && localStorage.getItem("divora.wizardSeen") === null) {
        localStorage.setItem("divora.wizardSeen", "true");
      }
    } catch {
      /* localStorage always present in chromium */
    }
  }, skipWizard);
}

/** v1.54.0: move the mock's Replay ring to `patch` (over its current status)
 *  and fire `replay-buffer`, as the backend does on every transition. The
 *  stubs read the same status, so a save afterwards answers for it. */
export async function emitReplay(page: Page, patch: Record<string, unknown>): Promise<void> {
  await page.evaluate((p) => {
    const w = window as unknown as {
      __E2E_REPLAY__: { status: Record<string, unknown> };
      __E2E_EMIT__: (e: string, payload: unknown) => void;
    };
    w.__E2E_REPLAY__.status = { ...w.__E2E_REPLAY__.status, ...p };
    w.__E2E_EMIT__("replay-buffer", w.__E2E_REPLAY__.status);
  }, patch);
}

/** v1.54.0: fire a `replay-saved` outcome directly. */
export async function emitReplaySaved(
  page: Page,
  saved: { path?: string | null; durationMs?: number; error?: string | null },
): Promise<void> {
  await page.evaluate((s) => {
    (
      window as unknown as { __E2E_EMIT__: (e: string, payload: unknown) => void }
    ).__E2E_EMIT__("replay-saved", { path: null, durationMs: 0, error: null, ...s });
  }, saved);
}

/** v1.55.0: move the mock's overlay status to `partial` (over its current
 *  status, which starts as the fixture's closed sample) and fire
 *  `overlay:status`, as the backend does on every change. The commands read
 *  the same status, so a later call answers for it. A key the wire doesn't
 *  have is refused, so a typo can't pass for a field. */
export async function emitOverlayStatus(
  page: Page,
  partial: Partial<OverlayStatus>,
): Promise<OverlayStatus> {
  const unknown = Object.keys(partial).filter((k) => !OVERLAY_STATUS_KEYS.includes(k));
  if (unknown.length > 0) throw new Error(`not OverlayStatus keys: ${unknown.join(", ")}`);
  return page.evaluate((p) => {
    const w = window as unknown as {
      __E2E_OVERLAY__: { status: Record<string, unknown> };
      __E2E_EMIT__: (e: string, payload: unknown) => void;
    };
    w.__E2E_OVERLAY__.status = { ...w.__E2E_OVERLAY__.status, ...p };
    w.__E2E_EMIT__("overlay:status", w.__E2E_OVERLAY__.status);
    return w.__E2E_OVERLAY__.status as unknown as OverlayStatus;
  }, partial);
}

/** The app commands invoked so far, with their args (`__E2E_INVOKES__`),
 *  optionally only `cmd`. */
export function invokesOf(
  page: Page,
  cmd?: string,
): Promise<Array<{ cmd: string; args: Record<string, unknown> }>> {
  return page.evaluate(
    (c) =>
      (
        window as unknown as {
          __E2E_INVOKES__: Array<{ cmd: string; args: Record<string, unknown> }>;
        }
      ).__E2E_INVOKES__
        .filter((i) => c === null || i.cmd === c)
        .map((i) => ({ cmd: i.cmd, args: i.args ?? {} })),
    cmd ?? null,
  );
}

/** v1.55.0: the plugin calls made so far (`__E2E_PLUGIN_INVOKES__`),
 *  optionally only `cmd`. */
export function pluginInvokesOf(
  page: Page,
  cmd?: string,
): Promise<Array<{ cmd: string; args: Record<string, unknown> }>> {
  return page.evaluate(
    (c) =>
      (
        window as unknown as {
          __E2E_PLUGIN_INVOKES__: Array<{ cmd: string; args: Record<string, unknown> }>;
        }
      ).__E2E_PLUGIN_INVOKES__
        .filter((i) => c === null || i.cmd === c)
        .map((i) => ({ cmd: i.cmd, args: i.args ?? {} })),
    cmd ?? null,
  );
}

/** v1.55.0: the payloads this page emitted on `event` (`plugin:event|emit`). */
export async function emitsOf(page: Page, event: string): Promise<unknown[]> {
  return (await pluginInvokesOf(page, "plugin:event|emit"))
    .filter((i) => i.args.event === event)
    .map((i) => i.args.payload);
}

/** v1.55.0: carry each page's broadcasts (`emit`) to the other, as the event
 *  bus carries them between the main window and the overlay. Call before
 *  either page loads. An event sent before the other page is listening is
 *  lost, as it would be. */
export async function linkWindows(a: Page, b: Page): Promise<void> {
  const relayTo = (to: Page) => async (event: string, payload: unknown) => {
    await to
      .evaluate(
        ([e, p]) => {
          const w = window as unknown as { __E2E_EMIT__?: (e: string, p: unknown) => void };
          w.__E2E_EMIT__?.(e as string, p);
        },
        [event, payload] as const,
      )
      .catch(() => undefined);
  };
  await a.exposeFunction("__E2E_BRIDGE__", relayTo(b));
  await b.exposeFunction("__E2E_BRIDGE__", relayTo(a));
}

/** Playwright `test` with the Tauri mock auto-installed and a
 *  `consoleErrors` array collecting any console.error / pageerror.
 *  `skipWizard` is an overridable option (default true) — the wizard
 *  spec does `test.use({ skipWizard: false })` to see the first run. */
export const test = base.extend<{
  skipWizard: boolean;
  consoleErrors: string[];
}>({
  skipWizard: [true, { option: true }],
  consoleErrors: async ({ page }, use) => {
    const errors: string[] = [];
    page.on("console", (m) => {
      if (m.type() === "error") errors.push(m.text());
    });
    page.on("pageerror", (e) => errors.push(String(e)));
    await use(errors);
  },
  page: async ({ page, skipWizard }, use) => {
    await installTauriMock(page, { skipWizard });
    await use(page);
  },
});

export { expect };
