# Stable Surface (frozen at v1.0)

This document is the **back-compat contract** for DivoraVoice. Everything
listed here is **stable as of v1.0.0**: the shapes, names, and on-disk
formats must not change in a breaking way across the 1.x line.

**The rule after v1.0 is _additive-only_:**

- ✅ Add a new Tauri command, a new event, a new optional struct field
  (`#[serde(default)]` / optional in TS), a new preset effect kind, a new
  `localStorage` key.
- ❌ Rename or remove a command, event, struct field, or storage key.
  ❌ Change a field's type or JSON casing. ❌ Make a previously-optional
  field required. ❌ Repurpose an existing key for a different meaning.

Several of these contracts are guarded by serialization tests so an
accidental break fails CI before it ships:

- Preset JSON schema + tag casing + legacy-load — `divora-core/src/presets/mod.rs` (`schema_freeze_tests`).
- `StreamInfo` keys — `divora-core/src/audio/engine.rs`.
- `EngineStatus` / `LevelUpdate` / `VoiceInfo` / `OnnxRuntimeStatus` keys — `src-tauri/src/lib.rs`.
- Every command wrapper's command-name string — `src/audio/api.test.ts`.

> See also [ARCHITECTURE.md](ARCHITECTURE.md) for *how* these pieces fit
> together; this file is only *what* is frozen.

---

## 1. Tauri command surface

All commands are registered in `src-tauri/src/lib.rs` and wrapped in
`src/audio/api.ts`. Arguments are camelCase (Tauri converts the Rust
snake_case parameter names). `Result<T, String>` returns surface as a
resolved `T` or a thrown string on the JS side.

### Audio devices & engine

| Command | Args | Returns |
|---|---|---|
| `list_audio_input_devices` | — | `DeviceInfo[]` |
| `list_audio_output_devices` | — | `DeviceInfo[]` |
| `start_audio_engine` | `inputName?`, `outputName?`, `monitorName?` | `StreamInfo` / error |
| `stop_audio_engine` | — | — |
| `set_audio_monitor` | `enabled: bool` | — |
| `audio_engine_status` | — | `EngineStatus` |

### DSP chain

| Command | Args | Returns |
|---|---|---|
| `set_effect_chain` | `specs: EffectSpec[]` | — |
| `set_effect_param` | `index`, `key`, `value` | — |
| `set_effect_enabled` | `index`, `enabled` | — |
| `clear_effect_chain` | — | — |
| `set_voice_model` | `index`, `path?` | — |

### Presets

| Command | Args | Returns |
|---|---|---|
| `list_presets` | — | `Preset[]` (bundled + user) |
| `save_user_preset` | `preset: Preset` | — / error |
| `delete_user_preset` | `id: string` | — / error |
| `export_preset_json` | `preset: Preset` | pretty JSON `string` / error |
| `preset_store_path` | — | `string` |

### Voice library

| Command | Args | Returns |
|---|---|---|
| `voices_dir` | — | `string` |
| `list_voices` | — | `VoiceInfo[]` |
| `onnx_runtime_status` | — | `OnnxRuntimeStatus` |

### Recording

| Command | Args | Returns |
|---|---|---|
| `recordings_dir` | — | `string` |
| `start_recording` | `filename: string` | full destination path `string` / error |
| `stop_recording` | — | — |

### Soundboard

| Command | Args | Returns |
|---|---|---|
| `scan_soundboard_folder` | `folder: string` | `SoundboardTile[]` / error |
| `play_soundboard_clip` | `clipId`, `path`, `gain?` | duration seconds `f32` / error |
| `stop_soundboard_clip` | `clipId: string` | — |
| `stop_all_soundboard_clips` | — | — |
| `set_soundboard_master_gain` | `gain: f32` | — |

### Virtual mic & hotkeys

| Command | Args | Returns |
|---|---|---|
| `detect_virtual_mic` | — | `VirtualMicStatus` |
| `register_global_shortcut` | `id`, `accelerator` | — / error |
| `unregister_global_shortcut` | `id` | — / error |
| `unregister_all_global_shortcuts` | — | — / error |

---

## 2. Events

Emitted by the backend, subscribed via `@tauri-apps/api/event`.

| Event | Payload | Cadence |
|---|---|---|
| `audio-levels` | `LevelUpdate` | ~30 Hz while the app runs |
| `global-shortcut` | `GlobalShortcutEvent` | per press / release |

---

## 3. Wire types (JSON shapes)

camelCase keys. Adding an optional field is allowed; renaming/removing is not.

```ts
DeviceInfo        { name, isDefault, defaultSampleRate, channels }
Levels            { rms, peak }                       // both 0..1
StreamInfo        { inputName, outputName, monitorName (nullable),
                    sampleRate, inputChannels, outputChannels }
EngineStatus      { running, monitoring, input: Levels, output: Levels,
                    dspLatencyMs, recording }
LevelUpdate       { input: Levels, output: Levels, running, monitoring,
                    dspLatencyMs, recording }
EffectSpec        { kind: EffectKindWire, enabled, params: {string: number} }
VoiceInfo         { id, name, path, sizeBytes }
OnnxRuntimeStatus { runtimeAvailable, voicesDir }
SoundboardTile    { id, path, label, extension, sizeBytes, modifiedSecs? }
VirtualMicStatus  { detected, cableInputDevice: DeviceInfo|null,
                    cableOutputDevice: DeviceInfo|null, downloadUrl }
GlobalShortcutEvent { id, accelerator, state: "pressed" | "released" }
```

`EffectKindWire` (frozen set; new kinds may be **added** after v1.0):

```
gate · denoiser · pitch · formant · eq · robot · distortion · echo · reverb · voice_convert
```

---

## 4. Preset JSON schema

The format saved to disk (`<id>.json`) and shared via **Export JSON**. This
is the most important contract to keep stable — exported files are shared
between users and must keep loading.

```jsonc
{
  "id": "velvet-demon",      // [a-z0-9_-]+ ; also the file name stem
  "version": 1,               // optional; defaults to 1 if absent
  "name": "Velvet Demon",
  "color": "#A78BFA",
  "glyph": "triangle",
  "tag": "User",              // "Bundled" | "User"  (PascalCase, frozen)
  "desc": "…",
  "chain": [
    { "id": "pitch", "enabled": true, "vals": { "semitones": -5.0 } }
  ]
}
```

Forward/back-compat guarantees (frozen):

- **Effect `id` is a free-form string.** An older client loading a preset
  with a future effect kind keeps the entry as-is rather than failing.
- **Unknown top-level fields are ignored** (no `deny_unknown_fields`), so a
  newer schema loads on an older client.
- **Missing `version` defaults to `1`.** Any field added after v1.0 must
  also be optional (`#[serde(default)]`) so pre-v1.0 files still load.
- A user-dir file claiming `"tag": "Bundled"` is downgraded to `User` on
  read — the `Bundled` tag is reserved for embedded presets.

---

## 5. Persisted frontend state (`localStorage`)

All keys are namespaced `divora.*`. Missing / unparseable values fall back
to defaults (never throw).

| Key | Holds |
|---|---|
| `divora.tweaks` | appearance (mystical / motion / mood / accent / grain / vignette) |
| `divora.activeVoice` | selected voice-model id (or null) |
| `divora.inputDevice` | selected input device name (or null) |
| `divora.outputDevice` | selected output device name (or null) |
| `divora.monitorDevice` | monitor output device name (or null) |
| `divora.soundboardFolder` | last picked soundboard folder path |
| `divora.recentFolders` | recent soundboard folders (cap 5) |
| `divora.tileColors` | per-tile color overrides |
| `divora.tileOrder` | per-folder tile order |
| `divora.tileHotkeys` | per-tile global hotkey accelerators |
| `divora.tileGains` | per-tile soundboard gain |
| `divora.soundboardMasterGain` | master soundboard gain |
| `divora.wizardSeen` | first-run wizard completion flag |

---

## 6. On-disk layout

Under `%APPDATA%\DivoraVoice\` (created at startup, best-effort):

```
presets/<id>.json        user presets (see §4)
voices/<id>.onnx         user-installed voice-conversion models
recordings/divora-<date>_<time>.wav   recorded modulated output (16-bit PCM mono)
```

Installer-bundled, read-only, next to the executable's resource dir:

```
onnxruntime.dll          ONNX runtime (also locatable via ORT_DYLIB_PATH)
voices/<id>.onnx         shipped voice models (shadowed by a same-id user file)
```
