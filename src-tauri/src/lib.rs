//! `DivoraVoice` Tauri shell.
//!
//! Phase 0: blank window with a smoke-test IPC command. The design system
//! and app shell land in Phase 1; the audio engine lands in Phase 2.

// Tauri command signatures take `State<'_, T>` by value; clippy can't
// see through the attribute macro and flags every command.
#![allow(clippy::needless_pass_by_value)]

use std::sync::Arc;
use std::time::Duration;

use divora_core::audio::{
    list_input_devices as list_input_devices_core, list_output_devices as list_output_devices_core,
    AudioEngine, DeviceInfo, Levels, StreamInfo,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

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

/// Tauri-managed shared state. Holds the live audio engine; commands
/// access it through `tauri::State<AppState>`.
struct AppState {
    engine: Arc<AudioEngine>,
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
        .setup(|app| {
            let engine = Arc::new(AudioEngine::new());
            spawn_level_emitter(app.handle().clone(), engine.clone());
            app.manage(AppState { engine });
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running DivoraVoice");
}
