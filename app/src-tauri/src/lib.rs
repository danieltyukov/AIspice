mod ai;
mod commands;
mod formats;
mod state;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = AppState::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            // File I/O
            commands::files::set_working_directory,
            commands::files::set_api_key,
            commands::files::get_api_keys,
            commands::files::remove_api_key,
            commands::files::has_api_key,
            commands::files::list_circuit_files,
            commands::files::read_circuit_file,
            commands::files::parse_circuit,
            // AI chat
            commands::chat::send_chat_message_stream,
            // Chat history
            commands::history::list_chat_sessions,
            commands::history::load_chat_session,
            commands::history::save_chat_session,
            commands::history::delete_chat_session,
            // Simulation
            commands::simulation::check_ngspice,
            commands::simulation::run_simulation,
            commands::simulation::parse_waveform_csv,
            // Export
            commands::export::export_schematic,
            // LTspice integration
            commands::ltspice::detect_ltspice,
            commands::ltspice::reload_ltspice,
            commands::ltspice::run_ltspice_batch,
            commands::ltspice::read_simulation_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AIspice");
}
