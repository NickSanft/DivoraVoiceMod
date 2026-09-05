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
//   __E2E_EMIT__(event, payload)          — fire a backend event

import { test as base, expect, type Page } from "@playwright/test";

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
    }) => {
      const invokes: Array<{ cmd: string; args: unknown }> = [];
      (window as unknown as { __E2E_INVOKES__: unknown }).__E2E_INVOKES__ =
        invokes;

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
        list_tts_voices: () => [
          { id: "af_heart", name: "Aria — warm (US)", lang: "en-us", installed: false },
          { id: "bm_george", name: "George — crisp (UK)", lang: "en-gb", installed: false },
        ],
        speak: () => {
          throw "text-to-speech voices are not installed";
        },
        stop_speak: () => null,
        list_speak_clips: () => [],
        speak_clips_dir: () => "C:/mock/speak-clips",
        open_speak_clips_folder: () => null,
        delete_speak_clip: () => null,
        set_reactive_config: () => null,
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
        preset_store_path: () => "C:/mock/presets",
        list_midi_inputs: () => [],
        audio_engine_status: () => engineStatus,
        start_audio_engine: (a: any) => ({
          inputName: a?.inputName ?? "Mock Mic",
          outputName: a?.outputName ?? "Mock Speakers",
          monitorName: a?.monitorName ?? null,
          sampleRate: 48000,
          inputChannels: 1,
          outputChannels: 2,
        }),
        export_preset_json: (a: any) =>
          JSON.stringify(a?.preset ?? {}, null, 2),
        scan_soundboard_folder: () => [],
        play_soundboard_clip: () => 1,
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
          // Other plugin calls (window/dialog/shell/unlisten/emit): no-op.
          if (cmd.indexOf("plugin:") === 0) return Promise.resolve(null);

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
      };
      (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ =
        internals;

      (window as unknown as { __E2E_EMIT__: unknown }).__E2E_EMIT__ = (
        event: string,
        payload: unknown,
      ) => {
        for (const id of listeners.get(event) ?? []) {
          const cb = callbacks.get(id);
          if (cb) cb({ event, id, payload });
        }
      };
    },
    { inputs: MOCK_INPUTS, outputs: MOCK_OUTPUTS, presets: MOCK_PRESETS },
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
