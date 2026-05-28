//! `DivoraVoice` Tauri shell.
//!
//! Phase 0: blank window with a smoke-test IPC command. The design system
//! and app shell land in Phase 1; the audio engine lands in Phase 2.

#[tauri::command]
fn ping() -> &'static str {
    "pong"
}

#[tauri::command]
fn project_name() -> &'static str {
    divora_core::project_name()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![ping, project_name])
        .run(tauri::generate_context!())
        .expect("error while running DivoraVoice");
}
