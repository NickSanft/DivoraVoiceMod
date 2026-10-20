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
- `EngineStatus` / `LevelUpdate` / `VoiceInfo` / `OnnxRuntimeStatus` / `VoiceReadingUpdate` keys — `src-tauri/src/lib.rs`.
- `ReadingSnapshot` keys — `divora-core/src/audio/engine.rs`.
- `InputMute` keys — `divora-core/src/audio/engine.rs` (`input_mute_json_keys_are_frozen`; v1.56.0).
- `TtsVoiceInfo` keys + the missing-`engine` default — `divora-core/src/tts/mod.rs`.
- `MidiMessage` keys — `src-tauri/src/midi.rs`.
- `ScreenRecordingUpdate` / `CaptureStatusDto` / `MonitorDto` / `DesktopSourceDto` keys and the `DivoraPlays` strings — `src-tauri/src/screen.rs` (`wire_keys_are_frozen`; v1.52.0, v1.53.0).
- `ReplayStatus` / `ReplayArmed` / `ReplaySaved` / `ReplayMemory` / `ReplayLength` keys and the state and held-reason strings — `src-tauri/src/replay.rs` (`replay_wire_keys_are_frozen`; v1.54.0).
- `OverlayStatus` keys, the mode strings and the `closed` / `painted` / `closing` phase strings (the phases in the fixture's three status samples; `starting` and `shown` aren't in it) — `src/overlay/wire.fixture.json`, read by `src-tauri/src/overlay_machine.rs` (`wire_fixture_matches_rust`) and `src/overlay/status.test.ts`; v1.55.0.
- `ImportedAsset` keys — the same fixture, `src-tauri/src/overlay_assets.rs` (`imported_asset_wire_keys_are_frozen`; v1.55.0).
- The `overlay:*` event names and the overlay page's three command names — the same fixture, read by `src/overlay/state.test.ts` and `status.test.ts`; `wire_fixture_matches_rust` checks the `overlay:status` name and parses the `mode` / `stage` strings (v1.55.0).
- The overlay window's label and title and the tray's Lock id — `src-tauri/src/overlay_machine.rs` (`title_and_label_are_the_obs_contract`); every tray menu id, label and their order — `src-tauri/src/lib.rs` (`tray_entries_for_each_combination`; v1.55.0).
- `divora.overlayScene`'s read and write rules (unknown keys, types and `src` values kept; nothing truncated) — `src/overlay/scene.test.ts` (v1.55.0).
- The `mute` hotkey id: a press flips the mute; its release, `Mute` and `mute:` ids don't — `src-tauri/src/lib.rs` (`hotkey_mute_request_table`; v1.56.0).
- Every command wrapper's command-name string — `src/audio/api.test.ts` (the screen-recording ones since v1.53.0, the stream-overlay ones since v1.55.0, the mute ones since v1.56.0).

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
| `set_monitor_gain` | `gain: number` (linear, 1.0 = unity) | — |
| `set_loudness_enabled` | `enabled: bool` | — |
| `set_loudness_target` | `dbfs: number` | — |
| `audio_engine_status` | — | `EngineStatus` |
| `set_input_muted` | `muted: bool` | `InputMute`, the state after — v1.56.0. Idempotent: the same value twice moves nothing, `seq` included. Never fails, apart from ``"invalid args `muted` for command `set_input_muted`: {e}"`` (Tauri's own, for a missing or non-boolean `muted`) |
| `toggle_input_muted` | — | `InputMute`, the state after — v1.56.0. Every call flips it, so two quick calls are two flips. Never fails |
| `input_mute` | — | `InputMute`, the current state — v1.56.0. Only reads: the main window calls it once each time the page loads, so a reload never changes the mute |

Divora's own mute of the voice (v1.56.0) — on the call, the headphone monitor
and Divora's recordings (WAV, screen recordings and their stems, Replay).
Soundboard clips, Speak and Critter Chatter still play. Engine-wide: it can be
set with the engine stopped, it outlives Stop, Start and device-loss rebuilds,
and it is never stored; every launch starts unmuted. Every change adds 1 to
`seq`. The three commands are sync (they run on the main thread, in turn with
each other and with the hotkey), and each is one atomic operation;
`LevelUpdate` carries the same state on every tick.

### DSP chain

| Command | Args | Returns |
|---|---|---|
| `set_effect_chain` | `specs: EffectSpec[]` | — |
| `set_effect_param` | `index`, `key`, `value` | — |
| `set_effect_enabled` | `index`, `enabled` | — |
| `clear_effect_chain` | — | — |
| `set_voice_model` | `index`, `path?` | — |
| `set_reactive_config` | `config: ReactiveConfig` | — / v1.46.0 (one whole config; route targets outside the modulation whitelist are dropped, not rejected) |

### Voice reading

| Command | Args | Returns |
|---|---|---|
| `set_voice_reading_enabled` | `enabled: bool` | — (off by default; while off the audio callback copies nothing and no analysis runs) |
| `voice_reading` | — | `VoiceReadingUpdate` (one-shot, for the panel's first paint) |

### Presets

| Command | Args | Returns |
|---|---|---|
| `list_presets` | — | `Preset[]` (bundled + user) |
| `save_user_preset` | `preset: Preset` | — / error |
| `delete_user_preset` | `id: string` | — / error |
| `export_preset_json` | `preset: Preset` | pretty JSON `string` / error |
| `import_preset` | `path: string` | saved User `Preset` / error — v1.14.0 |
| `preset_store_path` | — | `string` |

### Voice library

| Command | Args | Returns |
|---|---|---|
| `voices_dir` | — | `string` |
| `list_voices` | — | `VoiceInfo[]` |
| `onnx_runtime_status` | — | `OnnxRuntimeStatus` |

### Text-to-speech / "Speak" (v1.17.0)

| Command | Args | Returns |
|---|---|---|
| `list_tts_voices` | — | `TtsVoiceInfo[]` |
| `speak` | `text: string`, `voiceId: string` (preset id, Critter Chatter id `babble:*` (v1.50.0), **or** cloned id), `gain?: number` (v1.18.0, default 1.0), `previewOnly?: bool` (v1.18.0, default false → monitor-only) | duration seconds `f32` / error (e.g. "voices are not installed") — stops any utterance still playing first (v1.50.0) |
| `stop_speak` | — | — |
| `preview_voice` | `voiceId: string`, `useGpu?: bool` | duration seconds `f32` / error — v1.47.0 (fixed phrase, monitor-only, one take, disk-cached; Critter Chatter voices are never disk-cached, v1.50.0) |
| `stop_preview_voice` | — | — / v1.47.0 |
| `clone_voice` | `name: string`, `referencePath: string` | `ClonedVoiceInfo` / error — v1.20.0 |
| `start_voice_recording` | — | — / error ("start the engine first") — v1.23.0 |
| `stop_voice_recording` | `name: string` | `ClonedVoiceInfo` / error — v1.23.0 |
| `list_cloned_voices` | — | `ClonedVoiceInfo[]` — v1.20.0 |
| `rename_cloned_voice` | `id: string`, `name: string` | — / error — v1.43.0 (display name only; the `id` is stable) |
| `delete_cloned_voice` | `id: string` | — / error — v1.20.0 |
| `clone_models_status` | — | `{ ready: bool }` — v1.21.0 |
| `download_clone_models` | — | — / error (downloads ~157 MB on-demand) — v1.21.0 |
| `voxcpm_status` | — | `{ available: bool, readPrompt: string }` — v1.24.0 |
| `download_voxcpm_models` | — | — / error (downloads ~1.6 GB on-demand) — v1.24.0 |

**Voice ids (v1.50.0).** An id containing `:` is **engine-namespaced** and is
never a cloned-voice id — cloned ids are a single safe path component
(`[A-Za-z0-9_-]`), so the two can't collide and a namespaced id never touches
the cloned-voice or preview-cache folders. The procedural Critter Chatter
voices use the `babble:` namespace; their ids — `babble:bright`,
`babble:mellow`, `babble:gruff` — are permanent, because they are persisted in
`divora.ttsVoice` and in saved clips (display names may change; ids never do).

### Recording

| Command | Args | Returns |
|---|---|---|
| `recordings_dir` | — | `string` |
| `logs_dir` | — | `string` (logs directory, for "Open logs folder") — v1.14.0 |
| `start_recording` | `filename: string` | full destination path `string` / error |
| `stop_recording` | — | — |

### Screen recording (v1.52.0)

All `async` except `screen_recording_status`, which only reads the latest
report: the others wait on Windows, and a sync command would block the
window's event loop.

| Command | Args | Returns |
|---|---|---|
| `screen_capture_status` | — | `CaptureStatus` (never an error: an unsupported PC is `supported: false` with a `reason`) |
| `list_capture_monitors` | — | `CaptureMonitor[]`, the main screen first |
| `list_desktop_audio_sources` | — | `DesktopAudioSource[]` — v1.53.0: just `everything` where Windows can record everything except Divora; otherwise one `device:<id>` per active playback device |
| `start_screen_recording` | `monitorId: string`, `filename: string`, `fps?: 30 \| 60`, `quality?: "small" \| "balanced" \| "high"`, `cursor?: bool`, `avOffsetMs?: number` (±500); `desktopAudio?: string`, `desktopGainDb?: number`, `stems?: bool` — v1.53.0 | full destination path `string`, once the first frame is in / error ("Start the engine before recording." while the engine is stopped, the same words as the WAV recorder) |
| `stop_screen_recording` | — | full path of the finished MP4 `string` / error |
| `screen_recording_status` | — | `ScreenRecordingUpdate` (the latest report) |

v1.54.0: a take starts on the capture pipeline Replay uses. While Replay is on,
`start_screen_recording` records on Replay's pipeline and must ask for the same
screen, frame rate, quality, pointer, desktop audio, desktop volume (with
desktop audio) and A/V offset, or it fails with "Replay is on with a different
… — turn Replay off to record with others."; while Replay is held for a lost
screen or a failed encoder it fails with "Replay lost its screen …" /
"Replay lost its encoder …". `stems` and `filename` are the take's own.

`filename` keeps only its last path component and is always written as `.mp4`,
so nothing the frontend sends can leave `recordings/`. Unknown `quality`
values read as `"balanced"`, and any `fps` but 60 as 30.

**Desktop audio ids (v1.53.0).** `desktopAudio` is `everything` (process
loopback: what every other process plays, on whichever device, except Divora
and the processes it started — and except on a device loopback can't hear) or
`device:<id>` (one playback device, by its Windows endpoint id — endpoint
loopback: everything it plays, Divora included, where loopback can hear that
device). Absent, `off`, or anything else — a value from a later
version included — is no desktop audio: the MP4's audio track is exactly
v1.52.0's, though with `stems` a `<take>.voice.wav` is still written beside it.
The prefixes `everything` and `device:` are reserved. `desktopGainDb` is
clamped to −30…0 dB; absent or `null` (what JSON makes of `NaN` and
`Infinity`) is −6, and any other value that isn't a number fails the command.
A source that
can't be opened, or a device Divora sends your voice to, never fails the
take: it records without desktop audio, and `desktopError` says why. The
`avOffsetMs` offset moves desktop audio as it moves the voice.

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

### Replay (v1.54.0)

All `async`: an arm waits for the pipeline's first frame, a save for its file.

| Command | Args | Returns |
|---|---|---|
| `replay_buffer_status` | — | `ReplayStatus` |
| `replay_memory` | `monitorId: string`, `fps?: 30 \| 60`, `quality?: "small" \| "balanced" \| "high"` | `ReplayMemory` / error (the screen isn't connected) |
| `arm_replay_buffer` | `monitorId: string`, `seconds: 15 \| 30 \| 60 \| 120` (anything else is 30), `fps?`, `quality?`, `cursor?`, `avOffsetMs?` (±500), `desktopAudio?`, `desktopGainDb?` — as `start_screen_recording` reads them | `ReplayStatus` / error: the engine stopped ("Start the engine before recording."), unsupported (`ReplayStatus.reason`'s words), too little memory ("Replay of {N} s at these settings needs up to {cap} MB of memory; this PC allows {ceiling} MB." — the cap rounded up, the ceiling down, as the card's labels), Replay already on with other settings, a take running on the sink writer. While on with the same settings it changes the length. During a take on the pipeline it joins the take, on the take's settings |
| `disarm_replay_buffer` | — | `ReplayStatus` (a save already pressed still finishes) |
| `save_replay` | — | `ReplaySaved` (`path`, `durationMs`; `error` null) / error — every outcome, refusals included, is also one `replay-saved` event |

**Every arm goes through the frontend's `armReplay()`**, which holds the
explainer gate: Replay is never turned on — by the card, the `replay:toggle`
hotkey, or any action added later (MIDI, glyphs, timers) — before its
explainer has been seen. The tray has no item that turns Replay on.

**Global-shortcut ids** (the `id` argument above, and the `id` of the `global-shortcut` event). An id is a fixed action name or a reserved prefix; new ids are added, never renamed. Accelerators use the global-hotkey grammar, so the Windows key is `Super` (written "Win" only in the UI) — v1.51.4.

| Id | Meaning |
|---|---|
| `ptm` | push-to-modulate (press and release) |
| `panic` | stop all soundboard clips |
| `monitor` | toggle hearing yourself |
| `sb:<clipId>` | play one soundboard tile (prefix reserved) |
| `rec:<recorder>` | start / stop a recorder (prefix reserved) — v1.52.0. Defined: `rec:screen` (the screen recorder), `rec:wav` (the WAV recorder). Both unbound by default |
| `replay:<action>` | Replay (prefix reserved) — v1.54.0. Defined: `replay:save` (save the replay), `replay:toggle` (turn Replay on / off; it works once Replay has been turned on from the Record screen card). Both unbound by default |
| `overlay:<action>` | the stream overlay (prefix reserved) — v1.55.0. Defined: `overlay:arrange`, which switches the open overlay between Arrange and Locked. It is acted on by the backend when pressed; the frontend only shows a notice while the overlay is off or starting. Unbound by default. A different namespace from the `overlay:state` / `overlay:*` **event** names, despite the shared spelling |
| `mute` | mute or unmute your voice (a toggle) — v1.56.0. Acted on by the backend when pressed (a release does nothing), so it doesn't go through the page; the `global-shortcut` event still goes out, and the frontend does nothing with it. Unbound by default. The prefix `mute:` is reserved for later variants |

### Stream overlay (v1.55.0)

All `async`: Tauri 2 runs a sync command on the main thread, which both
windows share, and building a window from one deadlocks on Windows. The
backend creates, places and shows the overlay window and owns its mode; main
says where it goes and owns the scene (`divora.overlayScene`).

| Command | Args (camelCase) | Returns | Errors (exact) |
|---|---|---|---|
| `open_overlay` | `rect?: {x, y, width, height} \| null` (physical px, whole numbers: `x` and `y` an i32, `width` and `height` a u32; absent, `null`, or a width or height of 0 or over 32768 → Windows' default spot), `fit?: boolean` | `OverlayStatus` | `"Couldn't open the overlay window: {e}"`; `"The overlay is still closing. Try again in a moment."` (still closing after a 2 s wait); ``"invalid args `rect` for command `open_overlay`: {e}"`` (Tauri's own, before anything else: a field of `rect` missing, fractional or out of its type's range, such as a negative width). Already open or opening → the current status, nothing moved. Opens Locked |
| `close_overlay` | — | `OverlayStatus` | none (closed → no-op) |
| `place_overlay` | `rect: {x, y, width, height}` (physical px, typed as for `open_overlay`; a width or height of 0 or over 32768 → nothing moves and `fit` is ignored), `fit?: boolean` | `OverlayStatus` | ``"invalid args `rect` for command `place_overlay`: {e}"`` (Tauri's own, first, as for `open_overlay`); `"The overlay isn't open."` (closed or closing); `"The overlay is still starting. Try again in a moment."` (while the window is being built). Checked before the window is touched |
| `set_overlay_mode` | `mode: "locked" \| "arrange" \| "toggle"` | `OverlayStatus` | `"The overlay isn't open."` (arrange/toggle-to-arrange while closed or closing); `"The overlay is still starting. Try again in a moment."` (before painted); `"Unknown overlay mode."`. `"locked"` never fails |
| `set_overlay_hide_from_recordings` | `enabled: boolean` | `OverlayStatus` | none |
| `overlay_status` | — | `OverlayStatus` | none. Main's adoption read after a reload |
| `overlay_handshake` | `instance: number`, `stage: "ready" \| "painted"` | `OverlayStatus` | `"Unknown overlay stage."`. Another instance's call → the status, nothing changed |
| `overlay_alive` | `instance: number`, `gen: number` | `null` | none. A stale beat is ignored. Emits nothing |
| `import_overlay_asset` | `path: string` | `ImportedAsset` | the refusals below |
| `delete_overlay_asset` | `id: string` | `null` | `"That isn't a charm picture."` (not an asset id); `"Couldn't remove the picture: {e}"`. Missing file → ok |

`overlay_handshake` and `overlay_alive` are called only by the overlay page.
`fit` (default false) says main sized `rect` to a whole monitor: the window
is then not resizable, and `OverlayStatus.fit` reports it. The
`overlay:arrange` hotkey and the tray's **Lock the overlay** item change
the mode without a command, as `set_overlay_mode` would with `"toggle"` and
`"locked"`.

Rules for `import_overlay_asset`. Any regular file the user can read is
accepted, as for `import_preset`; its type comes from its first bytes, never
its name. Each refusal, in its exact words:

- not a file, or empty: "That isn't a picture file."
- over 50 MB (50,000,000 bytes): "That picture is over 50 MB. Pick a smaller one."
- not PNG, JPEG, GIF or WebP (APNG is PNG): "Divora can add PNG, JPEG, GIF and WebP pictures. That file isn't one."
- a JPEG whose first SOF marker isn't SOF0, SOF1 or SOF2 with 8-bit precision (baseline, extended sequential or progressive): "Divora can't show that kind of JPEG. Save it as a regular JPEG or a PNG and try again."
- a header that can't be read, or a zero side: "That picture looks damaged, so Divora can't add it."
- over 8192 px a side or 16,777,216 px: "That picture is too big (over 16 megapixels or 8192 pixels on a side). Make it smaller and try again."
- animated and over 4,194,304 px: "That animated picture is too big (over 4 megapixels). Make it smaller and try again."
- the file can't be read, or the copy can't be written: "Couldn't read that picture: {e}" / "Couldn't save the picture: {e}"

The size checked is the size as displayed: for a JPEG, after its EXIF
orientation; for a GIF, its logical screen grown to hold its first frame. The
destination name is derived from the content, never from the path. Same file
twice → same id, written once; a damaged stored copy is rewritten. A picture
over 20 MB, or an animated one over 2,097,152 px, is still added, with
`warning` set.

Capabilities don't restrict app commands (there is no app ACL manifest): the
overlay window can call every command in this section, and every other one.
It relies on `set_overlay_mode`, `overlay_handshake` and `overlay_alive`, and
must be granted them if an app manifest is ever added. Its own capability,
`src-tauri/capabilities/overlay.json`, grants only event listen / unlisten /
emit / emit-to and dragging its window; `default.json` now covers `main`
alone and no longer grants creating webview windows or setting a window's
position or size, which the backend does.

### MIDI control surfaces (v1.9.0)

| Command | Args | Returns |
|---|---|---|
| `list_midi_inputs` | — | `MidiInputInfo[]` |
| `open_midi_input` | `name: string` | — / error |
| `close_midi_input` | — | — / error |

---

## 2. Events

Emitted by the backend, subscribed via `@tauri-apps/api/event`.

| Event | Payload | Cadence |
|---|---|---|
| `audio-levels` | `LevelUpdate` | ~30 Hz while the app runs |
| `global-shortcut` | `GlobalShortcutEvent` | per press / release |
| `midi-message` | `MidiMessage` | per MIDI note / CC, while a port is open (v1.9.0) |
| `overlay:state` | `OverlayState` | main → overlay window, on state change, while the stream overlay is open (v1.16.0). v1.56.0: each emit waits for the one before it, and after the main window loads nothing goes until it knows the mute (its `input_mute` read settles or a state arrives; at most 500 ms). The payload is unchanged: while muted, `status` is `"muted"` |
| `overlay:ready` | — (no payload) | overlay window → main, once the overlay is listening for `overlay:state`; main answers with the current state (v1.51.3). v1.55.0: main also re-pushes the scene on it, with `ack` null, and the overlay listens for `overlay:scene` and `overlay:status` before sending it. The backend learns ready through `overlay_handshake`, not this event |
| `overlay:status` | `OverlayStatus` | backend → all windows, on every change of any field — v1.55.0. The same shape is returned by every command in §1 "Stream overlay" except `overlay_alive` and the two asset commands |
| `overlay:scene` | `ScenePush` | main → overlay window, while it is open: on every scene change, on each edit main acknowledges (changed or not), and on each `overlay:ready` — v1.55.0 |
| `overlay:edit` | `SceneEdit` | overlay window → all (heard by main): on pointerup of a gesture that changed something; 300 ms after the last arrow nudge or keyboard resize (sooner when another editing key, a press, losing focus or Lock comes first); on Delete; on `[` `]` `{` `}` — v1.55.0 |
| `clone-model-download` | `CloneDownloadProgress` | during the on-demand voice-cloning model download (v1.21.0) |
| `voice-reading` | `VoiceReadingUpdate` | ~5 Hz, **only while the Voice reading panel is on**; silent otherwise |
| `screen-recording` | `ScreenRecordingUpdate` | on every phase change of a take, and about once a second while one records (v1.52.0); silent otherwise — also while only Replay runs |
| `replay-buffer` | `ReplayStatus` | on every change of Replay's state or `saving`, and about once a second while armed; a held ring only on change (v1.54.0) |
| `replay-saved` | `ReplaySaved` | the outcome of every save attempt, refusals included — exactly one per attempt (v1.54.0) |

**Why `voice-reading` is its own event and not a field on `LevelUpdate`.**
Additive-only permits either, so this is a design choice, recorded here because
it constrains what may be added later. The reading is a *window* measurement
that changes about once a second, so riding the 30 Hz meter tick would resend
identical numbers six times over. The feature is off by default, and a field on
the always-on payload would carry an empty reading to every user who never
opens the panel. And the panel's accessible summary is text: a value updating
30 times a second is exactly what must never go near an `aria-live` region.
`LevelUpdate` keeps meaning "meter tick" and gains no reading fields.
v1.56.0 adds the mute to it (`inputMuted`, `inputMuteSeq`), like `monitoring`
and `recording`: a state every window may need, cheap, and the only always-on
channel that reaches a hidden page whoever changed it.

---

## 3. Wire types (JSON shapes)

camelCase keys. Adding an optional field is allowed; renaming/removing is not.

```ts
DeviceInfo        { name, isDefault, defaultSampleRate, channels }
Levels            { rms, peak }                       // both 0..1
StreamInfo        { inputName, outputName, monitorName (nullable),
                    sampleRate, outputRate, monitorRate (nullable),
                    inputChannels, outputChannels }
EngineStatus      { running, monitoring, input: Levels, output: Levels,
                    dspLatencyMs, recording, loudnessGainDb }
LevelUpdate       { input: Levels, output: Levels, running, monitoring,
                    dspLatencyMs, recording, loudnessGainDb,
                    deviceLost, modEnv,         // modEnv v1.46.0, 0..1
                    inputMuted, inputMuteSeq }  // v1.56.0: the InputMute pair
InputMute         { muted, seq }    // v1.56.0: seq counts changes since launch;
                                    // of two states, the larger is newer
EffectSpec        { kind: EffectKindWire, enabled, params: {string: number} }
ReactiveConfig    { enabled, intensity, floorDb, ceilDb,           // v1.46.0
                    attackMs, holdMs, releaseMs,
                    routes: ReactiveRouteSpec[] }
ReactiveRouteSpec { kind: EffectKindWire, nth, key, base, depth }   // v1.46.0
VoiceInfo         { id, name, path, sizeBytes }
OnnxRuntimeStatus { runtimeAvailable, voicesDir }
TtsVoiceInfo      { id, name, lang, installed, engine }  // v1.17.0; engine v1.50.0:
                  // "kokoro" | "babble" — absent reads as "kokoro"
ClonedVoiceInfo   { id, name, baseName }             // v1.20.0; baseName v1.22.0
CloneDownloadProgress { file, fileCount, received, total }  // v1.21.0
SoundboardTile    { id, path, label, extension, sizeBytes, modifiedSecs? }
VirtualMicStatus  { detected, cableInputDevice: DeviceInfo|null,
                    cableOutputDevice: DeviceInfo|null, downloadUrl }
GlobalShortcutEvent { id, accelerator, state: "pressed" | "released" }
MidiInputInfo     { id, name }                       // v1.9.0
MidiMessage       { channel, kind, data1, data2 }    // v1.9.0; kind e.g.
                  // "note-on" | "note-off" | "control-change"
ReadingMetrics    { energyDbfs, energyRangeDb, f0Hz, f0RangeSt,
                    voicedRatio, paceOps, brightnessHz }
VoiceReadingUpdate { state, dry: ReadingMetrics, wet: ReadingMetrics,
                    words: string[], calibrated, wetBypassed,
                    stale, ageMs,                    // wetBypassed v1.51.0
                    mutedInDivora }                  // v1.56.0
CaptureStatus     { supported, reason (nullable), h264Encoder (nullable),
                    hardware }                       // v1.52.0
CaptureMonitor    { id, number, x, y, width, height, primary }  // v1.52.0;
                  // id e.g. "\\.\DISPLAY1"; number 1-based, main first
ScreenRecordingUpdate { state: "idle" | "starting" | "recording" | "stopping",
                    path (nullable), elapsedMs, droppedFrames, driftPpm,
                    audioLostAt (nullable, seconds into the take),
                    error (nullable),                // v1.52.0
                    desktopSourceId (nullable: "everything" | "device:<id>"),
                    desktopSource (nullable, its name),
                    desktopQuietSeconds (nullable: seconds since desktop
                      audio last made a sound, −60 dBFS or louder),
                    desktopDoubled (the running engine's devices and monitor
                      switches put Divora's voice or soundboard on the device
                      being recorded, whether or not anything is sounding —
                      false while that device's sound is left out because
                      Divora sends your voice there; updated about once a
                      second),
                    desktopLostAt (nullable, seconds into the take; while
                      the source is being reopened),
                    desktopError (nullable: why the take has no desktop
                      audio — from the start, or from when it stopped for
                      good or came to carry your voice to the call),
                    stemsError (nullable) }
                                                     // the desktop* and stems* fields v1.53.0
DesktopAudioSource { id ("everything" | "device:<id>"), label,
                    kind: "everything" | "device", isDefault,
                    divoraPlays: "nothing" | "send" | "monitor" |
                      "onlyOutput" | "unknown" }     // v1.53.0
ReplayStatus      { supported, reason (nullable),
                    state: "off" | "arming" | "armed" | "held",
                    saving, seconds, bufferedMs, bytes, capBytes,
                    monitorId (nullable),
                    heldReason (nullable: "engine-stopped" | "screen-lost"
                      | "encoder-failed"),
                    armed (nullable: ReplayArmed — what the running ring
                      uses) }                        // v1.54.0
ReplayArmed       { fps, quality: "small" | "balanced" | "high", cursor,
                    desktopAudio (nullable: "everything" | "device:<id>"),
                    desktopGainDb, avOffsetMs }      // v1.54.0
ReplaySaved       { path (nullable), durationMs, error (nullable) }  // v1.54.0
ReplayMemory      { ceilingBytes, lengths: ReplayLength[] }          // v1.54.0
ReplayLength      { seconds, bytes (the estimate), capBytes (the most it
                    may hold: the figure shown), allowed }           // v1.54.0
OverlayState      { chain, status, motion, mystical, mood, accent, theme,
                    bg: "transparent" | "green" | "magenta" }        // v1.16.0
OverlayStatus     { open (any phase but closed),
                    phase: "closed" | "starting" | "shown" | "painted"
                      | "closing",           // a window being built reads as starting
                    instance (the current or last window's number; 0 before
                      any),
                    mode: "locked" | "arrange",
                    arrangeGen (bumped on every entry to Arrange; the page
                      echoes it in overlay_alive),
                    canArrange (phase is painted),
                    fit (the window was placed fitted to its monitor),
                    hideWanted (the Hide from recordings switch, as the
                      backend holds it),
                    hideAvailable (Windows can leave a window out of
                      captures on this PC),
                    capturing (a take in any phase, or Replay in any state
                      but off),
                    error (nullable: why the watchdog closed this window,
                      or why the lease ended Arrange; null again once the
                      window is gone, and the lease's at the next
                      Arrange) }                                     // v1.55.0
ScenePush         { session (new at each main-window load), revision
                      (bumped on every scene change),
                    scene: OverlayScene,
                    ack (nullable: { instance, seq } — the overlay's latest
                      edit this push includes),
                    expectViewport (nullable: { w, h } — set only by a Fit
                      change: keep drawing the previous scene until the
                      viewport is this size, for at most 1 s) }      // v1.55.0
SceneEdit         { instance (OverlayStatus.instance), seq (this window's
                      edits numbered from 0, in the order sent),
                    op: SceneOp }                                    // v1.55.0
SceneOp           { kind: "add", element (a "spellcircle" or "image"
                      element, as in OverlayScene) }
                  | { kind: "remove", id }
                  | { kind: "place", id, rect: { x, y, w, h }, viewport: { w, h } }
                  | { kind: "order", id, to: "front" | "back" | "forward" | "backward" }
                  | { kind: "reframe", from: { x, y, w, h }, to: { x, y, w, h },
                      frame: "window" | "monitor" }                  // v1.55.0
                  // CSS px. The overlay may send only remove, place and order;
                  // main ignores any other edit, and one it can't read
ImportedAsset     { id (the stored file's name: the first 16 hex digits of
                      the content's SHA-256, a dot, and png | jpg | gif |
                      webp by the content),
                    width, height (as displayed, in the picture's own
                      pixels), bytes (the file's size),
                    animated (a GIF with a second frame, an APNG, or a WebP
                      with its animation flag; a GIF or PNG that can't be
                      read far enough to tell counts as animated),
                    warning (nullable: words for a picture over 20 MB, or an
                      animated one over 2,097,152 px) }              // v1.55.0
OverlayRect       { x, y, width, height }   // physical px; an argument only; v1.55.0
OverlayScene      { version, designSize: { w, h }, frame,
                    elements: SceneElement[] }                       // v1.55.0
// version: an integer ≥ 1 (else read as 1), written as max(stored, 1); a larger value is read with the v1 rules
// designSize: CSS px, each side finite and 1..16384, else { 480, 480 }
// frame: "window" | "monitor"; missing or unknown reads as "window" and is kept until a reframe
// elements: never truncated; not an array → the default elements (the circle, centred, at most 440 px)
// SceneElement: { id, type, x, y, w, h, z }, plus { src, nw, nh, anim, bytes, name } for "image"
// id: an entry that isn't an object, or has no non-empty string id, is dropped; a later duplicate is dropped; nothing else is
// type: "spellcircle" (the first usable one is shown), "image", or anything else (kept verbatim, not shown)
// shown: an id of 1..64 characters and usable x/y/w/h, and for "image" a valid src; everything else is kept, not shown
// x/y/w/h: fractions of designSize, never clamped on read; usable when all four are finite and w, h > 0
// z: finite, else its array index (computed, not written)
// src: an asset id matching ^[0-9a-f]{16}\.(png|jpg|gif|webp)$, never a path or URL; anything else keeps the image, unshown
// nw/nh: the picture's own pixels; integers ≥ 1, else the aspect comes from the stored w·designSize.w / h·designSize.h
// anim: boolean (anything else → src ends in .gif); bytes: the file's size (missing or not positive → 0); both only for the scene budget
// name: shown cut to 120 characters, "Picture" when missing or blank (computed, not written)
// any other key, at any level: kept and written back unchanged
```

**`StreamInfo` notes.**

- `outputRate` and `monitorRate` (v1.51.2, additive) are the main output's and
  the separate monitor's device rates; `sampleRate` remains the **engine's**
  rate, which is the input device's. Any of them differing means that output is
  resampled. They exist because the struct used to carry one rate, so nothing
  in the app could report — or represent — two devices disagreeing, while the
  path that bridged the gap was destroying the audio. Windows sets each device's
  rate independently and can change it behind the app, so a user can land there
  without doing anything.

**`VoiceReadingUpdate` notes (frozen).**

- `state` is `"stopped" | "muted" | "quiet" | "speaking"`. The set is closed and
  the four are distinct on purpose: stopped, muted and quiet-but-present look
  identical at the meter, and collapsing them would let the panel go on
  describing a signal it no longer has.
- v1.56.0: `"muted"` is also reported while Divora's own mute is on
  (`"stopped"` still comes first); the microphone may be live then (the IN
  meter moves). `mutedInDivora` (additive, set in the same snapshot as
  `state`) tells the two apart; the state set stays closed.
- The wet tap is taken after the chain, loudness and the mute (v1.56.0). While
  muted nothing is analysed: both taps are dropped and the last reading is
  held, as through a pause.
- `dry` is the microphone **before any effect**; `wet` is the same voice
  **after the chain**. Soundboard / Speak / Critter Chatter audio is mixed in
  downstream of the wet tap, so it appears on neither half.
- `words` carries the already-resolved vocabulary (`"wide range"`, not
  `"narrow"`), never descriptor enum names, and never more than three. The
  closed vocabulary in `divora-core/src/dsp/reading.rs` is the **only** source
  of a shown word; the frontend keeps no list of its own, so there is one
  place to audit.
- `wetBypassed` (v1.51.0, additive) says the chain passed the signal through
  over that window, so the two halves *should* read alike. It is **measured**
  in the engine by correlating the taps, which is scale-invariant because the
  loudness stage sits between the chain and the wet tap. Do not re-derive it
  from the metrics here: they are summaries, and a level comparison is wrong
  whenever loudness normalization is on.
- Not every measured field is displayed. The card shows four numbers per half
  — pitch, pitch range, level, pace. `energyRangeDb` and `brightnessHz` reach
  the screen only through `words` (`flat`/`dynamic`, `dark`/`bright`), and
  `voicedRatio` is measured, used to decide voicing, and never shown.
- `stale` / `ageMs`: during a pause the last speaking window is **held** and
  flagged rather than decayed. A readout drifting toward "quiet, flat, narrow"
  through every pause would be a verdict on the speaker, not a measurement.
- Every metric field is always a finite number. A silent window is −∞ dBFS
  internally, which JSON cannot carry, so the bridge floors it (−120 dBFS)
  rather than emitting `null`.

**`OverlayStatus` notes (v1.55.0).**

- Every change goes out as `overlay:status`, and a command's answer can
  arrive after events that followed it (a close answers "closing" after
  "closed" has arrived). Main applies a command's answer only when no
  `overlay:status` event arrived while the call was in flight.
- The overlay page ignores a status whose `instance` isn't its own.
- The frontend reads a status with every key checked, and ignores keys a
  later version adds.
- Main toasts each new `error` as given. It is "The overlay window didn't
  start, so Divora closed it. Try turning it on again." (the watchdog: no
  ready handshake 10 s after the window was built); "The overlay didn't
  respond, so it stayed locked." (the Arrange lease: three 1 s ticks without
  the page's first beat); or "The overlay stopped responding, so Divora
  locked it. Turn it off and on again if it looks wrong." (three ticks
  without a beat after one).
- No key says whether Hide from recordings is in effect: Windows doesn't say
  whether a capture honours it, so `hideWanted` is the request and nothing
  more.

`EffectKindWire` (frozen set; new kinds may be **added** after v1.0):

```
gate · denoiser · pitch · formant · eq · robot · distortion · echo · reverb · chorus · harmonizer · compressor · deesser · radio_bandpass · vintage_noise · tremolo · breath · warble · bitcrush · voice_convert
```

(`chorus` added in v1.2.0, `harmonizer` in v1.2.1, `compressor` + `deesser` in v1.8.0, `radio_bandpass` + `vintage_noise` in v1.32.0, `tremolo` in v1.34.0, `breath` in v1.35.0, `warble` in v1.37.0, `bitcrush` in v1.48.0 — kinds are additive after v1.0.)

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
| `divora.tweaks` | appearance (theme / mystical / motion / mood / accent / grain / vignette) — `theme: "dark"\|"light"` added v1.11.0 |
| `divora.activeVoice` | selected voice-model id (or null) |
| `divora.inputDevice` | selected input device name (or null) |
| `divora.outputDevice` | selected output device name (or null) |
| `divora.monitorDevice` | monitor output device name (or null) |
| `divora.soundboardFolder` | last picked soundboard folder path |
| `divora.recentFolders` | recent soundboard folders (cap 5) |
| `divora.tileColors` | per-tile color overrides |
| `divora.tileOrder` | per-folder tile order |
| `divora.tileHotkeys` | per-tile global hotkey accelerators |
| `divora.hotkeyBindings` | `{ ptm, panic, monitor }` global hotkey accelerators, each `""` when unbound (the default for all three) — v1.51.4; plus `"rec:screen"` and `"rec:wav"` — v1.52.0; plus `"replay:save"` and `"replay:toggle"` — v1.54.0 (a save without them reads them as unbound); plus `"overlay:arrange"` — v1.55.0 (likewise); plus `"mute"` — v1.56.0 (likewise). Unknown keys are ignored on load; "Win" is read as "Super" |
| `divora.replayBuffer` | `{ seconds: 15\|30\|60\|120, explained: bool, explainedDesktop: bool }` — v1.54.0; each falls back alone (30, false, false). **Never an armed flag**: Replay is off after every launch |
| `divora.screenRecording` | `{ monitorId (nullable: the main screen), fps: 30\|60, quality: "small"\|"balanced"\|"high", cursor, avOffsetMs }` — v1.52.0; `desktopAudio: "everything"\|"off"\|"device:<id>", desktopGainDb (−30…0), stems` — v1.53.0. Each field falls back to its default on its own when missing or malformed — except `desktopAudio`, which reads as `"off"` when missing (settings saved before v1.53.0) or malformed, and is `"everything"` only on a first run with nothing saved |
| `divora.tileGains` | per-tile soundboard gain |
| `divora.soundboardMasterGain` | master soundboard gain |
| `divora.midiInput` | selected MIDI input port name (or null) — v1.9.0 |
| `divora.midiMappings` | MIDI note/CC → action mappings (array) — v1.9.0. Each `action` is one of `preset`, `tile`, `ptm`, `monitor`, `param` (v1.9.0), `mute` (v1.56.0: a note-on toggles. A CC acts only when it changes side: it mutes on rising to 64 or more and unmutes on falling to 59 or less; 60-63 keeps the side it was on. The side is kept per mapping, in memory only, and forgotten at launch (or a reload), when a MIDI input is picked and when that mapping changes; until it is known again, 64 or more mutes and 59 or less only records it. So a control that stays on one side never changes the mute, except that a move while its side is unknown mutes if it lands at 64 or more). An older build keeps a mapping whose kind it doesn't know and ignores it |
| `divora.calibrated` | mic calibration completed flag (suppresses the wizard step) — v1.10.0 |
| `divora.updateCheckEnabled` | opt-out for the in-app update check (default true) — v1.12.0 |
| `divora.glyphBindings` | built-in glyph → `GlyphAction` map (any glyph → any action) — v1.15.0. The action's `kind` is one of `preset`, `tile`, `monitor`, `mute`, `panic` |
| `divora.customGlyphs` | user-recorded custom glyphs (template + action) — v1.15.0. The action's `kind` as in `divora.glyphBindings` |
| `divora.overlay` | stream-overlay background mode (`{ bg }`) — v1.16.0; plus the window's last place, all optional (`x`, `y`, `w`, `h` in physical pixels, `monitor` name, `scale` factor) — v1.51.3. A `{ bg }`-only blob still loads, and unknown keys are kept on save; plus `fit` and `hideFromRecordings` (booleans, off unless stored as `true`; `fit` counts only with the Transparent background, so a stored `fit` with Green or Magenta reads as off) — v1.55.0. While the window is fitted, the place keys keep its last unfitted place. `hideFromRecordings` is sent to the backend each time the main window loads; the backend doesn't save it |
| `divora.overlayScene` | `{version, designSize, frame, elements[]}` — v1.55.0, the first versioned key. Each field falls back on its own (see `OverlayScene` in §3). Absent or unreadable, the scene is made from `divora.overlay`'s last window size (the circle centred, at most 440 CSS px) and first written when the overlay is next seen open (opened, or adopted after a main-window reload) or a charm is first added or removed, whichever comes first. Every stored element is kept and written back, including ones this version can't show (an unknown `type`, an unrecognised `src`, a second circle, unusable geometry); only an entry without an id and a later duplicate id are dropped. Caps apply when adding, never when reading: 64 elements, 16 shown pictures, one circle, and a budget over the distinct pictures of 200 MB of files, 16,777,216 animated px and 67,108,864 px in all. A picture file is kept while any string anywhere in the scene names it. `src` holds an asset id, never a path or URL; `version` is never lowered on write. **Never the mode**: the overlay opens Locked at every launch |
| `divora.ttsVoice` | selected "Speak" voice id (or null) — v1.17.0: a preset id, a cloned id (v1.20.0), or a Critter Chatter `babble:*` id (v1.50.0) |
| `divora.ttsVolume` | "Speak" playback volume (linear 0..2) — v1.18.0 |
| `divora.ttsPreviewOnly` | "Speak" preview-only (monitor-only) toggle — v1.18.0 |
| `divora.wizardSeen` | first-run wizard completion flag |
| `divora.whatsNewSeenVersion` | last app version whose release notes were announced (bare string, not JSON) — v1.49.0 |
| `divora.voiceReading` | Voice reading panel on/off (`{ enabled }`, default false) |

**Nothing derived from the user's voice is persisted.** The only Voice reading
key is the switch above. The rolling baseline the descriptors are relative to
is built in the engine, lives for the length of one session, and is never
written to disk. It is invalidated by: switching the panel on, an engine
start/stop, a device change, and device-loss recovery — each of those rebuilds
the session, and the baseline dies with the worker thread that held it.

**The mute itself is never stored (v1.56.0)**, and neither is the side a
mute CC was last on. Every launch starts unmuted; only the bindings above
(`divora.hotkeyBindings`, `divora.midiMappings`, the glyph keys) are kept.

> **Ruling (v1.56.0): `{kind: "mute"}` keeps its name and now mutes.** Until
> v1.56.0 the `mute` glyph action toggled a status label only. It was
> documented as a mute from the start (CHANGELOG v1.15.0; the Mixer's
> "Muted · Nothing is being sent", its words until v1.56.0), so making it
> silence the voice fulfils that meaning rather than repurposing the value. A
> cast no longer toggles: it sends `set_input_muted` with the state its omen
> names (Mute or Unmute, the opposite of what Divora shows). Settings now
> calls it "Mute / unmute". No kind is renamed or removed.

---

## 6. On-disk layout

Under `%APPDATA%\com.divora.voicemod\` (created at startup, best-effort):

```
presets/<id>.json        user presets (see §4)
voices/<id>.onnx         user-installed voice-conversion models
voices/cloned/<id>/      user-cloned TTS voices (se.bin + meta.json) — v1.20.0
tts/                     on-demand-downloaded OpenVoice cloning models — v1.21.0
recordings/divora-<date>_<time>.wav   recorded modulated output (16-bit PCM mono)
recordings/divora-screen-<date>_<time>.mp4   screen recording — v1.52.0: H.264 High
                         (yuv420p, BT.709 studio range) + AAC-LC 48 kHz stereo 192 kbps.
                         Written as `<name>.mp4.part` and renamed when finished, so a
                         take cut short by a crash is left as an obviously
                         incomplete `.part`
recordings/divora-screen-<date>_<time>.voice.wav     — v1.53.0, opt-in (`stems`):
recordings/divora-screen-<date>_<time>.desktop.wav   the take's voice + soundboard
                         (mono) and, when the take has desktop audio, its
                         desktop audio before its volume (stereo), 48 kHz 32-bit
                         float, starting at the take's first frame; written as
                         `.part` and renamed after the MP4. They stop at a WAV's
                         size limit: with desktop audio, both at the same frame
                         past about 3.1 hours; without it, the voice file alone
                         past about 6.2 hours
recordings/divora-replay-<date>_<time>.mp4   a saved replay — v1.54.0: the video as the
                         hardware encoder made it (H.264, a keyframe every 2 s,
                         BT.709 studio range) + AAC-LC 48 kHz stereo, both from
                         0 with no edit list. Named by the backend from local
                         time (`-2`, `-3`, … on a clash), written as `.part` and
                         renamed when finished; a failed or cancelled save
                         removes the `.part`, and one refused for disk space
                         writes nothing
overlay/assets/<16 hex>.<png|jpg|gif|webp>   charm pictures — v1.55.0: copied in by
                         `import_overlay_asset`, named by the first 16 hex digits
                         of their content's SHA-256 and the type their first bytes
                         name, written as a temp file, synced and renamed; removed
                         when no string in `divora.overlayScene` names them. Files
                         left behind (e.g. after clearing app storage, or a delete
                         that failed) are harmless. Only the overlay window can
                         load them. `tauri dev` builds use `overlay/assets-dev/`
                         (a `tauri build --debug` build uses `overlay/assets/`)
overlay/.import-<pid>-<nanos>.part   temporary file written while a picture is
                         imported — v1.55.0; renamed into the pictures' folder when
                         complete and removed if the import fails. One left behind
                         by a crash is incomplete and safe to delete
```

Installer-bundled, read-only, next to the executable's resource dir:

```
onnxruntime.dll          ONNX runtime (also locatable via ORT_DYLIB_PATH)
voices/<id>.onnx         shipped voice models (shadowed by a same-id user file)
```

---

## 7. Windows and tray

Other programs, and the app's own code, find a window or a tray item by
these names. Labels, titles and ids are added, never renamed.

**Windows** (v1.55.0):

| Label | Title | Why frozen |
|---|---|---|
| `overlay` | `DivoraVoice Overlay` | OBS Window Capture sources match the title. `getByLabel("overlay")` and the capability use the label |

The backend builds the `overlay` window (`open_overlay`), one at a time.

**Tray menu ids** (v1.55.0; the Replay ones backfilled from v1.54.0), in
menu order:

| Id | Label | Shown |
|---|---|---|
| `show` | Show DivoraVoice | always |
| `replay-save` | Save replay | while Replay is on (arming or armed) or held (v1.54.0) |
| `replay-off` | Turn Replay off | while Replay is on (arming or armed) or held (v1.54.0) |
| `overlay-lock` | Lock the overlay | while the overlay is in Arrange (v1.55.0) |
| `quit` | Quit | always |

Each optional group is set off by separators. The tray has no item that
turns Replay on or starts Arrange.
