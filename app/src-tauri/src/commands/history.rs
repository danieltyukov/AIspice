use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Metadata for a single chat session (stored in sessions.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
}

/// A single stored chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Sanitise a filename so it can be used as a directory name.
/// Replaces `/`, `\`, `:` with `_`.
fn sanitize_filename(name: &str) -> String {
    name.replace(['/', '\\', ':'], "_")
}

/// Return the chat storage directory for a given circuit file:
/// `<working_dir>/.aispice/chats/<sanitized_filename>/`
fn chat_dir(working_dir: &str, filename: &str) -> PathBuf {
    let sanitized = sanitize_filename(filename);
    Path::new(working_dir)
        .join(".aispice")
        .join("chats")
        .join(sanitized)
}

/// Read and parse the `sessions.json` file, returning an empty Vec if it
/// doesn't exist yet.
fn read_sessions(dir: &Path) -> Vec<SessionMeta> {
    let path = dir.join("sessions.json");
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Write the sessions index back to disk.
fn write_sessions(dir: &Path, sessions: &[SessionMeta]) -> Result<(), String> {
    let path = dir.join("sessions.json");
    let json =
        serde_json::to_string_pretty(sessions).map_err(|e| format!("Serialize error: {}", e))?;
    fs::write(&path, json).map_err(|e| format!("Failed to write sessions.json: {}", e))
}

/// Get the current UTC timestamp as an ISO-8601 string (simple fallback
/// without pulling in chrono — uses seconds since epoch).
fn now_iso() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    // Format as a numeric timestamp — frontend can format for display
    format!("{}", dur.as_secs())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// List all chat sessions for a given circuit file.
#[tauri::command]
pub fn list_chat_sessions(
    filename: String,
    state: State<'_, AppState>,
) -> Result<Vec<SessionMeta>, String> {
    let wd = state
        .working_directory
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let dir = wd
        .as_deref()
        .ok_or_else(|| "Working directory not set".to_string())?;

    let chat_path = chat_dir(dir, &filename);
    Ok(read_sessions(&chat_path))
}

/// Load the full message history for a specific chat session.
#[tauri::command]
pub fn load_chat_session(
    filename: String,
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<StoredMessage>, String> {
    let wd = state
        .working_directory
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let dir = wd
        .as_deref()
        .ok_or_else(|| "Working directory not set".to_string())?;

    let chat_path = chat_dir(dir, &filename);
    let session_file = chat_path.join(format!("{}.json", sanitize_filename(&session_id)));

    match fs::read_to_string(&session_file) {
        Ok(content) => {
            serde_json::from_str(&content).map_err(|e| format!("Parse error: {}", e))
        }
        Err(e) => Err(format!("Failed to read session {}: {}", session_id, e)),
    }
}

/// Save (create or update) a chat session.
#[tauri::command]
pub fn save_chat_session(
    filename: String,
    session_id: String,
    title: String,
    messages: Vec<StoredMessage>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let wd = state
        .working_directory
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let dir = wd
        .as_deref()
        .ok_or_else(|| "Working directory not set".to_string())?;

    let chat_path = chat_dir(dir, &filename);

    // Ensure directory exists
    fs::create_dir_all(&chat_path)
        .map_err(|e| format!("Failed to create chat directory: {}", e))?;

    // Write the messages file
    let session_file = chat_path.join(format!("{}.json", sanitize_filename(&session_id)));
    let json =
        serde_json::to_string_pretty(&messages).map_err(|e| format!("Serialize error: {}", e))?;
    fs::write(&session_file, json)
        .map_err(|e| format!("Failed to write session file: {}", e))?;

    // Update sessions index
    let mut sessions = read_sessions(&chat_path);
    let now = now_iso();

    if let Some(existing) = sessions.iter_mut().find(|s| s.id == session_id) {
        existing.title = title;
        existing.updated_at = now;
        existing.message_count = messages.len();
    } else {
        sessions.push(SessionMeta {
            id: session_id,
            title,
            created_at: now.clone(),
            updated_at: now,
            message_count: messages.len(),
        });
    }

    write_sessions(&chat_path, &sessions)?;
    Ok(())
}

/// Delete a chat session and remove it from the index.
#[tauri::command]
pub fn delete_chat_session(
    filename: String,
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let wd = state
        .working_directory
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let dir = wd
        .as_deref()
        .ok_or_else(|| "Working directory not set".to_string())?;

    let chat_path = chat_dir(dir, &filename);

    // Remove the session file (ignore error if it doesn't exist)
    let session_file = chat_path.join(format!("{}.json", sanitize_filename(&session_id)));
    let _ = fs::remove_file(&session_file);

    // Update sessions index
    let mut sessions = read_sessions(&chat_path);
    sessions.retain(|s| s.id != session_id);
    write_sessions(&chat_path, &sessions)?;

    Ok(())
}
