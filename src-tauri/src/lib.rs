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
use std::sync::{Arc, Mutex};
use std::time::Duration;

use divora_core::audio::{
    detect_virtual_mic as detect_virtual_mic_core, list_input_devices as list_input_devices_core,
    list_output_devices as list_output_devices_core, AudioEngine, DeviceInfo, Levels, StreamInfo,
    VirtualMicStatus,
};
use divora_core::dsp::{onnx_runtime_available, DspCommand, EffectSpec};
use divora_core::presets::{bundled_presets, Preset, PresetStore, PresetTag};
use divora_core::soundboard::{
    decode_clip, scan_folder, DecodedClip, SoundboardCommand, SoundboardTile,
};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

mod midi;
use midi::{
    list_inputs as list_midi_inputs_core, open_input as open_midi_input_core, MidiInputInfo,
};
use midir::MidiInputConnection;

/// Payload of the periodic `audio-levels` event. Frontend listens with
/// `tauri.event.listen("audio-levels", …)`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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
    preset_store: Arc<PresetStore>,
    clip_cache: Mutex<HashMap<String, DecodedClip>>,
    /// Maps a stable id (chosen by the frontend, e.g. "ptm") to the
    /// `Shortcut` we registered for it. Used to unregister cleanly.
    shortcuts: Mutex<HashMap<String, Shortcut>>,
    /// Phase 12: directory the user drops voice-conversion `.onnx`
    /// models into (`%APPDATA%/DivoraVoice/voices/`). Created at
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
    /// **downloaded** into on demand (`%APPDATA%/DivoraVoice/tts/`). They're
    /// no longer bundled (kept the installer small); cloning prompts a
    /// one-time download into here.
    clone_models_dir: PathBuf,
    /// Phase 16: directory recordings are written to
    /// (`%APPDATA%/DivoraVoice/recordings/`). Created at startup;
    /// surfaced to the UI so it can open it + show where files land.
    recordings_dir: PathBuf,
    /// v1.14.0: directory the rolling log file is written to
    /// (`%APPDATA%/DivoraVoice/logs/`). Created at startup; surfaced to
    /// the UI's "Open logs folder" support affordance.
    logs_dir: PathBuf,
    /// v1.9.0: the live MIDI input connection, if a control surface is
    /// connected. Held here so it stays alive — dropping it closes the
    /// port. Replaced on `open_midi_input`, cleared on `close_midi_input`.
    midi_connection: Mutex<Option<MidiInputConnection<()>>>,
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
        key: "model".to_string(),
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
/// replaces any in-flight utterance and `stop_speak` can target it.
const TTS_CLIP_ID: &str = "tts";

/// List the preset "Speak" voices, each flagged with whether its model assets
/// are installed (so the UI can show "voice not installed" instead of failing
/// on Speak). Never touches the ONNX runtime — pure filesystem probing.
#[tauri::command]
fn list_tts_voices(state: State<'_, AppState>) -> Vec<divora_core::tts::TtsVoiceInfo> {
    divora_core::tts::list_voices(&state.tts_assets_dir)
}

/// Synthesize `text` with preset `voice_id` and play it through the output by
/// reusing the soundboard mixer seam (so a Discord/stream listener hears it
/// too, mixed with the live mic). Returns the clip duration in seconds for the
/// playback progress ring.
///
/// `gain` (v1.18.0, optional, default 1.0) is the linear playback volume.
/// `preview_only` (v1.18.0, optional, default false) routes the speech to the
/// local monitor only — you hear it, the call doesn't — for previewing.
///
/// Synthesis is batch work; it runs here on the command's worker thread (as
/// `play_soundboard_clip` decodes synchronously). Until the Kokoro model is
/// staged, `synthesize` returns `NotInstalled` immediately, surfaced to the UI
/// as a graceful error string rather than a hang.
#[tauri::command]
fn speak(
    state: State<'_, AppState>,
    text: String,
    voice_id: String,
    gain: Option<f32>,
    preview_only: Option<bool>,
) -> Result<f32, String> {
    // A cloned-voice id resolves to a stored speaker embedding → route it
    // through the `OpenVoice` converter on the `am_puck` base. Otherwise it's a
    // preset id → plain Kokoro.
    let audio = match load_cloned_voice(&state.voices_dir, &voice_id) {
        Some((se, base)) => divora_core::tts::synthesize_cloned(
            &text,
            &base,
            &se,
            &state.tts_assets_dir,
            &clone_dir(&state),
            CLONE_TAU,
        ),
        None => divora_core::tts::synthesize(&text, &voice_id, &state.tts_assets_dir),
    }
    .map_err(|e| {
        tracing::info!(error = %e, voice = %voice_id, "speak: synthesis unavailable");
        e.to_string()
    })?;
    let sample_rate = audio.sample_rate;
    let samples = Arc::new(audio.samples);
    #[allow(clippy::cast_precision_loss)]
    let duration_secs = if sample_rate == 0 {
        0.0
    } else {
        samples.len() as f32 / sample_rate as f32
    };
    let gain = gain.unwrap_or(1.0);
    let clip_id = TTS_CLIP_ID.to_string();
    // Preview → monitor-only (you hear it, the call doesn't); otherwise the
    // normal both-paths play used by the soundboard.
    let cmd = if preview_only.unwrap_or(false) {
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
    state.engine.send_soundboard(cmd);
    Ok(duration_secs)
}

/// Stop any in-flight synthesized speech playing through the mixer.
#[tauri::command]
fn stop_speak(state: State<'_, AppState>) {
    state.engine.send_soundboard(SoundboardCommand::Stop {
        clip_id: TTS_CLIP_ID.to_string(),
    });
}

// ---- v1.20.0: voice cloning ("Your voices") ----

/// Default Kokoro base voice a cloned voice generates with before tone-color
/// conversion — `am_puck`, the closest stock voice in the Phase-2a validation.
/// Stored per-voice in meta so a later phase can vary it (e.g. by accent/gender).
const CLONE_BASE_VOICE: &str = "am_puck";
/// `OpenVoice` conversion temperature (its default).
const CLONE_TAU: f32 = 0.3;

/// One user-cloned voice (wire type for the Speak picker).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClonedVoiceInfo {
    /// Stable slug id (also the on-disk folder name).
    id: String,
    name: String,
}

/// On-disk metadata for a cloned voice (`<id>/meta.json`).
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct ClonedMeta {
    name: String,
    /// Kokoro base voice used for generation before tone-color conversion.
    #[serde(default = "default_clone_base")]
    base: String,
}

fn default_clone_base() -> String {
    CLONE_BASE_VOICE.to_string()
}

/// Directory holding user-cloned voices: `<voices>/cloned/`.
fn cloned_voices_dir(voices_dir: &Path) -> PathBuf {
    voices_dir.join("cloned")
}

/// Whether `id` is a single safe path component (no separators / traversal).
fn is_safe_voice_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() < 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Load a cloned voice's 256-d speaker embedding (`<id>/se.bin`, f32 LE) and
/// its base voice. `None` if `id` isn't a stored cloned voice.
fn load_cloned_voice(voices_dir: &Path, id: &str) -> Option<(Vec<f32>, String)> {
    if !is_safe_voice_id(id) {
        return None;
    }
    let dir = cloned_voices_dir(voices_dir).join(id);
    let bytes = std::fs::read(dir.join("se.bin")).ok()?;
    if bytes.len() != 256 * 4 {
        return None;
    }
    let se = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let base = std::fs::read_to_string(dir.join("meta.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<ClonedMeta>(&s).ok())
        .map_or_else(|| CLONE_BASE_VOICE.to_string(), |m| m.base);
    Some((se, base))
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
        if !p.is_dir() || !is_safe_voice_id(&id) || !p.join("se.bin").is_file() {
            continue;
        }
        let name = std::fs::read_to_string(p.join("meta.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<ClonedMeta>(&s).ok())
            .map_or_else(|| id.clone(), |m| m.name);
        out.push(ClonedVoiceInfo { id, name });
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// Create a cloned voice from a reference audio file: decode it, extract a
/// speaker embedding via the `OpenVoice` extractor, and store it under
/// `<voices>/cloned/<id>/`. Returns the new voice for the picker.
#[tauri::command]
fn clone_voice(
    state: State<'_, AppState>,
    name: String,
    reference_path: String,
) -> Result<ClonedVoiceInfo, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("voice name is required".to_string());
    }
    let clip = decode_clip(Path::new(&reference_path)).map_err(|e| e.to_string())?;
    let se =
        divora_core::tts::extract_voice_se(&clip.samples, clip.sample_rate, &clone_dir(&state))
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
    let meta = ClonedMeta {
        name: name.clone(),
        base: CLONE_BASE_VOICE.to_string(),
    };
    std::fs::write(
        voice_dir.join("meta.json"),
        serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    tracing::info!(id = %id, name = %name, "cloned voice created");
    Ok(ClonedVoiceInfo { id, name })
}

/// List the user's cloned voices for the Speak picker.
#[tauri::command]
fn list_cloned_voices(state: State<'_, AppState>) -> Vec<ClonedVoiceInfo> {
    list_cloned_voice_infos(&cloned_voices_dir(&state.voices_dir))
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
/// The cloning model files to fetch (extractor is tiny; converter is ~157 MB).
const CLONE_MODEL_FILES: &[&str] = &["openvoice-extractor.onnx", "openvoice-converter.onnx"];

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

/// Whether the cloning models are available (so the UI can prompt a
/// one-time download before the user records a voice).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CloneModelsStatus {
    ready: bool,
}

/// Progress event payload for the cloning-model download (`clone-model-download`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CloneDownloadProgress {
    /// Current file (1-based) and how many total.
    file: usize,
    file_count: usize,
    /// Bytes received / total for the current file (total 0 if unknown).
    received: u64,
    total: u64,
}

/// Whether the cloning models are installed.
#[tauri::command]
fn clone_models_status(state: State<'_, AppState>) -> CloneModelsStatus {
    CloneModelsStatus {
        ready: divora_core::tts::clone_models_present(&clone_dir(&state)),
    }
}

/// Download the cloning models into the user dir, emitting
/// `clone-model-download` progress events. Blocks until done (the worker
/// thread); the UI awaits it and tracks progress via the events. Skips files
/// already present, and writes via a `.part` temp so a failed/partial download
/// never looks complete.
#[tauri::command]
fn download_clone_models(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let dir = state.clone_models_dir.clone();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let count = CLONE_MODEL_FILES.len();
    for (i, name) in CLONE_MODEL_FILES.iter().enumerate() {
        let dest = dir.join(name);
        if dest.is_file() {
            continue;
        }
        let url = format!("{CLONE_MODELS_BASE_URL}/{name}");
        download_to_file(&app, &url, &dest, i + 1, count)
            .map_err(|e| format!("downloading {name}: {e}"))?;
    }
    tracing::info!(path = %dir.display(), "clone models downloaded");
    Ok(())
}

/// Stream `url` to `dest` (via a `.part` temp), emitting progress events.
fn download_to_file(
    app: &AppHandle,
    url: &str,
    dest: &Path,
    file: usize,
    file_count: usize,
) -> Result<(), String> {
    let resp = ureq::get(url).call().map_err(|e| e.to_string())?;
    let total: u64 = resp
        .header("Content-Length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let tmp = dest.with_extension("part");
    let mut file_w = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 1 << 16];
    let mut received: u64 = 0;
    let mut last_emit: u64 = 0;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file_w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        received += n as u64;
        // Throttle events to ~every 1 MB.
        if received - last_emit >= 1 << 20 {
            last_emit = received;
            let _ = app.emit(
                "clone-model-download",
                CloneDownloadProgress {
                    file,
                    file_count,
                    received,
                    total,
                },
            );
        }
    }
    file_w.flush().map_err(|e| e.to_string())?;
    drop(file_w);
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
    let _ = app.emit(
        "clone-model-download",
        CloneDownloadProgress {
            file,
            file_count,
            received,
            total,
        },
    );
    Ok(())
}

#[tauri::command]
fn detect_virtual_mic() -> VirtualMicStatus {
    detect_virtual_mic_core()
}

#[tauri::command]
fn register_global_shortcut(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    accelerator: String,
) -> Result<(), String> {
    let shortcut: Shortcut = accelerator.parse().map_err(|e| format!("{e:?}"))?;
    {
        let mut map = state.shortcuts.lock().map_err(|e| e.to_string())?;
        if let Some(old) = map.remove(&id) {
            let _ = app.global_shortcut().unregister(old);
        }
        map.insert(id.clone(), shortcut);
    }
    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn unregister_global_shortcut(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let mut map = state.shortcuts.lock().map_err(|e| e.to_string())?;
    if let Some(shortcut) = map.remove(&id) {
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
    let mut map = state.shortcuts.lock().map_err(|e| e.to_string())?;
    let mut last_err: Option<String> = None;
    for (_, shortcut) in map.drain() {
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
    cache.insert(clip_id.to_owned(), clip.clone());
    Ok(clip)
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

/// Spawn a background thread that emits `audio-levels` events to the
/// frontend at ~30 Hz. The frontend uses these to drive live meters
/// and the status pill without polling commands.
fn spawn_level_emitter(app: AppHandle, engine: Arc<AudioEngine>) {
    std::thread::Builder::new()
        .name("divora-level-emitter".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(33));
            let payload = LevelUpdate {
                input: engine.input_levels(),
                output: engine.output_levels(),
                running: engine.is_running(),
                monitoring: engine.is_monitoring(),
                dsp_latency_ms: engine.dsp_latency_ms(),
                recording: engine.is_recording(),
                loudness_gain_db: engine.loudness_gain_db(),
            };
            if app.emit("audio-levels", &payload).is_err() {
                // App is shutting down (no receivers / window gone).
                break;
            }
        })
        .expect("spawning the level-emitter thread should not fail");
}

/// Reveal + focus the main window (from a tray click / "Show" menu).
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Phase 15: build the system-tray icon + menu so the app can run in the
/// background (e.g. while gaming / on a Discord call). Left-click the
/// tray icon or "Show" to restore the window; "Quit" exits for real.
fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItem::with_id(app, "show", "Show DivoraVoice", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id("divora-tray")
        .tooltip("DivoraVoice")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
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
        // Phase 15: closing the window hides it to the tray instead of
        // quitting, so audio (and Discord routing) keep running in the
        // background. "Quit" in the tray menu exits for real.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            // v1.14.0: install file logging first so both startup and
            // runtime tracing events land in a rolling daily log under
            // %APPDATA%/DivoraVoice/logs/ — nothing collected them before
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

            // Locate %APPDATA%\DivoraVoice\presets\ (or platform equivalent)
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
            // (`%APPDATA%/DivoraVoice/tts/`). Best-effort create.
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
            tracing::info!(path = %clone_models_dir.display(), "clone models dir ready");

            app.manage(AppState {
                engine,
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
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_audio_input_devices,
            list_audio_output_devices,
            start_audio_engine,
            stop_audio_engine,
            set_audio_monitor,
            set_monitor_gain,
            set_loudness_enabled,
            set_loudness_target,
            audio_engine_status,
            set_effect_chain,
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
            scan_soundboard_folder,
            play_soundboard_clip,
            stop_soundboard_clip,
            stop_all_soundboard_clips,
            set_soundboard_master_gain,
            list_tts_voices,
            speak,
            stop_speak,
            clone_voice,
            list_cloned_voices,
            delete_cloned_voice,
            clone_models_status,
            download_clone_models,
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
        scan_voice_dir, slugify_preset_id, unique_preset_id, EngineStatus, LevelUpdate, Levels,
        OnnxRuntimeStatus, VoiceInfo,
    };
    use std::fs;
    use std::path::{Path, PathBuf};

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
        };
        assert_eq!(
            sorted_keys(&serde_json::to_value(&u).unwrap()),
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
}
