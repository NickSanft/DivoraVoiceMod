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
- The `overlay:*` event names and the overlay page's three command names — the same fixture, read by `src/overlay/state.test.ts` and `status.test.ts`; `wire_fixture_matches_rust` checks the `overlay:status` name and parses the `mode` / `stage` strings (v1.55.0). Since v1.59.0 the overlay page also listens to `timer-state` and calls `timer_status`; those two names are pinned by `timer.fixture.json` (below).
- The overlay window's label and title and the tray's Lock id — `src-tauri/src/overlay_machine.rs` (`title_and_label_are_the_obs_contract`); every tray menu id, label and their order — `src-tauri/src/lib.rs` (`tray_entries_for_each_combination`; v1.55.0).
- `divora.overlayScene`'s read and write rules (unknown keys, types and `src` values kept; nothing truncated) — `src/overlay/scene.test.ts` (v1.55.0); and, since v1.59.0, when a `timer` element is shown (`asTimer`), in the same file.
- The `mute` hotkey id: a press flips the mute; its release, `Mute` and `mute:` ids don't — `src-tauri/src/lib.rs` (`hotkey_mute_request_table`; v1.56.0).
- `PtmState` keys and the `PtmMode` values `"always"`, `"apply"`, `"bypass"` (nothing else parses) — `divora-core/src/audio/engine.rs` (`ptm_state_json_keys_are_frozen`, `ptm_mode_wire_values_are_frozen`; v1.58.0). `LevelUpdate`'s `ptm` — `src-tauri/src/lib.rs` (`level_update_json_keys_are_frozen`, and `the_level_tick_carries_ptm`, which builds the tick from a real engine; v1.58.0).
- `set_ptm_held`'s sources: `"key"`, `"button"`, `"midi"` parse; `"hotkey"`, other spellings and `""` don't — `src-tauri/src/lib.rs` (`ptm_page_source_wire_values_are_frozen`; v1.58.0).
- The `ptm` hotkey id: a press holds push-to-modulate and its release lets go; `PTM`, `ptm:toggle`, `mute` and `sb:ptm` do nothing to it — `src-tauri/src/lib.rs` (`hotkey_ptm_request_table`; v1.58.0).
- `set_ptm_mode` and `set_ptm_held` stay sync commands — `src-tauri/src/lib.rs` (`ptm_commands_stay_sync`; v1.58.0).
- `TimerSnapshot` / `TimerState` / `TimerRun` / `TimerDefWire` keys, the phase and kind strings, the timer id grammar, the `tm:` hotkey-id table, the timer limits, the run samples (what the page stores for a state, and what the backend seeds from it), and the `timer-state` event and `timer_status` command names — `src/overlay/timer.fixture.json`, read by `src-tauri/src/timers.rs` (`timer_wire_fixture_matches_rust`, `sync_entry_keys_are_the_fixtures`, `timer_id_table`, `hotkey_timer_request_table`, `seed_table`), `src-tauri/src/lib.rs` (`the_timer_commands_are_registered`) and `src/overlay/timerState.test.ts` / `src/data/timers.test.ts` (v1.59.0).
- The timer actions' wire (v1.60.0): the `TimerAction` kinds, their fields, where each runs and which list offers it; the action limits; how an action is read (the valid and skipped samples) and how a list is read (the list samples); `TimerActionsEvent`'s keys, the `why` and `hook` strings, and one event per `why` as the backend builds it from its plan; `TimerDefWire`'s optional `onStart` / `onFire`; the `timer-actions` and `soundboard-stopped` event names and the `finish_timer` command name — the same fixture's v1.60.0 keys, read by `src-tauri/src/timer_actions.rs` (`action_kinds_match_the_fixture`, `action_from_wire_table`, `to_wire_round_trips`, `hooks_from_wire_reads_eight`, `actions_event_samples_are_rusts`, `actions_event_samples_are_plans`), `src-tauri/src/timers.rs` (`timer_wire_fixture_matches_rust`, `sync_entry_keys_are_the_fixtures`), `src-tauri/src/lib.rs` (`the_timer_commands_are_registered`) and `src/data/timerActions.test.ts` / `src/data/timers.test.ts`. v1.59.0's `events`, `commands` and `limits` objects are left as they were, so `timerState.test.ts`'s exact checks of them still hold.
- A timer's actions: what runs at a start, at zero, on End now, on a fire more than 30 s late, after a sleep or a close, at a restart mid-run, and on a Reset or a delete — `src-tauri/src/timer_actions.rs` (`plan_table`, `late_boundary`, `restart_reapplies_only_the_mute`, `a_stored_end_runs_nothing`, `revert_never_runs_at_a_start`, `route_table`) and `src-tauri/src/timers.rs` (`op_table`, `a_cancel_undoes_what_its_start_did`; v1.60.0).
- The `tm:` hotkey ids: a press of `tm:<id>:toggle`, `tm:<id>:reset` or (since v1.60.0) `tm:<id>:finish` with a valid id acts on that timer; a release and any other spelling don't — `src-tauri/src/timers.rs` (`hotkey_timer_request_table`), and `src-tauri/src/lib.rs` (`the_shortcut_handler_runs_the_timer_hotkey`; v1.59.0).
- The nine Hourglass commands stay sync (eight in v1.59.0; `finish_timer` since v1.60.0) — `src-tauri/src/lib.rs` (`timer_commands_stay_sync`; v1.59.0).
- The main window sends five features' state-changing commands one at a time, one queue per feature, each once the one before it in that queue has answered or failed: the mute's set and toggle, push-to-modulate's held edges, mode and boot restore, and Hourglass's syncs, labelled commands (`finish_timer` since v1.60.0) and `tm:` key registers and unregisters (v1.59.0); and since v1.60.0 the effect chain's (`set_effect_chain`, `set_effect_param`, `set_effect_enabled`, `set_voice_model`) and the soundboard's (`play_soundboard_clip`, `stop_soundboard_clip`, `stop_all_soundboard_clips`, and Speak's saved-clip play). The reads (`input_mute`, `timer_status`) aren't held back, and no other command the page sends goes through these queues (the stream overlay's controller has kept its own since v1.55.0, for opening, closing, Fit, the background and pictures) — `src/data/inOrder.test.ts` and `src/stores/app.test.tsx` (the "one command at a time" and "one mute command at a time" tests, v1.59.0; "one chain and one soundboard command at a time", which holds each answer and lands what is in flight together reversed, v1.60.0). Being sync orders only commands that aren't in flight together. A timer's own actions in the backend (its mute, its Stop all clips and lines) go through none of these queues.
- `divora.timers`' read and write rules (unknown keys, kinds and hotkey verbs kept; a field that can't be read falls back; `version` never lowered) — `src/data/timers.test.ts` (`readTimers`; v1.59.0); since v1.60.0 `onStart`, `onFire` and `presetReturns` kept as written, whatever they hold (`readTimers`), a list sent only for a countdown with an action that runs (`defsForSync`), and a list read entry by entry with what can't run kept in place (`src/data/timerActions.test.ts`: `readStoredActions`, the editing-by-index rows, `readReturns`). What `sync_timers` does with each entry — `src-tauri/src/timers.rs` (`entry_from_wire_table`, `entry_from_wire_falls_back_field_by_field`, `run_from_wire_table`) and `src-tauri/src/timer_thread.rs` (`sync_skips_bad_entries_and_stops_at_64`); and with its `onStart` / `onFire`, since v1.60.0 — `src-tauri/src/timer_actions.rs` (`action_from_wire_table`, `hooks_from_wire_reads_eight`, `hooks_from_wire_keeps_one_line`, `to_wire_round_trips`).
- Every command wrapper's command-name string — `src/audio/api.test.ts` (the screen-recording ones since v1.53.0, the stream-overlay ones since v1.55.0, the mute ones since v1.56.0, the push-to-modulate ones since v1.58.0, the Hourglass ones since v1.59.0, and `finish_timer` since v1.60.0).

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
| `set_ptm_mode` | `mode: "always" \| "apply" \| "bypass"`, `restore?: bool` | `PtmState`, the state after — v1.58.0. Without `restore`, or with `false` (the Mixer card): sets the mode and lets go of every hold, a held global key included, which must then be pressed again; this is the way out of a stuck hold. With `restore: true` (the main window, once each time the page loads, with the mode saved in `divora.ptm`): sets the mode, lets go of the page's holds (the in-app key, the Mixer's button, MIDI) and keeps a global key that is still held, so a reload doesn't drop it. Idempotent: nothing to change moves nothing, `seq` included. Never fails, apart from Tauri's own ``"invalid args `mode` for command `set_ptm_mode`: {e}"`` (a missing or unknown `mode`) and the same for a `restore` that isn't a boolean |
| `set_ptm_held` | `source: "key" \| "button" \| "midi"`, `held: bool` | `PtmState`, the state after — v1.58.0. One source's hold, by level. Idempotent: a set to the value that source has moves nothing, `seq` included. `"hotkey"` is refused with Tauri's own ``"invalid args `source` for command `set_ptm_held`: {e}"``: only the backend holds the global key |

Divora's own mute of the voice (v1.56.0) — on the call, the headphone monitor
and Divora's recordings (WAV, screen recordings and their stems, Replay).
Soundboard clips, Speak and Critter Chatter still play. Engine-wide: it can be
set with the engine stopped, it outlives Stop, Start and device-loss rebuilds,
and it is never stored; every launch starts unmuted. (Since v1.60.0 a
countdown whose At start mutes, running or paused when Divora closed, mutes
again once the page's first `sync_timers` picks its run up: §1 "Hourglass".
A timer's Mute my voice and Unmute my voice are the backend's own plain
sets, made without these commands.) Every change adds 1 to
`seq`. The three commands are sync (they run on the main thread, in turn with
each other and with the hotkey), and each is one atomic operation;
`LevelUpdate` carries the same state on every tick. Being sync doesn't order
commands in flight together (§1 "Hourglass"), so since v1.59.0 the main
window sends its sets and toggles one at a time, each once the one before it
has answered.

Push-to-modulate (v1.58.0) switches your effects off and on for everything the
mute covers, and in the same place: the call, the headphone monitor and
Divora's recordings. Soundboard clips, Speak and Critter Chatter are never
switched. `mode` says what a hold does: `"always"` nothing (push-to-modulate
off: the effects always apply), `"apply"` the effects apply only while
something holds it, `"bypass"` they are off while something holds it. Four
sources hold it, each on its own: the global key (the backend's alone), the
in-app key, the Mixer's **Hold to switch** button and MIDI; it is held while
any of them holds it, so one letting go never lets go of another.
Engine-wide: it can be set with the engine stopped, and it outlives Stop,
Start and device-loss rebuilds, holds included. The backend never stores it:
every launch of the app starts at `"always"`, nothing held, `seq` 0, and the
page restores the mode it saved. Every change adds 1 to `seq`. Both commands
are sync: each runs to completion on the main thread, one at a time. That
doesn't order commands in flight together, which can reach Rust out of send
order (§1 "Hourglass"), so the main window sends both one at a time, each
once the one before it has answered (v1.59.0): that is what keeps a press from
overtaking its release. Each is one atomic operation; `LevelUpdate` carries
the same state on every tick.

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
| `clone_voice` | `name: string`, `referencePath: string` | `ClonedVoiceInfo` / error — v1.20.0. v1.57.0: runs off the main thread; one voice at a time, a second refused with "Divora is still making a voice. Try again when it's done." |
| `start_voice_recording` | — | v1.57.0: `string \| null` — the sentence to read when the take will become a VoxCPM voice, decided here and kept for the take, or null to speak freely (was `—`) / error ("start the engine first"; v1.57.0: "Divora is still making a voice…" while a clone is being made) — v1.23.0 |
| `stop_voice_recording` | `name: string` | `ClonedVoiceInfo` / error — v1.23.0. v1.57.0: off the main thread, the wait for the WAV included; makes the engine the start chose; refused like `clone_voice` while another voice is being made |
| `list_cloned_voices` | — | `ClonedVoiceInfo[]` — v1.20.0 |
| `rename_cloned_voice` | `id: string`, `name: string` | — / error — v1.43.0 (display name only; the `id` is stable) |
| `delete_cloned_voice` | `id: string` | — / error — v1.20.0 |
| `clone_models_status` | — | `{ ready: bool }` — v1.21.0 |
| `download_clone_models` | — | — / error (downloads ~157 MB on-demand) — v1.21.0. v1.57.0: off the main thread; answers when the set is down (at once if already on disk). One model download at a time: another while one runs rejects with "A model download is already running."; a connection silent for 60 s fails it |
| `voxcpm_status` | — | `{ available: bool, readPrompt: string }` — v1.24.0 |
| `download_voxcpm_models` | — | — / error (downloads ~1.6 GB on-demand) — v1.24.0. v1.57.0: as `download_clone_models` |
| `voxcpm_gpu_status` | — | `{ supported: bool, modelPresent: bool }` — v1.30.0 |
| `download_voxcpm_gpu_model` | — | — / error (downloads ~1.2 GB on-demand) — v1.30.0. v1.57.0: as `download_clone_models` |
| `model_download_status` | — | `CloneDownloadProgress \| null` — the running model download's latest progress, or null; for a page loaded mid-download — v1.57.0 |

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
| `stop_all_soundboard_clips` | — | — (v1.60.0: it also drops a timer's line that is waiting or still being prepared, so none prepared before the stop plays after it. It sends no event; a timer's Stop all clips and lines sends `soundboard-stopped`, §2) |
| `set_soundboard_master_gain` | `gain: f32` | — |

### Virtual mic & hotkeys

| Command | Args | Returns |
|---|---|---|
| `detect_virtual_mic` | — | `VirtualMicStatus` |
| `register_global_shortcut` | `id`, `accelerator` | — / error. v1.58.0: when `id` already holds exactly that key (any spelling of it) and the key is still registered, it does nothing and answers `Ok`, so a held `ptm` key stays held through a page reload's re-sync; a key whose earlier register failed is registered again. For `ptm`, every call that gets past that check lets go of the global key's hold, whatever it answers: a new key, a key the parser refuses (Shift with a digit arrives as "!"), an old key that stays registered for another id sharing it |
| `unregister_global_shortcut` | `id` | — / error. v1.58.0: for `ptm`, lets go of the global key's hold, even when the key stays registered for another id that shares it |
| `unregister_all_global_shortcuts` | — | — / error. v1.58.0: lets go of the global key's hold on push-to-modulate |

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
| `ptm` | push-to-modulate (press and release). v1.58.0: acted on by the backend, so it doesn't go through the page: a press holds push-to-modulate, its release lets go (which way that switches your effects is the mode, `set_ptm_mode`). The `global-shortcut` event still goes out, press and release, and the frontend does nothing with it. Unbound by default. The prefix `ptm:` is reserved (a latched push-to-modulate, later) |
| `panic` | stop all soundboard clips (v1.60.0: and a timer's line, playing, waiting or being prepared) |
| `monitor` | toggle hearing yourself |
| `sb:<clipId>` | play one soundboard tile (prefix reserved) |
| `rec:<recorder>` | start / stop a recorder (prefix reserved) — v1.52.0. Defined: `rec:screen` (the screen recorder), `rec:wav` (the WAV recorder). Both unbound by default |
| `replay:<action>` | Replay (prefix reserved) — v1.54.0. Defined: `replay:save` (save the replay), `replay:toggle` (turn Replay on / off; it works once Replay has been turned on from the Record screen card). Both unbound by default |
| `overlay:<action>` | the stream overlay (prefix reserved) — v1.55.0. Defined: `overlay:arrange`, which switches the open overlay between Arrange and Locked. It is acted on by the backend when pressed; the frontend only shows a notice while the overlay is off or starting. Unbound by default. A different namespace from the `overlay:state` / `overlay:*` **event** names, despite the shared spelling |
| `mute` | mute or unmute your voice (a toggle) — v1.56.0. Acted on by the backend when pressed (a release does nothing), so it doesn't go through the page; the `global-shortcut` event still goes out, and the frontend does nothing with it. Unbound by default. The prefix `mute:` is reserved for later variants |
| `tm:<timerId>:<verb>` | an Hourglass timer (prefix reserved) — v1.59.0. Defined: `toggle` (idle, done or ran out → start from the full length, a count-up from 0; running → pause; paused → resume) and `reset` (back to idle); and since v1.60.0 `finish` (End now: a running or paused countdown finishes now, as `finish_timer`). A start, a reset and End now run what `arm_timer`, `reset_timer` and `finish_timer` run (§1 "Hourglass"). Acted on by the backend when pressed (a release does nothing), so it doesn't go through the page; the `global-shortcut` event still goes out, and the frontend does nothing with it. A press for a timer the backend doesn't run does nothing. Unbound by default; the keys are stored with each timer in `divora.timers`, not in `divora.hotkeyBindings`. Any other verb or spelling does nothing. A timer id is 1-32 characters of `a-z`, `0-9` and `-`, starting and ending with a letter or digit, never a Windows device name (`con`, `prn`, `aux`, `nul`, `com0`-`com9`, `lpt0`-`lpt9`), so never contains `:` |

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
It relies on `set_overlay_mode`, `overlay_handshake`, `overlay_alive` and,
since v1.59.0, `timer_status` (§1 "Hourglass"), and must be granted them if
an app manifest is ever added. Its own capability,
`src-tauri/capabilities/overlay.json`, grants only event listen / unlisten /
emit / emit-to and dragging its window; `default.json` now covers `main`
alone and no longer grants creating webview windows or setting a window's
position or size, which the backend does.

### Opening links and folders (v1.57.0)

| Command | Args | Returns |
|---|---|---|
| `open_external` | `target: string` — an `http(s)://` link, or the absolute path of a folder that exists | — / error ("Divora opens only web links and folders.", or, for a link, why the desktop couldn't open it: no desktop shell, Explorer not answering within 3 s, or Explorer's own open failing). Opens it from the desktop's Explorer, so a browser it starts isn't in Divora's process tree (which "Everything except Divora" leaves out). A folder the desktop can't open is opened by starting the file manager from Divora; for a link that fails, the page falls back to the shell plugin's `open` |

### Hourglass (v1.59.0)

Countdowns and count-ups the backend keeps. All nine commands are sync
(eight in v1.59.0; `finish_timer` since v1.60.0): each
runs to completion on the main thread, one at a time and in turn with the
`tm:` hotkeys. Being sync doesn't keep commands in the order they were sent:
Tauri sends each invoke as its own request, and commands in flight together
can reach Rust out of send order (measured in the real app on this PC with 40
sent at once). A caller that needs order sends each command only
once the one before it has answered; the main window does, through one queue
for every command that changes the timers (`timer_status` stays out) and the
`tm:` keys' registers and unregisters, so a quick Pause then Resume can't land
reversed. Every one but `timers_dir` answers the whole
`TimerSnapshot`; of two snapshots, the larger `seq` is newer, and the
`timer-state` events (§2) carry the same shape. Each of those brings the
timers up to date before it acts — a countdown already due finishes, a sleep since the
last reading is handled — so a Pause sent just after a countdown's deadline
answers it `done`. A labelled command that doesn't fit the timer's phase
changes nothing and answers the timer as it is. `"no such timer"` is the
answer for an `id` the backend doesn't run (never synced, deleted, or kept
without being run).

Since v1.60.0 a countdown can carry two lists of actions, `onStart` (At
start) and `onFire` (At zero), and the backend decides what each change runs:
it runs the mute, Stop all clips and lines, a replay save and a line itself,
and tells the main window what it runs (preset switches and clips) with a
`timer-actions` event (§2). The rule is under `TimerAction` notes (§3). A
command's answer doesn't wait for the actions it caused, and can land before
or after their event. Pause, Resume and Adjust run no actions. Count-ups
have no lists.

| Command | Args | Returns |
|---|---|---|
| `sync_timers` | `timers: TimerDefWire[]` — every stored timer, in list order | `TimerSnapshot`. The definitions become `timers`. Reads each entry on its own and never fails, apart from Tauri's own ``"invalid args `timers` for command `sync_timers`: {e}"`` when `timers` isn't a list: an entry that isn't an object, has no `id` passing the timer id grammar (§1 hotkey ids) or no string `kind`, repeats an earlier entry's id, or comes after the 64th is skipped; a `name` that isn't 1-40 characters after trimming runs as "Countdown" or "Count-up"; a `durationMs` that isn't a finite number is 300 000 (5:00), any other is rounded and held to 1 000..86 400 000; a `resume` that can't be read is ignored and the timer starts idle; unknown keys are ignored. The first 16 entries of kind `"countdown"` or `"countup"` run; any other kind, and a known kind past the 16th, is kept, not run, and its file stays. A timer already running keeps its run through a new name or length; a new kind makes it idle. `resume` seeds a timer only for an id the backend hasn't accepted since it started (`TimerRun` notes, §3); after that the backend's own state wins. v1.60.0: an entry may carry `onStart` and `onFire` (`TimerAction[]`, §3). Each is read on its own: absent or not a list → empty; only its first 8 entries are read, each on its own (one that can't be read — an unknown kind, a missing or bad field — is skipped, the rest kept); a `speak` after the list's first is skipped; a count-up's lists are ignored. A list change, like a new name or length, keeps the run and runs nothing; the lists are read when something happens to the timer, so an `onFire` change counts for the run going on and an `onStart` change from the next start. A sync that stops a running or paused countdown — it changed kind, or it no longer runs (left out, past the 16th, or a kind this build doesn't run) — runs the At zero actions that undo its start (`timer-actions`, `why` `reset` for a kind change, else `deleted`). A seed that picks up a running or paused countdown runs its At start's `setMuted` actions again (so a break mutes you again), with no event; one that finds a running countdown's end passed while Divora was closed runs At zero's putting-back actions (`ranOut`) |
| `arm_timer` | `id: string` | `TimerSnapshot` / `"no such timer"`. Idle, done or ran out → running from the full length (a count-up from 0); v1.60.0: a countdown's start runs its At start list (`timer-actions`, `armed`). Otherwise unchanged |
| `pause_timer` | `id: string` | `TimerSnapshot` / `"no such timer"`. Running → paused. Otherwise unchanged |
| `resume_timer` | `id: string` | `TimerSnapshot` / `"no such timer"`. Paused → running. Otherwise unchanged |
| `reset_timer` | `id: string` | `TimerSnapshot` / `"no such timer"`. Anything → idle. v1.60.0: a running or paused countdown's Reset also runs the At zero actions that undo its start — Unmute my voice if the At start it started with muted, and Back to the preset before (`timer-actions`, `reset`); it isn't a finish (`fires` unchanged), and nothing else on either list runs. An ended or idle one runs nothing |
| `adjust_timer` | `id: string`, `deltaMs: number` | `TimerSnapshot` / `"no such timer"` / `"deltaMs must be a whole number of milliseconds, at most 24 hours either way"`. A running or paused countdown's time left moves by `deltaMs`, to at most 24 h, and, going down, to no less than 1 s (a time left already under 1 s stays as it is), so this never ends one; its `totalMs` moves by what the time left moved and can pass 24 h. A running or paused count-up's time counted moves by `deltaMs`, never below 0. Idle, done or ran out: unchanged. The app sends ±30 000 |
| `timer_status` | — | `TimerSnapshot`, the set now. Main and the overlay page each call it once after they start listening to `timer-state`, since a paused or idle set sends no events |
| `timers_dir` | — | `string`, the absolute path of the timers' folder (§6), created if missing / `"couldn't create the timers folder: {e}"` |
| `finish_timer` | `id: string` | `TimerSnapshot` / `"no such timer"` — v1.60.0, End now. A running or paused countdown finishes now: `done`, `fires` + 1, and its whole At zero list runs (`timer-actions`, `endNow`, `lateMs` 0). One already past a deadline nothing had read finishes as that deadline's (the tick before the op takes it), so it runs as a finish that late: `zero` at 30 000 ms or less, `late` beyond. Idle, done, ran out, or a count-up: unchanged |

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
| `overlay:state` | `OverlayState` | main → overlay window, on state change, while the stream overlay is open (v1.16.0). v1.56.0: each emit waits for the one before it, and after the main window loads nothing goes until it knows the mute (its `input_mute` read settles or a state arrives; at most 500 ms). The payload is unchanged: while muted, `status` is `"muted"`. v1.58.0: and nothing goes until push-to-modulate's boot `set_ptm_mode` (with `restore`) settles, by its answer or a failure, or 500 ms pass; a level tick doesn't count for this one. `status` now follows push-to-modulate as the engine switches it: `"modulated"` while an effect other than the noise gate is on and push-to-modulate isn't switching them off, so with push-to-modulate off (the default) a preset with effects reads `"modulated"` where it used to read `"clean"`; a gate-only chain reads `"clean"`. The shape and the three values are unchanged |
| `overlay:ready` | — (no payload) | overlay window → main, once the overlay is listening for `overlay:state`; main answers with the current state (v1.51.3). v1.55.0: main also re-pushes the scene on it, with `ack` null, and the overlay listens for `overlay:scene` and `overlay:status` before sending it. The backend learns ready through `overlay_handshake`, not this event. v1.59.0: the overlay also listens for `timer-state` before sending it |
| `overlay:status` | `OverlayStatus` | backend → all windows, on every change of any field — v1.55.0. The same shape is returned by every command in §1 "Stream overlay" except `overlay_alive` and the two asset commands |
| `overlay:scene` | `ScenePush` | main → overlay window, while it is open: on every scene change, on each edit main acknowledges (changed or not), and on each `overlay:ready` — v1.55.0 |
| `overlay:edit` | `SceneEdit` | overlay window → all (heard by main): on pointerup of a gesture that changed something; 300 ms after the last arrow nudge or keyboard resize (sooner when another editing key, a press, losing focus or Lock comes first); on Delete; on `[` `]` `{` `}` — v1.55.0 |
| `clone-model-download` | `CloneDownloadProgress` | during the on-demand voice-cloning model download (v1.21.0); VoxCPM's (v1.24.0) and the GPU model's (v1.30.0) too, about every MiB and at each file's end; v1.57.0: with `kind` |
| `clone-model-download-done` | `{ kind, error: string \| null }` | once when a model download ends, after the next one can start and before the command answers; `error` is the string the command rejects with. None for a set already on disk — v1.57.0 |
| `voice-reading` | `VoiceReadingUpdate` | ~5 Hz, **only while the Voice reading panel is on**; silent otherwise |
| `screen-recording` | `ScreenRecordingUpdate` | on every phase change of a take, and about once a second while one records (v1.52.0); silent otherwise — also while only Replay runs |
| `replay-buffer` | `ReplayStatus` | on every change of Replay's state or `saving`, and about once a second while armed; a held ring only on change (v1.54.0) |
| `replay-saved` | `ReplaySaved` | the outcome of every save attempt, refusals included — exactly one per attempt (v1.54.0); since v1.60.0 a timer's Save a replay too, which the backend makes without the command |
| `timer-state` | `TimerSnapshot` | backend → all windows, on every change of a timer the backend runs: a command or `tm:` key that changed one, every `sync_timers`, a countdown finishing or running out, a sleep handled, an anchor moved after a wall-clock change, and each whole-second change of a running timer's `text`; nothing on a schedule beyond that, so a paused or idle set sends nothing — v1.59.0. Sent by the backend's timer thread alone, in `seq` order; a command's answer can arrive after the event that follows it, so readers keep the snapshot with the larger `seq` |
| `timer-actions` | `TimerActionsEvent` | backend → all windows, once for each countdown start (`armed`) and each end (`zero`, `endNow`, `late`, `ranOut`, `reset`, `deleted`), even one with no page actions (an end still lets the page settle a preset return); never for a restart's re-mute — v1.60.0. Sent by the timer thread, before its `timer-state` for the same change; a command's answer can arrive before or after either. Only the main window acts on it. An event no page listens for (a reload, the moments before the page's listeners are up) is lost: nothing sends it again |
| `soundboard-stopped` | `null` | backend → all windows, when a timer's Stop all clips and lines has stopped every clip and line (v1.60.0). The page's own `stop_all_soundboard_clips` sends none |

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
channel that reaches a hidden page whoever changed it. v1.58.0 adds
push-to-modulate (`ptm`, a `PtmState`) for the same reasons: the backend
changes it with no page involved (the global key), and the page needs it to
say Clean or Modulated. Hourglass (v1.59.0) has its own event, `timer-state`,
for the reading's reasons: it changes about once a second, and only for
people who use timers.

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
                    inputMuted, inputMuteSeq,   // v1.56.0: the InputMute pair
                    ptm }                       // v1.58.0: a PtmState, every
                                                // tick, engine running or not;
                                                // optional in TS (absent is no
                                                // information, never "always")
InputMute         { muted, seq }    // v1.56.0: seq counts changes since launch;
                                    // of two states, the larger is newer
PtmState          { mode: "always" | "apply" | "bypass",
                    held (some source holds it),
                    bypassed (what the engine is switching to from this
                      state on: toward the effects at the next buffer, away
                      from them only after the chain's reported latency, so
                      what you said while they were on keeps them),
                    seq }           // v1.58.0: seq counts changes since launch;
                                    // of two states, the larger is newer.
                                    // The commands' answer and LevelUpdate.ptm
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
CloneDownloadProgress { kind, file, fileCount, received, total }  // v1.21.0;
                  // kind v1.57.0: "clone" | "voxcpm" | "voxcpmGpu"
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
                      element, as in OverlayScene; v1.59.0: or a "timer"
                      element, which main adds itself — it never comes from
                      the overlay) }
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
// SceneElement: { id, type, x, y, w, h, z }, plus { src, nw, nh, anim, bytes, name } for "image", plus { timer } for "timer" (v1.59.0)
// id: an entry that isn't an object, or has no non-empty string id, is dropped; a later duplicate is dropped; nothing else is
// type: "spellcircle" (the first usable one is shown), "image", "timer" (shown since v1.59.0; kept, not shown, from v1.55.0 to v1.58.0), or anything else (kept verbatim, not shown)
// shown: an id of 1..64 characters and usable x/y/w/h, and for "image" a valid src, for "timer" a `timer` passing the timer id grammar (§1 hotkey ids); everything else is kept, not shown
// timer: a timer id, never a name; the element is shown whether or not that timer exists, and its face draws only while the backend's latest TimerSnapshot has that timer. Its aspect is its stored w·designSize.w / h·designSize.h (3:1 when added); it counts toward the 64 elements only, not the 16 pictures or the budget
// x/y/w/h: fractions of designSize, never clamped on read; usable when all four are finite and w, h > 0
// z: finite, else its array index (computed, not written)
// src: an asset id matching ^[0-9a-f]{16}\.(png|jpg|gif|webp)$, never a path or URL; anything else keeps the image, unshown
// nw/nh: the picture's own pixels; integers ≥ 1, else the aspect comes from the stored w·designSize.w / h·designSize.h
// anim: boolean (anything else → src ends in .gif); bytes: the file's size (missing or not positive → 0); both only for the scene budget
// name: shown cut to 120 characters, "Picture" when missing or blank (computed, not written)
// any other key, at any level: kept and written back unchanged
TimerSnapshot     { seq, timers: TimerState[] }      // v1.59.0: seq rises with
                                                     // every snapshot built,
                                                     // answers and events alike;
                                                     // of two, the larger is
                                                     // newer. Only the timers the
                                                     // backend runs, in sync order
TimerState        { id, name,
                    kind: "countdown" | "countup",
                    phase: "idle" | "running" | "paused" | "done" | "expired",
                    ms (a countdown's time left, rounded up to whole ms; a
                      count-up's time counted, rounded down; idle: a
                      countdown's length, a count-up's 0; done, expired: 0),
                    totalMs (a countdown's length ± adjusts since it started,
                      which can pass 24 h; 0 for a count-up),
                    text ("MM:SS" under an hour, "H:MM:SS" from one: "05:00",
                      "1:02:03", "100:00:00"; a countdown rounds up to whole
                      seconds, a count-up down; always agrees with ms),
                    anchorMs (nullable: while running, the Unix ms a
                      countdown ends at, or a count-up effectively started
                      at; set when the run is set or a held wall-clock
                      change moves it, not per snapshot; null otherwise),
                    fires (how often this timer has finished since the
                      backend began running its id in this run of Divora;
                      it isn't stored, so a restart, or a deleted and
                      re-added id, starts at 0) }
                                                     // v1.59.0
TimerRun          { phase: "running" | "paused" | "done" | "expired",
                    ms?, totalMs?, anchorMs? }       // v1.59.0: whole numbers
                    // (ms, totalMs ≥ 0). Stored as divora.timers' `run`, sent
                    // as `resume`. Running: anchorMs (required) and a
                    // countdown's totalMs; paused: ms and a countdown's
                    // totalMs; done or expired: totalMs. Idle has none
TimerDefWire      { id, name, kind, durationMs, resume?: TimerRun,
                    onStart?: TimerAction[], onFire?: TimerAction[] }
                    // v1.59.0: an argument only (sync_timers). The page sends
                    // validated fields only; later releases add fields, which
                    // older builds ignore. v1.60.0: onStart / onFire, a
                    // countdown's At start and At zero; absent → empty to the
                    // backend. The page sends each only for a countdown, and
                    // only when it holds an action that runs (as resume goes
                    // only with a run)
TimerAction       { kind: "preset", presetId } | { kind: "randomPreset" }
                  | { kind: "revertPreset" } | { kind: "tile", tileId }
                  | { kind: "speak", text, voiceId } | { kind: "saveReplay" }
                  | { kind: "setMuted", muted: boolean } | { kind: "panic" }
                    // v1.60.0: in sync_timers, in divora.timers (§5), and
                    // out in timer-actions. preset, tile and panic keep
                    // GlyphAction's shape
TimerActionsEvent { seq, id, hook: "start" | "end",
                    why: "armed" | "zero" | "endNow" | "late" | "ranOut"
                         | "reset" | "deleted",
                    lateMs, actions: TimerAction[] }  // v1.60.0
```

**`TimerState` notes (v1.59.0).**

- `done`: a countdown reached zero while the PC was awake and Divora was
  running, and finished — late, if Divora was held up; or (v1.60.0) End now
  finished it. `expired`: it reached
  zero while the PC slept or Divora was closed, and didn't finish. Both show
  `00:00`. A count-up is only ever `idle`, `running` or `paused`. (From the
  code and fake-clock tests; no real sleep was tried. Two edge cases differ,
  both from the code: a second sleep of 5 s or less within a minute of one
  Divora handled counts as awake time, so a countdown due in it ends `done`,
  up to about 5 s late; and on a clock that doesn't count sleep, with no
  counted sleep in the last minute, a resume signal that lands while Divora is
  held up more than 500 ms makes countdowns due in that hold `expired` with
  the PC awake, and ones due during the sleep itself end `done`, late by its
  length.)
- A countdown never finishes before its deadline unless End now ends it
  (`finish_timer` or the `finish` key, v1.60.0), and finishes at most once
  until it is started again (`arm_timer`, or the hotkey's toggle from `done`
  or `expired`); each finish, End now's included, adds 1 to `fires`.
- The page reads a snapshot with every timer's keys checked: a timer entry it
  can't read (a missing key, a phase or kind it doesn't know) is dropped and
  the rest kept, keys a later version adds are ignored, and a snapshot without
  a whole-number `seq` and a `timers` list is ignored whole.
- `text` is the one string every surface shows — the overlay, the text file,
  Settings — so its format is part of the contract: no sign, no overtime, and
  paused shows the digits as they stopped.

**`TimerAction` notes (v1.60.0).**

- Fields: `presetId` a non-empty string, any length; `tileId` and `voiceId`
  strings of 1 to 128 characters; `text` 1 to 200 characters (Unicode scalar
  values) after trimming, read and sent trimmed; `muted` a boolean. Nothing
  is coerced; extra keys and another kind's fields are ignored. An entry
  that isn't one of these — not an object, a `kind` missing or unknown (the
  glyph's `monitor` and `mute` included), a field missing or of the wrong
  type or length — can't be read: the backend skips it, and the page keeps
  it in storage and never sends it. The page also refuses a `text` holding a
  lone surrogate.
- A list: only its first 8 entries are read, readable or not; a `speak`
  after the list's first readable one is skipped (one line per list).
- Where each runs: `preset`, `randomPreset`, `revertPreset` and `tile` in the
  main window; `speak`, `saveReplay`, `setMuted` and `panic` in the backend.
  `revertPreset` never runs at a start.
- When: At start runs when a countdown starts from idle, done or ran out
  (`arm_timer`, or the toggle key from those), never on resume. At zero runs
  whole at 00:00 when 30 000 ms late or less, and on End now. A countdown
  found due more than 30 000 ms late, or that ran out while the PC slept or
  Divora was closed, runs only At zero's putting-back actions: `setMuted
  {muted: false}` and `revertPreset`. A Reset (or a kind change) or a delete
  of a running or paused countdown runs only those of them that undo its
  start: `setMuted {muted: false}` when the `onStart` its run started with
  has `setMuted {muted: true}`, and `revertPreset` (which the page acts on
  only with a return the start noted, §5). A relaunch that picks a running or
  paused countdown up runs its `onStart`'s `setMuted` actions again, in the
  backend only, with no event. Nothing else runs. Lists are read when the
  change happens, so an `onFire` edit counts for the run going on.
- `setMuted` is a plain set, never a toggle: the last one wins, and an
  unmute unmutes a mute made any other way.

**`TimerActionsEvent` notes (v1.60.0).**

- `actions` is the page's half only (`preset`, `randomPreset`,
  `revertPreset`, `tile`), in list order; the backend runs its own half
  without the page. It can be empty: an end still lets the page settle a
  preset return.
- `lateMs` is the finish's lateness for `zero` and `late`, and 0 for every
  other `why`.
- `seq` rises by 1 for each start or end the backend plans in this run of
  Divora; a restart's re-mute takes one and isn't sent, so a gap is
  expected. Of two, the larger is newer, and the page acts on each once.
- The page reads it with every key checked and ignores keys a later version
  adds; an action in it the page can't read is skipped.

**`TimerRun` notes (v1.59.0): what a stored run seeds.** The backend seeds a
timer from its `resume` only the first time it accepts that id after it
starts; otherwise its own state wins. Time passes while Divora is closed:

- running countdown: time left = `anchorMs` − now, at most `totalMs` and 24 h,
  so a clock set back while Divora is closed can add back time that had
  already run, never past `totalMs`; 0 or less → `expired`;
- running count-up: time counted = now − `anchorMs`, from 0 to 10 000 h;
- paused countdown: paused at `ms` when it is 1..86 400 000, its total the
  larger of `totalMs` and `ms`;
- paused count-up: paused at `ms`, at most 10 000 h;
- done or expired countdown: the same phase, its total `totalMs` or its
  length;
- anything else — a running or paused countdown without `totalMs` (or a
  running one with 0), a count-up done or expired, a run that isn't one of
  the shapes above — starts idle, and the timer is kept. `fires` is never
  stored or seeded.

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
  held, as through a pause. v1.58.0: and after push-to-modulate's switch,
  which sits between the chain and loudness, so while push-to-modulate has
  the effects off the wet tap carries your voice without them — what the
  call hears.
- `dry` is the microphone **before any effect**; `wet` is the same voice
  **after the chain**. Soundboard / Speak / Critter Chatter audio is mixed in
  downstream of the wet tap, so it appears on neither half. v1.58.0: `dry` is
  the voice the chain is given (after Divora's mute closes the chain's input),
  delayed by the chain's reported latency, so the two halves are the same
  moment of speech; `wet` is what the voice stage sends — the chain's output,
  or that delayed `dry` while push-to-modulate has the effects off.
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
- v1.58.0: `wetBypassed` also lights while push-to-modulate has the effects
  off, for every preset. It goes with the `wet` numbers beside it, never ahead
  of them: the voice crossing between the effects and the dry drops the
  after-effects window, the previous numbers and their verdict stay until
  0.4 s of new audio has refilled it, and the new verdict comes with the new
  numbers, at the next emit — about 0.45 to 0.75 s after the voice crosses. A
  held reading's `wetBypassed` describes the held window, not the moment
  ("over that window", above, now kept through pauses too). The cost: a chain
  of identity effects that still report latency — pitch or formant at exactly
  0, the noise remover at mix 0 — no longer reads as a passthrough, because
  the dry half is delayed by a latency their output doesn't have.
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
| `divora.hotkeyBindings` | `{ ptm, panic, monitor }` global hotkey accelerators, each `""` when unbound (the default for all three) — v1.51.4; plus `"rec:screen"` and `"rec:wav"` — v1.52.0; plus `"replay:save"` and `"replay:toggle"` — v1.54.0 (a save without them reads them as unbound); plus `"overlay:arrange"` — v1.55.0 (likewise); plus `"mute"` — v1.56.0 (likewise). Unknown keys are ignored on load; "Win" is read as "Super". `ptm`'s key also sets the in-app push-to-modulate key (`Space` while unbound). Hourglass's `tm:` keys (v1.59.0) aren't kept here: each timer's are in `divora.timers` |
| `divora.timers` | `{version: 1, timers: [{id, name, kind, durationMs, hotkeys?: {toggle?, reset?, finish?, …}, run?: TimerRun, onStart?, onFire?}], presetReturns?}` — v1.59.0 (`finish`, `onStart`, `onFire`, `presetReturns` v1.60.0), Hourglass's timers in list order. Read field by field, each falling back alone: missing, unparseable or not an object → no timers; `version` an integer ≥ 1 (else 1), a larger one read with these rules and never lowered on write; an entry is dropped when it isn't an object, its `id` fails the timer id grammar (§1 hotkey ids) or repeats an earlier one, or its `kind` isn't a string; a `name` that isn't 1-40 characters after trimming gets a default ("Countdown", "Count-up", "Timer", numbered " 2", " 3"… to stay unique); a `"countdown"` or `"countup"` `durationMs` that isn't a finite number is 300 000, any other is rounded and held to 1 000..86 400 000 (another kind's finite value is kept as it is); `hotkeys` keeps every string value, read as `divora.hotkeyBindings`' are ("Win" → "Super"), and anything else is `{}`; a `run` that isn't a `TimerRun` (§3) is dropped and the timer is idle. Unknown keys, kinds and hotkey verbs are kept and written back. The page adds no timer once 16 are stored (a kind it doesn't know counted); the first 16 of kind `"countdown"` or `"countup"` among the first 64 entries run, and the rest are kept, shown with Delete only, and their keys never registered. v1.59.0 registers `toggle` and `reset` only; since v1.60.0 `finish` (End now, a countdown's) is registered too. `run` is the last running, paused, done or ran-out state the page saw, written only when a snapshot changes it (a start, a pause, a finish — never a running timer's text alone); every `sync_timers` sends it as `resume`, and the backend uses it only for an id it hasn't accepted since it started. Absent: boot sends no timer command. Once written it stays, `timers: []` after the last delete. **Never `fires`**, and never the backend's clock. v1.60.0: `onStart` and `onFire` are a countdown's At start and At zero lists of `TimerAction` (§3), kept exactly as written whatever they hold. The page reads a list that isn't an array as empty, reads its first 8 entries as §3 says, and sends only those that run; an entry that doesn't run (one it can't read, a second line, one past the 8th) stays in place, is written back and is never sent. An edit changes one entry by its index, so nothing it can't read is dropped, and Add stops at 8 stored entries, readable or not. A count-up's lists are kept and never sent. `presetReturns` is `[{timer, from, to}]`: where a timer's start switched the preset from and to, at most one per timer, read leniently (an entry that can't be read is left out of the reading, and the key is kept as written until the page next writes it) and removed once empty. A return is settled when its timer's end is heard, or at the next page load when no page heard it (a reload, or a run-out while Divora was closed); one left for a timer no longer stored means "switch back" |
| `divora.ptm` | `{ on: bool, hold: "apply" \| "bypass" }` — v1.58.0: whether push-to-modulate is on, and which way the Mixer card's hold goes (kept while it is off, so turning it back on restores it). Written only from the engine's answer to `set_ptm_mode`, from the mode that answer carries, so a refused call changes nothing. Absent or malformed (not an object, `on` not a boolean, `hold` neither value) reads as `{ on: false, hold: "apply" }`: off. Sent back to the engine as `set_ptm_mode {mode, restore: true}` each time the main window loads (`mode` is `hold` while on, `"always"` while off). **Never a hold**: what holds it lives in the engine |
| `divora.replayBuffer` | `{ seconds: 15\|30\|60\|120, explained: bool, explainedDesktop: bool }` — v1.54.0; each falls back alone (30, false, false). **Never an armed flag**: Replay is off after every launch |
| `divora.screenRecording` | `{ monitorId (nullable: the main screen), fps: 30\|60, quality: "small"\|"balanced"\|"high", cursor, avOffsetMs }` — v1.52.0; `desktopAudio: "everything"\|"off"\|"device:<id>", desktopGainDb (−30…0), stems` — v1.53.0. Each field falls back to its default on its own when missing or malformed — except `desktopAudio`, which reads as `"off"` when missing (settings saved before v1.53.0) or malformed, and is `"everything"` only on a first run with nothing saved |
| `divora.tileGains` | per-tile soundboard gain |
| `divora.soundboardMasterGain` | master soundboard gain |
| `divora.midiInput` | selected MIDI input port name (or null) — v1.9.0 |
| `divora.midiMappings` | MIDI note/CC → action mappings (array) — v1.9.0. Each `action` is one of `preset`, `tile`, `ptm`, `monitor`, `param` (v1.9.0), `mute` (v1.56.0: a note-on toggles. A CC acts only when it changes side: it mutes on rising to 64 or more and unmutes on falling to 59 or less; 60-63 keeps the side it was on. The side is kept per mapping, in memory only, and forgotten at launch (or a reload), when a MIDI input is picked and when that mapping changes; until it is known again, 64 or more mutes and 59 or less only records it. So a control that stays on one side never changes the mute, except that a move while its side is unknown mutes if it lands at 64 or more). `ptm` keeps its name and, since v1.58.0, holds push-to-modulate for real: a note-on holds and its note-off lets go; a CC holds when it rises to 64 or more and lets go when it falls to 59 or less, 60-63 keeping the side it was on (the mute's rule, side unknown included: a first move to 64 or more holds, one to 59 or less only records it). Several `ptm` mappings combine: the MIDI source holds while any of them is down, so one pad's note-off can't let go of another. Which mappings are down, and each CC's side, live in memory only: forgotten at launch (or a reload), for every mapping when a MIDI input is picked or MIDI-learn starts on any mapping, and for one mapping when it is changed or removed; each lets go if nothing is left down. Turning push-to-modulate on or off on the Mixer, or picking its hold, forgets which are down but keeps the CC sides, so a CC left up holds again only after it comes down and back up. An older build keeps a mapping whose kind it doesn't know and ignores it |
| `divora.calibrated` | mic calibration completed flag (suppresses the wizard step) — v1.10.0 |
| `divora.updateCheckEnabled` | opt-out for the in-app update check (default true) — v1.12.0 |
| `divora.glyphBindings` | built-in glyph → `GlyphAction` map (any glyph → any action) — v1.15.0. The action's `kind` is one of `preset`, `tile`, `monitor`, `mute`, `panic` |
| `divora.customGlyphs` | user-recorded custom glyphs (template + action) — v1.15.0. The action's `kind` as in `divora.glyphBindings` |
| `divora.overlay` | stream-overlay background mode (`{ bg }`) — v1.16.0; plus the window's last place, all optional (`x`, `y`, `w`, `h` in physical pixels, `monitor` name, `scale` factor) — v1.51.3. A `{ bg }`-only blob still loads, and unknown keys are kept on save; plus `fit` and `hideFromRecordings` (booleans, off unless stored as `true`; `fit` counts only with the Transparent background, so a stored `fit` with Green or Magenta reads as off) — v1.55.0. While the window is fitted, the place keys keep its last unfitted place. `hideFromRecordings` is sent to the backend each time the main window loads; the backend doesn't save it |
| `divora.overlayScene` | `{version, designSize, frame, elements[]}` — v1.55.0, the first versioned key. Each field falls back on its own (see `OverlayScene` in §3). Absent or unreadable, the scene is made from `divora.overlay`'s last window size (the circle centred, at most 440 CSS px) and first written when the overlay is next seen open (opened, or adopted after a main-window reload) or a charm is first added or removed, whichever comes first. Every stored element is kept and written back, including ones this version can't show (an unknown `type`, an unrecognised `src`, a second circle, unusable geometry); only an entry without an id and a later duplicate id are dropped. v1.59.0: `type: "timer"` elements, kept and not shown since v1.55.0, are shown (an Hourglass timer's face; `OverlayScene` in §3); Settings adds one per timer and removes every element naming a timer when its Show on overlay goes off or the timer is deleted. Caps apply when adding, never when reading: 64 elements, 16 shown pictures, one circle, and a budget over the distinct pictures of 200 MB of files, 16,777,216 animated px and 67,108,864 px in all; a timer face counts toward the 64 only. A picture file is kept while any string anywhere in the scene names it. `src` holds an asset id, never a path or URL; `version` is never lowered on write. **Never the mode**: the overlay opens Locked at every launch |
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
Since v1.60.0 a countdown whose At start mutes, running or paused when
Divora closed, mutes you again once the page's first `sync_timers` picks its
stored run up, and the page says which timer did it.

**Push-to-modulate's on/off and direction are stored, in `divora.ptm`
(v1.58.0); its holds never are**, nor which MIDI mappings are down or the side
a `ptm` CC was last on. The page restores the setting at each load and lets go
of every hold the page had; a global key still held across a reload stays
held, because the backend tracks that one.

**Hourglass's timers are stored by the page, in `divora.timers` (v1.59.0);
the backend keeps no timer state on disk** (its text files, §6, are output
only). Since v1.60.0 so are each countdown's action lists and the preset
returns. Each timer's last run is kept with wall-clock anchors, so a restart
picks a running timer up with the time that passed while Divora was closed. A
page reload changes nothing in the backend, which keeps its own state and
ignores the runs the reload sends.

> **Ruling (v1.56.0): `{kind: "mute"}` keeps its name and now mutes.** Until
> v1.56.0 the `mute` glyph action toggled a status label only. It was
> documented as a mute from the start (CHANGELOG v1.15.0; the Mixer's
> "Muted · Nothing is being sent", its words until v1.56.0), so making it
> silence the voice fulfils that meaning rather than repurposing the value. A
> cast no longer toggles: it sends `set_input_muted` with the state its omen
> names (Mute or Unmute, the opposite of what Divora shows). Settings now
> calls it "Mute / unmute". No kind is renamed or removed.

> **Ruling (v1.58.0): `ptm` — the hotkey id and the MIDI kind — keeps its name
> and now switches the effects.** Until v1.58.0 its key, pad and the Mixer's
> button changed only the Clean / Modulated status (`ui.pressed`), and the
> call heard the preset whatever they did. It was documented as switching
> the effects from the start (CHANGELOG v0.1.0: holding the key flips the
> voice state, in apply or bypass mode; v1.9.0: **Hold push-to-modulate**,
> momentary; the README: hold the key for effects on, release for bypass), so
> making it real fulfils that meaning rather than repurposing the value. The
> hotkey is now acted on by the backend, press and release, and the `ptm:`
> prefix is reserved. No id or kind is renamed or removed.

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
timers/<id>.txt          an Hourglass timer's text — v1.59.0: exactly what the timer
                         shows ("05:00", "1:02:03"), UTF-8, no BOM, no newline, for an
                         OBS text source's Read from file. Written by the backend for
                         each timer it runs, from its first sync of each run on,
                         whenever the text changes. Replaced by writing
                         `.<id>.tmp` and renaming it over the file, never removed
                         first, so the file is never missing, empty or
                         part-written (an open that lands inside a rename can
                         fail with a sharing violation; OBS's Text (GDI+) then
                         shows nothing until the next change — from OBS's code;
                         OBS itself wasn't run). Removed once
                         nothing holds it when its timer
                         leaves the sync (deleted); written again if it goes missing.
                         Kept, with its last text, while Divora is closed. Named by
                         the timer's id, which a rename never changes; a deleted
                         timer's `timer-<n>` goes to the next timer added. A kept,
                         not-run timer's file is left as it is until that timer is
                         deleted
timers/.<id>.tmp         a write in flight — v1.59.0. A `.timer-<n>.tmp` left by a
                         crash is removed at the next run's first sync
                         The sweep: once per run, at the backend's first sync, every
                         `timer-<n>.txt` (n 1-99999, no leading zero) whose id isn't
                         in that sync, and every `.timer-<n>.tmp`, is removed.
                         Nothing else in the folder is ever touched, so a user's own
                         files there stay. The folder is created at the first write,
                         or by `timers_dir` (Settings' Open folder and Copy path),
                         never at startup.
                         `tauri dev` builds use `timers-dev/` (a `tauri build --debug`
                         build uses `timers/`, as the installed app does)
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
