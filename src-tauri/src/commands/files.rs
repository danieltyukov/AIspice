use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::State;

use crate::formats::{asc, encoding, netlist};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// File extensions we recognise as circuit files.
const CIRCUIT_EXTENSIONS: &[&str] = &["asc", "cir", "spice", "net"];

/// Recursively walk `dir` and collect files whose extension matches one of
/// `CIRCUIT_EXTENSIONS`.
fn collect_circuit_files(dir: &Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                results.extend(collect_circuit_files(&path));
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if CIRCUIT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
                    results.push(path);
                }
            }
        }
    }
    results
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Store the working directory in application state.
#[tauri::command]
pub fn set_working_directory(dir: String, state: State<'_, AppState>) -> Result<(), String> {
    let path = Path::new(&dir);
    if !path.is_dir() {
        return Err(format!("Not a valid directory: {}", dir));
    }
    let mut wd = state
        .working_directory
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    *wd = Some(dir);
    Ok(())
}

/// Store an API key for a specific provider and persist to config file.
#[tauri::command]
pub fn set_api_key(
    provider: String,
    key: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let mut keys = state
            .api_keys
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if key.is_empty() {
            keys.remove(&provider);
        } else {
            keys.insert(provider, key);
        }
    }
    state.persist_keys()?;
    Ok(())
}

/// Get all configured API keys with values masked (first 4 chars + "...").
#[tauri::command]
pub fn get_api_keys(state: State<'_, AppState>) -> Result<HashMap<String, String>, String> {
    let keys = state
        .api_keys
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let masked: HashMap<String, String> = keys
        .iter()
        .map(|(provider, key)| {
            let display = if key.len() > 4 {
                format!("{}...", &key[..4])
            } else {
                "****".to_string()
            };
            (provider.clone(), display)
        })
        .collect();
    Ok(masked)
}

/// Remove an API key for a specific provider and persist.
#[tauri::command]
pub fn remove_api_key(provider: String, state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut keys = state
            .api_keys
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        keys.remove(&provider);
    }
    state.persist_keys()?;
    Ok(())
}

/// Check whether any API key has been configured (non-empty).
#[tauri::command]
pub fn has_api_key(state: State<'_, AppState>) -> Result<bool, String> {
    let keys = state
        .api_keys
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    Ok(!keys.is_empty())
}

/// List all circuit files (`.asc`, `.cir`, `.spice`, `.net`) found
/// recursively under the working directory.  Returns paths relative to the
/// working directory.
#[tauri::command]
pub fn list_circuit_files(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let wd = state
        .working_directory
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let dir = wd
        .as_deref()
        .ok_or_else(|| "Working directory not set".to_string())?;
    let base = Path::new(dir);

    let files = collect_circuit_files(base);
    let relative: Vec<String> = files
        .iter()
        .filter_map(|p| p.strip_prefix(base).ok())
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    Ok(relative)
}

/// Read a circuit file from the working directory with automatic encoding
/// detection.  Returns the decoded text content.
#[tauri::command]
pub fn read_circuit_file(filename: String, state: State<'_, AppState>) -> Result<String, String> {
    let wd = state
        .working_directory
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let dir = wd
        .as_deref()
        .ok_or_else(|| "Working directory not set".to_string())?;
    let path = Path::new(dir).join(&filename);

    let bytes = std::fs::read(&path).map_err(|e| format!("Failed to read {filename}: {e}"))?;

    let (text, _encoding) = encoding::detect_and_decode(&bytes);
    Ok(text)
}

/// Parse a circuit file's content based on its extension and return the
/// parsed structure as a JSON string.
#[tauri::command]
pub fn parse_circuit(filename: String, content: String) -> Result<String, String> {
    let ext = Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "asc" => {
            let parsed = asc::parse_asc(&content)?;
            serde_json::to_string(&parsed).map_err(|e| format!("Serialization error: {}", e))
        }
        "cir" | "spice" | "net" => {
            let parsed = netlist::parse_netlist(&content)?;
            serde_json::to_string(&parsed).map_err(|e| format!("Serialization error: {}", e))
        }
        _ => Err(format!("Unsupported file extension: .{}", ext)),
    }
}
