//! Chat sessions, stored per project under `.aispice/sessions/`.
//!
//! A session keeps two views of the same conversation: the UI's messages
//! (text, thinking and tool cards as the user saw them) and the agent's own
//! history, which is what lets a conversation continue after a restart.

use aispice_agent::Message;
use aispice_tools::Project;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub circuit: Option<String>,
    pub updated: u64,
    pub message_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSession {
    pub meta: SessionMeta,
    /// `ChatMessage[]` in the UI's shape.
    pub messages: Vec<Value>,
    #[serde(default)]
    pub history: Vec<Message>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn dir(p: &Project) -> Result<std::path::PathBuf, String> {
    p.state_subdir(&["sessions"]).map_err(|e| e.to_string())
}

pub fn list(p: &Project, circuit: Option<&str>) -> Result<Vec<SessionMeta>, String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir(p)?)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let path = e.path();
        if path.extension().is_some_and(|x| x == "json") {
            let meta = std::fs::symlink_metadata(&path).ok();
            if meta.is_some_and(|m| m.file_type().is_file())
                && let Some(s) = std::fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<StoredSession>(&b).ok())
                && (circuit.is_none() || s.meta.circuit.as_deref() == circuit)
            {
                out.push(s.meta);
            }
        }
    }
    out.sort_by(|a, b| b.updated.cmp(&a.updated));
    Ok(out)
}

pub fn load(p: &Project, id: &str) -> Result<StoredSession, String> {
    if !valid_id(id) {
        return Err("invalid session id".into());
    }
    let path = dir(p)?.join(format!("{id}.json"));
    let meta = std::fs::symlink_metadata(&path).map_err(|_| format!("no session {id}"))?;
    if !meta.file_type().is_file() {
        return Err("invalid session file".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| format!("session {id} is unreadable: {e}"))
}

pub fn save(p: &Project, s: &StoredSession) -> Result<(), String> {
    if !valid_id(&s.meta.id) {
        return Err("invalid session id".into());
    }
    let path = dir(p)?.join(format!("{}.json", s.meta.id));
    let bytes = serde_json::to_vec(s).map_err(|e| e.to_string())?;
    aispice_tools::project::atomic_write(&path, &bytes).map_err(|e| e.to_string())
}

pub fn delete(p: &Project, id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err("invalid session id".into());
    }
    let path = dir(p)?.join(format!("{id}.json"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn new_meta(circuit: Option<String>) -> SessionMeta {
    let now = aispice_tools::project::now_ms();
    SessionMeta {
        id: format!("s{now}"),
        title: "New chat".into(),
        circuit,
        updated: now,
        message_count: 0,
    }
}
