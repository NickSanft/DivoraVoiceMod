//! `DivoraVoice` Tauri shell.
//!
//! Phase 0: blank window with a smoke-test IPC command. The design system
//! and app shell land in Phase 1; the audio engine lands in Phase 2.

// Tauri command signatures take `State<'_, T>` by value; clippy can't
// see through the attribute macro and flags every command.
#![allow(clippy::needless_pass_by_value)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use divora_core::audio::{
    detect_virtual_mic as detect_virtual_mic_core, list_input_devices as list_input_devices_core,
    list_output_devices as list_output_devices_core, AudioEngine, DeviceInfo, InputMute, Levels,
    PtmMode, PtmPageSource, PtmSource, PtmState, ReadingSnapshot, StreamInfo, VirtualMicStatus,
};
use divora_core::dsp::{
    onnx_runtime_available, DspCommand, EffectSpec, Metrics, ReactiveConfig, ReadingState,
    MAX_DESCRIPTORS, MODEL_RESOURCE_KEY,
};
use divora_core::presets::{bundled_presets, Preset, PresetStore, PresetTag};
use divora_core::soundboard::{
    decode_clip, scan_folder, DecodedClip, SoundboardCommand, SoundboardTile,
};
use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

mod midi;
pub mod overlay_assets;
pub mod overlay_machine;
mod overlay_windows;
pub mod replay;
mod screen;
use midi::{
    list_inputs as list_midi_inputs_core, open_input as open_midi_input_core, MidiInputInfo,
};
use midir::MidiInputConnection;
use overlay_assets::{ImportedAsset, ServeGate, CHARM_SCHEME, SERVE_SLOTS};
use overlay_machine::{
    HandshakeStage, ModeRequest, OverlayRect, OverlayStatus, BAD_MODE, BAD_STAGE,
};
use replay::{
    replay_memory as replay_memory_for, sanitize_seconds, ReplayMemory, ReplaySaved, ReplayStatus,
    TrayIconKind, TrayView, REPLAY_ICON, REPLAY_ICON_SIZE, TRAY_OFF_ID, TRAY_OFF_LABEL,
    TRAY_SAVE_ID, TRAY_SAVE_LABEL,
};
use screen::{
    desktop_source_dtos, monitor_dtos, parse_desktop_source, sanitize_desktop_gain_db,
    sanitize_fps, screen_recording_path, ArmRequest, CaptureStatusDto, DesktopSourceDto,
    MonitorDto, Quality, ScreenRecorder, ScreenRecordingUpdate, StartRequest,
};

/// Payload of the periodic `audio-levels` event. Frontend listens with
/// `tauri.event.listen("audio-levels", …)`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)] // flat status flags for one wire event
struct LevelUpdate {
    input: Levels,
    output: Levels,
    running: bool,
    monitoring: bool,
    /// Phase 14: latency added by the active DSP chain, in ms.
    dsp_latency_ms: f32,
    /// Phase 16: true while the modulated output is being recorded.
    recording: bool,
    /// v1.7.0: makeup gain the loudness normalizer is applying, in dB
    /// (0 while disabled). Drives the Mixer "it's working" readout.
    loudness_gain_db: f32,
    /// v1.39.0: true when device-loss auto-recovery gave up (engine stopped,
    /// no audio) — drives the "device lost — restart" banner.
    device_lost: bool,
    /// v1.46.0: current reactive-modulation depth, 0..=1 (0 when the feature
    /// is off). Drives the live envelope meter.
    mod_env: f32,
    /// v1.56.0: Divora's mute of the voice, engine running or not.
    input_muted: bool,
    /// v1.56.0: the mute's change count, read with `input_muted` in one load.
    /// Orders ticks against command answers: the larger is newer.
    input_mute_seq: u64,
    /// v1.58.0: push-to-modulate, engine running or not, from one load: its
    /// mode, whether anything holds it, what the callback switches to, and
    /// its change count (the larger `seq` is newer).
    ptm: PtmState,
}

/// Level shown when a window measured nothing.
///
/// `Metrics::silent()` uses −∞ dBFS, which `serde_json` writes as `null`. The
/// panel would then have to special-case a null it never shows anyway (it
/// only renders numbers when there is a reading), so the bridge floors it to
/// a number instead of shipping a hole in the payload.
const READING_FLOOR_DBFS: f32 = -120.0;

/// One acoustic measurement of one signal, camelCase for the bridge.
///
/// Mirrors `divora_core::dsp::Metrics` rather than re-exporting it: the core
/// type serializes `snake_case`, and every key on this surface is camelCase.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadingMetrics {
    energy_dbfs: f32,
    energy_range_db: f32,
    f0_hz: f32,
    f0_range_st: f32,
    voiced_ratio: f32,
    pace_ops: f32,
    brightness_hz: f32,
}

impl ReadingMetrics {
    /// Non-finite values are floored rather than shipped: JSON has no
    /// infinity, so they would otherwise arrive as `null`.
    fn from_core(m: Metrics) -> Self {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        Self {
            energy_dbfs: finite(m.energy_dbfs, READING_FLOOR_DBFS),
            energy_range_db: finite(m.energy_range_db, 0.0),
            f0_hz: finite(m.f0_hz, 0.0),
            f0_range_st: finite(m.f0_range_st, 0.0),
            voiced_ratio: finite(m.voiced_ratio, 0.0),
            pace_ops: finite(m.pace_ops, 0.0),
            brightness_hz: finite(m.brightness_hz, 0.0),
        }
    }
}

/// Payload of the `voice-reading` event and of the `voice_reading` command.
///
/// `words` carries the already-resolved vocabulary rather than the descriptor
/// names, deliberately: the closed vocabulary in `divora_core::dsp::reading`
/// is the only place a shown word may come from, and a second word table on
/// the frontend would be a second place it could drift to. The frontend
/// renders what it is handed and owns no list of its own.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)] // flat facts about one reading on one wire event
struct VoiceReadingUpdate {
    /// "stopped" | "muted" | "quiet" | "speaking".
    state: ReadingState,
    /// The mic, before any effect.
    dry: ReadingMetrics,
    /// The same voice after the chain — what the call hears. Not the
    /// soundboard: Speak and clips are mixed in after this tap.
    wet: ReadingMetrics,
    words: Vec<&'static str>,
    calibrated: bool,
    /// The chain passed the signal through unchanged over this window, so the
    /// two halves *should* read alike.
    ///
    /// Measured in the engine by correlating the taps, which is scale
    /// invariant — the wet tap sits after the loudness stage, so a bypassed
    /// chain still arrives at a different level. The frontend cannot work
    /// this out for itself: it only sees summary numbers, and its own
    /// attempt required the levels to match, which they do not when loudness
    /// normalization is on.
    wet_bypassed: bool,
    /// The numbers come from an earlier window, held rather than decayed.
    stale: bool,
    age_ms: u32,
    /// v1.56.0: `state` is "muted" because of Divora's own mute (the
    /// microphone may be live). Set in the same snapshot as `state`.
    muted_in_divora: bool,
}

impl VoiceReadingUpdate {
    fn from_snapshot(snap: &ReadingSnapshot) -> Self {
        Self {
            state: snap.reading.state,
            dry: ReadingMetrics::from_core(snap.reading.dry),
            wet: ReadingMetrics::from_core(snap.reading.wet),
            wet_bypassed: snap.reading.wet_bypassed,
            // Capped here as well as at the source. More than three stops
            // reading as a description and starts reading as a verdict, and
            // that limit should not depend on who filled the vector.
            words: snap
                .reading
                .descriptors
                .iter()
                .take(MAX_DESCRIPTORS)
                .map(|d| d.word())
                .collect(),
            calibrated: snap.reading.calibrated,
            stale: snap.stale,
            age_ms: snap.age_ms,
            muted_in_divora: snap.muted_in_divora,
        }
    }
}

/// One-shot status snapshot used by the frontend at startup and after
/// engine state changes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EngineStatus {
    running: bool,
    monitoring: bool,
    input: Levels,
    output: Levels,
    dsp_latency_ms: f32,
    /// Phase 16: true while the modulated output is being recorded.
    recording: bool,
    /// v1.7.0: makeup gain the loudness normalizer is applying, in dB.
    loudness_gain_db: f32,
}

/// Tauri-managed shared state. Holds the live audio engine, the user
/// preset store, the decoded-clip cache (keyed by `tile id`), and the
/// active global-shortcut bindings.
struct AppState {
    engine: Arc<AudioEngine>,
    /// v1.47.0: monotonic id for the current audition.
    ///
    /// Preview synthesis runs on a blocking thread with no way to abort
    /// it, so cancellation is expressed as invalidation instead: anything
    /// that supersedes an audition bumps this, and a render whose captured
    /// value no longer matches discards its result. Without it a slow
    /// render lands late, stops whatever is playing, and plays itself.
    preview_epoch: AtomicU64,
    preset_store: Arc<PresetStore>,
    clip_cache: Mutex<HashMap<String, DecodedClip>>,
    /// Maps a stable id (chosen by the frontend, e.g. "ptm") to the
    /// `Shortcut` we registered for it. Used to unregister cleanly.
    shortcuts: Mutex<HashMap<String, Shortcut>>,
    /// Phase 12: directory the user drops voice-conversion `.onnx`
    /// models into (`%APPDATA%/com.divora.voicemod/voices/`). Created at
    /// startup; surfaced to the UI so it can list + open it.
    voices_dir: PathBuf,
    /// Phase 12.4: read-only directory of voices shipped in the
    /// installer's resource bundle. `None` in dev builds / when no
    /// resources were bundled. Listed alongside `voices_dir`.
    bundled_voices_dir: Option<PathBuf>,
    /// v1.17.0: directory holding the bundled TTS ("Speak") assets —
    /// `kokoro-v1.0.int8.onnx`, `voices-v1.0.bin`, `kokoro-config.json`,
    /// and the espeak-ng binary + `espeak-ng-data` (resource dir's `tts/`
    /// subfolder). The expected path is always set; the files only exist
    /// once `voice-assets-v2` is fetched into the bundle, so until then
    /// `list_tts_voices` reports every voice as not-installed and `speak`
    /// degrades to a clear error rather than synthesizing.
    tts_assets_dir: PathBuf,
    /// v1.21.0: user-writable dir the voice-cloning models are
    /// **downloaded** into on demand (`%APPDATA%/com.divora.voicemod/tts/`). They're
    /// no longer bundled (kept the installer small); cloning prompts a
    /// one-time download into here.
    clone_models_dir: PathBuf,
    /// Phase 16: directory recordings are written to
    /// (`%APPDATA%/com.divora.voicemod/recordings/`). Created at startup;
    /// surfaced to the UI so it can open it + show where files land.
    recordings_dir: PathBuf,
    /// v1.14.0: directory the rolling log file is written to
    /// (`%APPDATA%/com.divora.voicemod/logs/`). Created at startup; surfaced to
    /// the UI's "Open logs folder" support affordance.
    logs_dir: PathBuf,
    /// v1.9.0: the live MIDI input connection, if a control surface is
    /// connected. Held here so it stays alive — dropping it closes the
    /// port. Replaced on `open_midi_input`, cleared on `close_midi_input`.
    midi_connection: Mutex<Option<MidiInputConnection<()>>>,
    /// v1.23.0: temp WAV path of an in-progress in-app voice recording (dry
    /// mic), set by `start_voice_recording` and consumed by
    /// `stop_voice_recording`. `None` when no capture is active.
    ///
    /// v1.57.0: the `bool` is whether the recording becomes a `VoxCPM` voice,
    /// decided when it starts (see [`start_voice_recording`]).
    voice_recording_path: Mutex<Option<(PathBuf, bool)>>,
    /// v1.57.0: set while `clone_voice` or `stop_voice_recording` makes a
    /// voice; taken through [`CloneClaim`], so one runs at a time.
    voice_clone_busy: Arc<AtomicBool>,
    /// v1.57.0: the running model download's latest progress, `None` when
    /// none runs; taken through [`DownloadSlot`], so one runs at a time.
    model_download: Arc<Mutex<Option<CloneDownloadProgress>>>,
    /// `VoxCPM` accent-preserving cloning engine (4 ONNX sessions + tokenizer),
    /// lazy-loaded on first `VoxCPM` cloned-voice speak and held (each is hundreds
    /// of MB). `None` until first use / when the models aren't installed. The
    /// `bool` is the GPU mode the cached engine was built with — when a speak
    /// requests a different mode, the engine is rebuilt (the `decode_step` swaps
    /// between the CPU and the `DirectML` fp16 build).
    voxcpm_engine: Mutex<Option<(bool, divora_core::tts::voxcpm::VoxCpmEngine)>>,
    /// v1.52.0: the screen recorder. Outlives engine sessions — a take
    /// carries on through a device-loss rebuild and a Stop/Start.
    screen: Arc<ScreenRecorder>,
    /// v1.55.0: where the stream overlay's charm pictures are kept
    /// (`%APPDATA%/com.divora.voicemod/overlay/assets/`; `assets-dev/` for
    /// development builds). Created at startup.
    overlay_assets_dir: PathBuf,
}

/// One installed voice model. `id` (and `name`) are the file stem; the
/// `VoiceConverter` derives the same id from the path it loads.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceInfo {
    id: String,
    name: String,
    path: String,
    size_bytes: u64,
}

/// Whether voice conversion can actually run on this machine, plus the
/// directory where models live (for the Settings panel's guidance).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OnnxRuntimeStatus {
    /// True when an `onnxruntime` shared library is locatable.
    runtime_available: bool,
    voices_dir: String,
}

/// Wire payload emitted by `global-shortcut` events.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GlobalShortcutEvent {
    id: String,
    accelerator: String,
    state: &'static str, // "pressed" | "released"
}

#[tauri::command]
fn list_audio_input_devices() -> Vec<DeviceInfo> {
    list_input_devices_core()
}

#[tauri::command]
fn list_audio_output_devices() -> Vec<DeviceInfo> {
    list_output_devices_core()
}

#[tauri::command]
fn start_audio_engine(
    state: State<'_, AppState>,
    input_name: Option<String>,
    output_name: Option<String>,
    monitor_name: Option<String>,
) -> Result<StreamInfo, String> {
    state
        .engine
        .start(
            input_name.as_deref(),
            output_name.as_deref(),
            monitor_name.as_deref(),
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn stop_audio_engine(state: State<'_, AppState>) {
    state.engine.stop();
}

#[tauri::command]
fn set_audio_monitor(state: State<'_, AppState>, enabled: bool) {
    state.engine.set_monitor(enabled);
}

/// v1.29.0: toggle hearing Speak/soundboard previews in the monitor —
/// independent of the Mixer's mic monitor ([`set_audio_monitor`]). Takes effect
/// next buffer; no restart needed.
#[tauri::command]
fn set_speak_monitor(state: State<'_, AppState>, enabled: bool) {
    state.engine.set_speak_monitor(enabled);
}

/// v1.56.0: mute or unmute the voice; answers the state after. Takes effect
/// on the next buffer once the command runs. Sync commands share the main
/// thread with each other and with `WM_HOTKEY`, so a mute waits for a sync
/// command already running; since v1.57.0 the model downloads and voice
/// cloning are not sync commands, so it no longer waits minutes behind
/// them. Idempotent.
#[tauri::command]
fn set_input_muted(state: State<'_, AppState>, muted: bool) -> InputMute {
    state.engine.set_input_muted(muted).after
}

/// v1.56.0: flip the mute; answers the state after. The `mute` hotkey does
/// this in Rust (see `global_shortcut_handler`); the frontend calls it for
/// MIDI pads.
#[tauri::command]
fn toggle_input_muted(state: State<'_, AppState>) -> InputMute {
    state.engine.toggle_input_muted().after
}

/// v1.56.0: the current mute, for the store's first read after a (re)load.
#[tauri::command]
fn input_mute(state: State<'_, AppState>) -> InputMute {
    state.engine.input_mute()
}

/// v1.58.0: push-to-modulate's mode; answers the state after. Without
/// `restore` (the Mixer card) it lets go of every hold, a held global key
/// included, which must then be pressed again. With `restore: true` (the main
/// window, once at each page load, with the saved mode) it lets go of the
/// page's holds and keeps a held global key, so a reload doesn't drop it.
/// Sync on purpose (D23): sync commands run in order on the main thread, so
/// a press can't overtake its release. Idempotent.
#[tauri::command]
fn set_ptm_mode(state: State<'_, AppState>, mode: PtmMode, restore: Option<bool>) -> PtmState {
    ptm_mode_command(&state.engine, mode, restore)
}

/// v1.58.0: [`set_ptm_mode`]'s body, on the engine, so a test runs the real
/// choice between the card's set and the boot's restore (the command itself
/// needs a `State`). Only `restore: true` restores; absent or `false` sets.
fn ptm_mode_command(engine: &AudioEngine, mode: PtmMode, restore: Option<bool>) -> PtmState {
    if restore == Some(true) {
        engine.restore_ptm_mode(mode).after
    } else {
        engine.set_ptm_mode(mode).after
    }
}

/// v1.58.0: one page source's hold on push-to-modulate (the in-app key, the
/// Mixer's button, MIDI), by level; answers the state after. `"hotkey"` is
/// refused (only the backend holds that one). Sync for the same reason as
/// [`set_ptm_mode`]. Idempotent.
#[tauri::command]
fn set_ptm_held(state: State<'_, AppState>, source: PtmPageSource, held: bool) -> PtmState {
    state.engine.set_ptm_held(source.into(), held).after
}

/// v1.6.0: set the monitor ("hear yourself") stream gain (linear, 1.0 =
/// unity). Takes effect next buffer; no restart needed.
#[tauri::command]
fn set_monitor_gain(state: State<'_, AppState>, gain: f32) {
    state.engine.set_monitor_gain(gain);
}

/// v1.7.0: enable/disable output loudness normalization (auto-gain +
/// limiter). Takes effect next buffer; no restart needed.
#[tauri::command]
fn set_loudness_enabled(state: State<'_, AppState>, enabled: bool) {
    state.engine.set_loudness_enabled(enabled);
}

/// v1.7.0: set the loudness target level in dBFS. The engine clamps to
/// its supported window.
#[tauri::command]
fn set_loudness_target(state: State<'_, AppState>, dbfs: f32) {
    state.engine.set_loudness_target(dbfs);
}

#[tauri::command]
fn audio_engine_status(state: State<'_, AppState>) -> EngineStatus {
    EngineStatus {
        running: state.engine.is_running(),
        monitoring: state.engine.is_monitoring(),
        input: state.engine.input_levels(),
        output: state.engine.output_levels(),
        dsp_latency_ms: state.engine.dsp_latency_ms(),
        recording: state.engine.is_recording(),
        loudness_gain_db: state.engine.loudness_gain_db(),
    }
}

#[tauri::command]
fn set_effect_chain(state: State<'_, AppState>, specs: Vec<EffectSpec>) {
    state.engine.send_dsp(DspCommand::SetChain { specs });
}

/// v1.46.0: replace the whole reactive-modulation configuration.
///
/// Sent as one message rather than per-field setters so the audio thread can
/// never observe a half-applied state. Routes naming a parameter outside the
/// modulation whitelist are dropped in `divora-core` rather than rejected
/// here, so a newer or older frontend cannot wedge the engine.
#[tauri::command]
fn set_reactive_config(state: State<'_, AppState>, config: ReactiveConfig) {
    state.engine.send_reactive(config);
}

/// Turn the voice-reading panel's analysis on or off.
///
/// Off is the default and nothing runs until this says otherwise: with it off
/// the audio callback copies nothing, so no audio of the user's voice reaches
/// the analysis at all.
#[tauri::command]
fn set_voice_reading_enabled(state: State<'_, AppState>, enabled: bool) {
    state.engine.set_reading_enabled(enabled);
}

/// One-shot reading, for the panel's first paint before the first event.
#[tauri::command]
fn voice_reading(state: State<'_, AppState>) -> VoiceReadingUpdate {
    VoiceReadingUpdate::from_snapshot(&state.engine.voice_reading())
}

#[tauri::command]
fn set_effect_param(state: State<'_, AppState>, index: usize, key: String, value: f32) {
    state
        .engine
        .send_dsp(DspCommand::SetParam { index, key, value });
}

#[tauri::command]
fn set_effect_enabled(state: State<'_, AppState>, index: usize, enabled: bool) {
    state
        .engine
        .send_dsp(DspCommand::SetEnabled { index, enabled });
}

#[tauri::command]
fn clear_effect_chain(state: State<'_, AppState>) {
    state.engine.send_dsp(DspCommand::Clear);
}

#[tauri::command]
fn list_presets(state: State<'_, AppState>) -> Vec<Preset> {
    let mut all = bundled_presets();
    match state.preset_store.list_user() {
        Ok(user) => all.extend(user),
        Err(e) => tracing::warn!(?e, "failed to enumerate user presets"),
    }
    all
}

#[tauri::command]
fn save_user_preset(state: State<'_, AppState>, preset: Preset) -> Result<(), String> {
    state.preset_store.save(&preset).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_user_preset(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.preset_store.delete(&id).map_err(|e| e.to_string())
}

#[tauri::command]
fn export_preset_json(preset: Preset) -> Result<String, String> {
    serde_json::to_string_pretty(&preset).map_err(|e| e.to_string())
}

/// v1.14.0: import a preset from an arbitrary `.json` file (the counterpart
/// to Export). Read + parse it, force it to a **User** preset, give it a
/// unique filesystem-safe id (so it never clobbers a bundled or existing
/// user preset), persist it, and return it so the UI can select it.
#[tauri::command]
fn import_preset(state: State<'_, AppState>, path: String) -> Result<Preset, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("couldn't read the file: {e}"))?;
    let mut preset: Preset = serde_json::from_slice(&bytes)
        .map_err(|e| format!("that doesn't look like a preset file: {e}"))?;
    // An imported preset is always a user preset (a bundled-tagged file
    // becomes a user copy), and gets a fresh id unique across the full set.
    preset.tag = PresetTag::User;
    let taken: std::collections::HashSet<String> = {
        let mut ids: std::collections::HashSet<String> =
            bundled_presets().into_iter().map(|p| p.id).collect();
        if let Ok(user) = state.preset_store.list_user() {
            ids.extend(user.into_iter().map(|p| p.id));
        }
        ids
    };
    preset.id = unique_preset_id(&preset.id, &preset.name, &taken);
    state
        .preset_store
        .save(&preset)
        .map_err(|e| e.to_string())?;
    Ok(preset)
}

/// Slugify into a filesystem-safe preset id: lowercase ASCII alphanumerics,
/// runs of other characters collapse to a single `-`, keeping existing
/// `-`/`_`. Matches the `is_safe_id` rule in divora-core's preset store.
fn slugify_preset_id(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if (ch == '-' || ch == '_') && !out.is_empty() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

/// Pick a unique id from the imported preset's id (or its name as a
/// fallback), appending `-2`, `-3`, … until it's free in `taken`.
fn unique_preset_id(raw_id: &str, name: &str, taken: &std::collections::HashSet<String>) -> String {
    let mut base = slugify_preset_id(raw_id);
    if base.is_empty() {
        base = slugify_preset_id(name);
    }
    if base.is_empty() {
        base = "imported-preset".to_string();
    }
    if !taken.contains(&base) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !taken.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[tauri::command]
fn preset_store_path(state: State<'_, AppState>) -> String {
    state.preset_store.base_dir().to_string_lossy().into_owned()
}

// ---- Phase 12: voice library ----

/// Absolute path of the voices directory (for "open folder" + guidance).
#[tauri::command]
fn voices_dir(state: State<'_, AppState>) -> String {
    state.voices_dir.to_string_lossy().into_owned()
}

/// Enumerate `*.onnx` models in the voices directory. Missing dir →
/// empty list (not an error — the user just hasn't installed any).
///
/// Scans the user voices dir first, then the bundled resource voices
/// dir (installer-shipped models). A user-installed voice with the same
/// id shadows a bundled one, so users can override a shipped voice by
/// dropping their own `<id>.onnx` into the user dir.
#[tauri::command]
fn list_voices(state: State<'_, AppState>) -> Vec<VoiceInfo> {
    let mut out: Vec<VoiceInfo> = Vec::new();
    scan_voice_dir(&state.voices_dir, &mut out);
    if let Some(bundled) = state.bundled_voices_dir.as_ref() {
        scan_voice_dir(bundled, &mut out);
    }
    out.sort_by_key(|v| v.name.to_lowercase());
    out
}

/// Append `*.onnx` voices from `dir` into `out`, skipping ids already
/// present (first dir wins → user voices shadow bundled ones).
fn scan_voice_dir(dir: &Path, out: &mut Vec<VoiceInfo>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("onnx") {
            continue;
        }
        let id = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if id.is_empty() || out.iter().any(|v| v.id == id) {
            continue;
        }
        let size_bytes = entry.metadata().map_or(0, |m| m.len());
        out.push(VoiceInfo {
            name: id.clone(),
            id,
            path: path.to_string_lossy().into_owned(),
            size_bytes,
        });
    }
}

/// Runtime + voices-dir status for the Settings panel.
#[tauri::command]
fn onnx_runtime_status(state: State<'_, AppState>) -> OnnxRuntimeStatus {
    OnnxRuntimeStatus {
        runtime_available: onnx_runtime_available(),
        voices_dir: state.voices_dir.to_string_lossy().into_owned(),
    }
}

/// Point the `VoiceConvert` effect at `index` in the chain at a model
/// file (or `None` to clear → passthrough). The actual load happens on
/// a background thread inside the effect, so this returns immediately.
#[tauri::command]
fn set_voice_model(state: State<'_, AppState>, index: usize, path: Option<String>) {
    state.engine.send_dsp(DspCommand::SetResource {
        index,
        // The shared constant, not a literal: the engine matches on this key
        // to know it should prepare a voice model, and a silent mismatch
        // would leave the effect in passthrough with nothing to show why.
        key: MODEL_RESOURCE_KEY.to_string(),
        value: path,
    });
}

// ---- Phase 16: recording the modulated output ----

/// Absolute path of the recordings directory (for "open folder" + the
/// "saved to …" hint). Created at startup.
#[tauri::command]
fn recordings_dir(state: State<'_, AppState>) -> String {
    state.recordings_dir.to_string_lossy().into_owned()
}

/// v1.14.0: absolute path of the logs directory (for the "Open logs
/// folder" support affordance). Created at startup.
#[tauri::command]
fn logs_dir(state: State<'_, AppState>) -> String {
    state.logs_dir.to_string_lossy().into_owned()
}

/// Begin recording the modulated output to a WAV file. `filename` is the
/// desired name (the frontend builds it from the local time, e.g.
/// `divora-2026-05-31_14-30-00.wav`); only its final path component is
/// honored and a `.wav` extension is forced, so it can never escape the
/// recordings dir. Returns the full destination path so the UI can show
/// where the file lands. No-op until the next start if the engine isn't
/// running (the writer thread only exists while streams are live).
#[tauri::command]
fn start_recording(state: State<'_, AppState>, filename: String) -> Result<String, String> {
    let stem = Path::new(&filename)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "divora-recording.wav".to_string());
    let mut dest = state.recordings_dir.join(stem);
    dest.set_extension("wav");

    std::fs::create_dir_all(&state.recordings_dir).map_err(|e| e.to_string())?;
    state.engine.start_recording(dest.clone());
    Ok(dest.to_string_lossy().into_owned())
}

/// Stop the current recording and finalize the WAV file.
#[tauri::command]
fn stop_recording(state: State<'_, AppState>) {
    state.engine.stop_recording();
}

// ---- v1.52.0: recording the screen ----
//
// All `async`: Tauri 2 runs a sync command on the main thread, which every
// window shares, and these wait on Windows (Media Foundation's first start,
// a take's first frame, the file being finished).

/// Whether this PC can record the screen, and if not, what to tell the user.
/// Checked once per run; the first call starts Media Foundation to find out.
#[tauri::command]
async fn screen_capture_status() -> Result<CaptureStatusDto, String> {
    tauri::async_runtime::spawn_blocking(|| divora_capture::capture_status().into())
        .await
        .map_err(|e| e.to_string())
}

/// The screens that can be recorded, the main one first.
#[tauri::command]
async fn list_capture_monitors() -> Result<Vec<MonitorDto>, String> {
    tauri::async_runtime::spawn_blocking(|| monitor_dtos(divora_capture::list_monitors()))
        .await
        .map_err(|e| e.to_string())
}

/// v1.53.0: where a screen recording's desktop audio can come from —
/// "everything except Divora" where Windows can do that, otherwise each
/// playback device, with what Divora itself plays there.
#[tauri::command]
async fn list_desktop_audio_sources(
    state: State<'_, AppState>,
) -> Result<Vec<DesktopSourceDto>, String> {
    let engine = Arc::clone(&state.engine);
    tauri::async_runtime::spawn_blocking(move || {
        let everything = divora_capture::process_loopback_available();
        let devices = if everything {
            Vec::new()
        } else {
            divora_capture::list_playback_devices().unwrap_or_default()
        };
        desktop_source_dtos(everything, &devices, engine.stream_info().as_ref())
    })
    .await
    .map_err(|e| e.to_string())
}

/// Record `monitor_id` to `recordings/<filename>` (an `.mp4`, whatever the
/// name says) with the modulated voice and the soundboard — and, given
/// `desktop_audio`, what the PC plays (v1.53.0). Resolves once the first
/// frame is in, with the file's full path; refuses while the engine is
/// stopped, as the WAV recorder does.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // one per field of the invoke payload
async fn start_screen_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    monitor_id: String,
    filename: String,
    fps: Option<u32>,
    quality: Option<String>,
    cursor: Option<bool>,
    av_offset_ms: Option<i32>,
    desktop_audio: Option<String>,
    desktop_gain_db: Option<f64>,
    stems: Option<bool>,
) -> Result<String, String> {
    std::fs::create_dir_all(&state.recordings_dir).map_err(|e| e.to_string())?;
    let req = StartRequest {
        monitor_id,
        path: screen_recording_path(&state.recordings_dir, &filename),
        fps: sanitize_fps(fps),
        quality: Quality::parse(quality.as_deref()),
        cursor: cursor.unwrap_or(true),
        av_offset_ms: av_offset_ms.unwrap_or(0),
        desktop: parse_desktop_source(desktop_audio.as_deref()),
        desktop_gain_db: sanitize_desktop_gain_db(desktop_gain_db),
        stems: stems.unwrap_or(false),
    };
    let recorder = Arc::clone(&state.screen);
    let engine = Arc::clone(&state.engine);
    tauri::async_runtime::spawn_blocking(move || recorder.start(&app, &engine, req))
        .await
        .map_err(|e| e.to_string())?
        .map(|p| p.to_string_lossy().into_owned())
}

/// Stop the take and wait for the MP4 to be finished; resolves with its path.
#[tauri::command]
async fn stop_screen_recording(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let recorder = Arc::clone(&state.screen);
    tauri::async_runtime::spawn_blocking(move || recorder.stop(&app))
        .await
        .map_err(|e| e.to_string())?
        .map(|p| p.to_string_lossy().into_owned())
}

/// The latest `screen-recording` report, for a window that opens mid-take.
#[tauri::command]
fn screen_recording_status(state: State<'_, AppState>) -> ScreenRecordingUpdate {
    state.screen.status()
}

// ---- v1.54.0: Replay ----
//
// The last 15, 30, 60 or 120 seconds of one screen kept in memory, saved on
// request. All `async` for the same reason as the screen recorder's: they
// wait on Windows (an arm waits for the pipeline's first frame, a save for
// its file).

/// Replay now.
#[tauri::command]
async fn replay_buffer_status(state: State<'_, AppState>) -> Result<ReplayStatus, String> {
    let recorder = Arc::clone(&state.screen);
    tauri::async_runtime::spawn_blocking(move || recorder.replay_status())
        .await
        .map_err(|e| e.to_string())
}

/// What each length of Replay needs on `monitor_id` at these settings, and
/// what this PC allows.
#[tauri::command]
async fn replay_memory(
    monitor_id: String,
    fps: Option<u32>,
    quality: Option<String>,
) -> Result<ReplayMemory, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let monitor = divora_capture::list_monitors()
            .into_iter()
            .find(|m| m.id == monitor_id)
            .ok_or_else(|| "That screen isn't connected any more — pick another.".to_string())?;
        Ok(replay_memory_for(
            (monitor.width, monitor.height),
            sanitize_fps(fps),
            Quality::parse(quality.as_deref()),
            divora_capture::total_physical_memory(),
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Turn Replay on for `monitor_id`, keeping `seconds` (15, 30, 60 or 120),
/// with the same settings a screen recording there would use — or change
/// its length while it is on. Resolves once the pipeline's first frame is
/// in; refuses in the user's words (the engine stopped, too little memory,
/// another screen already kept, …).
#[tauri::command]
#[allow(clippy::too_many_arguments)] // one per field of the invoke payload
async fn arm_replay_buffer(
    app: AppHandle,
    state: State<'_, AppState>,
    monitor_id: String,
    seconds: Option<u32>,
    fps: Option<u32>,
    quality: Option<String>,
    cursor: Option<bool>,
    av_offset_ms: Option<i32>,
    desktop_audio: Option<String>,
    desktop_gain_db: Option<f64>,
) -> Result<ReplayStatus, String> {
    let req = ArmRequest {
        screen: StartRequest {
            monitor_id,
            path: PathBuf::new(),
            fps: sanitize_fps(fps),
            quality: Quality::parse(quality.as_deref()),
            cursor: cursor.unwrap_or(true),
            av_offset_ms: av_offset_ms.unwrap_or(0),
            desktop: parse_desktop_source(desktop_audio.as_deref()),
            desktop_gain_db: sanitize_desktop_gain_db(desktop_gain_db),
            stems: false,
        },
        seconds: sanitize_seconds(seconds),
    };
    let recorder = Arc::clone(&state.screen);
    let engine = Arc::clone(&state.engine);
    tauri::async_runtime::spawn_blocking(move || recorder.arm_replay(&app, &engine, req))
        .await
        .map_err(|e| e.to_string())?
}

/// Turn Replay off. A Save already pressed still finishes.
#[tauri::command]
async fn disarm_replay_buffer(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ReplayStatus, String> {
    let recorder = Arc::clone(&state.screen);
    tauri::async_runtime::spawn_blocking(move || recorder.disarm_replay(&app))
        .await
        .map_err(|e| e.to_string())
}

/// Save what Replay holds, ending on the frame showing now. Resolves with
/// the clip; every outcome is also a `replay-saved` event.
#[tauri::command]
async fn save_replay(app: AppHandle, state: State<'_, AppState>) -> Result<ReplaySaved, String> {
    // The press, not when the blocking pool gets to it.
    let at = Instant::now();
    let recorder = Arc::clone(&state.screen);
    let dir = state.recordings_dir.clone();
    tauri::async_runtime::spawn_blocking(move || recorder.save_replay(&app, dir, at))
        .await
        .map_err(|e| e.to_string())?
        .map(|clip| ReplaySaved::saved(&clip))
}

// ---- v1.55.0: the stream overlay ----
//
// All `async`: Tauri 2 runs a sync command on the main thread, which both
// windows share, and building a window from a sync command deadlocks on
// Windows. The rules are in `overlay_machine`, the window work in
// `overlay_windows`; these only read their arguments.

/// Open the stream overlay at `rect` (physical px; Windows' default spot
/// when absent), fitted to its monitor when `fit`. Already open: its status.
#[tauri::command]
async fn open_overlay(
    app: AppHandle,
    rect: Option<OverlayRect>,
    fit: Option<bool>,
) -> Result<OverlayStatus, String> {
    overlay_windows::open(app, rect, fit.unwrap_or(false)).await
}

/// Close the stream overlay. Closed already: nothing happens.
#[tauri::command]
async fn close_overlay(app: AppHandle) -> OverlayStatus {
    overlay_windows::close(&app)
}

/// Move and size the open overlay to `rect` (physical px).
#[tauri::command]
async fn place_overlay(
    app: AppHandle,
    rect: OverlayRect,
    fit: Option<bool>,
) -> Result<OverlayStatus, String> {
    overlay_windows::place(app, rect, fit.unwrap_or(false)).await
}

/// `"locked"`, `"arrange"` or `"toggle"`. Locking never fails.
#[tauri::command]
async fn set_overlay_mode(app: AppHandle, mode: String) -> Result<OverlayStatus, String> {
    let req = ModeRequest::parse(&mode).ok_or_else(|| BAD_MODE.to_owned())?;
    overlay_windows::set_mode(&app, req)
}

/// The Settings toggle: ask Windows to leave the locked overlay out of
/// Divora's captures.
#[tauri::command]
async fn set_overlay_hide_from_recordings(app: AppHandle, enabled: bool) -> OverlayStatus {
    overlay_windows::set_hide(&app, enabled)
}

/// The overlay now, for a main window that reloaded.
#[tauri::command]
async fn overlay_status(app: AppHandle) -> OverlayStatus {
    overlay_windows::status(&app)
}

/// The overlay page is listening (`"ready"`) or has drawn since it was shown
/// (`"painted"`). Only the overlay page calls it.
#[tauri::command]
async fn overlay_handshake(
    app: AppHandle,
    instance: u32,
    stage: String,
) -> Result<OverlayStatus, String> {
    let stage = HandshakeStage::parse(&stage).ok_or_else(|| BAD_STAGE.to_owned())?;
    Ok(overlay_windows::handshake(&app, instance, stage))
}

/// The overlay page's beat while arranging. Only the overlay page calls it.
#[tauri::command]
async fn overlay_alive(app: AppHandle, instance: u32, gen: u32) {
    overlay_windows::alive(&app, instance, gen);
}

/// Copy the picture at `path` into the overlay's pictures.
#[tauri::command]
async fn import_overlay_asset(
    state: State<'_, AppState>,
    path: String,
) -> Result<ImportedAsset, String> {
    let dir = state.overlay_assets_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        overlay_assets::import(&dir, Path::new(&path)).map_err(|e| e.message())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Remove the overlay picture `id`. Already gone: fine.
#[tauri::command]
async fn delete_overlay_asset(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let dir = state.overlay_assets_dir.clone();
    tauri::async_runtime::spawn_blocking(move || overlay_assets::delete(&dir, &id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn scan_soundboard_folder(folder: String) -> Result<Vec<SoundboardTile>, String> {
    let path = PathBuf::from(&folder);
    scan_folder(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn play_soundboard_clip(
    state: State<'_, AppState>,
    clip_id: String,
    path: String,
    gain: Option<f32>,
) -> Result<f32, String> {
    let decoded = decode_or_cache(&state, &clip_id, Path::new(&path))?;
    state.engine.send_soundboard(SoundboardCommand::Play {
        clip_id: clip_id.clone(),
        samples: decoded.samples.clone(),
        sample_rate: decoded.sample_rate,
        gain: gain.unwrap_or(1.0),
    });
    Ok(decoded.duration_secs)
}

#[tauri::command]
fn stop_soundboard_clip(state: State<'_, AppState>, clip_id: String) {
    state
        .engine
        .send_soundboard(SoundboardCommand::Stop { clip_id });
}

#[tauri::command]
fn stop_all_soundboard_clips(state: State<'_, AppState>) {
    state.engine.send_soundboard(SoundboardCommand::StopAll);
}

/// Phase 15: set the master soundboard gain (linear, 1.0 = unity).
#[tauri::command]
fn set_soundboard_master_gain(state: State<'_, AppState>, gain: f32) {
    state
        .engine
        .send_soundboard(SoundboardCommand::SetMasterGain(gain));
}

// ---- v1.17.0: text-to-speech ("Speak") ----

/// Stable clip id for synthesized speech in the soundboard mixer, so `speak`
/// can stop the previous utterance and `stop_speak` can target it. The id alone
/// replaces nothing — see [`speak_commands`].
const TTS_CLIP_ID: &str = "tts";

/// The mixer commands that start a Speak utterance: stop the previous one, then
/// play this one.
///
/// The Stop is load-bearing. The mixer takes an idle slot rather than replacing
/// a same-id voice, so without it every press stacks another voice on top of
/// the last. A multi-second Kokoro render mostly hid that; a babble render is
/// near-instant, so rapid presses would talk over themselves.
fn speak_commands(
    samples: Arc<Vec<f32>>,
    sample_rate: u32,
    gain: f32,
    preview_only: bool,
) -> [SoundboardCommand; 2] {
    let clip_id = TTS_CLIP_ID.to_string();
    let stop = SoundboardCommand::Stop {
        clip_id: clip_id.clone(),
    };
    // Preview → monitor-only (you hear it, the call doesn't); otherwise the
    // normal both-paths play used by the soundboard.
    let play = if preview_only {
        SoundboardCommand::PlayMonitorOnly {
            clip_id,
            samples,
            sample_rate,
            gain,
        }
    } else {
        SoundboardCommand::Play {
            clip_id,
            samples,
            sample_rate,
            gain,
        }
    };
    [stop, play]
}

/// Hand every command from [`speak_commands`] to `sink`, in order.
///
/// The send loop lives here rather than inline in [`speak`] so a test can
/// watch what actually reaches the engine: a version that sent only the Play
/// would look fine in a test that checked `speak_commands` alone.
fn emit_speak(
    mut sink: impl FnMut(SoundboardCommand),
    samples: Arc<Vec<f32>>,
    sample_rate: u32,
    gain: f32,
    preview_only: bool,
) {
    for cmd in speak_commands(samples, sample_rate, gain, preview_only) {
        sink(cmd);
    }
}

/// List the built-in "Speak" voices — Kokoro presets, then the procedural
/// Critter Chatter (babble) voices — each flagged with whether its model assets
/// are installed (so the UI can show "voice not installed" instead of failing
/// on Speak). Never touches the ONNX runtime — pure filesystem probing.
#[tauri::command]
fn list_tts_voices(state: State<'_, AppState>) -> Vec<divora_core::tts::TtsVoiceInfo> {
    divora_core::tts::list_voices(&state.tts_assets_dir)
}

/// Synthesize `text` with `voice_id` (a Kokoro preset, a procedural babble
/// voice, or a cloned voice) and play it through the output by reusing the
/// soundboard mixer seam (so a Discord/stream listener hears it too, mixed with
/// the live mic). Any utterance still playing is stopped first. Returns the
/// clip duration in seconds for the playback progress ring.
///
/// `gain` (v1.18.0, optional, default 1.0) is the linear playback volume.
/// `preview_only` (v1.18.0, optional, default false) routes the speech to the
/// local monitor only — you hear it, the call doesn't — for previewing.
///
/// Synthesis is batch work; it runs here on the command's worker thread (as
/// `play_soundboard_clip` decodes synchronously). Until the Kokoro model is
/// staged, `synthesize` returns `NotInstalled` immediately for a preset,
/// surfaced to the UI as a graceful error string rather than a hang. Babble
/// voices need no assets and render in well under a second.
#[tauri::command]
async fn speak(
    app: AppHandle,
    text: String,
    voice_id: String,
    gain: Option<f32>,
    preview_only: Option<bool>,
    candidates: Option<usize>,
    use_gpu: Option<bool>,
) -> Result<f32, String> {
    // Best-of-N is tens of seconds of CPU-bound synthesis. A *synchronous*
    // command runs on the main thread and would block the window's event loop —
    // freezing the whole UI (you couldn't even switch panels). So this command
    // is async and the synthesis + playback hand-off run on a blocking thread;
    // the event loop stays free to pump window messages and deliver the
    // `tts-progress` events emitted from inside.
    let candidates = candidates.unwrap_or(divora_core::tts::voxcpm::DEFAULT_CANDIDATES);
    let use_gpu = use_gpu.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || -> Result<f32, String> {
        let state = app.state::<AppState>();
        // A cloned-voice id routes through its stored engine: `VoxCPM` (accent-
        // preserving, from the reference clip) or OpenVoice (timbre-only, from
        // the stored SE on the `am_puck` base). Anything else → `synthesize`,
        // which renders presets with Kokoro and babble ids procedurally. A
        // babble id can never resolve as a clone (its ':' fails
        // `is_safe_voice_id`), so it never reaches the `VoxCPM` engine.
        let to_err = |e: divora_core::tts::TtsError| {
            tracing::info!(error = %e, voice = %voice_id, "speak: synthesis unavailable");
            e.to_string()
        };
        let audio = match load_cloned_voice(&state.voices_dir, &voice_id) {
            Some(ClonedVoice::VoxCpm { reference }) => {
                speak_voxcpm(&app, &state, &text, &reference, candidates, use_gpu)?
            }
            Some(ClonedVoice::OpenVoice { se, base }) => divora_core::tts::synthesize_cloned(
                &text,
                &base,
                &se,
                &state.tts_assets_dir,
                &clone_dir(&state),
                CLONE_TAU,
            )
            .map_err(to_err)?,
            None => divora_core::tts::synthesize(&text, &voice_id, &state.tts_assets_dir)
                .map_err(to_err)?,
        };
        let sample_rate = audio.sample_rate;
        let samples = Arc::new(audio.samples);
        #[allow(clippy::cast_precision_loss)]
        let duration_secs = if sample_rate == 0 {
            0.0
        } else {
            samples.len() as f32 / sample_rate as f32
        };
        // v1.28.0: persist the generated clip to the "Saved clips" folder so the
        // user can replay/keep it. Best-effort — never fails the speak.
        save_tts_clip(
            &speak_clips_path(&state),
            &samples,
            sample_rate,
            &text,
            &voice_id,
        );
        emit_speak(
            |cmd| state.engine.send_soundboard(cmd),
            samples,
            sample_rate,
            gain.unwrap_or(1.0),
            preview_only.unwrap_or(false),
        );
        Ok(duration_secs)
    })
    .await
    .map_err(|e| format!("synthesis task failed: {e}"))?
}

// ---- v1.47.0: tap-to-audition voice previews ----

/// Mixer slot for auditions. Deliberately NOT [`TTS_CLIP_ID`]: a preview must
/// not clobber the main Speak progress ring, and Speak's Stop must not kill a
/// preview (or vice versa).
const PREVIEW_CLIP_ID: &str = "tts-preview";

/// The line every preview says.
///
/// Fixed rather than the user's own text for two reasons. Holding the content
/// constant is what makes voices comparable by ear — the point of an audition.
/// And a stable phrase is what makes the cache possible at all: keying on the
/// text box would invalidate on every keystroke, which for a `VoxCPM` clone
/// means paying seconds of synthesis per tap forever.
///
/// It carries a question and a statement, so you hear both rising and falling
/// intonation, plus plosives and sibilants — and it is short, which bounds
/// both synthesis cost and the rambling a single take can produce.
const PREVIEW_TEXT: &str =
    "Hi there — how does this sound to you? Give me a line and I'll read it back.";

/// Bumped whenever [`PREVIEW_TEXT`] changes, so old cached audio is ignored
/// without needing migration code. It is part of the preset cache filename and
/// is checked against the clone sidecar.
const PREVIEW_TEXT_VERSION: u32 = 1;

/// Preset previews live in their own directory; presets have no per-voice
/// folder to hang them off. Sibling of `speak-clips/`.
fn preview_cache_dir(state: &AppState) -> PathBuf {
    state.recordings_dir.parent().map_or_else(
        || state.recordings_dir.join("preview-cache"),
        |p| p.join("preview-cache"),
    )
}

/// Where a voice's cached preview lives.
///
/// A cloned voice keeps its preview INSIDE its own directory, next to
/// `reference.wav` and `meta.json`. That placement buys correctness for free:
/// deleting the voice deletes the preview with it, and a rename touches only
/// `meta.json` (the id is stable), so a rename correctly does NOT invalidate —
/// the label changed, the timbre did not.
fn preview_path(state: &AppState, voice_id: &str) -> Option<PathBuf> {
    preview_path_in(&state.voices_dir, &preview_cache_dir(state), voice_id)
}

/// [`preview_path`] over explicit directories, so it is testable.
///
/// `None` means "don't cache". That is the answer for an unsafe id, and so for
/// every babble id (the ':' fails [`is_safe_voice_id`]) — deliberately: a babble
/// preview renders near-instantly, and a cached one would keep playing the old
/// sound after the synthesiser changes in an update.
fn preview_path_in(voices_dir: &Path, cache_dir: &Path, voice_id: &str) -> Option<PathBuf> {
    if !is_safe_voice_id(voice_id) {
        return None;
    }
    let cloned = cloned_voices_dir(voices_dir).join(voice_id);
    if cloned.is_dir() {
        return Some(cloned.join(format!("preview-v{PREVIEW_TEXT_VERSION}.wav")));
    }
    Some(cache_dir.join(format!("{voice_id}-v{PREVIEW_TEXT_VERSION}.wav")))
}

/// Read a cached preview back as raw mono samples.
fn read_preview(path: &Path) -> Option<(Vec<f32>, u32)> {
    let mut reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_format != hound::SampleFormat::Float {
        return None;
    }
    // Fail on the FIRST read error rather than filtering errors away: hound
    // drives this from the declared data-chunk length, so a truncated file
    // yields Err at EOF. Dropping that would hand back a short-but-non-empty
    // buffer and serve it as a valid cache hit — exactly what the temp-file
    // write dance exists to prevent.
    let samples = reader
        .samples::<f32>()
        .collect::<Result<Vec<f32>, _>>()
        .ok()?;
    if samples.is_empty() {
        return None;
    }
    Some((samples, spec.sample_rate))
}

/// Write a rendered preview to the cache. Best-effort: a cache we cannot write
/// costs a re-render next time, nothing more.
fn write_preview(path: &Path, samples: &[f32], sample_rate: u32) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!(?e, "could not create preview cache dir");
            return;
        }
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    // Write beside then rename, so an interrupted write cannot leave a
    // truncated file that would then be served as a valid cache hit.
    // Unique per call: two renders of the same voice can genuinely be in
    // flight (one the user abandoned, plus the one they asked for next), and a
    // shared temp name would let them interleave into a single file and publish
    // one take's header over another take's tail.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let tmp = path.with_extension(format!("{}.{nanos}.part", std::process::id()));
    match hound::WavWriter::create(&tmp, spec) {
        Ok(mut w) => {
            let mut write_failed = false;
            for &sample in samples {
                if w.write_sample(sample).is_err() {
                    write_failed = true;
                    break;
                }
            }
            // A disk-full mid-write can still finalize cleanly, stamping a
            // header for however many samples landed — so a write error is
            // fatal rather than publishing a silently short preview.
            if write_failed || w.finalize().is_err() {
                let _ = std::fs::remove_file(&tmp);
                return;
            }
        }
        Err(e) => {
            tracing::warn!(?e, "could not write preview wav");
            // hound already opened the file, so a mid-header failure leaves
            // debris nothing else would ever clean up.
            let _ = std::fs::remove_file(&tmp);
            return;
        }
    }
    if std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// v1.47.0: play a short fixed sample in `voice_id` so the user can shop for a
/// voice by ear.
///
/// Deliberately a separate command rather than a flag on [`speak`], because
/// three of its behaviours are wrong for an audition:
///
/// 1. `speak` saves every utterance to the user's Saved clips *before* it
///    branches on `preview_only`, so auditions would fill that library with
///    dozens of identical entries.
/// 2. It renders at the user's best-of-N tier — up to 6 takes, tens of
///    seconds. An audition answers "is this the right voice?", which one take
///    answers, so this forces N=1.
/// 3. It shares the `tts` mixer slot with the main Speak button.
///
/// Returns the clip duration in seconds. Cached renders return near-instantly;
/// babble voices are never cached (see [`preview_path_in`]) and don't need to be.
#[tauri::command]
async fn preview_voice(
    app: AppHandle,
    voice_id: String,
    use_gpu: Option<bool>,
) -> Result<f32, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<f32, String> {
        let state = app.state::<AppState>();
        // Claim this audition. Anything that supersedes it — another preview or
        // an explicit stop — bumps the epoch, and the check before playback
        // then discards this render.
        let epoch = state.preview_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        let cache = preview_path(&state, &voice_id);

        let cached = cache.as_deref().and_then(read_preview);
        let (samples, sample_rate) = if let Some(hit) = cached {
            hit
        } else {
            let to_err = |e: divora_core::tts::TtsError| e.to_string();
            let audio = match load_cloned_voice(&state.voices_dir, &voice_id) {
                // N=1: an audition is not a production take.
                //
                // The GPU flag is passed through rather than hardcoded:
                // `speak_voxcpm` keeps ONE shared engine and rebuilds all four
                // ONNX graphs (~1.6 GB) whenever the requested mode differs
                // from the cached one. Forcing CPU here would tear down a GPU
                // user's engine for an audition, then force another rebuild on
                // their very next Speak.
                Some(ClonedVoice::VoxCpm { reference }) => speak_voxcpm(
                    &app,
                    &state,
                    PREVIEW_TEXT,
                    &reference,
                    1,
                    use_gpu.unwrap_or(false),
                )?,
                Some(ClonedVoice::OpenVoice { se, base }) => divora_core::tts::synthesize_cloned(
                    PREVIEW_TEXT,
                    &base,
                    &se,
                    &state.tts_assets_dir,
                    &clone_dir(&state),
                    CLONE_TAU,
                )
                .map_err(to_err)?,
                None => {
                    divora_core::tts::synthesize(PREVIEW_TEXT, &voice_id, &state.tts_assets_dir)
                        .map_err(to_err)?
                }
            };
            if let Some(path) = cache.as_deref() {
                write_preview(path, &audio.samples, audio.sample_rate);
            }
            (audio.samples, audio.sample_rate)
        };

        #[allow(clippy::cast_precision_loss)]
        let duration_secs = if sample_rate == 0 {
            0.0
        } else {
            samples.len() as f32 / sample_rate as f32
        };

        // Superseded while rendering? Drop the result silently: it must not
        // stop the audition the user is actually listening to, and it must not
        // play itself long after they moved on.
        if state.preview_epoch.load(Ordering::Acquire) != epoch {
            return Ok(0.0);
        }
        // Stop any preview already playing BEFORE starting this one. The mixer
        // takes an idle slot rather than replacing a same-id voice, so without
        // this, shopping quickly through the grid plays several voices at once
        // — the exact opposite of what an audition is for.
        state.engine.send_soundboard(SoundboardCommand::Stop {
            clip_id: PREVIEW_CLIP_ID.to_string(),
        });
        // Monitor-only: auditioning is for you, not for the call.
        state
            .engine
            .send_soundboard(SoundboardCommand::PlayMonitorOnly {
                clip_id: PREVIEW_CLIP_ID.to_string(),
                samples: Arc::new(samples),
                sample_rate,
                gain: 1.0,
            });
        Ok(duration_secs)
    })
    .await
    .map_err(|e| format!("preview task failed: {e}"))?
}

/// Stop a preview, and invalidate any render still in flight.
///
/// Bumping the epoch is the half that matters for an uncached clone: the mixer
/// Stop only silences a clip that is already sounding, and the render itself
/// cannot be aborted — so it is invalidated instead and discards its result.
#[tauri::command]
fn stop_preview_voice(state: State<'_, AppState>) {
    state.preview_epoch.fetch_add(1, Ordering::AcqRel);
    state.engine.send_soundboard(SoundboardCommand::Stop {
        clip_id: PREVIEW_CLIP_ID.to_string(),
    });
}

/// Stop any in-flight synthesized speech playing through the mixer.
#[tauri::command]
fn stop_speak(state: State<'_, AppState>) {
    state.engine.send_soundboard(SoundboardCommand::Stop {
        clip_id: TTS_CLIP_ID.to_string(),
    });
}

// ---- v1.28.0: saved Speak clips ("Saved clips") ----

/// One saved Speak (TTS) clip: metadata stored as `<id>.json` next to the
/// `<id>.wav`. `path` (the WAV's absolute path) is filled in at list time so the
/// UI can replay it via [`play_soundboard_clip`]; it isn't persisted.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpeakClip {
    id: String,
    text: String,
    voice: String,
    /// Milliseconds since the Unix epoch.
    created_at: u64,
    duration_secs: f32,
    #[serde(default)]
    path: String,
}

/// Directory saved Speak clips are written to (`%APPDATA%/com.divora.voicemod/
/// speak-clips/`), a sibling of the recordings dir. Created lazily on first save.
fn speak_clips_path(state: &AppState) -> PathBuf {
    state.recordings_dir.parent().map_or_else(
        || state.recordings_dir.join("speak-clips"),
        |p| p.join("speak-clips"),
    )
}

/// Best-effort: persist a generated TTS clip (WAV + JSON sidecar) so it appears
/// in the "Saved clips" list. Never fails the `speak` call — errors are logged.
#[allow(clippy::cast_precision_loss)]
fn save_tts_clip(dir: &Path, samples: &[f32], sample_rate: u32, text: &str, voice: &str) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::warn!(?e, "could not create speak-clips dir");
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let id = now.as_nanos().to_string();
    let created_at = u64::try_from(now.as_millis()).unwrap_or(0);
    let duration_secs = if sample_rate == 0 {
        0.0
    } else {
        samples.len() as f32 / sample_rate as f32
    };
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    match hound::WavWriter::create(dir.join(format!("{id}.wav")), spec) {
        Ok(mut w) => {
            for &s in samples {
                let _ = w.write_sample(s);
            }
            let _ = w.finalize();
        }
        Err(e) => {
            tracing::warn!(?e, "could not write speak clip wav");
            return;
        }
    }
    let meta = SpeakClip {
        id,
        text: text.to_string(),
        voice: voice.to_string(),
        created_at,
        duration_secs,
        path: String::new(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&meta) {
        let _ = std::fs::write(dir.join(format!("{}.json", meta.id)), json);
    }
}

/// Absolute path of the saved-clips directory (for an "Open folder" button).
#[tauri::command]
fn speak_clips_dir(state: State<'_, AppState>) -> String {
    speak_clips_path(&state).to_string_lossy().into_owned()
}

/// Open the saved-clips folder in the OS file manager (Explorer on Windows).
/// Creates it first so it opens even before the first clip is saved. Uses a
/// native process rather than the frontend shell plugin so it reliably reveals
/// a *directory* (the plugin's `open` is scoped/validated for URLs + files).
///
/// v1.57.0: through the desktop's Explorer first ([`open_folder`]), off the
/// main thread.
#[tauri::command]
async fn open_speak_clips_folder(state: State<'_, AppState>) -> Result<(), String> {
    let dir = speak_clips_path(&state);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || open_folder(&dir))
        .await
        .map_err(|e| format!("opening the folder stopped: {e}"))?
}

/// v1.57.0: open `dir` in the file manager. On Windows the desktop's
/// Explorer opens it ([`divora_capture::open_outside_tree`]), so nothing the
/// user starts from that window becomes Divora's descendant (and left out of
/// "Everything except Divora"); where that fails, Divora starts the file
/// manager itself, as before.
fn open_folder(dir: &Path) -> Result<(), String> {
    if divora_capture::open_outside_tree(&dir.to_string_lossy(), None, divora_capture::Show::Normal)
        .is_ok()
    {
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let program = "xdg-open";
    // `explorer` returns a non-zero exit even on success, so just spawn (don't
    // wait) — a successful launch is all we need.
    std::process::Command::new(program)
        .arg(dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// v1.57.0: what [`open_external`] opens: a web link, or a folder that exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalTarget {
    Link,
    Folder,
}

/// v1.57.0: what the page may ask [`open_external`] to open. Only `http(s)`
/// links and absolute paths of folders that exist — what the page opens (the
/// changelog, GitHub, the update and VB-Cable pages, the voice-library and
/// logs folders) — never a file or a program, which Explorer would run.
fn external_target(target: &str) -> Result<ExternalTarget, String> {
    const REFUSED: &str = "Divora opens only web links and folders.";
    if target.is_empty() || target.chars().any(char::is_control) {
        return Err(REFUSED.to_string());
    }
    let lower = target.to_ascii_lowercase();
    for scheme in ["https://", "http://"] {
        if let Some(rest) = lower.strip_prefix(scheme) {
            return if rest.is_empty()
                || rest.starts_with('/')
                || target.contains(char::is_whitespace)
            {
                Err(REFUSED.to_string())
            } else {
                Ok(ExternalTarget::Link)
            };
        }
    }
    let path = Path::new(target);
    if path.is_absolute() && path.is_dir() {
        Ok(ExternalTarget::Folder)
    } else {
        Err(REFUSED.to_string())
    }
}

/// v1.57.0: open a web link or a folder from the desktop's Explorer, so a
/// browser it starts is Explorer's child, not Divora's: "Everything except
/// Divora" leaves out Divora's whole process tree, and a browser the shell
/// plugin's `open` started (when it wasn't running) was in it, so its sound
/// was missing from recordings until it was restarted. A folder that the
/// desktop can't open is opened by starting the file manager here
/// ([`open_folder`]), as the Speak clips folder is. Errors when the target
/// isn't a link or folder, or, for a link, when the desktop can't open it (no
/// desktop shell, Explorer not answering within `DESKTOP_WAIT`, or Explorer's
/// own open failed); the page then opens the link the old way. Async with
/// `spawn_blocking`: the cross-process call takes tens of milliseconds, and
/// the main thread is the hotkeys' too.
#[tauri::command]
async fn open_external(target: String) -> Result<(), String> {
    let kind = external_target(&target)?;
    tauri::async_runtime::spawn_blocking(move || match kind {
        ExternalTarget::Folder => open_folder(Path::new(&target)),
        ExternalTarget::Link => {
            divora_capture::open_outside_tree(&target, None, divora_capture::Show::Normal)
        }
    })
    .await
    .map_err(|e| format!("opening it stopped: {e}"))?
}

/// List saved Speak clips, newest first (each with its WAV `path` for replay).
#[tauri::command]
fn list_speak_clips(state: State<'_, AppState>) -> Vec<SpeakClip> {
    let dir = speak_clips_path(&state);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<SpeakClip> = entries
        .flatten()
        .filter_map(|entry| {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                return None;
            }
            let mut clip: SpeakClip = std::fs::read_to_string(&p)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())?;
            let wav = dir.join(format!("{}.wav", clip.id));
            if !wav.is_file() {
                return None; // orphaned sidecar
            }
            clip.path = wav.to_string_lossy().into_owned();
            Some(clip)
        })
        .collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.created_at));
    out
}

/// Delete a saved Speak clip (its WAV + sidecar) by id.
#[tauri::command]
fn delete_speak_clip(state: State<'_, AppState>, id: String) -> Result<(), String> {
    if !is_safe_voice_id(&id) {
        return Err("invalid clip id".to_string());
    }
    let dir = speak_clips_path(&state);
    let _ = std::fs::remove_file(dir.join(format!("{id}.wav")));
    let _ = std::fs::remove_file(dir.join(format!("{id}.json")));
    Ok(())
}

// ---- v1.20.0: voice cloning ("Your voices") ----

/// Default Kokoro base voice a cloned voice generates with before tone-color
/// conversion — `am_puck`, the closest stock voice in the Phase-2a validation.
/// Stored per-voice in meta so a later phase can vary it (e.g. by accent/gender).
const CLONE_BASE_VOICE: &str = "am_puck";
/// `OpenVoice` conversion temperature (its default).
const CLONE_TAU: f32 = 0.3;
/// Fallback `VoxCPM` diffusion seed if the system clock is unavailable (normally
/// each utterance seeds from the clock — see `speak_voxcpm`).
const VOXCPM_SEED: u64 = 0x4469_766F_7261; // "Divora"

/// One user-cloned voice (wire type for the Speak picker).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClonedVoiceInfo {
    /// Stable slug id (also the on-disk folder name).
    id: String,
    name: String,
    /// Short label of the preset this clone was matched to (v1.22.0), e.g.
    /// `"Puck"`. Empty if the stored base isn't a known preset.
    base_name: String,
    /// Cloning engine: `"voxcpm"` (accent-preserving) or `"openvoice"`
    /// (timbre-only). Lets the UI show the best-of-N quality control for
    /// `VoxCPM` voices only (v1.25.0).
    engine: String,
}

/// On-disk metadata for a cloned voice (`<id>/meta.json`).
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct ClonedMeta {
    name: String,
    /// Kokoro base voice used for generation before tone-color conversion
    /// (`OpenVoice` engine only).
    #[serde(default = "default_clone_base")]
    base: String,
    /// Cloning engine: `"openvoice"` (timbre-only, stores `se.bin`) or
    /// `"voxcpm"` (accent-preserving, stores `reference.wav`). Defaults to
    /// `"openvoice"` for voices cloned before v2 (no field).
    #[serde(default = "default_engine")]
    engine: String,
}

fn default_clone_base() -> String {
    CLONE_BASE_VOICE.to_string()
}

fn default_engine() -> String {
    "openvoice".to_string()
}

/// A resolved cloned voice ready for synthesis — the two engines need different
/// inputs (`OpenVoice` an SE + Kokoro base; `VoxCPM` the raw reference clip).
enum ClonedVoice {
    OpenVoice { se: Vec<f32>, base: String },
    VoxCpm { reference: PathBuf },
}

/// Directory holding user-cloned voices: `<voices>/cloned/`.
fn cloned_voices_dir(voices_dir: &Path) -> PathBuf {
    voices_dir.join("cloned")
}

/// Whether `id` is a single safe path component (no separators / traversal).
///
/// This is also what keeps engine-namespaced ids such as `babble:bright` out of
/// the cloned-voice and preview-cache paths: ':' is not allowed, so they can
/// never name (or collide with) a folder on disk.
fn is_safe_voice_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() < 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Resolve a stored cloned voice for synthesis. `None` if `id` isn't a cloned
/// voice — including every preset and babble id, which callers then hand to
/// `tts::synthesize`. Branches on the stored `engine`: `VoxCPM` voices carry a
/// `reference.wav`; `OpenVoice` voices (incl. pre-v2, no meta) carry `se.bin`.
fn load_cloned_voice(voices_dir: &Path, id: &str) -> Option<ClonedVoice> {
    if !is_safe_voice_id(id) {
        return None;
    }
    let dir = cloned_voices_dir(voices_dir).join(id);
    let meta = std::fs::read_to_string(dir.join("meta.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<ClonedMeta>(&s).ok());

    if meta.as_ref().is_some_and(|m| m.engine == "voxcpm") {
        let reference = dir.join("reference.wav");
        return reference
            .is_file()
            .then_some(ClonedVoice::VoxCpm { reference });
    }

    // OpenVoice (or legacy, no meta): 256-d SE + Kokoro base.
    let bytes = std::fs::read(dir.join("se.bin")).ok()?;
    if bytes.len() != 256 * 4 {
        return None;
    }
    let (quads, _tail) = bytes.as_chunks::<4>();
    let se = quads.iter().copied().map(f32::from_le_bytes).collect();
    let base = meta.map_or_else(|| CLONE_BASE_VOICE.to_string(), |m| m.base);
    Some(ClonedVoice::OpenVoice { se, base })
}

/// Scan `<voices>/cloned/` for valid cloned voices (folders with an `se.bin`).
fn list_cloned_voice_infos(dir: &Path) -> Vec<ClonedVoiceInfo> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let Some(id) = p.file_name().and_then(|s| s.to_str()).map(str::to_string) else {
            continue;
        };
        // A valid clone has either an OpenVoice SE or a `VoxCPM` reference clip.
        let has_voice = p.join("se.bin").is_file() || p.join("reference.wav").is_file();
        if !p.is_dir() || !is_safe_voice_id(&id) || !has_voice {
            continue;
        }
        let meta = std::fs::read_to_string(p.join("meta.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<ClonedMeta>(&s).ok());
        let name = meta.as_ref().map_or_else(|| id.clone(), |m| m.name.clone());
        let base_name = meta
            .as_ref()
            .and_then(|m| divora_core::tts::preset_short_name(&m.base))
            .unwrap_or_default()
            .to_string();
        // Fall back to a `reference.wav`-based guess when meta is missing.
        let engine = meta.as_ref().map_or_else(
            || {
                if p.join("reference.wav").is_file() {
                    "voxcpm".to_string()
                } else {
                    "openvoice".to_string()
                }
            },
            |m| m.engine.clone(),
        );
        out.push(ClonedVoiceInfo {
            id,
            name,
            base_name,
            engine,
        });
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// Create a cloned voice from a reference audio file: decode it, extract a
/// speaker embedding via the `OpenVoice` extractor, and store it under
/// `<voices>/cloned/<id>/`. Returns the new voice for the picker.
///
/// v1.57.0: runs on a blocking thread, not the main thread: the whole-file
/// decode and a fresh ONNX session take seconds to tens of seconds. One voice
/// at a time ([`CloneClaim`]).
#[tauri::command]
async fn clone_voice(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    reference_path: String,
) -> Result<ClonedVoiceInfo, String> {
    let claim = CloneClaim::try_claim(&state.voice_clone_busy)?;
    tauri::async_runtime::spawn_blocking(move || {
        let made = clone_from_path(&app.state::<AppState>(), &name, Path::new(&reference_path));
        drop(claim);
        made
    })
    .await
    .map_err(|e| format!("voice cloning stopped: {e}"))?
}

/// v1.57.0: what a second voice-making command refuses with, and a recording
/// started while one runs.
const VOICE_CLONE_BUSY: &str = "Divora is still making a voice. Try again when it's done.";

/// v1.57.0: held by [`clone_voice`] and [`stop_voice_recording`] while they
/// make a voice, so only one runs at a time, a page reload included. Taken
/// before the work is handed to its blocking thread and given back when
/// dropped, so an error (or a panic in a debug build) can't leave it held.
#[must_use]
struct CloneClaim(Arc<AtomicBool>);

impl CloneClaim {
    /// Takes `busy`, or refuses with [`VOICE_CLONE_BUSY`] while it's held.
    fn try_claim(busy: &Arc<AtomicBool>) -> Result<Self, String> {
        busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| VOICE_CLONE_BUSY.to_owned())?;
        Ok(Self(Arc::clone(busy)))
    }
}

impl Drop for CloneClaim {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// v1.57.0: refuses while a voice is being made. `stop_voice_recording`'s
/// thread still reads, then deletes, the one temp WAV that a new recording
/// would overwrite.
fn voice_clone_idle(busy: &AtomicBool) -> Result<(), String> {
    if busy.load(Ordering::Acquire) {
        Err(VOICE_CLONE_BUSY.to_owned())
    } else {
        Ok(())
    }
}

/// Decode a reference clip, extract a speaker embedding, auto-pick the closest
/// base (v1.22.0), and store the cloned voice under `<voices>/cloned/<id>/`.
/// Shared by [`clone_voice`] (imported file) and [`stop_voice_recording`]
/// (in-app dry-mic capture, v1.23.0).
fn clone_from_path(
    state: &AppState,
    name: &str,
    reference_path: &Path,
) -> Result<ClonedVoiceInfo, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("voice name is required".to_string());
    }
    // Cap at creation as well as rename (v1.43.0): an over-long name would
    // otherwise produce a voice that can't be renamed without first hand-
    // trimming it, and a very long one slugifies to an id that fails
    // `is_safe_voice_id`, leaving the voice on disk but invisible forever.
    if name.chars().count() > MAX_CLONE_NAME_LEN {
        return Err(format!(
            "name must be {MAX_CLONE_NAME_LEN} characters or fewer"
        ));
    }
    let clip = decode_clip(reference_path).map_err(|e| e.to_string())?;
    let se = divora_core::tts::extract_voice_se(&clip.samples, clip.sample_rate, &clone_dir(state))
        .map_err(|e| e.to_string())?;

    let dir = cloned_voices_dir(&state.voices_dir);
    let taken: std::collections::HashSet<String> = list_cloned_voice_infos(&dir)
        .into_iter()
        .map(|v| v.id)
        .collect();
    let id = unique_preset_id(&name, &name, &taken);
    let voice_dir = dir.join(&id);
    std::fs::create_dir_all(&voice_dir).map_err(|e| e.to_string())?;

    let mut bytes = Vec::with_capacity(se.len() * 4);
    for v in &se {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(voice_dir.join("se.bin"), bytes).map_err(|e| e.to_string())?;
    // Auto-pick the closest-sounding preset as the synthesis base (v1.22.0),
    // so the clone inherits the nearest accent/gender instead of one fixed
    // base. Falls back to `CLONE_BASE_VOICE` if `base-ses.json` is absent.
    let base = divora_core::tts::pick_base_voice(&se, &state.tts_assets_dir);
    let meta = ClonedMeta {
        name: name.clone(),
        base,
        engine: "openvoice".to_string(),
    };
    std::fs::write(
        voice_dir.join("meta.json"),
        serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    tracing::info!(id = %id, name = %name, base = %meta.base, "cloned voice created");
    let base_name = divora_core::tts::preset_short_name(&meta.base)
        .unwrap_or_default()
        .to_string();
    Ok(ClonedVoiceInfo {
        id,
        name,
        base_name,
        engine: meta.engine.clone(),
    })
}

/// v1.23.0: begin capturing the dry mic to a temp WAV for an in-app clone
/// reference. Requires the engine to be running (the dry tap lives in the
/// live input callback). The path is held in `AppState` until
/// [`stop_voice_recording`] consumes it.
///
/// v1.57.0: which engine the recording becomes is decided here, not at stop.
/// `VoxCPM` is given [`READ_ALOUD_PROMPT`](divora_core::tts::voxcpm::READ_ALOUD_PROMPT)
/// as the clip's transcript, and the page asks for that sentence only when
/// `VoxCPM` is installed as the take begins. A recording can now run while the
/// `VoxCPM` download does; checked at stop, a take spoken freely became a
/// `VoxCPM` voice whenever the download finished during it. The answer is the
/// sentence to read when the take becomes a `VoxCPM` voice, or `None` to speak
/// freely, so the page's hint is this decision, not its own status read (which
/// lags the disk by a round trip after a download ends).
#[tauri::command]
fn start_voice_recording(state: State<'_, AppState>) -> Result<Option<String>, String> {
    voice_clone_idle(&state.voice_clone_busy)?;
    if !state.engine.is_running() {
        return Err("start the engine first, then record your voice".to_string());
    }
    let dir = std::env::temp_dir().join("DivoraVoice");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // A fixed name is fine — only one capture runs at a time and it's consumed
    // (deleted) on stop.
    let path = dir.join("voice-reference.wav");
    // The check `voxcpm_status` answers the page with.
    let voxcpm = divora_core::tts::voxcpm::models_present(&voxcpm_dir(&state));
    *state.voice_recording_path.lock().unwrap() = Some((path.clone(), voxcpm));
    state.engine.start_reference_recording(path);
    Ok(voxcpm.then(|| divora_core::tts::voxcpm::READ_ALOUD_PROMPT.to_string()))
}

/// v1.23.0: stop the dry-mic capture (blocks until the WAV is finalized) and
/// turn it into a cloned voice named `name`. The temp clip is deleted after.
///
/// v1.57.0: all of it, the wait for the WAV included, runs on a blocking
/// thread, not the main thread; one voice at a time ([`CloneClaim`]).
#[tauri::command]
async fn stop_voice_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<ClonedVoiceInfo, String> {
    let claim = CloneClaim::try_claim(&state.voice_clone_busy)?;
    tauri::async_runtime::spawn_blocking(move || {
        let made = clone_from_recording(&app.state::<AppState>(), &name);
        drop(claim);
        made
    })
    .await
    .map_err(|e| format!("voice cloning stopped: {e}"))?
}

/// [`stop_voice_recording`]'s work, on its blocking thread.
fn clone_from_recording(state: &AppState, name: &str) -> Result<ClonedVoiceInfo, String> {
    state.engine.stop_reference_recording();
    let (path, voxcpm) = state
        .voice_recording_path
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| "no voice recording in progress".to_string())?;
    // A `VoxCPM` reference (accent-preserving) when `VoxCPM` was installed as
    // the recording started, so the page had the user read READ_ALOUD_PROMPT;
    // else OpenVoice timbre-transfer. Not checked again here: v1.57.0 lets the
    // `VoxCPM` download finish during a take (see `start_voice_recording`).
    let result = if voxcpm {
        store_voxcpm_clone(state, name, &path)
    } else {
        clone_from_path(state, name, &path)
    };
    // Best-effort cleanup of the transient reference clip.
    let _ = std::fs::remove_file(&path);
    result
}

/// Store a `VoxCPM` cloned voice: keep the recorded `reference` clip + a meta
/// marking the `voxcpm` engine. Unlike the `OpenVoice` path, no SE is extracted —
/// `VoxCPM` conditions on the raw clip at speak time.
fn store_voxcpm_clone(
    state: &AppState,
    name: &str,
    reference: &Path,
) -> Result<ClonedVoiceInfo, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("voice name is required".to_string());
    }
    // Same cap as the OpenVoice path + rename — see `clone_from_path`.
    if name.chars().count() > MAX_CLONE_NAME_LEN {
        return Err(format!(
            "name must be {MAX_CLONE_NAME_LEN} characters or fewer"
        ));
    }
    // Validate the recording decodes before committing it.
    decode_clip(reference).map_err(|e| e.to_string())?;

    let dir = cloned_voices_dir(&state.voices_dir);
    let taken: std::collections::HashSet<String> = list_cloned_voice_infos(&dir)
        .into_iter()
        .map(|v| v.id)
        .collect();
    let id = unique_preset_id(&name, &name, &taken);
    let voice_dir = dir.join(&id);
    std::fs::create_dir_all(&voice_dir).map_err(|e| e.to_string())?;
    std::fs::copy(reference, voice_dir.join("reference.wav")).map_err(|e| e.to_string())?;

    let meta = ClonedMeta {
        name: name.clone(),
        base: CLONE_BASE_VOICE.to_string(),
        engine: "voxcpm".to_string(),
    };
    std::fs::write(
        voice_dir.join("meta.json"),
        serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    tracing::info!(id = %id, name = %name, "voxcpm cloned voice created");
    // `VoxCPM` clones the user directly (no preset base), so no "based on" label.
    Ok(ClonedVoiceInfo {
        id,
        name,
        base_name: String::new(),
        engine: meta.engine.clone(),
    })
}

/// List the user's cloned voices for the Speak picker.
#[tauri::command]
fn list_cloned_voices(state: State<'_, AppState>) -> Vec<ClonedVoiceInfo> {
    list_cloned_voice_infos(&cloned_voices_dir(&state.voices_dir))
}

/// Longest display name accepted for a cloned voice (characters, not bytes).
const MAX_CLONE_NAME_LEN: usize = 64;

/// v1.43.0: rewrite a cloned voice's DISPLAY name in `<id>/meta.json`.
///
/// The folder (and thus the id) deliberately stays put: the id is the stable
/// key that the persisted voice selection, saved clips, and any in-flight
/// synthesis all reference, so renaming the directory would orphan them.
/// Split out from the command so it is testable without a Tauri `State`.
fn rename_cloned_voice_in(voices_dir: &Path, id: &str, name: &str) -> Result<(), String> {
    if !is_safe_voice_id(id) {
        return Err("invalid voice id".to_string());
    }
    let name = name.trim();
    if name.is_empty() {
        return Err("name cannot be empty".to_string());
    }
    if name.chars().count() > MAX_CLONE_NAME_LEN {
        return Err(format!(
            "name must be {MAX_CLONE_NAME_LEN} characters or fewer"
        ));
    }
    let dir = cloned_voices_dir(voices_dir).join(id);
    if !dir.is_dir() {
        return Err("voice not found".to_string());
    }

    // Preserve base/engine — only the name changes. A pre-v2 clone may have no
    // meta.json at all, so synthesize one using the same engine inference
    // `list_cloned_voice_infos` displays, keeping the badge and what actually
    // gets loaded for synthesis in agreement.
    let mut meta = std::fs::read_to_string(dir.join("meta.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<ClonedMeta>(&s).ok())
        .unwrap_or_else(|| ClonedMeta {
            name: id.to_string(),
            base: default_clone_base(),
            engine: if dir.join("reference.wav").is_file() {
                "voxcpm".to_string()
            } else {
                default_engine()
            },
        });
    meta.name = name.to_string();

    // Write to a temp file and rename over the original, so an interrupted
    // write can't leave a truncated meta.json (which would silently drop the
    // voice's base/engine and fall back to the id as its name). The temp name
    // is unique per call — a fixed one would let two concurrent renames of the
    // same voice interleave their writes and publish corrupt JSON, which is
    // exactly what this dance exists to prevent. It sits in the voice's own
    // directory so the rename is same-volume (and therefore atomic), and one
    // level below the dir `list_cloned_voice_infos` scans, so a stray temp file
    // can never show up as a voice.
    let json = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let tmp = dir.join(format!("meta.json.{}.{nanos}.tmp", std::process::id()));
    std::fs::write(&tmp, json).map_err(|e| {
        std::fs::remove_file(&tmp).ok(); // a partial write must not linger
        e.to_string()
    })?;
    std::fs::rename(&tmp, dir.join("meta.json")).map_err(|e| {
        std::fs::remove_file(&tmp).ok(); // don't leave the temp file behind
        e.to_string()
    })?;
    tracing::info!(id = %id, name = %name, "cloned voice renamed");
    Ok(())
}

/// Rename a cloned voice. Only the display name changes; the id is stable.
#[tauri::command]
fn rename_cloned_voice(state: State<'_, AppState>, id: String, name: String) -> Result<(), String> {
    rename_cloned_voice_in(&state.voices_dir, &id, &name)
}

/// Delete a cloned voice by id.
#[tauri::command]
fn delete_cloned_voice(state: State<'_, AppState>, id: String) -> Result<(), String> {
    if !is_safe_voice_id(&id) {
        return Err("invalid voice id".to_string());
    }
    let voice_dir = cloned_voices_dir(&state.voices_dir).join(&id);
    if voice_dir.is_dir() {
        std::fs::remove_dir_all(&voice_dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---- v1.21.0: on-demand OpenVoice model download ----

/// GitHub release hosting the bundled voice assets (the cloning models are
/// downloaded from here on demand rather than shipped in the installer).
const CLONE_MODELS_BASE_URL: &str =
    "https://github.com/NickSanft/DivoraVoiceMod/releases/download/voice-assets-v2";
/// Where the model downloads come from: [`CLONE_MODELS_BASE_URL`], or in a
/// debug build `DIVORA_MODELS_BASE_URL` when it is set, so a real-app check can
/// serve them from a local server. Release builds never read the variable.
fn models_base_url() -> String {
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("DIVORA_MODELS_BASE_URL") {
        return url;
    }
    CLONE_MODELS_BASE_URL.to_string()
}

/// The cloning model files to fetch (extractor is tiny; converter is ~157 MB).
const CLONE_MODEL_FILES: &[&str] = &["openvoice-extractor.onnx", "openvoice-converter.onnx"];
/// The `VoxCPM` accent-cloning model files to fetch (Q8 graphs + tokenizer,
/// ~1.6 GB total). Hosted on the same `voice-assets-v2` release.
const VOXCPM_MODEL_FILES: &[&str] = &[
    "voxcpm-prefill.onnx",
    "voxcpm-decode-step.onnx",
    "voxcpm-vae-encoder.onnx",
    "voxcpm-vae-decoder.onnx",
    "voxcpm-tokenizer.json",
    // WeSpeaker best-of-N reranker (~25 MB) — see `divora_core::tts::speaker`.
    "voxcpm-speaker.onnx",
];
/// The optional fp16 `DirectML` decode model (~1.2 GB) GPU cloning uses.
const VOXCPM_GPU_MODEL_FILES: &[&str] = &["voxcpm-decode-step-gpu.onnx"];

/// Where to read the cloning models from: the downloaded user dir if it
/// has them, else the bundled/dev asset dir (covers dev, where they're staged
/// in `resources/tts/`), else the user dir (the download target).
fn clone_dir(state: &AppState) -> PathBuf {
    if divora_core::tts::clone_models_present(&state.clone_models_dir) {
        state.clone_models_dir.clone()
    } else if divora_core::tts::clone_models_present(&state.tts_assets_dir) {
        state.tts_assets_dir.clone()
    } else {
        state.clone_models_dir.clone()
    }
}

/// Where to read the `VoxCPM` models from (the 4 graphs + tokenizer): the
/// downloaded user dir if present, else the bundled/dev asset dir, else the
/// download target. Mirrors [`clone_dir`].
fn voxcpm_dir(state: &AppState) -> PathBuf {
    use divora_core::tts::voxcpm::models_present;
    if models_present(&state.clone_models_dir) {
        state.clone_models_dir.clone()
    } else if models_present(&state.tts_assets_dir) {
        state.tts_assets_dir.clone()
    } else {
        state.clone_models_dir.clone()
    }
}

/// Synthesize `text` in a `VoxCPM`-cloned voice from its stored `reference` clip
/// (the user read [`READ_ALOUD_PROMPT`](divora_core::tts::voxcpm::READ_ALOUD_PROMPT)
/// when recording, so the transcript is known). Lazy-loads + holds the engine.
fn speak_voxcpm(
    app: &AppHandle,
    state: &AppState,
    text: &str,
    reference: &Path,
    candidates: usize,
    use_gpu: bool,
) -> Result<divora_core::tts::TtsAudio, String> {
    use divora_core::tts::voxcpm;
    let clip = decode_clip(reference).map_err(|e| e.to_string())?;
    let dir = voxcpm_dir(state);
    let mut guard = state.voxcpm_engine.lock().unwrap();
    // (Re)load when nothing is cached or the GPU mode changed — the engine holds a
    // CPU- or `DirectML`-built decode_step, so a toggle means a rebuild.
    if guard.as_ref().is_none_or(|(gpu, _)| *gpu != use_gpu) {
        *guard = voxcpm::VoxCpmEngine::load(&dir, use_gpu).map(|e| (use_gpu, e));
    }
    let engine = guard
        .as_mut()
        .map(|(_, e)| e)
        .ok_or_else(|| "voice-cloning models are not installed".to_string())?;
    // Fresh diffusion noise each utterance (a per-utterance seed). A fixed seed
    // can occasionally fail to trigger the model's stop and ramble to the length
    // cap; varying it keeps generation healthy (and natural per-utterance
    // variation is fine for a voice changer). Best-of-N then takes seeds
    // `seed..seed+N` and the reranker keeps the take closest to the reference.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0u128, |d| d.as_nanos());
    let seed = u64::try_from(nanos).unwrap_or(VOXCPM_SEED);
    engine
        .synthesize(
            text,
            &clip.samples,
            clip.sample_rate,
            voxcpm::READ_ALOUD_PROMPT,
            voxcpm::DEFAULT_CFG,
            seed,
            candidates.clamp(1, voxcpm::MAX_CANDIDATES),
            |done, total| {
                // Per-take progress → the Speak panel ("Take 2 of 6"), so a
                // multi-take generation shows motion instead of a frozen UI.
                let _ = app.emit("tts-progress", TtsProgress { done, total });
            },
        )
        .ok_or_else(|| "voice synthesis failed".to_string())
}

/// Best-of-N progress for the Speak UI: generating take `done` of `total`.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TtsProgress {
    done: usize,
    total: usize,
}

/// `VoxCPM` accent-preserving cloning status for the recorder UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoxCpmStatus {
    /// True when the `VoxCPM` models are installed, so the in-app recorder
    /// produces an accent-preserving clone (and should show the read prompt).
    available: bool,
    /// The sentence to display for the user to read aloud, so the reference
    /// transcript is known.
    read_prompt: String,
}

/// Whether `VoxCPM` cloning is available + the fixed sentence to read.
#[tauri::command]
fn voxcpm_status(state: State<'_, AppState>) -> VoxCpmStatus {
    VoxCpmStatus {
        available: divora_core::tts::voxcpm::models_present(&voxcpm_dir(&state)),
        read_prompt: divora_core::tts::voxcpm::READ_ALOUD_PROMPT.to_string(),
    }
}

/// Whether the cloning models are available (so the UI can prompt a
/// one-time download before the user records a voice).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CloneModelsStatus {
    ready: bool,
}

/// The model downloads' progress event (named before the `VoxCPM` sets
/// shared it).
const MODEL_DOWNLOAD_EVENT: &str = "clone-model-download";
/// v1.57.0: sent once when a download ends, either way ([`ModelDownloadDone`]).
const MODEL_DOWNLOAD_DONE_EVENT: &str = "clone-model-download-done";

/// v1.57.0: which model set a download fetches, on its events and in
/// [`model_download_status`], so a page knows which status to refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
enum ModelDownloadKind {
    /// [`CLONE_MODEL_FILES`], by [`download_clone_models`].
    #[serde(rename = "clone")]
    Clone,
    /// [`VOXCPM_MODEL_FILES`], by [`download_voxcpm_models`].
    #[serde(rename = "voxcpm")]
    VoxCpm,
    /// [`VOXCPM_GPU_MODEL_FILES`], by [`download_voxcpm_gpu_model`].
    #[serde(rename = "voxcpmGpu")]
    VoxCpmGpu,
}

/// Progress event payload for the model downloads ([`MODEL_DOWNLOAD_EVENT`]),
/// and what [`model_download_status`] answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CloneDownloadProgress {
    /// v1.57.0: which download this is.
    kind: ModelDownloadKind,
    /// Current file (1-based) and how many total.
    file: usize,
    file_count: usize,
    /// Bytes received / total for the current file (total 0 if unknown).
    received: u64,
    total: u64,
}

/// v1.57.0: payload of [`MODEL_DOWNLOAD_DONE_EVENT`]. `error` is the string
/// the command rejects with; `None` when every file is on disk.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelDownloadDone {
    kind: ModelDownloadKind,
    error: Option<String>,
}

/// v1.57.0: what a download started while another runs refuses with. The UI
/// shows it as it is.
const MODEL_DOWNLOAD_BUSY: &str = "A model download is already running.";

/// v1.57.0: the one model download allowed at a time, keeping its latest
/// progress for [`model_download_status`]. Two downloads of one set would
/// write the same `.part` files, and any two would mix their progress on one
/// event and in the one snapshot. Given back when dropped, so an error, or a
/// panic in a debug build, can't leave it held.
#[must_use]
struct DownloadSlot {
    snapshot: Arc<Mutex<Option<CloneDownloadProgress>>>,
    kind: ModelDownloadKind,
    file_count: usize,
}

impl DownloadSlot {
    /// Takes the slot for `kind`'s `file_count` files, or refuses with
    /// [`MODEL_DOWNLOAD_BUSY`] while another download holds it.
    fn try_claim(
        snapshot: &Arc<Mutex<Option<CloneDownloadProgress>>>,
        kind: ModelDownloadKind,
        file_count: usize,
    ) -> Result<Self, String> {
        let mut latest = snapshot.lock().unwrap_or_else(PoisonError::into_inner);
        if latest.is_some() {
            return Err(MODEL_DOWNLOAD_BUSY.to_owned());
        }
        let slot = Self {
            snapshot: Arc::clone(snapshot),
            kind,
            file_count,
        };
        *latest = Some(slot.payload(1, 0, 0));
        Ok(slot)
    }

    /// The progress of `file` (1-based), kept as the latest.
    fn progress(&self, file: usize, received: u64, total: u64) -> CloneDownloadProgress {
        let progress = self.payload(file, received, total);
        *self.snapshot.lock().unwrap_or_else(PoisonError::into_inner) = Some(progress.clone());
        progress
    }

    fn payload(&self, file: usize, received: u64, total: u64) -> CloneDownloadProgress {
        CloneDownloadProgress {
            kind: self.kind,
            file,
            file_count: self.file_count,
            received,
            total,
        }
    }
}

impl Drop for DownloadSlot {
    fn drop(&mut self) {
        *self.snapshot.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// The running download's latest progress, `None` when none runs.
fn download_snapshot(
    snapshot: &Mutex<Option<CloneDownloadProgress>>,
) -> Option<CloneDownloadProgress> {
    snapshot
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Whether the cloning models are installed.
#[tauri::command]
fn clone_models_status(state: State<'_, AppState>) -> CloneModelsStatus {
    CloneModelsStatus {
        ready: divora_core::tts::clone_models_present(&clone_dir(&state)),
    }
}

/// v1.57.0: the running model download's latest progress, `None` when none
/// runs, so a page that loads mid-download can show it. Reads the slot only.
#[tauri::command]
fn model_download_status(state: State<'_, AppState>) -> Option<CloneDownloadProgress> {
    download_snapshot(&state.model_download)
}

/// Download the cloning models into the user dir, emitting
/// `clone-model-download` progress events. Resolves once every file is on
/// disk; the UI awaits it and tracks progress via the events. Skips files
/// already present, and writes via a `.part` temp so a failed/partial download
/// never looks complete. v1.57.0: off the main thread ([`run_model_download`]).
#[tauri::command]
async fn download_clone_models(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    run_model_download(app, &state, ModelDownloadKind::Clone, CLONE_MODEL_FILES).await
}

/// Download the `VoxCPM` accent-cloning models (~1.6 GB) into the user dir,
/// reusing the `clone-model-download` progress events. Mirrors
/// [`download_clone_models`]; skips files already present.
#[tauri::command]
async fn download_voxcpm_models(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    run_model_download(app, &state, ModelDownloadKind::VoxCpm, VOXCPM_MODEL_FILES).await
}

/// `DirectML` GPU-acceleration status for the Speak UI: whether the platform can run
/// it and whether the opt-in fp16 GPU decode model is installed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoxCpmGpuStatus {
    /// Platform supports the `DirectML` build (Windows + DX12). The app is
    /// Windows-only, so effectively always true — kept explicit for the UI/future.
    supported: bool,
    /// The fp16 GPU decode model is present, so GPU mode can be turned on.
    model_present: bool,
}

/// Whether GPU (`DirectML`) cloning can be offered, and whether its model is on disk.
#[tauri::command]
fn voxcpm_gpu_status(state: State<'_, AppState>) -> VoxCpmGpuStatus {
    let paths = divora_core::tts::voxcpm::VoxCpmPaths::new(&voxcpm_dir(&state));
    VoxCpmGpuStatus {
        supported: cfg!(windows),
        model_present: paths.gpu_decode_present(),
    }
}

/// Download the optional fp16 `DirectML` decode model (~1.2 GB) that GPU cloning
/// uses. Reuses the `clone-model-download` progress events; no-op if present.
#[tauri::command]
async fn download_voxcpm_gpu_model(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    run_model_download(
        app,
        &state,
        ModelDownloadKind::VoxCpmGpu,
        VOXCPM_GPU_MODEL_FILES,
    )
    .await
}

/// v1.57.0: one model download, the body of the three download commands.
///
/// Tauri runs a sync command on the main thread, which also handles the
/// hotkeys (the mute among them), the tray and every other sync command, so
/// until v1.57.0 a download held all of them for minutes. The files now come
/// down on a blocking thread, one download at a time ([`DownloadSlot`]); a
/// set already on disk answers at once without taking the slot. At the end
/// the slot is given back, then [`MODEL_DOWNLOAD_DONE_EVENT`] goes out, then
/// the command answers, so a page that starts the next download on that
/// event isn't refused.
async fn run_model_download(
    app: AppHandle,
    state: &AppState,
    kind: ModelDownloadKind,
    files: &'static [&'static str],
) -> Result<(), String> {
    let dir = state.clone_models_dir.clone();
    if files.iter().all(|name| dir.join(name).is_file()) {
        return Ok(());
    }
    let slot = DownloadSlot::try_claim(&state.model_download, kind, files.len())?;
    let emitter = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let agent = model_agent(MODEL_READ_TIMEOUT);
        let result = download_model_files(&agent, &models_base_url(), &dir, files, &slot, |p| {
            let _ = emitter.emit(MODEL_DOWNLOAD_EVENT, p);
        });
        drop(slot);
        result
    })
    .await
    .unwrap_or_else(|e| Err(format!("model download stopped: {e}")));
    if let Err(e) = &result {
        tracing::warn!(?kind, error = %e, "model download failed");
    }
    let done = ModelDownloadDone {
        kind,
        error: result.as_ref().err().cloned(),
    };
    let _ = app.emit(MODEL_DOWNLOAD_DONE_EVENT, done);
    result
}

/// v1.57.0: the file loop of [`run_model_download`]. Each of `files` not yet
/// in `dir` comes from `base_url`, in order, with its progress kept in `slot`
/// and handed to `emit`. An error names the file, as the commands' always have.
fn download_model_files(
    agent: &ureq::Agent,
    base_url: &str,
    dir: &Path,
    files: &[&str],
    slot: &DownloadSlot,
    mut emit: impl FnMut(CloneDownloadProgress),
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for (i, name) in files.iter().enumerate() {
        let dest = dir.join(name);
        if dest.is_file() {
            continue;
        }
        let file = i + 1;
        // A page that reads the status now sees this file, not the last one.
        slot.progress(file, 0, 0);
        download_to_file(
            agent,
            &format!("{base_url}/{name}"),
            &dest,
            |received, total| {
                emit(slot.progress(file, received, total));
            },
        )
        .map_err(|e| format!("downloading {name}: {e}"))?;
    }
    tracing::info!(kind = ?slot.kind, path = %dir.display(), "models downloaded");
    Ok(())
}

/// v1.57.0: how long a model download waits for its next byte.
const MODEL_READ_TIMEOUT: Duration = Duration::from_secs(60);

/// v1.57.0: the HTTP agent the model downloads use. ureq's default agent has
/// no read timeout, so a connection that stalls mid-file never errors and the
/// download would sit at its last percent for good; once `read_timeout` goes
/// by without a byte, of the headers or of the body, it fails and can be
/// retried.
///
/// Every request gets a new connection. ureq 2 clears a connection's
/// timeouts when it keeps it for reuse and sets the read timeout again only
/// for the body, so from a set's second file on, GitHub's redirect and the
/// file itself would come over reused connections whose wait for the headers
/// has no timeout at all. The cost is a connection, TLS included, per request.
fn model_agent(read_timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(read_timeout)
        .max_idle_connections(0)
        .build()
}

/// Stream `url` to `dest` via a `.part` temp beside it, calling
/// `on_progress(received, total)` every MiB or so and once more when the file
/// is in place (`total` 0 when the server gave no length).
///
/// v1.57.0: takes the agent and a callback rather than an `AppHandle`, so the
/// tests run it against a local server. Any error once the `.part` exists
/// removes it, and a body that doesn't match its `Content-Length` is an
/// error. A process that ends mid-download (a tray Quit, a kill) leaves the
/// `.part`: the next start removes it ([`sweep_model_parts`]), and downloading
/// that file again empties and reuses it. On Windows the `.part` is opened
/// unshared: a second Divora downloading the same file at once gets an error
/// instead of emptying this one's `.part` and writing into it.
fn download_to_file(
    agent: &ureq::Agent,
    url: &str,
    dest: &Path,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    let resp = agent.get(url).call().map_err(|e| e.to_string())?;
    let total: u64 = resp
        .header("Content-Length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let tmp = dest.with_extension("part");
    let file = create_part(&tmp).map_err(|e| e.to_string())?;
    let landed = write_body(resp.into_reader(), file, total, &mut on_progress).and_then(|n| {
        std::fs::rename(&tmp, dest)
            .map(|()| n)
            .map_err(|e| e.to_string())
    });
    match landed {
        Ok(received) => {
            on_progress(received, total);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Creates (or empties) a download's `.part`; on Windows no one else can open
/// it while it's held (see [`download_to_file`]).
fn create_part(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    options.open(path)
}

/// v1.57.0: the `.part` files the model downloads write into `dir`, one for
/// each file of every set, and nothing else.
fn model_part_paths(dir: &Path) -> Vec<PathBuf> {
    CLONE_MODEL_FILES
        .iter()
        .chain(VOXCPM_MODEL_FILES)
        .chain(VOXCPM_GPU_MODEL_FILES)
        .map(|name| dir.join(name).with_extension("part"))
        .collect()
}

/// v1.57.0: at startup, removes the `.part`s a download left in `dir` when
/// the app quit during it (a `.part` can be up to 1.2 GB). Best effort: a
/// missing one is the usual case, and one a second Divora is writing right
/// now is held unshared ([`create_part`]), so its removal fails and it stays.
fn sweep_model_parts(dir: &Path) {
    for part in model_part_paths(dir) {
        if std::fs::remove_file(&part).is_ok() {
            tracing::info!(path = %part.display(), "removed an unfinished model download");
        }
    }
}

/// Copies `body` into `file`, which is closed on return, calling
/// `on_progress` every MiB or so; answers the bytes copied. A `total` other
/// than 0 that the body doesn't match is an error.
fn write_body(
    mut body: impl Read,
    mut file: std::fs::File,
    total: u64,
    on_progress: &mut impl FnMut(u64, u64),
) -> Result<u64, String> {
    let mut buf = vec![0u8; 1 << 16];
    let mut received: u64 = 0;
    let mut reported: u64 = 0;
    loop {
        let n = body.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        received += n as u64;
        // Throttle reports to ~every 1 MiB.
        if received - reported >= 1 << 20 {
            reported = received;
            on_progress(received, total);
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    // ureq already fails a body that ends before its length; this also
    // catches a chunked body sent with a length it doesn't match.
    if total != 0 && received != total {
        return Err(format!("expected {total} bytes, got {received}"));
    }
    Ok(received)
}

#[tauri::command]
fn detect_virtual_mic() -> VirtualMicStatus {
    detect_virtual_mic_core()
}

/// Takes `id`'s shortcut out of `map`, returning it to unregister only when
/// no other action still holds the same key. Windows keeps one registration
/// per key, so two actions bound to one key share it: until v1.56.0,
/// changing or clearing one of them unregistered it and silenced the other,
/// whose key Settings still showed.
fn release(map: &mut HashMap<String, Shortcut>, id: &str) -> Option<Shortcut> {
    let old = map.remove(id)?;
    (!map.values().any(|held| *held == old)).then_some(old)
}

/// Gives `id` the key `accelerator` names in `map`, returning the shortcut
/// to unregister (see [`release`]) and the parsed new one (to register). The
/// old one comes out before the parse: until v1.56.0 a key the parser refuses
/// (Shift with a digit arrives as "!") returned early and left the old key
/// live under a binding Settings no longer showed — a mute hotkey still
/// muting from a key the user thought was free.
fn rebind(
    map: &mut HashMap<String, Shortcut>,
    id: &str,
    accelerator: &str,
) -> (Option<Shortcut>, Result<Shortcut, String>) {
    let old = release(map, id);
    let new = accelerator
        .parse::<Shortcut>()
        .map_err(|e| format!("{e:?}"));
    if let Ok(shortcut) = new {
        map.insert(id.to_owned(), shortcut);
    }
    (old, new)
}

/// v1.58.0: the parsed shortcut when `id` already holds exactly the key
/// `accelerator` names. Re-registering it while it is live does nothing, so a
/// page reload's re-sync keeps a held push-to-modulate key held (U4).
fn same_binding(map: &HashMap<String, Shortcut>, id: &str, accelerator: &str) -> Option<Shortcut> {
    let parsed = accelerator.parse::<Shortcut>().ok()?;
    (map.get(id) == Some(&parsed)).then_some(parsed)
}

/// v1.58.0: [`rebind`], then [`ptm_key_unbound`], at the map update and
/// before anything can return early (ST-5): once `ptm` leaves its key, that
/// key's release can no longer reach the handler.
fn take_binding(
    map: &mut HashMap<String, Shortcut>,
    engine: &AudioEngine,
    id: &str,
    accelerator: &str,
) -> (Option<Shortcut>, Result<Shortcut, String>) {
    let taken = rebind(map, id, accelerator);
    ptm_key_unbound(engine, id);
    taken
}

/// v1.58.0: [`release`], then [`ptm_key_unbound`], whether or not a
/// shortcut comes back to unregister (a shared key doesn't).
fn drop_binding(
    map: &mut HashMap<String, Shortcut>,
    engine: &AudioEngine,
    id: &str,
) -> Option<Shortcut> {
    let old = release(map, id);
    ptm_key_unbound(engine, id);
    old
}

/// v1.58.0: every binding out of the map, to unregister, then
/// [`ptm_key_unbound`] for `ptm`.
fn drain_bindings(map: &mut HashMap<String, Shortcut>, engine: &AudioEngine) -> Vec<Shortcut> {
    let all = map.drain().map(|(_, shortcut)| shortcut).collect();
    ptm_key_unbound(engine, HOTKEY_PTM);
    all
}

// v1.58.0: the three shortcut commands call the plugin only after
// `AppState.shortcuts` is unlocked (D16). The plugin runs the handler under
// its own store lock, and the handler takes `AppState.shortcuts`; a command
// that held `AppState.shortcuts` into the plugin's `unregister`, which takes
// the store lock, could deadlock the main thread against a key released on
// global-hotkey's poller thread. Commands run one at a time on the main
// thread, so unlocking early races nothing.
#[tauri::command]
fn register_global_shortcut(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    accelerator: String,
) -> Result<(), String> {
    let same = {
        let map = state.shortcuts.lock().map_err(|e| e.to_string())?;
        same_binding(&map, &id, &accelerator)
    };
    if let Some(same) = same {
        // The same key, still registered: nothing to do, and a held key
        // stays held. One whose register failed earlier is tried again.
        if app.global_shortcut().is_registered(same) {
            return Ok(());
        }
    }
    let (old, new) = {
        let mut map = state.shortcuts.lock().map_err(|e| e.to_string())?;
        take_binding(&mut map, &state.engine, &id, &accelerator)
    };
    if let Some(old) = old {
        let _ = app.global_shortcut().unregister(old);
    }
    app.global_shortcut()
        .register(new?)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn unregister_global_shortcut(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let old = {
        let mut map = state.shortcuts.lock().map_err(|e| e.to_string())?;
        drop_binding(&mut map, &state.engine, &id)
    };
    if let Some(shortcut) = old {
        app.global_shortcut()
            .unregister(shortcut)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn unregister_all_global_shortcuts(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let all = {
        let mut map = state.shortcuts.lock().map_err(|e| e.to_string())?;
        drain_bindings(&mut map, &state.engine)
    };
    let mut last_err: Option<String> = None;
    for shortcut in all {
        if let Err(e) = app.global_shortcut().unregister(shortcut) {
            last_err = Some(e.to_string());
        }
    }
    match last_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// v1.9.0: list the available MIDI input ports for the Control surfaces
/// picker. Resilient — returns empty when no MIDI subsystem / ports.
#[tauri::command]
fn list_midi_inputs() -> Vec<MidiInputInfo> {
    list_midi_inputs_core()
}

/// v1.9.0: open the named MIDI input port. Each parsed message is
/// forwarded to the frontend as a `midi-message` event (drives
/// MIDI-learn + the mapping router). Opening replaces any existing
/// connection.
#[tauri::command]
fn open_midi_input(app: AppHandle, state: State<'_, AppState>, name: String) -> Result<(), String> {
    // Close any existing connection first — WinMM won't reopen a port
    // we already hold open.
    {
        let mut slot = state.midi_connection.lock().map_err(|e| e.to_string())?;
        *slot = None;
    }
    let conn = open_midi_input_core(&app, &name)?;
    let mut slot = state.midi_connection.lock().map_err(|e| e.to_string())?;
    *slot = Some(conn);
    Ok(())
}

/// v1.9.0: close the live MIDI input port (if any). Dropping the stored
/// connection releases it.
#[tauri::command]
fn close_midi_input(state: State<'_, AppState>) -> Result<(), String> {
    let mut slot = state.midi_connection.lock().map_err(|e| e.to_string())?;
    *slot = None;
    Ok(())
}

/// Decode a clip on first play; subsequent plays reuse the cached
/// `Arc<Vec<f32>>` so a hot soundboard doesn't decode anything twice.
/// v1.39.0: cap on the decoded-clip cache — bounds accumulated PCM for a very
/// large sound library. A soundboard rarely has this many distinct clips.
const MAX_CACHED_CLIPS: usize = 64;

fn decode_or_cache(
    state: &State<'_, AppState>,
    clip_id: &str,
    path: &Path,
) -> Result<DecodedClip, String> {
    {
        let cache = state.clip_cache.lock().map_err(|e| e.to_string())?;
        if let Some(clip) = cache.get(clip_id) {
            return Ok(clip.clone());
        }
    }
    let clip = decode_clip(path).map_err(|e| e.to_string())?;
    let mut cache = state.clip_cache.lock().map_err(|e| e.to_string())?;
    // v1.39.0: bound the cache so a big library can't accumulate decoded PCM
    // without limit (the same unbounded-growth class the v1.33.0 freeze fix
    // addressed). Once full, evict one arbitrary entry to make room — a
    // soundboard rarely has this many distinct clips, so it seldom bites.
    if cache.len() >= MAX_CACHED_CLIPS && !cache.contains_key(clip_id) {
        if let Some(k) = cache.keys().next().cloned() {
            cache.remove(&k);
        }
    }
    cache.insert(clip_id.to_owned(), clip.clone());
    Ok(clip)
}

/// v1.56.0: the hotkey id the backend mutes on. The `mute:` prefix is
/// reserved for later variants (hold-to-mute, on, off).
const HOTKEY_MUTE: &str = "mute";

/// v1.56.0: whether this shortcut event flips the mute: a press of `mute`
/// only. Release does nothing.
fn hotkey_mute_request(id: &str, pressed: bool) -> bool {
    pressed && id == HOTKEY_MUTE
}

/// v1.56.0: what a shortcut event does to the mute: a press of `mute` flips
/// it, anything else leaves it. The handler's whole mute step, so a test can
/// run it against a real engine (the handler itself needs an `AppHandle`).
fn mute_on_hotkey(engine: &AudioEngine, id: &str, pressed: bool) {
    if hotkey_mute_request(id, pressed) {
        engine.toggle_input_muted();
    }
}

/// v1.58.0: the hotkey id the backend holds push-to-modulate on, press and
/// release. The `ptm:` prefix is reserved (a latch, later).
const HOTKEY_PTM: &str = "ptm";

/// v1.58.0: what this shortcut event does to the hotkey's hold on
/// push-to-modulate: `Some(true)` for a press of `ptm`, `Some(false)` for its
/// release, `None` for any other id.
fn hotkey_ptm_request(id: &str, pressed: bool) -> Option<bool> {
    (id == HOTKEY_PTM).then_some(pressed)
}

/// v1.58.0: the handler's push-to-modulate step: a press of `ptm` holds, its
/// release lets go. Releases come on global-hotkey's poller thread; the
/// setter is one atomic op, safe there. Separate so a test can run it against
/// a real engine (the handler itself needs an `AppHandle`).
fn ptm_on_hotkey(engine: &AudioEngine, id: &str, pressed: bool) {
    if let Some(held) = hotkey_ptm_request(id, pressed) {
        engine.set_ptm_held(PtmSource::Hotkey, held);
    }
}

/// v1.58.0: `id` left its key in the shortcut map. For `ptm` that key's
/// release can no longer reach the handler (the plugin dropped the key, or
/// another id shares it and the handler finds that one), so let go of the
/// hotkey's hold here. A no-op for any other id.
fn ptm_key_unbound(engine: &AudioEngine, id: &str) {
    if id == HOTKEY_PTM {
        engine.set_ptm_held(PtmSource::Hotkey, false);
    }
}

/// Handle a global-shortcut event by looking up the registration id
/// from the shared state and emitting a `global-shortcut` Tauri event
/// the frontend can react to.
fn global_shortcut_handler(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    let state_opt = app.try_state::<AppState>();
    let Some(state) = state_opt else { return };
    let hit = {
        let Ok(map) = state.shortcuts.lock() else {
            return;
        };
        map.iter()
            .find(|(_, s)| *s == shortcut)
            .map(|(id, _)| id.clone())
    };
    if let Some(id) = hit {
        // v1.55.0: the overlay acts on its own hotkey here, inside
        // `WM_HOTKEY`, while the press's right to the foreground is fresh.
        // The event still goes out (the frontend only shows a notice).
        overlay_windows::hotkey(app, &id, event.state == ShortcutState::Pressed);
        // v1.56.0: the mute hotkey is acted on here too, so it works while
        // the page is hidden, reloading or busy. The frontend ignores the
        // event (acting there would flip it twice).
        mute_on_hotkey(&state.engine, &id, event.state == ShortcutState::Pressed);
        // v1.58.0: and push-to-modulate's, press and release, for the same
        // reasons. The frontend does nothing with the event either.
        ptm_on_hotkey(&state.engine, &id, event.state == ShortcutState::Pressed);
        let state_str = match event.state {
            ShortcutState::Pressed => "pressed",
            ShortcutState::Released => "released",
        };
        let payload = GlobalShortcutEvent {
            id,
            accelerator: shortcut.to_string(),
            state: state_str,
        };
        let _ = app.emit("global-shortcut", &payload);
    }
}

/// One `audio-levels` tick's payload. Extracted so a test runs the emitter's
/// real builder. The mute is one load, so its two fields always agree; so is
/// push-to-modulate (v1.58.0).
fn level_update(engine: &AudioEngine) -> LevelUpdate {
    let mute = engine.input_mute();
    LevelUpdate {
        input: engine.input_levels(),
        output: engine.output_levels(),
        running: engine.is_running(),
        monitoring: engine.is_monitoring(),
        dsp_latency_ms: engine.dsp_latency_ms(),
        recording: engine.is_recording(),
        loudness_gain_db: engine.loudness_gain_db(),
        device_lost: engine.device_recovery_failed(),
        mod_env: engine.reactive_depth(),
        input_muted: mute.muted,
        input_mute_seq: mute.seq,
        ptm: engine.ptm(),
    }
}

/// Spawn a background thread that emits `audio-levels` events to the
/// frontend at ~30 Hz. The frontend uses these to drive live meters
/// and the status pill without polling commands.
fn spawn_level_emitter(app: AppHandle, engine: Arc<AudioEngine>) {
    std::thread::Builder::new()
        .name("divora-level-emitter".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(33));
            let payload = level_update(&engine);
            if app.emit("audio-levels", &payload).is_err() {
                // App is shutting down (no receivers / window gone).
                break;
            }
        })
        .expect("spawning the level-emitter thread should not fail");
}

/// Spawn the `voice-reading` emitter (~5 Hz, and only while the panel is on).
///
/// Its own event rather than more fields on `audio-levels`, for three
/// reasons. The reading is a window measurement that changes about once a
/// second, so riding a 30 Hz meter tick would send the same numbers six times
/// over. The feature is off by default, and a field on the always-on payload
/// would carry an empty reading to every user who never opens the panel.
/// And the panel's accessible summary is text: at 30 Hz it would be a value
/// that changes many times a second, which is precisely what must not go near
/// a live region. Additive either way under `docs/STABLE-SURFACE.md`; this
/// keeps `LevelUpdate` meaning what it has always meant.
fn spawn_reading_emitter(app: AppHandle, engine: Arc<AudioEngine>) {
    std::thread::Builder::new()
        .name("divora-reading-emitter".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(200));
            // Silent while off: nothing is being measured, so there is
            // nothing to say and no reason to wake the frontend.
            if !engine.reading_enabled() {
                continue;
            }
            let payload = VoiceReadingUpdate::from_snapshot(&engine.voice_reading());
            if app.emit("voice-reading", &payload).is_err() {
                // App is shutting down (no receivers / window gone).
                break;
            }
        })
        .expect("spawning the reading-emitter thread should not fail");
}

/// Whether a close request on the window labelled `label` should hide it
/// instead of closing it.
///
/// Only the main window hides to the tray (Phase 15). Until v1.51.3 the
/// handler hid *every* window, so the stream overlay's `close()` only hid it:
/// it was never destroyed, `tauri://destroyed` never fired, and the next
/// "Show overlay window" found the hidden window and left it hidden.
#[doc(hidden)]
pub fn hides_on_close(label: &str) -> bool {
    label == "main"
}

/// Reveal + focus the main window (from a tray click / "Show" menu).
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// The tray icon's id.
pub(crate) const TRAY_ID: &str = "divora-tray";

/// What the tray menu holds besides Show and Quit.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrayItems {
    /// Replay is on: Save replay and Turn Replay off (v1.54.0).
    pub replay: bool,
    /// The overlay is in Arrange: Lock the overlay (v1.55.0).
    pub lock_overlay: bool,
}

/// One line of the tray menu: an item's id and label, or a separator.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEntry {
    Item(&'static str, &'static str),
    Separator,
}

/// The tray menu's lines: Show · Save replay · Turn Replay off · Lock the
/// overlay · Quit, each group set off by separators, the optional ones only
/// while they apply. The tray never turns Replay on (that goes through the
/// card) and never starts Arrange.
#[doc(hidden)]
pub fn tray_entries(items: TrayItems) -> Vec<TrayEntry> {
    let mut out = vec![TrayEntry::Item("show", "Show DivoraVoice")];
    if items.replay {
        out.extend([
            TrayEntry::Separator,
            TrayEntry::Item(TRAY_SAVE_ID, TRAY_SAVE_LABEL),
            TrayEntry::Item(TRAY_OFF_ID, TRAY_OFF_LABEL),
        ]);
    }
    if items.lock_overlay {
        out.extend([
            TrayEntry::Separator,
            TrayEntry::Item(
                overlay_machine::TRAY_LOCK_ID,
                overlay_machine::TRAY_LOCK_LABEL,
            ),
        ]);
    }
    if items.replay || items.lock_overlay {
        out.push(TrayEntry::Separator);
    }
    out.push(TrayEntry::Item("quit", "Quit"));
    out
}

/// The tray menu for `items`.
fn tray_menu<R: tauri::Runtime, M: Manager<R>>(
    app: &M,
    items: TrayItems,
) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;
    for entry in tray_entries(items) {
        match entry {
            TrayEntry::Item(id, label) => {
                menu.append(&MenuItem::with_id(app, id, label, true, None::<&str>)?)?;
            }
            TrayEntry::Separator => menu.append(&PredefinedMenuItem::separator(app)?)?,
        }
    }
    Ok(menu)
}

/// v1.55.0: Replay's half of the tray menu, as `sync_tray` last set it, so
/// the overlay can rebuild the menu without asking the recorder.
static TRAY_REPLAY: AtomicBool = AtomicBool::new(false);

/// v1.55.0: rebuild only the tray menu (not its tooltip or icon) from the
/// latest Replay and overlay state: the overlay's Lock item comes and goes
/// with Arrange. On the main thread, as `sync_tray`.
pub(crate) fn refresh_tray_menu(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id(TRAY_ID) else {
            return;
        };
        let items = TrayItems {
            replay: TRAY_REPLAY.load(Ordering::SeqCst),
            lock_overlay: overlay_windows::arranging(&handle),
        };
        if let Ok(menu) = tray_menu(&handle, items) {
            let _ = tray.set_menu(Some(menu));
        }
    });
}

/// v1.54.0: show `view` in the tray — its tooltip, menu and icon. On the
/// main thread and without waiting for it: the tray's setters block when
/// called from elsewhere, and Quit holds the main thread while it waits.
pub(crate) fn sync_tray(app: &AppHandle, view: TrayView) {
    TRAY_REPLAY.store(view.replay_items, Ordering::SeqCst);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id(TRAY_ID) else {
            return;
        };
        let _ = tray.set_tooltip(Some(&view.tooltip));
        let items = TrayItems {
            replay: view.replay_items,
            lock_overlay: overlay_windows::arranging(&handle),
        };
        if let Ok(menu) = tray_menu(&handle, items) {
            let _ = tray.set_menu(Some(menu));
        }
        let icon = match view.icon {
            TrayIconKind::Replay => {
                Some(Image::new(REPLAY_ICON, REPLAY_ICON_SIZE, REPLAY_ICON_SIZE))
            }
            TrayIconKind::Normal => handle.default_window_icon().cloned(),
        };
        let _ = tray.set_icon(icon);
    });
}

/// Phase 15: build the system-tray icon + menu so the app can run in the
/// background (e.g. while gaming / on a Discord call). Left-click the
/// tray icon or "Show" to restore the window; "Quit" exits for real.
fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let menu = tray_menu(app, TrayItems::default())?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("DivoraVoice")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "quit" => {
                // v1.52.0: a screen recording in progress is finished first;
                // exiting under it would leave an unplayable `.part`.
                // v1.54.0: then a replay being saved, before Replay goes.
                if let Some(state) = app.try_state::<AppState>() {
                    if state.screen.is_busy() {
                        let _ = state.screen.stop(app);
                    }
                    state.screen.quit_replay(app);
                }
                // v1.55.0: the overlay's `NonRudeHWND` comes off first; its
                // window isn't closed through `CloseRequested` on the way out.
                overlay_windows::before_exit(app);
                app.exit(0);
            }
            TRAY_SAVE_ID => {
                if let Some(state) = app.try_state::<AppState>() {
                    let at = Instant::now();
                    let recorder = Arc::clone(&state.screen);
                    let dir = state.recordings_dir.clone();
                    let app = app.clone();
                    // The event and the tooltip say how it went.
                    tauri::async_runtime::spawn_blocking(move || {
                        let _ = recorder.save_replay(&app, dir, at);
                    });
                }
            }
            TRAY_OFF_ID => {
                if let Some(state) = app.try_state::<AppState>() {
                    let recorder = Arc::clone(&state.screen);
                    let app = app.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        recorder.disarm_replay(&app);
                    });
                }
            }
            // v1.55.0: present only while the overlay is in Arrange. The
            // keyboard stays put: the tray's window has it now.
            overlay_machine::TRAY_LOCK_ID => {
                let _ = overlay_windows::set_mode(app, ModeRequest::Locked);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// v1.55.0: bounds how many charm pictures the `charm` scheme holds at once.
static SERVE_GATE: OnceLock<ServeGate> = OnceLock::new();

/// v1.55.0: answer one `charm` request (`http://charm.localhost/<id>`).
///
/// This runs on `WebView2`'s UI thread, so it only hands the request to a
/// blocking thread, which holds a [`SERVE_GATE`] place from before the read
/// until the response is handed back.
fn charm_request(
    ctx: tauri::UriSchemeContext<'_, tauri::Wry>,
    req: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    let webview = ctx.webview_label().to_owned();
    let method = req.method().as_str().to_owned();
    let path = req.uri().path().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let _slot = SERVE_GATE
            .get_or_init(|| ServeGate::new(SERVE_SLOTS))
            .acquire();
        // No state yet (the app is still starting): `serve` answers 503.
        let dir = app
            .try_state::<AppState>()
            .map(|s| s.overlay_assets_dir.clone());
        responder.respond(overlay_assets::serve(
            dir.as_deref(),
            &webview,
            &method,
            &path,
            || overlay_windows::overlay_live(&app),
        ));
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::too_many_lines)] // builder chain + setup closure; clearer inline
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(global_shortcut_handler)
                .build(),
        )
        // v1.55.0: the stream overlay's charm pictures.
        .register_asynchronous_uri_scheme_protocol(CHARM_SCHEME, charm_request)
        // Phase 15: closing the main window hides it to the tray instead of
        // quitting, so audio (and Discord routing) keep running in the
        // background. "Quit" in the tray menu exits for real. Only `main`:
        // every other window (the stream overlay) closes for real — see
        // `hides_on_close`.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if hides_on_close(window.label()) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            // v1.55.0: the overlay's close (Alt+F4, `close_overlay`, the
            // watchdog) and its end reach the shell.
            if window.label() == overlay_machine::OVERLAY_LABEL {
                overlay_windows::on_window_event(window.app_handle(), event);
            }
        })
        .setup(|app| {
            // v1.14.0: install file logging first so both startup and
            // runtime tracing events land in a rolling daily log under
            // %APPDATA%/com.divora.voicemod/logs/ — nothing collected them before
            // (there was no subscriber), so logs were silently dropped.
            let logs_dir = match app.path().app_data_dir() {
                Ok(dir) => dir.join("logs"),
                Err(_) => std::env::temp_dir().join("DivoraVoice").join("logs"),
            };
            let _ = std::fs::create_dir_all(&logs_dir);
            let _ = tracing_subscriber::fmt()
                .with_writer(tracing_appender::rolling::daily(&logs_dir, "divora.log"))
                .with_ansi(false)
                .try_init();
            tracing::info!(path = %logs_dir.display(), "file logging ready");

            if let Err(e) = setup_tray(app) {
                tracing::warn!(?e, "failed to set up system tray");
            }

            let engine = Arc::new(AudioEngine::new());
            spawn_level_emitter(app.handle().clone(), engine.clone());
            spawn_reading_emitter(app.handle().clone(), engine.clone());

            // Locate %APPDATA%\com.divora.voicemod\presets\ (or platform equivalent)
            // and prepare the user preset store. Failures here are logged
            // but non-fatal — the bundled presets still ship.
            let presets_dir = match app.path().app_data_dir() {
                Ok(dir) => dir.join("presets"),
                Err(e) => {
                    tracing::warn!(?e, "no app data dir; falling back to temp for presets");
                    std::env::temp_dir().join("DivoraVoice").join("presets")
                }
            };
            let preset_store =
                Arc::new(PresetStore::new(presets_dir.clone()).unwrap_or_else(|e| {
                    tracing::warn!(?e, "preset store init failed; using temp fallback");
                    let fallback = std::env::temp_dir().join("DivoraVoice").join("presets");
                    PresetStore::new(fallback).expect("temp preset store must init")
                }));
            tracing::info!(path = %preset_store.base_dir().display(), "preset store ready");

            // Phase 12: voices directory. Best-effort create; a failure
            // here just means the library lists empty until the dir
            // exists.
            let voices_dir = match app.path().app_data_dir() {
                Ok(dir) => dir.join("voices"),
                Err(e) => {
                    tracing::warn!(?e, "no app data dir; falling back to temp for voices");
                    std::env::temp_dir().join("DivoraVoice").join("voices")
                }
            };
            if let Err(e) = std::fs::create_dir_all(&voices_dir) {
                tracing::warn!(?e, path = %voices_dir.display(), "could not create voices dir");
            }
            tracing::info!(path = %voices_dir.display(), "voices dir ready");

            // Phase 16: recordings directory. Best-effort create; the
            // start_recording command also ensures it before writing.
            let recordings_dir = match app.path().app_data_dir() {
                Ok(dir) => dir.join("recordings"),
                Err(e) => {
                    tracing::warn!(?e, "no app data dir; falling back to temp for recordings");
                    std::env::temp_dir().join("DivoraVoice").join("recordings")
                }
            };
            if let Err(e) = std::fs::create_dir_all(&recordings_dir) {
                tracing::warn!(?e, path = %recordings_dir.display(), "could not create recordings dir");
            }
            tracing::info!(path = %recordings_dir.display(), "recordings dir ready");

            // v1.55.0: the stream overlay's charm pictures. `tauri dev`
            // builds keep their own scene, so they keep their own pictures:
            // one build can't delete the other's. Not `debug_assertions`,
            // which `tauri build --debug` shares with the installed app.
            let assets_name = overlay_assets::assets_dir_name(tauri::is_dev());
            let overlay_assets_dir = match app.path().app_data_dir() {
                Ok(dir) => dir.join("overlay").join(assets_name),
                Err(e) => {
                    tracing::warn!(?e, "no app data dir; falling back to temp for overlay assets");
                    std::env::temp_dir()
                        .join("DivoraVoice")
                        .join("overlay")
                        .join(assets_name)
                }
            };
            if let Err(e) = std::fs::create_dir_all(&overlay_assets_dir) {
                tracing::warn!(?e, path = %overlay_assets_dir.display(), "could not create overlay assets dir");
            }
            tracing::info!(path = %overlay_assets_dir.display(), "overlay assets dir ready");

            // Phase 12.4: discover bundled voice assets shipped in the
            // installer's resource dir (onnxruntime.dll + voices/*.onnx).
            // Two things to wire:
            //   1. Point `ort` (load-dynamic) at the bundled runtime DLL
            //      via ORT_DYLIB_PATH — unless the user already set it,
            //      or the DLL sits next to the exe (dev builds). This is
            //      also what divora-core's `onnx_runtime_available()`
            //      probe checks, so the Voice library reports "detected".
            //   2. Expose the bundled voices dir so `list_voices` lists
            //      the shipped model alongside user-installed ones.
            let bundled_voices_dir = match app.path().resource_dir() {
                Ok(res) => {
                    let dll = res.join("onnxruntime.dll");
                    if dll.is_file() && std::env::var_os("ORT_DYLIB_PATH").is_none() {
                        std::env::set_var("ORT_DYLIB_PATH", &dll);
                        tracing::info!(path = %dll.display(), "ORT_DYLIB_PATH set to bundled runtime");
                    }
                    let vdir = res.join("voices");
                    vdir.is_dir().then_some(vdir)
                }
                Err(e) => {
                    tracing::warn!(?e, "no resource dir; bundled voices unavailable");
                    None
                }
            };

            // v1.17.0: the bundled TTS asset dir (resource dir's `tts/`
            // subfolder). Always resolved to an expected path even when the
            // files aren't bundled yet — `list_tts_voices`/`speak` probe the
            // individual files and degrade to "not installed" if absent.
            let tts_assets_dir = match app.path().resource_dir() {
                Ok(res) => res.join("tts"),
                Err(e) => {
                    tracing::warn!(?e, "no resource dir; bundled TTS assets unavailable");
                    std::env::temp_dir().join("DivoraVoice").join("tts")
                }
            };
            tracing::info!(path = %tts_assets_dir.display(), "tts assets dir resolved");

            // Dev fallback: a `tauri dev` (debug) build ships no bundled
            // resources, so the resource-dir path above is empty and Speak
            // would report "not installed". Point at the source-staged assets
            // next to the crate (`src-tauri/resources/tts/`, populated by
            // `scripts/fetch-voice-assets.ps1`) so Speak works in dev too —
            // and set `ORT_DYLIB_PATH` to the source-staged runtime so the
            // Kokoro session can actually load. Release builds skip this
            // entirely and use the real bundled resource dir.
            #[cfg(debug_assertions)]
            let tts_assets_dir = if tts_assets_dir.join("kokoro-v1.0.int8.onnx").exists() {
                tts_assets_dir
            } else {
                let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("resources")
                    .join("tts");
                if dev.join("kokoro-v1.0.int8.onnx").exists() {
                    if std::env::var_os("ORT_DYLIB_PATH").is_none() {
                        let dll = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("resources")
                            .join("onnxruntime.dll");
                        if dll.is_file() {
                            std::env::set_var("ORT_DYLIB_PATH", &dll);
                            tracing::info!(path = %dll.display(), "dev: ORT_DYLIB_PATH set to source-staged runtime");
                        }
                    }
                    tracing::info!(path = %dev.display(), "dev: using source-staged TTS assets");
                    dev
                } else {
                    tts_assets_dir
                }
            };

            // v1.21.0: user dir the OpenVoice cloning models download into
            // (`%APPDATA%/com.divora.voicemod/tts/`). Best-effort create.
            let clone_models_dir = match app.path().app_data_dir() {
                Ok(dir) => dir.join("tts"),
                Err(e) => {
                    tracing::warn!(?e, "no app data dir; clone models fall back to temp");
                    std::env::temp_dir().join("DivoraVoice").join("tts")
                }
            };
            if let Err(e) = std::fs::create_dir_all(&clone_models_dir) {
                tracing::warn!(?e, path = %clone_models_dir.display(), "could not create clone models dir");
            }
            // v1.57.0: a Quit no longer waits for a running download, so it
            // can leave a `.part` behind.
            sweep_model_parts(&clone_models_dir);
            tracing::info!(path = %clone_models_dir.display(), "clone models dir ready");

            app.manage(AppState {
                engine,
                preview_epoch: AtomicU64::new(0),
                preset_store,
                clip_cache: Mutex::new(HashMap::new()),
                shortcuts: Mutex::new(HashMap::new()),
                voices_dir,
                bundled_voices_dir,
                tts_assets_dir,
                clone_models_dir,
                recordings_dir,
                logs_dir,
                midi_connection: Mutex::new(None),
                voice_recording_path: Mutex::new(None),
                voice_clone_busy: Arc::new(AtomicBool::new(false)),
                model_download: Arc::new(Mutex::new(None)),
                voxcpm_engine: Mutex::new(None),
                screen: Arc::new(ScreenRecorder::default()),
                overlay_assets_dir,
            });
            overlay_windows::init(app);

            // The window is created hidden (`visible: false`) so the user never
            // sees an empty black frame while WebView2 initializes. The frontend
            // reveals it the instant the boot splash paints (see index.html) —
            // that's the only moment we know WebView2 has rendered. This Rust
            // timer is just a safety net: if that frontend reveal never runs
            // (JS error, non-web target), show the window anyway so it can't get
            // stuck hidden. `show()` is idempotent, so the earlier of the two
            // wins. By label (v1.55.0): the overlay is a backend-made window
            // too, so "the first window" isn't necessarily the main one.
            if let Some(win) = app.get_webview_window("main") {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(10));
                    let _ = win.show();
                    let _ = win.set_focus();
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_audio_input_devices,
            list_audio_output_devices,
            start_audio_engine,
            stop_audio_engine,
            set_audio_monitor,
            set_speak_monitor,
            set_input_muted,
            toggle_input_muted,
            input_mute,
            set_ptm_mode,
            set_ptm_held,
            set_monitor_gain,
            set_loudness_enabled,
            set_loudness_target,
            audio_engine_status,
            set_effect_chain,
            set_reactive_config,
            set_voice_reading_enabled,
            voice_reading,
            set_effect_param,
            set_effect_enabled,
            clear_effect_chain,
            list_presets,
            save_user_preset,
            delete_user_preset,
            export_preset_json,
            import_preset,
            preset_store_path,
            voices_dir,
            list_voices,
            onnx_runtime_status,
            set_voice_model,
            recordings_dir,
            logs_dir,
            start_recording,
            stop_recording,
            screen_capture_status,
            list_desktop_audio_sources,
            list_capture_monitors,
            start_screen_recording,
            stop_screen_recording,
            screen_recording_status,
            replay_buffer_status,
            replay_memory,
            arm_replay_buffer,
            disarm_replay_buffer,
            save_replay,
            open_overlay,
            close_overlay,
            place_overlay,
            set_overlay_mode,
            set_overlay_hide_from_recordings,
            overlay_status,
            overlay_handshake,
            overlay_alive,
            import_overlay_asset,
            delete_overlay_asset,
            scan_soundboard_folder,
            play_soundboard_clip,
            stop_soundboard_clip,
            stop_all_soundboard_clips,
            set_soundboard_master_gain,
            list_tts_voices,
            speak,
            stop_speak,
            preview_voice,
            stop_preview_voice,
            speak_clips_dir,
            open_speak_clips_folder,
            open_external,
            list_speak_clips,
            delete_speak_clip,
            clone_voice,
            start_voice_recording,
            stop_voice_recording,
            list_cloned_voices,
            rename_cloned_voice,
            delete_cloned_voice,
            clone_models_status,
            download_clone_models,
            voxcpm_status,
            download_voxcpm_models,
            voxcpm_gpu_status,
            download_voxcpm_gpu_model,
            model_download_status,
            detect_virtual_mic,
            register_global_shortcut,
            unregister_global_shortcut,
            unregister_all_global_shortcuts,
            list_midi_inputs,
            open_midi_input,
            close_midi_input,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DivoraVoice");
}

#[cfg(test)]
mod tests {
    use super::{
        cloned_voices_dir, default_clone_base, drain_bindings, drop_binding, emit_speak,
        external_target, hides_on_close, hotkey_mute_request, hotkey_ptm_request, is_safe_voice_id,
        level_update, list_cloned_voice_infos, load_cloned_voice, mute_on_hotkey, preview_path_in,
        ptm_mode_command, ptm_on_hotkey, read_preview, rebind, release, rename_cloned_voice_in,
        same_binding, save_tts_clip, scan_voice_dir, slugify_preset_id, speak_commands,
        take_binding, unique_preset_id, write_preview, ClonedMeta, EngineStatus, ExternalTarget,
        LevelUpdate, Levels, OnnxRuntimeStatus, SpeakClip, VoiceInfo, VoiceReadingUpdate,
        MAX_CLONE_NAME_LEN, PREVIEW_CLIP_ID, PREVIEW_TEXT, PREVIEW_TEXT_VERSION,
        READING_FLOOR_DBFS, TTS_CLIP_ID,
    };
    use super::{
        create_part, download_model_files, download_snapshot, download_to_file, model_agent,
        model_part_paths, sweep_model_parts, voice_clone_idle, CloneClaim, CloneDownloadProgress,
        DownloadSlot, ModelDownloadDone, ModelDownloadKind, MODEL_DOWNLOAD_BUSY,
        MODEL_DOWNLOAD_DONE_EVENT, MODEL_DOWNLOAD_EVENT, VOICE_CLONE_BUSY,
    };
    use divora_core::audio::{
        AudioEngine, InputMute, PtmMode, PtmPageSource, PtmSource, PtmState, ReadingSnapshot,
    };
    use divora_core::dsp::{Descriptor, Metrics, ReadingState, VoiceReading, MAX_DESCRIPTORS};
    use divora_core::soundboard::{SoundboardCommand, SoundboardMixer};
    use divora_core::tts::babble;
    use std::collections::HashMap;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};
    use tauri_plugin_global_shortcut::Shortcut;

    /// Every combination, in the order the menu shows it. The first two are
    /// v1.53.0's and v1.54.0's menus exactly: the overlay's item must never
    /// cost Replay its two.
    #[test]
    fn tray_entries_for_each_combination() {
        use super::{tray_entries, TrayEntry, TrayItems};
        use TrayEntry::{Item, Separator};
        let show = Item("show", "Show DivoraVoice");
        let save = Item("replay-save", "Save replay");
        let off = Item("replay-off", "Turn Replay off");
        let lock = Item("overlay-lock", "Lock the overlay");
        let quit = Item("quit", "Quit");
        let items = |replay, lock_overlay| TrayItems {
            replay,
            lock_overlay,
        };
        assert_eq!(tray_entries(items(false, false)), [show, quit]);
        assert_eq!(
            tray_entries(items(true, false)),
            [show, Separator, save, off, Separator, quit]
        );
        assert_eq!(
            tray_entries(items(false, true)),
            [show, Separator, lock, Separator, quit]
        );
        assert_eq!(
            tray_entries(items(true, true)),
            [show, Separator, save, off, Separator, lock, Separator, quit]
        );
        assert_eq!(TrayItems::default(), items(false, false));
    }

    #[test]
    fn only_the_main_window_hides_on_close() {
        // The main window hides to the tray so audio keeps running.
        assert!(hides_on_close("main"));
        // The overlay must really close: a hidden overlay is never destroyed,
        // so the frontend's toggle and the window disagree from then on.
        assert!(!hides_on_close("overlay"));
        // Any future window (per-monitor overlays) closes for real by default.
        assert!(!hides_on_close("overlay-2"));
        assert!(!hides_on_close(""));
    }

    #[test]
    fn save_tts_clip_writes_wav_and_parseable_meta() {
        let dir = std::env::temp_dir().join(format!("divora-clips-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        save_tts_clip(
            &dir,
            &[0.1, -0.1, 0.2, 0.0],
            16_000,
            "hello world",
            "af_heart",
        );

        let paths: Vec<PathBuf> = fs::read_dir(&dir)
            .expect("clips dir")
            .flatten()
            .map(|e| e.path())
            .collect();
        let ext_count = |ext: &str| {
            paths
                .iter()
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(ext))
                .count()
        };
        assert_eq!(ext_count("wav"), 1, "one wav written");
        assert_eq!(ext_count("json"), 1, "one sidecar written");

        let json = paths
            .iter()
            .find(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
            .unwrap();
        let meta: SpeakClip = serde_json::from_str(&fs::read_to_string(json).unwrap()).unwrap();
        assert_eq!(meta.text, "hello world");
        assert_eq!(meta.voice, "af_heart");
        assert!(meta.duration_secs > 0.0);
        let _ = fs::remove_dir_all(&dir);
    }

    fn sorted_keys(v: &serde_json::Value) -> Vec<String> {
        let mut k: Vec<String> = v.as_object().expect("object").keys().cloned().collect();
        k.sort();
        k
    }

    /// v1.21.0: prove `ureq` can fetch a release asset (follows GitHub's
    /// redirect, streams the body, Content-Length matches) — the core of the
    /// on-demand cloning-model download. `#[ignore]`d (network).
    #[test]
    #[ignore = "network: downloads a release asset"]
    fn ureq_fetches_release_asset() {
        use std::io::Read;
        let url = format!("{}/openvoice-extractor.onnx", super::CLONE_MODELS_BASE_URL);
        let resp = ureq::get(&url).call().unwrap();
        let len: usize = resp.header("Content-Length").unwrap().parse().unwrap();
        let mut buf = Vec::new();
        resp.into_reader().read_to_end(&mut buf).unwrap();
        assert_eq!(buf.len(), len);
        assert_eq!(buf.len(), 3_364_792);
    }

    // ---- v1.57.0: model downloads and voice cloning off the main thread ----

    /// A one-request HTTP server on 127.0.0.1 for the download tests. Once
    /// the request is in it sends `response` as it is and closes, or with
    /// `hold` keeps the connection open and silent until `release` sends (10 s
    /// at most). Joining it gives the request line.
    fn http_stub(response: Vec<u8>, hold: bool) -> (String, mpsc::Sender<()>, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (release, released) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0, "the client hung up mid-request");
                request.extend_from_slice(&buf[..n]);
            }
            // The client may have given up already; what it saw is the test.
            let _ = stream.write_all(&response);
            if hold {
                let _ = released.recv_timeout(Duration::from_secs(10));
            }
            let _ = stream.shutdown(Shutdown::Write);
            let line = String::from_utf8_lossy(&request);
            line.lines().next().unwrap_or_default().to_owned()
        });
        (url, release, server)
    }

    fn ok_response(body: &[u8]) -> Vec<u8> {
        let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
        [head.as_bytes(), body].concat()
    }

    /// Neither the file nor its `.part` is on disk.
    fn nothing_at(dest: &Path) -> bool {
        !dest.exists() && !dest.with_extension("part").exists()
    }

    /// v1.57.0: a whole body becomes `dest` with no `.part` left, reported
    /// once per MiB and once more, at the whole length, when it's in place.
    #[test]
    fn a_download_lands_whole_and_reports_its_end() {
        let dir = scratch("dl-whole");
        let dest = dir.join("model.onnx");
        // 2.5 MiB: two throttled reports, both short of the end (a read is at
        // most 64 KiB), so only the end report can carry the whole length.
        let body: Vec<u8> = (0..5_u32 << 19)
            .map(|i| u8::try_from(i % 251).unwrap())
            .collect();
        let (url, _release, server) = http_stub(ok_response(&body), false);
        let mut seen = Vec::new();
        download_to_file(
            &model_agent(Duration::from_secs(10)),
            &format!("{url}/model.onnx"),
            &dest,
            |received, total| seen.push((received, total)),
        )
        .unwrap();
        assert_eq!(server.join().unwrap(), "GET /model.onnx HTTP/1.1");
        assert_eq!(fs::read(&dest).unwrap(), body);
        assert!(!dest.with_extension("part").exists());
        let len = body.len() as u64;
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert!(
            seen[0].0 >= 1 << 20 && seen[1].0 - seen[0].0 >= 1 << 20,
            "{seen:?}"
        );
        assert!(seen[1].0 < len, "{seen:?}");
        assert_eq!(seen[2], (len, len));
        fs::remove_dir_all(&dir).ok();
    }

    /// v1.57.0: a connection that closes before `Content-Length` is an error
    /// and leaves neither the file nor its `.part`.
    #[test]
    fn a_body_cut_short_leaves_nothing_behind() {
        let dir = scratch("dl-short");
        let dest = dir.join("model.onnx");
        let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n".to_vec();
        response.extend_from_slice(&[7; 400]);
        let (url, _release, server) = http_stub(response, false);
        let agent = model_agent(Duration::from_secs(10));
        let err = download_to_file(&agent, &url, &dest, |_, _| {}).unwrap_err();
        server.join().unwrap();
        assert!(nothing_at(&dest), "{err}");
        fs::remove_dir_all(&dir).ok();
    }

    /// v1.57.0: ureq reads a chunked body to its end whatever length the
    /// headers give, so a mismatch there is caught by the length check.
    #[test]
    fn a_body_that_misses_its_length_is_refused() {
        let dir = scratch("dl-chunked");
        let dest = dir.join("model.onnx");
        let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\
            Content-Length: 1000\r\n\r\n190\r\n"
            .to_vec();
        response.extend_from_slice(&[7; 400]);
        response.extend_from_slice(b"\r\n0\r\n\r\n");
        let (url, _release, server) = http_stub(response, false);
        let agent = model_agent(Duration::from_secs(10));
        let err = download_to_file(&agent, &url, &dest, |_, _| {}).unwrap_err();
        server.join().unwrap();
        assert_eq!(err, "expected 1000 bytes, got 400");
        assert!(nothing_at(&dest));
        fs::remove_dir_all(&dir).ok();
    }

    /// v1.57.0: a server that stops sending mid-file fails the download
    /// within the read timeout (ureq's default agent waits forever), and the
    /// `.part` goes.
    #[test]
    fn a_stalled_download_times_out_and_cleans_up() {
        let dir = scratch("dl-stall");
        let dest = dir.join("model.onnx");
        let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n".to_vec();
        response.extend_from_slice(&[7; 100]);
        let (url, release, server) = http_stub(response, true);
        let agent = model_agent(Duration::from_millis(300));
        let started = Instant::now();
        let result = download_to_file(&agent, &url, &dest, |_, _| {});
        let took = started.elapsed();
        let _ = release.send(());
        server.join().unwrap();
        assert!(result.is_err());
        assert!(
            took < Duration::from_secs(5),
            "took {took:?}: no read timeout"
        );
        assert!(nothing_at(&dest));
        fs::remove_dir_all(&dir).ok();
    }

    /// Reads one request's head from `stream`; false if it closes first.
    fn read_request(stream: &mut TcpStream) -> bool {
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => return false,
                Ok(n) => request.extend_from_slice(&buf[..n]),
            }
        }
        true
    }

    /// A keep-alive HTTP server on 127.0.0.1: it answers the first request
    /// with `body` and keeps that connection open, then answers nothing more
    /// until `release` sends (10 s at most). Joining it says whether the
    /// client sent its next request down that same connection.
    fn keep_alive_stub(body: &[u8]) -> (String, mpsc::Sender<()>, JoinHandle<bool>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let response = ok_response(body);
        let (release, released) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            assert!(read_request(&mut stream), "the client hung up mid-request");
            stream.write_all(&response).unwrap();
            // A client that doesn't keep the connection closes it, and its
            // next request waits unanswered in the listener's queue.
            let reused = read_request(&mut stream);
            let _ = released.recv_timeout(Duration::from_secs(10));
            let _ = stream.shutdown(Shutdown::Write);
            reused
        });
        (url, release, server)
    }

    /// v1.57.0: a set's next file can't hang on the connection the last one
    /// came over. ureq 2 waits for a reused connection's headers with no
    /// timeout, so a server that took the request and went quiet would hold
    /// the download, and the slot with it, until the app closed.
    #[test]
    fn a_quiet_server_cannot_hang_the_next_file() {
        let dir = scratch("dl-reuse");
        let agent = model_agent(Duration::from_millis(300));
        let (url, release, server) = keep_alive_stub(b"the first file");
        let first = dir.join("a.onnx");
        download_to_file(&agent, &format!("{url}/a.onnx"), &first, |_, _| {}).unwrap();
        let dest = dir.join("b.onnx");
        let started = Instant::now();
        let result = download_to_file(&agent, &format!("{url}/b.onnx"), &dest, |_, _| {});
        let took = started.elapsed();
        let _ = release.send(());
        let reused = server.join().unwrap();
        assert!(result.is_err());
        assert!(
            took < Duration::from_secs(5),
            "took {took:?}; the request reused the connection: {reused}"
        );
        assert!(nothing_at(&dest));
        assert_eq!(fs::read(&first).unwrap(), b"the first file");
        fs::remove_dir_all(&dir).ok();
    }

    /// v1.57.0: what the startup sweep removes: the `.part` of each file of
    /// every set (a Quit mid-download leaves one), nothing else, and on
    /// Windows not one a second Divora is writing right now.
    #[test]
    fn the_startup_sweep_takes_only_the_downloads_parts() {
        let dir = scratch("dl-sweep");
        let parts: Vec<PathBuf> = [
            "openvoice-extractor.part",
            "openvoice-converter.part",
            "voxcpm-prefill.part",
            "voxcpm-decode-step.part",
            "voxcpm-vae-encoder.part",
            "voxcpm-vae-decoder.part",
            "voxcpm-tokenizer.part",
            "voxcpm-speaker.part",
            "voxcpm-decode-step-gpu.part",
        ]
        .iter()
        .map(|name| dir.join(name))
        .collect();
        assert_eq!(model_part_paths(&dir), parts);
        for part in &parts {
            fs::write(part, b"half a model").unwrap();
        }
        // A finished model, and a `.part` that isn't a model download's.
        let others = ["voxcpm-prefill.onnx", "preview.part"];
        for name in others {
            fs::write(dir.join(name), b"keep").unwrap();
        }
        sweep_model_parts(&dir);
        for part in &parts {
            assert!(!part.exists(), "{}", part.display());
        }
        for name in others {
            assert!(dir.join(name).exists(), "{name}");
        }
        #[cfg(windows)]
        {
            let held = create_part(&parts[1]).unwrap();
            sweep_model_parts(&dir);
            assert!(parts[1].exists(), "removed while another download held it");
            drop(held);
            sweep_model_parts(&dir);
            assert!(!parts[1].exists());
        }
        fs::remove_dir_all(&dir).ok();
        // And the app runs it as it starts, on the folder the downloads use.
        let file = include_str!("lib.rs").replace('\r', "");
        let tests_at = file
            .find(concat!("#[cfg(test)]\nmod ", "tests {"))
            .expect("the tests module");
        let setup = body_of(&file[..tests_at], concat!(".setup(", "|app|"));
        assert!(setup.contains(concat!("sweep_model_parts(", "&clone_models_dir);")));
    }

    /// v1.57.0: an error status is an error, with nothing written.
    #[test]
    fn a_missing_file_writes_nothing() {
        let dir = scratch("dl-404");
        let dest = dir.join("model.onnx");
        let response = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec();
        let (url, _release, server) = http_stub(response, false);
        let agent = model_agent(Duration::from_secs(10));
        let err = download_to_file(&agent, &url, &dest, |_, _| {}).unwrap_err();
        server.join().unwrap();
        assert!(err.contains("404"), "{err}");
        assert!(nothing_at(&dest));
        fs::remove_dir_all(&dir).ok();
    }

    /// v1.57.0: Divora isn't single-instance, so two copies can download one
    /// file at once. The second can't open the first one's `.part`, so it
    /// fails instead of emptying it and writing into it.
    #[cfg(windows)]
    #[test]
    fn a_second_download_cannot_write_into_the_first_ones_part() {
        let dir = scratch("dl-shared");
        let dest = dir.join("model.onnx");
        let mut first = create_part(&dest.with_extension("part")).unwrap();
        first.write_all(b"first").unwrap();
        let (url, _release, server) = http_stub(ok_response(b"second"), false);
        let agent = model_agent(Duration::from_secs(10));
        let err = download_to_file(&agent, &url, &dest, |_, _| {}).unwrap_err();
        server.join().unwrap();
        assert!(!dest.exists(), "{err}");
        drop(first);
        assert_eq!(fs::read(dest.with_extension("part")).unwrap(), b"first");
        fs::remove_dir_all(&dir).ok();
    }

    /// v1.57.0: the file loop skips a file already on disk, numbers the rest
    /// against the whole set, keeps the slot's snapshot current, and names
    /// the file an error came from, as the commands always have.
    #[test]
    fn the_file_loop_skips_whats_there_and_names_what_failed() {
        let dir = scratch("dl-loop");
        fs::write(dir.join("a.onnx"), b"already here").unwrap();
        let files = ["a.onnx", "b.onnx"];
        let agent = model_agent(Duration::from_secs(10));
        let snapshot = Arc::new(Mutex::new(None));

        let body = vec![9; 1000];
        let (url, _release, server) = http_stub(ok_response(&body), false);
        let slot = DownloadSlot::try_claim(&snapshot, ModelDownloadKind::VoxCpm, 2).unwrap();
        let mut seen = Vec::new();
        download_model_files(&agent, &url, &dir, &files, &slot, |p| seen.push(p)).unwrap();
        assert_eq!(server.join().unwrap(), "GET /b.onnx HTTP/1.1");
        assert_eq!(fs::read(dir.join("a.onnx")).unwrap(), b"already here");
        assert_eq!(fs::read(dir.join("b.onnx")).unwrap(), body);
        let end = CloneDownloadProgress {
            kind: ModelDownloadKind::VoxCpm,
            file: 2,
            file_count: 2,
            received: 1000,
            total: 1000,
        };
        assert_eq!(seen, std::slice::from_ref(&end));
        assert_eq!(download_snapshot(&snapshot), Some(end));
        drop(slot);

        fs::remove_file(dir.join("b.onnx")).unwrap();
        let response = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec();
        let (url, _release, server) = http_stub(response, false);
        let slot = DownloadSlot::try_claim(&snapshot, ModelDownloadKind::VoxCpm, 2).unwrap();
        let err = download_model_files(&agent, &url, &dir, &files, &slot, |_| {}).unwrap_err();
        server.join().unwrap();
        assert!(err.starts_with("downloading b.onnx: "), "{err}");
        drop(slot);
        fs::remove_dir_all(&dir).ok();
    }

    /// v1.57.0: one model download at a time. A second claim, of any set, is
    /// refused in words the UI shows as they are; the snapshot follows the
    /// progress; dropping the slot frees it.
    #[test]
    fn one_model_download_at_a_time() {
        use ModelDownloadKind as Kind;
        let snapshot = Arc::new(Mutex::new(None));
        assert_eq!(download_snapshot(&snapshot), None);
        let slot = DownloadSlot::try_claim(&snapshot, Kind::Clone, 2).unwrap();
        let progress = |file, received, total| CloneDownloadProgress {
            kind: Kind::Clone,
            file,
            file_count: 2,
            received,
            total,
        };
        assert_eq!(download_snapshot(&snapshot), Some(progress(1, 0, 0)));
        for kind in [Kind::Clone, Kind::VoxCpm, Kind::VoxCpmGpu] {
            let second = DownloadSlot::try_claim(&snapshot, kind, 1);
            assert_eq!(second.err().as_deref(), Some(MODEL_DOWNLOAD_BUSY));
        }
        assert_eq!(download_snapshot(&snapshot), Some(progress(1, 0, 0)));
        assert_eq!(slot.progress(2, 5, 10), progress(2, 5, 10));
        assert_eq!(download_snapshot(&snapshot), Some(progress(2, 5, 10)));
        drop(slot);
        assert_eq!(download_snapshot(&snapshot), None);
        drop(DownloadSlot::try_claim(&snapshot, Kind::VoxCpmGpu, 1).unwrap());
    }

    /// v1.57.0: a download thread that panics (debug builds; release aborts)
    /// still gives the slot back, as it unwinds.
    #[test]
    fn a_panicking_download_gives_the_slot_back() {
        let snapshot = Arc::new(Mutex::new(None));
        let unwound = catch_unwind(AssertUnwindSafe(|| {
            let _slot = DownloadSlot::try_claim(&snapshot, ModelDownloadKind::VoxCpm, 6).unwrap();
            panic!("a download thread panicked");
        }));
        assert!(unwound.is_err());
        assert_eq!(download_snapshot(&snapshot), None);
        drop(DownloadSlot::try_claim(&snapshot, ModelDownloadKind::VoxCpm, 6).unwrap());
    }

    /// v1.57.0: one voice is made at a time, a recording can't start while
    /// one is, and the claim is given back on drop, an unwinding panic
    /// included.
    #[test]
    fn one_voice_at_a_time_and_no_recording_meanwhile() {
        let busy = Arc::new(AtomicBool::new(false));
        assert_eq!(voice_clone_idle(&busy), Ok(()));
        let claim = CloneClaim::try_claim(&busy).unwrap();
        let refused = Err(VOICE_CLONE_BUSY.to_owned());
        assert_eq!(CloneClaim::try_claim(&busy).map(|_| ()), refused);
        assert_eq!(voice_clone_idle(&busy), refused);
        drop(claim);
        assert_eq!(voice_clone_idle(&busy), Ok(()));
        let unwound = catch_unwind(AssertUnwindSafe(|| {
            let _claim = CloneClaim::try_claim(&busy).unwrap();
            panic!("a cloning thread panicked");
        }));
        assert!(unwound.is_err());
        assert_eq!(voice_clone_idle(&busy), Ok(()));
        drop(CloneClaim::try_claim(&busy).unwrap());
    }

    /// v1.57.0: the names and payloads the frontend listens for and reads.
    #[test]
    fn the_download_wire_is_what_the_frontend_reads() {
        use ModelDownloadKind as Kind;
        assert_eq!(MODEL_DOWNLOAD_EVENT, "clone-model-download");
        assert_eq!(MODEL_DOWNLOAD_DONE_EVENT, "clone-model-download-done");
        for (kind, wire) in [
            (Kind::Clone, "clone"),
            (Kind::VoxCpm, "voxcpm"),
            (Kind::VoxCpmGpu, "voxcpmGpu"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), wire);
        }
        let progress = CloneDownloadProgress {
            kind: Kind::VoxCpmGpu,
            file: 1,
            file_count: 1,
            received: 2,
            total: 3,
        };
        assert_eq!(
            serde_json::to_value(&progress).unwrap(),
            serde_json::json!({
                "kind": "voxcpmGpu", "file": 1, "fileCount": 1, "received": 2, "total": 3
            })
        );
        let done = |kind, error: Option<&str>| ModelDownloadDone {
            kind,
            error: error.map(str::to_owned),
        };
        assert_eq!(
            serde_json::to_value(done(Kind::Clone, None)).unwrap(),
            serde_json::json!({ "kind": "clone", "error": null })
        );
        assert_eq!(
            serde_json::to_value(done(Kind::VoxCpm, Some("downloading a.onnx: boom"))).unwrap(),
            serde_json::json!({ "kind": "voxcpm", "error": "downloading a.onnx: boom" })
        );
    }

    /// The `{…}` body of the first item in `prod` that starts with `head`.
    fn body_of<'a>(prod: &'a str, head: &str) -> &'a str {
        let at = prod.find(head).unwrap_or_else(|| panic!("{head:?}"));
        let open = at + prod[at..].find('{').expect("a body");
        let mut depth = 0_usize;
        let close = prod[open..]
            .char_indices()
            .find_map(|(i, c)| {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                (depth == 0).then_some(open + i)
            })
            .expect("the body closes");
        &prod[open..=close]
    }

    /// v1.57.0: the commands need an `AppHandle`, so this reads the source.
    /// The five slow ones are `async fn`s whose work runs under
    /// `spawn_blocking` (the downloads through `run_model_download`), none is
    /// a sync `fn` again, a recording checks for a voice being made, and
    /// `model_download_status` is a registered, sync command. And what the
    /// pure parts' tests can't see: the two voice-making commands take their
    /// claim before handing off and give it back on the thread, a download
    /// uses the agent with the read timeout, and frees its slot before the
    /// done event goes out (a page may start the next download on it).
    #[test]
    fn the_slow_commands_run_off_the_main_thread() {
        let file = include_str!("lib.rs").replace('\r', "");
        let tests_at = file
            .find(concat!("#[cfg(test)]\nmod ", "tests {"))
            .expect("the tests module");
        let prod = &file[..tests_at];
        let command = |name: &str| format!("#[tauri::command]\nasync fn {name}(");
        let blocking = "tauri::async_runtime::spawn_blocking(move ||";
        let at = |body: &str, needle: &str| body.find(needle).unwrap_or_else(|| panic!("{needle}"));
        for name in ["clone_voice", "stop_voice_recording"] {
            let body = body_of(prod, &command(name));
            let claim = at(
                body,
                concat!(
                    "let claim = CloneClaim::",
                    "try_claim(&state.voice_clone_busy)?;"
                ),
            );
            let handed_off = at(body, blocking);
            let given_back = at(body, concat!("drop(", "claim);"));
            assert!(claim < handed_off && handed_off < given_back, "{name}");
        }
        for name in [
            "download_clone_models",
            "download_voxcpm_models",
            "download_voxcpm_gpu_model",
        ] {
            let body = body_of(prod, &command(name));
            assert!(body.contains("run_model_download("), "{name}");
        }
        let download = body_of(prod, "async fn run_model_download(");
        assert!(download.contains(concat!("model_agent(", "MODEL_READ_TIMEOUT)")));
        let handed_off = at(download, blocking);
        let freed = at(download, concat!("drop(", "slot);"));
        assert!(handed_off < freed);
        // The body's first use of the done event's name is its emit.
        assert!(freed < at(download, concat!("MODEL_DOWNLOAD_", "DONE_EVENT")));
        for name in [
            "clone_voice",
            "stop_voice_recording",
            "download_clone_models",
            "download_voxcpm_models",
            "download_voxcpm_gpu_model",
        ] {
            assert!(
                !prod.contains(&format!("\nfn {name}(")),
                "{name} is sync again"
            );
        }
        let start = body_of(prod, "fn start_voice_recording(");
        assert!(start.contains("voice_clone_idle(&state.voice_clone_busy)?;"));
        assert!(prod.contains("#[tauri::command]\nfn model_download_status("));
        let handler = &prod[prod.find("tauri::generate_handler![").expect("the handler")..];
        let handler = &handler[..handler.find(']').expect("its end")];
        assert!(handler.contains("\n            model_download_status,\n"));
        // Every emit goes through the two names the frontend listens for.
        assert_eq!(prod.matches("\"clone-model-download").count(), 2);
    }

    /// v1.57.0: a recording becomes the voice its start chose. A recording
    /// can now run while the `VoxCPM` download does, so checking at stop made
    /// a freely spoken take a `VoxCPM` voice, with the read-aloud sentence as
    /// its transcript, whenever the download finished during it. The commands
    /// need a running engine, so this reads the source: the start checks and
    /// keeps the answer with the path, and the stop goes by that answer.
    #[test]
    fn a_recording_keeps_the_engine_it_started_with() {
        let file = include_str!("lib.rs").replace('\r', "");
        let tests_at = file
            .find(concat!("#[cfg(test)]\nmod ", "tests {"))
            .expect("the tests module");
        let prod = &file[..tests_at];
        let start = body_of(prod, concat!("fn start_voice_", "recording("));
        assert!(start.contains(concat!(
            "let voxcpm = divora_core::tts::voxcpm::",
            "models_present(&voxcpm_dir(&state));"
        )));
        assert!(start.contains(concat!("Some((path.clone(), ", "voxcpm));")));
        // The page's hint is the start's answer: the sentence exactly when
        // the take becomes a `VoxCPM` voice.
        assert!(start.contains(concat!(
            "Ok(voxcpm.then(|| divora_core::tts::voxcpm::",
            "READ_ALOUD_PROMPT.to_string()))"
        )));
        let stop = body_of(prod, concat!("fn clone_from_", "recording("));
        assert!(stop.contains(concat!("let (path, ", "voxcpm) = state")));
        assert!(stop.contains(concat!("let result = if ", "voxcpm {")));
        for again in ["models_present", "voxcpm_dir(", "present()"] {
            assert!(!stop.contains(again), "checked again at stop: {again}");
        }
    }

    // ---- v1.14.0: preset-import id helpers -----------------------------

    #[test]
    fn slugify_preset_id_makes_safe_ids() {
        assert_eq!(slugify_preset_id("Hollow King"), "hollow-king");
        assert_eq!(slugify_preset_id("My Custom #1!"), "my-custom-1");
        assert_eq!(slugify_preset_id("already-safe_id"), "already-safe_id");
        assert_eq!(slugify_preset_id("  spaced  out  "), "spaced-out");
        assert_eq!(slugify_preset_id("!!!"), "");
    }

    #[test]
    fn unique_preset_id_dedupes_against_taken() {
        let mut taken = std::collections::HashSet::new();
        taken.insert("velvet-demon".to_string());
        taken.insert("velvet-demon-2".to_string());
        // A fresh id passes through unchanged.
        assert_eq!(
            unique_preset_id("static-wraith", "Static Wraith", &taken),
            "static-wraith"
        );
        // Collisions append the next free numeric suffix.
        assert_eq!(
            unique_preset_id("velvet-demon", "Velvet Demon", &taken),
            "velvet-demon-3"
        );
        // An empty / unsafe id falls back to the slugified name.
        assert_eq!(unique_preset_id("", "Brand New", &taken), "brand-new");
        // Empty id + empty name → the constant fallback.
        assert_eq!(unique_preset_id("", "", &taken), "imported-preset");
    }

    // ---- v1.0 freeze: IPC payload shapes -------------------------------
    // These structs cross the Tauri bridge (the `audio-levels` event +
    // command returns) and the frontend reads them by camelCase key, so
    // the shapes are a back-compat contract. Renames/removals break the
    // UI silently; additions after v1.0 must be optional. See
    // `docs/STABLE-SURFACE.md`.

    #[test]
    fn engine_status_json_keys_are_frozen() {
        let s = EngineStatus {
            running: true,
            monitoring: false,
            input: Levels {
                rms: 0.0,
                peak: 0.0,
            },
            output: Levels {
                rms: 0.0,
                peak: 0.0,
            },
            dsp_latency_ms: 0.0,
            recording: false,
            loudness_gain_db: 0.0,
        };
        assert_eq!(
            sorted_keys(&serde_json::to_value(&s).unwrap()),
            [
                "dspLatencyMs",
                "input",
                "loudnessGainDb",
                "monitoring",
                "output",
                "recording",
                "running"
            ]
        );
    }

    #[test]
    fn level_update_json_keys_are_frozen() {
        let u = LevelUpdate {
            input: Levels {
                rms: 0.0,
                peak: 0.0,
            },
            output: Levels {
                rms: 0.0,
                peak: 0.0,
            },
            running: false,
            monitoring: true,
            dsp_latency_ms: 0.0,
            recording: false,
            loudness_gain_db: 0.0,
            device_lost: false,
            mod_env: 0.0,
            input_muted: false,
            input_mute_seq: 0,
            ptm: PtmState::default(),
        };
        assert_eq!(
            sorted_keys(&serde_json::to_value(&u).unwrap()),
            [
                "deviceLost",
                "dspLatencyMs",
                "input",
                // v1.56.0: the mute and its change count.
                "inputMuteSeq",
                "inputMuted",
                "loudnessGainDb",
                "modEnv",
                "monitoring",
                "output",
                // v1.58.0: push-to-modulate (a `PtmState`).
                "ptm",
                "recording",
                "running"
            ]
        );
    }

    /// v1.58.0: `set_ptm_held`'s `source`, as the page sends it. Only the
    /// three page sources parse: `"hotkey"` is refused, so only the backend
    /// ever holds that bit, and the names are exact.
    #[test]
    fn ptm_page_source_wire_values_are_frozen() {
        let parse = |wire: &str| serde_json::from_value::<PtmPageSource>(serde_json::json!(wire));
        for (wire, source) in [
            ("key", PtmPageSource::Key),
            ("button", PtmPageSource::Button),
            ("midi", PtmPageSource::Midi),
        ] {
            assert_eq!(parse(wire).ok(), Some(source), "{wire:?}");
        }
        for refused in ["hotkey", "Key", "BUTTON", "Midi", "always", ""] {
            assert!(parse(refused).is_err(), "{refused:?} parsed");
        }
    }

    #[test]
    fn every_word_on_the_wire_comes_from_the_vocabulary() {
        // Provenance, swept rather than spot-checked. `words` is a plain
        // Vec<&str>, so nothing in the type stops this bridge appending a
        // string of its own — review proved it by pushing "sounds anxious"
        // here behind an `if !calibrated`, which the one existing test could
        // not reach because it builds a single calibrated snapshot.
        let vocabulary: std::collections::HashSet<&str> =
            Descriptor::ALL.iter().map(|d| d.word()).collect();
        let states = [
            ReadingState::Stopped,
            ReadingState::Muted,
            ReadingState::Quiet,
            ReadingState::Speaking,
        ];
        for state in states {
            for calibrated in [false, true] {
                for stale in [false, true] {
                    for descriptors in [
                        Vec::new(),
                        vec![Descriptor::Steady],
                        Descriptor::ALL.to_vec(),
                    ] {
                        let snap = ReadingSnapshot {
                            reading: VoiceReading {
                                state,
                                dry: Metrics::silent(),
                                wet: Metrics::silent(),
                                descriptors,
                                calibrated,
                                held: stale,
                                held_descriptors: Vec::new(),
                                wet_bypassed: false,
                                wet_settled: true,
                            },
                            stale,
                            age_ms: 0,
                            muted_in_divora: false,
                        };
                        let u = VoiceReadingUpdate::from_snapshot(&snap);
                        assert!(
                            u.words.len() <= MAX_DESCRIPTORS,
                            "{state:?}/{calibrated}/{stale}: {} words",
                            u.words.len()
                        );
                        for w in &u.words {
                            assert!(
                                vocabulary.contains(w),
                                "{state:?}/{calibrated}/{stale}: {w:?} is not a vocabulary word"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn voice_reading_update_json_keys_are_frozen() {
        let u = VoiceReadingUpdate::from_snapshot(&ReadingSnapshot::idle(ReadingState::Stopped));
        assert_eq!(
            sorted_keys(&serde_json::to_value(&u).unwrap()),
            [
                "ageMs",
                "calibrated",
                "dry",
                // v1.56.0: "muted" because of Divora's own mute.
                "mutedInDivora",
                "stale",
                "state",
                "wet",
                // Additive: the engine measures passthrough by correlating
                // the taps, and the panel must not re-derive it from these
                // summaries — that comparison is wrong with loudness on.
                "wetBypassed",
                "words"
            ]
        );
        let v = serde_json::to_value(&u).unwrap();
        assert_eq!(v["state"], "stopped");
        assert_eq!(
            sorted_keys(&v["dry"]),
            [
                "brightnessHz",
                "energyDbfs",
                "energyRangeDb",
                "f0Hz",
                "f0RangeSt",
                "paceOps",
                "voicedRatio"
            ]
        );
    }

    /// v1.56.0: the tick is built by the emitter's own builder, and carries
    /// the engine's mute with its change count, engine running or not.
    #[test]
    fn the_level_tick_carries_the_mute() {
        let engine = AudioEngine::new();
        let tick = level_update(&engine);
        assert!(!tick.input_muted && tick.input_mute_seq == 0);
        engine.set_input_muted(true);
        let tick = level_update(&engine);
        assert!(tick.input_muted && tick.input_mute_seq == 1);
        engine.toggle_input_muted();
        let tick = level_update(&engine);
        assert!(!tick.input_muted && tick.input_mute_seq == 2);
        // Under the names the frontend reads.
        let v = serde_json::to_value(&tick).unwrap();
        assert_eq!(v["inputMuted"], false);
        assert_eq!(v["inputMuteSeq"], 2);
    }

    /// v1.58.0: the tick carries push-to-modulate from the engine's word,
    /// engine running or not, under the names and mode strings the frontend
    /// reads.
    #[test]
    fn the_level_tick_carries_ptm() {
        let engine = AudioEngine::new();
        let st = |mode, held, bypassed, seq| PtmState {
            mode,
            held,
            bypassed,
            seq,
        };
        let tick = level_update(&engine);
        assert_eq!(tick.ptm, st(PtmMode::Always, false, false, 0));
        engine.set_ptm_mode(PtmMode::Apply);
        let tick = level_update(&engine);
        assert_eq!(tick.ptm, st(PtmMode::Apply, false, true, 1));
        assert_eq!(
            serde_json::to_value(&tick).unwrap()["ptm"],
            serde_json::json!({ "mode": "apply", "held": false, "bypassed": true, "seq": 1 })
        );
        ptm_on_hotkey(&engine, "ptm", true);
        let tick = level_update(&engine);
        assert_eq!(tick.ptm, st(PtmMode::Apply, true, false, 2));
        // The card's set lets go of the global key too.
        engine.set_ptm_mode(PtmMode::Bypass);
        let tick = level_update(&engine);
        assert_eq!(tick.ptm, st(PtmMode::Bypass, false, false, 3));
        assert_eq!(
            serde_json::to_value(&tick).unwrap()["ptm"]["mode"],
            "bypass"
        );
        engine.set_ptm_mode(PtmMode::Always);
        let v = serde_json::to_value(level_update(&engine)).unwrap();
        assert_eq!(
            v["ptm"],
            serde_json::json!({ "mode": "always", "held": false, "bypassed": false, "seq": 4 })
        );
    }

    /// v1.56.0: "Muted in Divora" crosses the bridge on the same snapshot as
    /// the state it explains, both ways.
    #[test]
    fn the_reading_update_carries_muted_in_divora() {
        for muted_in_divora in [false, true] {
            let mut snap = ReadingSnapshot::idle(ReadingState::Muted);
            snap.muted_in_divora = muted_in_divora;
            let u = VoiceReadingUpdate::from_snapshot(&snap);
            assert_eq!(u.muted_in_divora, muted_in_divora);
            let v = serde_json::to_value(&u).unwrap();
            assert_eq!(v["mutedInDivora"], muted_in_divora);
            assert_eq!(v["state"], "muted");
        }
    }

    /// v1.56.0: only a press of `mute` flips the mute. Its release, other
    /// ids and the reserved `mute:` prefix do nothing.
    #[test]
    fn hotkey_mute_request_table() {
        assert!(hotkey_mute_request("mute", true));
        assert!(!hotkey_mute_request("mute", false));
        for id in ["Mute", "mute:hold", "monitor", "sb:mute", ""] {
            assert!(!hotkey_mute_request(id, true), "{id:?} pressed");
            assert!(!hotkey_mute_request(id, false), "{id:?} released");
        }
    }

    /// v1.58.0: `ptm` holds on its press and lets go on its release. Other
    /// ids, a different case and the reserved `ptm:` prefix ask nothing.
    #[test]
    fn hotkey_ptm_request_table() {
        assert_eq!(hotkey_ptm_request("ptm", true), Some(true));
        assert_eq!(hotkey_ptm_request("ptm", false), Some(false), "its release");
        for id in ["PTM", "ptm:toggle", "mute", "sb:ptm", ""] {
            assert_eq!(hotkey_ptm_request(id, true), None, "{id:?} pressed");
            assert_eq!(hotkey_ptm_request(id, false), None, "{id:?} released");
        }
    }

    /// v1.57.0: the page may open web links and folders that exist, and
    /// nothing Explorer would run.
    #[test]
    fn open_external_takes_only_links_and_folders() {
        for link in [
            "https://github.com/NickSanft/DivoraVoiceMod/blob/main/CHANGELOG.md",
            "https://vb-audio.com/Cable/",
            "HTTP://example.com",
        ] {
            assert_eq!(external_target(link), Ok(ExternalTarget::Link), "{link}");
        }
        let dir = std::env::temp_dir();
        let folder = dir.to_string_lossy().into_owned();
        assert_eq!(external_target(&folder), Ok(ExternalTarget::Folder));
        let file = dir.join("divora-open-external-test.txt");
        fs::write(&file, b"x").unwrap();
        let exe = std::env::current_exe().unwrap();
        for refused in [
            String::new(),
            "https://".to_string(),
            "https:///etc".to_string(),
            "https://example.com/a b".to_string(),
            "https://example.com/\n".to_string(),
            "file:///C:/Windows/System32/calc.exe".to_string(),
            "ms-settings:privacy".to_string(),
            "javascript:alert(1)".to_string(),
            "calc.exe".to_string(),
            "relative\\folder".to_string(),
            file.to_string_lossy().into_owned(),
            exe.to_string_lossy().into_owned(),
            dir.join("no-such-folder-divora")
                .to_string_lossy()
                .into_owned(),
        ] {
            assert!(
                external_target(&refused).is_err(),
                "{refused:?} was let through"
            );
        }
        let _ = fs::remove_file(&file);
    }

    /// v1.57.0: `open_external` and the clips folder go through
    /// `open_outside_tree` off the main thread; they need an `AppHandle` and a
    /// desktop, so this reads the source (`divora-capture`'s
    /// `tests/launch_outside_tree.rs` measures what it changes).
    #[test]
    fn links_and_folders_open_from_the_desktop() {
        let file = include_str!("lib.rs").replace('\r', "");
        let tests_at = file
            .find(concat!("#[cfg(test)]\nmod ", "tests {"))
            .expect("the tests module");
        let prod = &file[..tests_at];
        for (needle, n) in [
            (concat!("async fn open_", "external(target: String)"), 1),
            (concat!("async fn open_speak_", "clips_folder("), 1),
            (concat!("let kind = external_", "target(&target)?;"), 1),
            (
                concat!(
                    "ExternalTarget::Folder => open_",
                    "folder(Path::new(&target)),"
                ),
                1,
            ),
            (concat!("divora_capture::open_", "outside_tree("), 2),
            (concat!("spawn_blocking(move || open_", "folder(&dir))"), 1),
            (concat!("            open_", "external,\n"), 1),
        ] {
            assert_eq!(prod.matches(needle).count(), n, "{needle}");
        }
    }

    /// v1.56.0: the handler's mute step, against a real engine: a press of
    /// `mute` flips the mute and counts, its release and other ids don't.
    #[test]
    fn the_mute_hotkey_flips_the_engine_on_press_only() {
        let engine = AudioEngine::new();
        let st = |muted, seq| InputMute { muted, seq };
        mute_on_hotkey(&engine, "mute", true);
        assert_eq!(engine.input_mute(), st(true, 1));
        mute_on_hotkey(&engine, "mute", false);
        assert_eq!(engine.input_mute(), st(true, 1), "a release flipped it");
        for id in ["Mute", "mute:hold", "monitor", "sb:mute", ""] {
            mute_on_hotkey(&engine, id, true);
            mute_on_hotkey(&engine, id, false);
        }
        assert_eq!(engine.input_mute(), st(true, 1), "another id flipped it");
        mute_on_hotkey(&engine, "mute", true);
        assert_eq!(engine.input_mute(), st(false, 2));
    }

    /// v1.56.0: a rebind takes the old key out first, so a key the parser
    /// refuses leaves the action unbound instead of still on its old key.
    #[test]
    fn a_refused_key_never_leaves_the_old_one_bound() {
        let f9: Shortcut = "F9".parse().unwrap();
        let mut map = HashMap::from([("mute".to_owned(), f9)]);
        for refused in ["Ctrl+Shift+!", "Ctrl++", "Ctrl+*", "Ctrl+É", "ZZZ", ""] {
            map.insert("mute".to_owned(), f9);
            let (old, new) = rebind(&mut map, "mute", refused);
            assert_eq!(old, Some(f9), "{refused:?}: the old key to unregister");
            assert!(new.is_err(), "{refused:?} parsed");
            assert!(!map.contains_key("mute"), "{refused:?} left mute bound");
        }
        let (old, new) = rebind(&mut map, "mute", "Ctrl+Shift+1");
        assert_eq!(old, None);
        let digit = new.expect("Ctrl+Shift+1 parses");
        assert_eq!(map.get("mute"), Some(&digit));
        let (old, new) = rebind(&mut map, "mute", "F9");
        assert_eq!((old, new), (Some(digit), Ok(f9)));
        assert_eq!(map.get("mute"), Some(&f9));
    }

    /// v1.56.0: two actions on one key share Windows' one registration, so
    /// changing or clearing one leaves the key registered for the other; the
    /// key is unregistered once the last action lets it go.
    #[test]
    fn a_shared_key_stays_registered_for_the_other_action() {
        let f9: Shortcut = "F9".parse().unwrap();
        let mut map = HashMap::from([("mute".to_owned(), f9), ("panic".to_owned(), f9)]);
        let (old, new) = rebind(&mut map, "mute", "F10");
        assert_eq!(old, None, "moving mute off F9 unregistered panic's key");
        assert!(new.is_ok());
        assert_eq!(map.get("panic"), Some(&f9));
        map.insert("mute".to_owned(), f9);
        assert_eq!(release(&mut map, "mute"), None, "clearing mute did");
        assert_eq!(map.get("panic"), Some(&f9));
        assert_eq!(release(&mut map, "panic"), Some(f9), "the last holder");
        assert!(map.is_empty());
        assert_eq!(release(&mut map, "panic"), None, "an unbound action");
    }

    /// v1.56.0: the shortcut commands need an `AppHandle`, so this reads the
    /// source: they take an action's key out only through `rebind` and
    /// `release` (since v1.58.0 by way of `take_binding` and `drop_binding`,
    /// which also let go of a held push-to-modulate key), or every key at
    /// once through `drain_bindings` (which lets go too), and nothing else
    /// takes one out of the map.
    #[test]
    fn the_shortcut_commands_let_keys_go_through_release() {
        let file = include_str!("lib.rs").replace('\r', "");
        let tests_at = file
            .find(concat!("#[cfg(test)]\nmod ", "tests {"))
            .expect("the tests module");
        let prod = &file[..tests_at];
        for (call, n) in [
            (
                "take_binding(&mut map, &state.engine, &id, &accelerator)",
                1,
            ),
            ("drop_binding(&mut map, &state.engine, &id)", 1),
            ("drain_bindings(&mut map, &state.engine)", 1),
            ("rebind(map, id, accelerator)", 1),
            // `rebind`'s own, and `drop_binding`'s.
            ("release(map, id)", 2),
            ("map.remove(", 1),
        ] {
            assert_eq!(prod.matches(call).count(), n, "{call}");
        }
    }

    /// v1.56.0: the shortcut handler needs an `AppHandle`, so no test can run
    /// it; this reads the source instead, like engine.rs's callback needles.
    /// The handler passes the event's real press state to the mute step, and
    /// doesn't touch the mute any other way. Needles are split by `concat!`
    /// so they can't match themselves.
    #[test]
    fn the_shortcut_handler_runs_the_mute_hotkey() {
        let file = include_str!("lib.rs").replace('\r', "");
        let tests_at = file
            .find(concat!("#[cfg(test)]\nmod ", "tests {"))
            .expect("the tests module");
        let prod = &file[..tests_at];
        let head = prod
            .find(concat!("fn global_shortcut", "_handler("))
            .expect("the handler");
        let open = head + prod[head..].find('{').expect("its body");
        let mut depth = 0_usize;
        let close = prod[open..]
            .char_indices()
            .find_map(|(i, c)| {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                (depth == 0).then_some(open + i)
            })
            .expect("its body closes");
        let handler = &prod[open..=close];
        let call = concat!(
            "mute_on_hotkey(&state.engine, &id, ",
            "event.state == ShortcutState::Pressed);"
        );
        assert_eq!(
            handler.matches(call).count(),
            1,
            "the handler must run the mute step once, with the press state"
        );
        assert!(
            !handler.contains(concat!("_input", "_muted(")),
            "the handler must not touch the mute any other way"
        );
    }

    // ---- v1.58.0: push-to-modulate's global key and commands ----------

    /// v1.58.0: the handler's push-to-modulate step, against a real engine.
    /// In Hold to apply a press of `ptm` applies the effects and its release
    /// takes them off again; other ids, pressed or released, change nothing.
    #[test]
    fn the_ptm_hotkey_holds_and_lets_go() {
        let engine = AudioEngine::new();
        let others = ["PTM", "ptm:toggle", "mute", "sb:ptm", ""];
        let apply = |held, bypassed, seq| PtmState {
            mode: PtmMode::Apply,
            held,
            bypassed,
            seq,
        };
        engine.set_ptm_mode(PtmMode::Apply);
        assert_eq!(engine.ptm(), apply(false, true, 1));
        ptm_on_hotkey(&engine, "ptm", true);
        assert_eq!(engine.ptm(), apply(true, false, 2), "the press applies");
        ptm_on_hotkey(&engine, "ptm", true);
        assert_eq!(engine.ptm(), apply(true, false, 2), "a second press moved");
        for id in others {
            ptm_on_hotkey(&engine, id, false);
        }
        assert_eq!(engine.ptm(), apply(true, false, 2), "another id let go");
        ptm_on_hotkey(&engine, "ptm", false);
        assert_eq!(engine.ptm(), apply(false, true, 3), "the release lets go");
        for id in others {
            ptm_on_hotkey(&engine, id, true);
        }
        assert_eq!(engine.ptm(), apply(false, true, 3), "another id held");
        // In Hold to bypass the same key takes the effects off while held.
        engine.set_ptm_mode(PtmMode::Bypass);
        ptm_on_hotkey(&engine, "ptm", true);
        assert!(engine.ptm().held && engine.ptm().bypassed);
        ptm_on_hotkey(&engine, "ptm", false);
        assert!(!engine.ptm().held && !engine.ptm().bypassed);
    }

    /// v1.58.0: `set_ptm_mode`'s real body. The card's set (`restore` absent
    /// or `false`) lets go of every hold, the global key's included; the
    /// boot's restore lets go of the page's holds and keeps a held global
    /// key, so a reload doesn't drop it (U4). Each answers the state after.
    #[test]
    fn set_ptm_mode_restores_only_when_asked() {
        let held_by_both = || {
            let engine = AudioEngine::new();
            ptm_on_hotkey(&engine, "ptm", true);
            engine.set_ptm_held(PtmSource::Key, true);
            engine
        };
        for restore in [None, Some(false)] {
            let engine = held_by_both();
            let answer = ptm_mode_command(&engine, PtmMode::Apply, restore);
            assert_eq!(answer, engine.ptm(), "{restore:?}: not the state after");
            assert!(!answer.held, "{restore:?}: a hold survived the card's set");
            assert_eq!((answer.mode, answer.seq), (PtmMode::Apply, 3));
        }
        let engine = held_by_both();
        let answer = ptm_mode_command(&engine, PtmMode::Apply, Some(true));
        assert_eq!(answer, engine.ptm(), "not the state after");
        assert_eq!(
            (answer.mode, answer.held, answer.bypassed, answer.seq),
            (PtmMode::Apply, true, false, 3),
            "the restore let go of the global key"
        );
        // It kept only the global key's: letting that go leaves none held.
        ptm_on_hotkey(&engine, "ptm", false);
        assert!(!engine.ptm().held, "the restore kept the page's hold");
        // Nothing left to change: the same restore moves nothing.
        let seq = engine.ptm().seq;
        assert_eq!(
            ptm_mode_command(&engine, PtmMode::Apply, Some(true)).seq,
            seq
        );
    }

    /// v1.58.0: §7.4's second table. `same_binding` names the key only when
    /// `id` already holds exactly it, however the accelerator spells it;
    /// then a live registration is left alone, and a held push-to-modulate
    /// key stays held through a page's re-sync (U4).
    #[test]
    fn same_binding_table() {
        let f13: Shortcut = "F13".parse().unwrap();
        let ptm_on = |key: Shortcut| HashMap::from([("ptm".to_owned(), key)]);
        let shared = HashMap::from([("ptm".to_owned(), f13), ("mute".to_owned(), f13)]);
        let mute_on = HashMap::from([("mute".to_owned(), f13)]);
        assert_eq!(same_binding(&ptm_on(f13), "ptm", "F13"), Some(f13));
        assert_eq!(same_binding(&ptm_on(f13), "ptm", "F14"), None);
        assert_eq!(same_binding(&ptm_on(f13), "ptm", "Shift+!"), None);
        assert_eq!(same_binding(&shared, "ptm", "F14"), None);
        assert_eq!(same_binding(&shared, "ptm", "F13"), Some(f13), "shared");
        assert_eq!(same_binding(&HashMap::new(), "ptm", "F13"), None);
        assert_eq!(same_binding(&mute_on, "ptm", "F13"), None, "mute's key");
        let chord: Shortcut = "Ctrl+Shift+M".parse().unwrap();
        assert_eq!(
            same_binding(&ptm_on(chord), "ptm", "shift+ctrl+m"),
            Some(chord),
            "the same key, spelled another way"
        );
    }

    /// v1.58.0: once `ptm` leaves its key that key's release can't reach the
    /// handler, so `take_binding` lets go of the global key's hold at the map
    /// update whatever comes back: a different key, a key the parser refuses
    /// (the old one still comes out to unregister), a key shared with `mute`
    /// (nothing to unregister), or no key before. Another action's rebind
    /// leaves the hold alone.
    #[test]
    fn take_binding_lets_go_of_ptm() {
        let f13: Shortcut = "F13".parse().unwrap();
        let f14: Shortcut = "F14".parse().unwrap();
        let ptm_on = HashMap::from([("ptm".to_owned(), f13)]);
        let shared = HashMap::from([("ptm".to_owned(), f13), ("mute".to_owned(), f13)]);
        let take = |mut map: HashMap<String, Shortcut>, id: &str, accelerator: &str| {
            let engine = AudioEngine::new();
            ptm_on_hotkey(&engine, "ptm", true);
            assert!(engine.ptm().held && engine.ptm().seq == 1);
            let (old, new) = take_binding(&mut map, &engine, id, accelerator);
            (old, new.ok(), map, engine.ptm())
        };
        let (old, new, map, after) = take(ptm_on.clone(), "ptm", "F14");
        assert_eq!((old, new), (Some(f13), Some(f14)));
        assert_eq!(map.get("ptm"), Some(&f14));
        assert!(!after.held, "a different key kept the hold");
        let (old, new, map, after) = take(ptm_on, "ptm", "Shift+!");
        assert_eq!((old, new), (Some(f13), None));
        assert!(!map.contains_key("ptm"));
        assert!(!after.held, "an unparsable key kept the hold");
        let (old, new, map, after) = take(shared.clone(), "ptm", "F14");
        assert_eq!((old, new), (None, Some(f14)));
        assert_eq!(map.get("mute"), Some(&f13));
        assert!(!after.held, "a key shared with mute kept the hold");
        let (old, new, _, after) = take(HashMap::new(), "ptm", "F13");
        assert_eq!((old, new), (None, Some(f13)));
        assert!(!after.held, "binding a first key kept the hold");
        let (old, new, map, after) = take(shared, "mute", "F14");
        assert_eq!((old, new), (None, Some(f14)));
        assert_eq!(map.get("ptm"), Some(&f13));
        assert!(after.held && after.seq == 1, "mute's rebind let go of ptm");
    }

    /// v1.58.0: clearing `ptm`'s key lets go of the global key's hold, a
    /// shared key or not, and so does clearing every key; clearing another
    /// action's key leaves it.
    #[test]
    fn drop_and_drain_let_go_of_ptm() {
        let f13: Shortcut = "F13".parse().unwrap();
        let f14: Shortcut = "F14".parse().unwrap();
        let held = || {
            let engine = AudioEngine::new();
            ptm_on_hotkey(&engine, "ptm", true);
            engine
        };
        for (name, mut map, unregister) in [
            ("alone", HashMap::from([("ptm".to_owned(), f13)]), Some(f13)),
            (
                "shared with mute",
                HashMap::from([("ptm".to_owned(), f13), ("mute".to_owned(), f13)]),
                None,
            ),
            ("unbound", HashMap::new(), None),
        ] {
            let engine = held();
            assert_eq!(drop_binding(&mut map, &engine, "ptm"), unregister, "{name}");
            assert!(!map.contains_key("ptm"), "{name}");
            assert!(!engine.ptm().held, "{name}: clearing ptm kept the hold");
        }
        let both = || HashMap::from([("ptm".to_owned(), f13), ("mute".to_owned(), f14)]);
        let engine = held();
        let mut map = both();
        assert_eq!(drop_binding(&mut map, &engine, "mute"), Some(f14));
        assert!(
            engine.ptm().held && engine.ptm().seq == 1,
            "clearing mute let go"
        );
        let mut map = both();
        let all = drain_bindings(&mut map, &engine);
        assert!(map.is_empty());
        assert!(all.len() == 2 && all.contains(&f13) && all.contains(&f14));
        assert!(!engine.ptm().held, "clearing every key kept the hold");
    }

    /// v1.58.0 (D23): the two push-to-modulate commands stay sync. The
    /// commands need an `AppHandle`, so this reads the source.
    #[test]
    fn ptm_commands_stay_sync() {
        let file = include_str!("lib.rs").replace('\r', "");
        let tests_at = file
            .find(concat!("#[cfg(test)]\nmod ", "tests {"))
            .expect("the tests module");
        let prod = &file[..tests_at];
        for name in ["set_ptm_mode", "set_ptm_held"] {
            assert!(
                !prod.contains(&format!("async fn {name}(")),
                "{name} must stay a sync command (D23): sync commands run in \
                 order on the main thread, so a press can't overtake its \
                 release. An async one runs on a tokio worker, in any order, \
                 and a quick tap could land release-then-press: stuck held"
            );
            assert!(
                prod.contains(&format!("#[tauri::command]\nfn {name}(")),
                "{name} is not a sync command"
            );
        }
        let handler = &prod[prod.find("tauri::generate_handler![").expect("the handler")..];
        let handler = &handler[..handler.find(']').expect("its end")];
        assert!(handler.contains(concat!(
            "\n            set_ptm_mode,\n",
            "            set_ptm_held,\n"
        )));
    }

    /// v1.58.0: the handler passes the event's real press state to the
    /// push-to-modulate step, press and release, and doesn't touch PTM any
    /// other way. The handler needs an `AppHandle`, so this reads the source.
    #[test]
    fn the_shortcut_handler_runs_the_ptm_hotkey() {
        let file = include_str!("lib.rs").replace('\r', "");
        let tests_at = file
            .find(concat!("#[cfg(test)]\nmod ", "tests {"))
            .expect("the tests module");
        let handler = body_of(
            &file[..tests_at],
            concat!("fn global_shortcut", "_handler("),
        );
        let call = concat!(
            "ptm_on_hotkey(&state.engine, &id, ",
            "event.state == ShortcutState::Pressed);"
        );
        assert_eq!(
            handler.matches(call).count(),
            1,
            "the handler must run the push-to-modulate step once, with the press state"
        );
        for other in [concat!("set_ptm", "_held("), concat!("_ptm", "_mode(")] {
            assert!(!handler.contains(other), "the handler calls {other}");
        }
    }

    /// v1.58.0 (D16): the plugin runs the handler under its own store lock,
    /// and the handler takes `AppState.shortcuts`. A command that called the
    /// plugin while holding `AppState.shortcuts` could deadlock the main
    /// thread against a key released on global-hotkey's poller thread, and
    /// push-to-modulate makes releases the most frequent hotkey event. So in
    /// each shortcut command no plugin call (`is_registered` included) sits in
    /// a block that holds the map's lock, and the map is updated, through the
    /// helpers that let go of a held PTM key, inside such a block and before
    /// the plugin call it feeds. The commands need an `AppHandle`, so this
    /// reads the source.
    #[test]
    fn the_shortcut_commands_unlock_before_the_plugin() {
        let file = include_str!("lib.rs").replace('\r', "");
        let tests_at = file
            .find(concat!("#[cfg(test)]\nmod ", "tests {"))
            .expect("the tests module");
        let prod = &file[..tests_at];
        let at = |body: &str, needle: &str| body.find(needle).unwrap_or_else(|| panic!("{needle}"));
        // Each lock's block: the innermost `{…}` around it, whose end drops
        // the guard.
        let locked = |body: &str| -> Vec<(usize, usize)> {
            body.match_indices(concat!("state.shortcuts", ".lock()"))
                .map(|(lock, _)| {
                    let mut depth = 0_usize;
                    let open = body[..lock]
                        .char_indices()
                        .rev()
                        .find_map(|(i, c)| match c {
                            '}' => {
                                depth += 1;
                                None
                            }
                            '{' if depth == 0 => Some(i),
                            '{' => {
                                depth -= 1;
                                None
                            }
                            _ => None,
                        })
                        .expect("a block around the lock");
                    (open, open + body_of(&body[open..], "{").len())
                })
                .collect()
        };
        for (command, updates) in [
            (
                "fn register_global_shortcut(",
                &[
                    "same_binding(&map, &id, &accelerator)",
                    "take_binding(&mut map, &state.engine, &id, &accelerator)",
                ][..],
            ),
            (
                "fn unregister_global_shortcut(",
                &["drop_binding(&mut map, &state.engine, &id)"][..],
            ),
            (
                "fn unregister_all_global_shortcuts(",
                &["drain_bindings(&mut map, &state.engine)"][..],
            ),
        ] {
            let body = body_of(prod, command);
            let blocks = locked(body);
            assert!(!blocks.is_empty(), "{command} never locks the map");
            assert!(body.contains("global_shortcut()"), "{command}");
            for &(open, close) in &blocks {
                for plugin in [
                    "global_shortcut()",
                    ".is_registered(",
                    ".register(",
                    ".unregister(",
                ] {
                    assert!(
                        !body[open..close].contains(plugin),
                        "{command} calls {plugin} with the map locked (D16)"
                    );
                }
            }
            for update in updates {
                let i = at(body, update);
                assert!(
                    blocks.iter().any(|&(open, close)| open < i && i < close),
                    "{command}: {update} outside the lock"
                );
            }
        }
        let register = body_of(prod, "fn register_global_shortcut(");
        let order = [
            "same_binding(",
            ".is_registered(",
            "take_binding(",
            ".unregister(",
            ".register(new?)",
        ]
        .map(|step| at(register, step));
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
        for (command, update) in [
            ("fn unregister_global_shortcut(", "drop_binding("),
            ("fn unregister_all_global_shortcuts(", "drain_bindings("),
        ] {
            let body = body_of(prod, command);
            assert!(at(body, update) < at(body, ".unregister("), "{command}");
        }
    }

    /// A silent window is −∞ dBFS, which JSON cannot carry: serialized as-is
    /// it arrives as `null` and every number on the panel becomes a
    /// null-check. The bridge floors it to a real number instead.
    #[test]
    fn a_silent_window_crosses_the_bridge_as_numbers() {
        let u = VoiceReadingUpdate::from_snapshot(&ReadingSnapshot::idle(ReadingState::Quiet));
        let v = serde_json::to_value(&u).unwrap();
        for half in ["dry", "wet"] {
            for key in [
                "brightnessHz",
                "energyDbfs",
                "energyRangeDb",
                "f0Hz",
                "f0RangeSt",
                "paceOps",
                "voicedRatio",
            ] {
                assert!(
                    v[half][key].is_f64(),
                    "{half}.{key} is not a number: {:?}",
                    v[half][key]
                );
            }
        }
        assert!(
            (v["dry"]["energyDbfs"].as_f64().unwrap() - f64::from(READING_FLOOR_DBFS)).abs() < 1e-6
        );
    }

    /// The panel shows words, never descriptor names, and never more than
    /// three of them — past three it stops reading as a description of a
    /// sound and starts reading as a verdict on a person. Capped here as
    /// well as at the source so it does not depend on who filled the list.
    #[test]
    fn the_wire_sends_vocabulary_words_capped_at_three() {
        let snap = ReadingSnapshot {
            reading: VoiceReading {
                state: ReadingState::Speaking,
                dry: Metrics::silent(),
                wet: Metrics::silent(),
                descriptors: vec![
                    Descriptor::Bright,
                    Descriptor::Wide,
                    Descriptor::Fast,
                    Descriptor::Loud,
                ],
                calibrated: true,
                held: false,
                held_descriptors: Vec::new(),
                wet_bypassed: false,
                wet_settled: true,
            },
            stale: false,
            age_ms: 0,
            muted_in_divora: false,
        };
        let u = VoiceReadingUpdate::from_snapshot(&snap);
        assert_eq!(u.words.len(), MAX_DESCRIPTORS);
        assert_eq!(u.words, ["bright", "wide range", "fast"]);
        // Every word came from the vocabulary's own `word()`, so the
        // frontend never needs a list of its own to drift from.
        for w in &u.words {
            assert!(
                Descriptor::ALL.iter().any(|d| d.word() == *w),
                "{w} is off-vocabulary"
            );
        }
    }

    #[test]
    fn voice_info_json_keys_are_frozen() {
        let v = VoiceInfo {
            id: "a".into(),
            name: "A".into(),
            path: "C:/a.onnx".into(),
            size_bytes: 1,
        };
        assert_eq!(
            sorted_keys(&serde_json::to_value(&v).unwrap()),
            ["id", "name", "path", "sizeBytes"]
        );
    }

    #[test]
    fn onnx_runtime_status_json_keys_are_frozen() {
        let s = OnnxRuntimeStatus {
            runtime_available: true,
            voices_dir: "C:/voices".into(),
        };
        assert_eq!(
            sorted_keys(&serde_json::to_value(&s).unwrap()),
            ["runtimeAvailable", "voicesDir"]
        );
    }

    /// Unique scratch dir under the OS temp area.
    fn scratch(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("divora-voices-{tag}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(dir: &Path, name: &str) {
        fs::write(dir.join(name), b"fake-onnx").unwrap();
    }

    #[test]
    fn scan_voice_dir_lists_onnx_only_and_skips_others() {
        let dir = scratch("ext");
        touch(&dir, "alpha.onnx");
        touch(&dir, "notes.txt");
        touch(&dir, "beta.onnx");
        let mut out: Vec<VoiceInfo> = Vec::new();
        scan_voice_dir(&dir, &mut out);
        let mut ids: Vec<_> = out.iter().map(|v| v.id.clone()).collect();
        ids.sort();
        assert_eq!(ids, vec!["alpha", "beta"]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn user_voices_shadow_bundled_by_id() {
        let user = scratch("user");
        let bundled = scratch("bundled");
        touch(&user, "alpha.onnx");
        touch(&user, "shared.onnx");
        touch(&bundled, "shared.onnx"); // same id — should be shadowed
        touch(&bundled, "gamma.onnx");

        let mut out: Vec<VoiceInfo> = Vec::new();
        scan_voice_dir(&user, &mut out); // user first → wins
        scan_voice_dir(&bundled, &mut out);

        let mut ids: Vec<_> = out.iter().map(|v| v.id.clone()).collect();
        ids.sort();
        assert_eq!(ids, vec!["alpha", "gamma", "shared"]);
        // The retained `shared` must be the USER copy, not the bundled one.
        let shared = out.iter().find(|v| v.id == "shared").unwrap();
        assert!(
            shared.path.contains("divora-voices-user-"),
            "shared should come from the user dir, got {}",
            shared.path
        );
        fs::remove_dir_all(&user).ok();
        fs::remove_dir_all(&bundled).ok();
    }

    #[test]
    fn scan_missing_dir_is_a_no_op() {
        let mut out: Vec<VoiceInfo> = Vec::new();
        scan_voice_dir(&PathBuf::from("/no/such/voices/dir"), &mut out);
        assert!(out.is_empty());
    }

    // ---- v1.47.0: voice-preview cache ----

    #[test]
    fn preview_round_trips_through_the_cache() {
        let dir = scratch("preview-rt");
        let path = dir.join("preview-v1.wav");
        let samples: Vec<f32> = (0..480)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / 24_000.0;
                (t * 440.0).sin() * 0.4
            })
            .collect();
        write_preview(&path, &samples, 24_000);
        assert!(path.is_file(), "cache file should exist");

        let (back, rate) = read_preview(&path).expect("cached preview must read back");
        assert_eq!(rate, 24_000);
        assert_eq!(back.len(), samples.len());
        for (a, b) in samples.iter().zip(&back) {
            assert!((a - b).abs() < 1e-6, "sample mismatch {a} vs {b}");
        }
        fs::remove_dir_all(&dir).ok();
    }

    /// A half-written file must not be served as a cache hit — the writer goes
    /// through a `.part` file and renames, so an interrupted write leaves the
    /// cache absent rather than truncated.
    #[test]
    fn an_interrupted_write_leaves_no_cache_entry() {
        let dir = scratch("preview-partial");
        let path = dir.join("preview-v1.wav");
        fs::create_dir_all(&dir).unwrap();
        // Simulate the debris of a failed write.
        fs::write(path.with_extension("part"), b"not a wav").unwrap();
        assert!(
            read_preview(&path).is_none(),
            "no cache entry should be visible"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_non_wav_file_is_not_treated_as_a_cache_hit() {
        let dir = scratch("preview-junk");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preview-v1.wav");
        fs::write(&path, b"definitely not a wav").unwrap();
        assert!(read_preview(&path).is_none());
        fs::remove_dir_all(&dir).ok();
    }

    /// The filename carries the phrase version, so changing `PREVIEW_TEXT` in a
    /// future release ignores every stale render with no migration code.
    #[test]
    fn the_cache_filename_is_version_stamped() {
        let dir = scratch("preview-ver");
        let path = dir.join(format!("preview-v{PREVIEW_TEXT_VERSION}.wav"));
        write_preview(&path, &[0.1_f32; 64], 24_000);
        assert!(path.is_file());
        // A different version reads as a miss rather than as stale audio.
        let other = dir.join(format!("preview-v{}.wav", PREVIEW_TEXT_VERSION + 1));
        assert!(read_preview(&other).is_none());
        fs::remove_dir_all(&dir).ok();
    }

    /// The preview phrase is what makes caching viable, and it is also what a
    /// user hears when comparing voices — pin its shape.
    #[test]
    fn the_preview_phrase_is_short_and_carries_both_intonations() {
        let n = PREVIEW_TEXT.chars().count();
        // Long enough to judge a voice, short enough to sit through and to
        // bound synthesis cost on the slow (cloned) path.
        assert!(
            (40..=160).contains(&n),
            "preview phrase should be 40-160 chars, got {n}"
        );
        assert!(
            PREVIEW_TEXT.contains('?'),
            "needs a question for rising intonation"
        );
        assert!(
            PREVIEW_TEXT.contains('.'),
            "needs a statement for falling intonation"
        );
    }

    /// The preview slot must be distinct from the main Speak slot, or an
    /// audition would clobber the Speak progress ring and the two Stop buttons
    /// would kill each other.
    #[test]
    fn the_preview_clip_slot_is_distinct_from_speak() {
        assert_ne!(PREVIEW_CLIP_ID, TTS_CLIP_ID);
    }

    // ---- v1.50.0: Speak never stacks; babble ids stay off disk ----

    fn utterance() -> Arc<Vec<f32>> {
        Arc::new(vec![0.25_f32; 4_800])
    }

    /// Why `speak` must stop before it plays: the mixer takes an idle slot
    /// rather than replacing a voice with the same clip id, so two Plays of
    /// `tts` sound at once.
    #[test]
    fn two_plays_of_the_same_clip_id_stack_in_the_mixer() {
        let mut mixer = SoundboardMixer::new();
        for _ in 0..2 {
            // The engine routes the retirement to the graveyard; here there is
            // nowhere to route it, so drop it off the audio thread's behalf.
            drop(mixer.apply(SoundboardCommand::Play {
                clip_id: TTS_CLIP_ID.to_string(),
                samples: utterance(),
                sample_rate: 48_000,
                gain: 1.0,
            }));
        }
        assert_eq!(mixer.active_voice_count(), 2);
    }

    #[test]
    fn emit_speak_hands_over_both_commands_in_order() {
        // Guards the send loop itself, not just what `speak_commands` builds:
        // a version that emitted only the Play would still satisfy a test of
        // `speak_commands` alone, and would bring back the stacking bug.
        let mut sent: Vec<SoundboardCommand> = Vec::new();
        emit_speak(|cmd| sent.push(cmd), utterance(), 24_000, 1.0, false);
        assert_eq!(sent.len(), 2, "both commands must reach the engine");
        assert!(matches!(&sent[0], SoundboardCommand::Stop { clip_id } if clip_id == TTS_CLIP_ID));
        assert!(
            matches!(&sent[1], SoundboardCommand::Play { clip_id, .. } if clip_id == TTS_CLIP_ID)
        );
    }

    #[test]
    fn speak_commands_stop_the_tts_slot_then_play_into_it() {
        let [stop, play] = speak_commands(utterance(), 24_000, 0.5, false);
        assert!(
            matches!(&stop, SoundboardCommand::Stop { clip_id } if clip_id == TTS_CLIP_ID),
            "first command must stop the previous utterance, got {stop:?}"
        );
        let SoundboardCommand::Play {
            clip_id,
            sample_rate,
            gain,
            ..
        } = play
        else {
            panic!("expected a main-send Play, got {play:?}");
        };
        assert_eq!((clip_id.as_str(), sample_rate), (TTS_CLIP_ID, 24_000));
        assert!((gain - 0.5).abs() < f32::EPSILON);

        let [_, preview] = speak_commands(utterance(), 24_000, 1.0, true);
        assert!(
            matches!(&preview, SoundboardCommand::PlayMonitorOnly { clip_id, .. } if clip_id == TTS_CLIP_ID),
            "preview-only must stay off the main send, got {preview:?}"
        );
    }

    /// Rapid Speak presses — the babble case, where each render is near-instant
    /// — leave exactly one utterance sounding, whichever path it plays on.
    #[test]
    fn repeated_speaks_never_overlap() {
        let mut mixer = SoundboardMixer::new();
        for preview_only in [false, false, true, false] {
            for cmd in speak_commands(utterance(), 48_000, 1.0, preview_only) {
                drop(mixer.apply(cmd));
            }
            assert_eq!(mixer.active_voice_count(), 1);
        }
        // The survivor plays at its own level, not the sum of stacked takes.
        let mut out = vec![0.0_f32; 64];
        mixer.mix_into(&mut out, 48_000);
        assert!((out[0] - 0.25).abs() < 1e-3, "out[0] = {}", out[0]);
    }

    /// A babble id must never resolve as a cloned voice. Besides routing to the
    /// wrong engine, joining one onto a path is unsafe on NTFS, where `a:b`
    /// names an alternate data stream of a file `a`.
    #[test]
    fn babble_ids_are_never_cloned_voice_ids() {
        let voices = scratch("babble-not-clone");
        // A real clone alongside, so a None below is the id's doing, not an
        // empty directory's.
        seed_clone(
            &voices,
            "bright",
            Some(r#"{"name":"Bright","base":"am_puck","engine":"voxcpm"}"#),
            "reference.wav",
        );
        assert!(load_cloned_voice(&voices, "bright").is_some());
        for v in babble::VOICES {
            assert!(!is_safe_voice_id(v.id), "{} must fail the path check", v.id);
            assert!(load_cloned_voice(&voices, v.id).is_none());
        }
        fs::remove_dir_all(&voices).ok();
    }

    /// Babble previews render near-instantly, and a disk cache would go on
    /// serving the old sound after the synthesiser changes in an update.
    #[test]
    fn babble_previews_are_never_disk_cached() {
        let voices = scratch("babble-preview");
        let cache = voices.join("preview-cache");
        for v in babble::VOICES {
            assert_eq!(preview_path_in(&voices, &cache, v.id), None);
        }
        // A preset still caches, so the None above is down to the id.
        assert!(preview_path_in(&voices, &cache, "af_heart").is_some_and(|p| p.starts_with(&cache)));
        fs::remove_dir_all(&voices).ok();
    }

    // ---- v1.43.0: renaming a cloned voice ------------------------------

    /// Build `<voices>/cloned/<id>/` with a voice payload + optional meta.
    fn seed_clone(voices: &Path, id: &str, meta: Option<&str>, payload: &str) {
        let dir = cloned_voices_dir(voices).join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(payload), b"x").unwrap();
        if let Some(m) = meta {
            fs::write(dir.join("meta.json"), m).unwrap();
        }
    }

    fn read_meta(voices: &Path, id: &str) -> ClonedMeta {
        let s = fs::read_to_string(cloned_voices_dir(voices).join(id).join("meta.json")).unwrap();
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn rename_updates_the_name_and_preserves_base_and_engine() {
        let voices = scratch("rename-ok");
        seed_clone(
            &voices,
            "velvet-demon",
            Some(r#"{"name":"Velvet Demon","base":"am_puck","engine":"voxcpm"}"#),
            "reference.wav",
        );

        rename_cloned_voice_in(&voices, "velvet-demon", "  Static Wraith  ").unwrap();

        let meta = read_meta(&voices, "velvet-demon");
        assert_eq!(meta.name, "Static Wraith", "name is renamed and trimmed");
        assert_eq!(meta.base, "am_puck", "base must survive a rename");
        assert_eq!(meta.engine, "voxcpm", "engine must survive a rename");
        // The id/folder is the stable key — it must NOT move.
        assert!(cloned_voices_dir(&voices).join("velvet-demon").is_dir());
        // And the rename is visible through the normal listing path.
        let infos = list_cloned_voice_infos(&cloned_voices_dir(&voices));
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].id, "velvet-demon");
        assert_eq!(infos[0].name, "Static Wraith");
        // No temp file left behind.
        assert!(!cloned_voices_dir(&voices)
            .join("velvet-demon")
            .join("meta.json.tmp")
            .exists());
        fs::remove_dir_all(&voices).ok();
    }

    #[test]
    fn rename_a_legacy_clone_without_meta_writes_one() {
        let voices = scratch("rename-legacy");
        // Pre-v2 OpenVoice clone: an `se.bin` and no meta.json at all.
        seed_clone(&voices, "old-voice", None, "se.bin");

        rename_cloned_voice_in(&voices, "old-voice", "Renamed Legacy").unwrap();

        let meta = read_meta(&voices, "old-voice");
        assert_eq!(meta.name, "Renamed Legacy");
        // Engine inference must match what the listing shows for this voice.
        assert_eq!(meta.engine, "openvoice");
        assert_eq!(meta.base, default_clone_base());
        fs::remove_dir_all(&voices).ok();
    }

    #[test]
    fn rename_rejects_bad_input_without_touching_disk() {
        let voices = scratch("rename-bad");
        seed_clone(
            &voices,
            "velvet-demon",
            Some(r#"{"name":"Velvet Demon","base":"am_puck","engine":"voxcpm"}"#),
            "reference.wav",
        );

        // Path traversal / unsafe ids are refused.
        assert!(rename_cloned_voice_in(&voices, "../escape", "Nope").is_err());
        assert!(rename_cloned_voice_in(&voices, "", "Nope").is_err());
        // Empty / whitespace-only names are refused.
        assert!(rename_cloned_voice_in(&voices, "velvet-demon", "   ").is_err());
        // Absurdly long names are refused.
        let long = "x".repeat(MAX_CLONE_NAME_LEN + 1);
        assert!(rename_cloned_voice_in(&voices, "velvet-demon", &long).is_err());
        // A voice that doesn't exist is refused.
        assert!(rename_cloned_voice_in(&voices, "no-such-voice", "Nope").is_err());

        // None of that disturbed the stored meta.
        let meta = read_meta(&voices, "velvet-demon");
        assert_eq!(meta.name, "Velvet Demon");
        assert_eq!(meta.engine, "voxcpm");
        fs::remove_dir_all(&voices).ok();
    }

    #[test]
    fn rename_accepts_a_name_at_the_length_limit() {
        let voices = scratch("rename-limit");
        seed_clone(&voices, "edge", None, "se.bin");
        let exact = "y".repeat(MAX_CLONE_NAME_LEN);
        rename_cloned_voice_in(&voices, "edge", &exact).unwrap();
        assert_eq!(read_meta(&voices, "edge").name, exact);
        fs::remove_dir_all(&voices).ok();
    }

    /// The limit counts CHARACTERS, not bytes — a 64-char multibyte name is
    /// 128 bytes and must still be accepted.
    #[test]
    fn rename_length_limit_counts_characters_not_bytes() {
        let voices = scratch("rename-utf8");
        seed_clone(&voices, "edge", None, "se.bin");
        let multibyte = "é".repeat(MAX_CLONE_NAME_LEN);
        assert!(multibyte.len() > MAX_CLONE_NAME_LEN, "sanity: >1 byte/char");
        rename_cloned_voice_in(&voices, "edge", &multibyte).unwrap();
        assert_eq!(read_meta(&voices, "edge").name, multibyte);
        // One character over is still refused.
        let over = "é".repeat(MAX_CLONE_NAME_LEN + 1);
        assert!(rename_cloned_voice_in(&voices, "edge", &over).is_err());
        fs::remove_dir_all(&voices).ok();
    }

    /// The other arm of the legacy engine inference: a meta-less dir holding a
    /// `reference.wav` is a `VoxCPM` clone, matching what the listing displays.
    #[test]
    fn rename_a_legacy_voxcpm_clone_infers_the_voxcpm_engine() {
        let voices = scratch("rename-legacy-vox");
        seed_clone(&voices, "old-vox", None, "reference.wav");
        rename_cloned_voice_in(&voices, "old-vox", "Renamed Vox").unwrap();
        let meta = read_meta(&voices, "old-vox");
        assert_eq!(meta.name, "Renamed Vox");
        assert_eq!(meta.engine, "voxcpm");
        // And the listing agrees with what we just persisted.
        let infos = list_cloned_voice_infos(&cloned_voices_dir(&voices));
        assert_eq!(infos[0].engine, "voxcpm");
        fs::remove_dir_all(&voices).ok();
    }
}
