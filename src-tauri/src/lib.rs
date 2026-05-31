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
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
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
    /// Phase 14: latency added by the active DSP chain, in ms.
    dsp_latency_ms: f32,
    /// Phase 16: true while the modulated output is being recorded.
    recording: bool,
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
    /// Phase 16: directory recordings are written to
    /// (`%APPDATA%/DivoraVoice/recordings/`). Created at startup;
    /// surfaced to the UI so it can open it + show where files land.
    recordings_dir: PathBuf,
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

#[tauri::command]
fn audio_engine_status(state: State<'_, AppState>) -> EngineStatus {
    EngineStatus {
        running: state.engine.is_running(),
        monitoring: state.engine.is_monitoring(),
        input: state.engine.input_levels(),
        output: state.engine.output_levels(),
        dsp_latency_ms: state.engine.dsp_latency_ms(),
        recording: state.engine.is_recording(),
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
                dsp_latency_ms: engine.dsp_latency_ms(),
                recording: engine.is_recording(),
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

            app.manage(AppState {
                engine,
                preset_store,
                clip_cache: Mutex::new(HashMap::new()),
                shortcuts: Mutex::new(HashMap::new()),
                voices_dir,
                bundled_voices_dir,
                recordings_dir,
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
            recordings_dir,
            start_recording,
            stop_recording,
            scan_soundboard_folder,
            play_soundboard_clip,
            stop_soundboard_clip,
            stop_all_soundboard_clips,
            set_soundboard_master_gain,
            detect_virtual_mic,
            register_global_shortcut,
            unregister_global_shortcut,
            unregister_all_global_shortcuts,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DivoraVoice");
}

#[cfg(test)]
mod tests {
    use super::{scan_voice_dir, VoiceInfo};
    use std::fs;
    use std::path::{Path, PathBuf};

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
