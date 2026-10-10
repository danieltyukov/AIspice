//! The aispice desktop app: a thin Tauri shell over aispice-tools, so the
//! app, the CLI and the MCP server behave the same.

mod bridge;
mod commands;
mod ltspice;
mod sessions;
mod settings;
mod state;
mod watch;
mod wave;

use serde_json::json;
use std::sync::Arc;
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::new())
        .setup(|app| {
            let handle = app.handle().clone();
            let st = app.state::<state::AppState>();
            let emit_handle = handle.clone();
            let ws_for_hook = st.ws.clone();
            st.ws.set_hooks(aispice_tools::Hooks {
                approver: Some(Arc::new(bridge::UiApprover {
                    app: handle.clone(),
                })),
                after_save: Some(Arc::new(move |path: &std::path::Path| {
                    let rel = ws_for_hook
                        .project()
                        .map(|p| p.relative(path))
                        .unwrap_or_else(|_| path.display().to_string());
                    let _ = emit_handle.emit(
                        commands::EVENT,
                        json!({"type": "circuit_changed", "path": rel, "external": false}),
                    );
                    let st = emit_handle.state::<state::AppState>();
                    let reload = st.prefs.read().map(|p| p.reload_ltspice).unwrap_or(false);
                    if reload && ltspice::is_running() {
                        let configured = st.config.read().ok().and_then(|c| c.ltspice_path.clone());
                        let _ = ltspice::open(path, configured.as_deref());
                    }
                })),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::doctor,
            commands::open_project,
            commands::recent_projects,
            commands::list_circuits,
            commands::read_circuit,
            commands::new_circuit,
            commands::import_netlist,
            commands::simulate,
            commands::waveform,
            commands::check_specs,
            commands::history,
            commands::undo,
            commands::redo,
            commands::restore,
            commands::open_in_ltspice,
            commands::sessions,
            commands::load_session,
            commands::new_session,
            commands::delete_session,
            bridge::send,
            commands::cancel,
            commands::approve,
            commands::settings,
            commands::save_settings,
            commands::key_status,
            commands::set_key,
            commands::remove_key,
            commands::models,
        ])
        .run(tauri::generate_context!())
        .expect("error while running aispice");
}
