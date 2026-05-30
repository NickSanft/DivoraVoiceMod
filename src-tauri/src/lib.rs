//! `DivoraVoice` Tauri shell.
//!
//! Phase 0: blank window with a smoke-test IPC command. The design system
//! and app shell land in Phase 1; the audio engine lands in Phase 2.

// Tauri command signatures take `State<'_, T>` by value; clippy can't
// see through the attribute macro and flags every command.
#![allow(clippy::needless_pass_by_value)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use divora_core::audio::{
    detect_virtual_mic as detect_virtual_mic_core, list_input_devices as list_input_devices_core,
    list_output_devices as list_output_devices_core, AudioEngine, DeviceInfo, Levels, StreamInfo,
    VirtualMicStatus,
};
use divora_core::dsp::{onnx_runtime_available, DspCommand, EffectSpec};
use divora_core::presets::{bundled_presets, Preset, PresetStore};
use divora_core::soundboard::{
    decode_clip, scan_folder, DecodedClip, SoundboardCommand, SoundboardTile,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

/// Payload of the periodic `audio-levels` event. Frontend listens with
/// `tauri.event.listen("audio-levels", …)`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LevelUpdate {
    input: Levels,
    output: Levels,
    running: bool,
    monitoring: bool,
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
fn ping() -> &'static str {
    "pong"
}

#[tauri::command]
fn project_name() -> &'static str {
    divora_core::project_name()
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
) -> Result<StreamInfo, String> {
    state
        .engine
        .start(input_name.as_deref(), output_name.as_deref())
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

#[tauri::command]
fn audio_engine_status(state: State<'_, AppState>) -> EngineStatus {
    EngineStatus {
        running: state.engine.is_running(),
        monitoring: state.engine.is_monitoring(),
        input: state.engine.input_levels(),
        output: state.engine.output_levels(),
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
#[tauri::command]
fn list_voices(state: State<'_, AppState>) -> Vec<VoiceInfo> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&state.voices_dir) else {
        return out;
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
        if id.is_empty() {
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
    out.sort_by_key(|v| v.name.to_lowercase());
    out
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
) -> Result<f32, String> {
    let decoded = decode_or_cache(&state, &clip_id, Path::new(&path))?;
    state.engine.send_soundboard(SoundboardCommand::Play {
        clip_id: clip_id.clone(),
        samples: decoded.samples.clone(),
        sample_rate: decoded.sample_rate,
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
            };
            if app.emit("audio-levels", &payload).is_err() {
                // App is shutting down (no receivers / window gone).
                break;
            }
        })
        .expect("spawning the level-emitter thread should not fail");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(global_shortcut_handler)
                .build(),
        )
        .setup(|app| {
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

            app.manage(AppState {
                engine,
                preset_store,
                clip_cache: Mutex::new(HashMap::new()),
                shortcuts: Mutex::new(HashMap::new()),
                voices_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            project_name,
            list_audio_input_devices,
            list_audio_output_devices,
            start_audio_engine,
            stop_audio_engine,
            set_audio_monitor,
            audio_engine_status,
            set_effect_chain,
            set_effect_param,
            set_effect_enabled,
            clear_effect_chain,
            list_presets,
            save_user_preset,
            delete_user_preset,
            export_preset_json,
            preset_store_path,
            voices_dir,
            list_voices,
            onnx_runtime_status,
            set_voice_model,
            scan_soundboard_folder,
            play_soundboard_clip,
            stop_soundboard_clip,
            stop_all_soundboard_clips,
            detect_virtual_mic,
            register_global_shortcut,
            unregister_global_shortcut,
            unregister_all_global_shortcuts,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DivoraVoice");
}
