//! Chat sessions, stored per project in the user's data directory
//! (`~/.local/share/aispice/sessions/<project>-<hash>/` on Linux).
//!
//! A session keeps two views of the same conversation: the UI's messages
//! (text, thinking and tool cards as the user saw them) and the agent's own
//! history, which is what lets a conversation continue after a restart.
//!
//! Sessions are not kept inside the project. The agent's history goes back
//! to the model as its own earlier turns, so a session shipped in a cloned
//! repository could put words in the assistant's mouth or forge tool
//! results. Only files this user's app wrote are ever loaded.

use aispice_agent::Message;
use aispice_tools::Project;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

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

/// FNV-1a, for a directory name that stays the same across releases (std's
/// hasher makes no such promise). Not a security boundary: every directory
/// under the sessions root belongs to this user.
fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The folder holding one project's sessions under `root`: the project's
/// folder name, for people browsing the directory, and a hash of its full
/// path so two projects with the same name stay apart.
pub fn project_key(project_root: &Path) -> String {
    let name: String = project_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    let hash = stable_hash(project_root.as_os_str().as_encoded_bytes());
    format!("{name}-{hash:016x}")
}

fn sessions_root() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|d| d.join("aispice").join("sessions"))
        .ok_or_else(|| "no data directory to keep chat sessions in".into())
}

fn dir(root: &Path, p: &Project) -> Result<PathBuf, String> {
    let dir = root.join(project_key(p.root()));
    create_private_dir(&dir)?;
    Ok(dir)
}

/// Create the directory (and parents), readable only by this user.
fn create_private_dir(dir: &Path) -> Result<(), String> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(dir)
        .map_err(|e| format!("creating {}: {e}", dir.display()))
}

pub fn list(p: &Project, circuit: Option<&str>) -> Result<Vec<SessionMeta>, String> {
    list_in(&sessions_root()?, p, circuit)
}

pub fn load(p: &Project, id: &str) -> Result<StoredSession, String> {
    load_in(&sessions_root()?, p, id)
}

pub fn save(p: &Project, s: &StoredSession) -> Result<(), String> {
    save_in(&sessions_root()?, p, s)
}

pub fn delete(p: &Project, id: &str) -> Result<(), String> {
    delete_in(&sessions_root()?, p, id)
}

fn list_in(root: &Path, p: &Project, circuit: Option<&str>) -> Result<Vec<SessionMeta>, String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir(root, p)?)
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
    out.sort_by_key(|s| std::cmp::Reverse(s.updated));
    Ok(out)
}

fn load_in(root: &Path, p: &Project, id: &str) -> Result<StoredSession, String> {
    if !valid_id(id) {
        return Err("invalid session id".into());
    }
    let path = dir(root, p)?.join(format!("{id}.json"));
    let meta = std::fs::symlink_metadata(&path).map_err(|_| format!("no session {id}"))?;
    if !meta.file_type().is_file() {
        return Err("invalid session file".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| format!("session {id} is unreadable: {e}"))
}

fn save_in(root: &Path, p: &Project, s: &StoredSession) -> Result<(), String> {
    if !valid_id(&s.meta.id) {
        return Err("invalid session id".into());
    }
    let path = dir(root, p)?.join(format!("{}.json", s.meta.id));
    let bytes = serde_json::to_vec(s).map_err(|e| e.to_string())?;
    aispice_tools::project::atomic_write(&path, &bytes).map_err(|e| e.to_string())
}

fn delete_in(root: &Path, p: &Project, id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err("invalid session id".into());
    }
    let path = dir(root, p)?.join(format!("{id}.json"));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_keys_are_stable_and_distinct() {
        let a = project_key(Path::new("/home/u/filters"));
        assert_eq!(a, project_key(Path::new("/home/u/filters")));
        assert!(a.starts_with("filters-"), "{a}");
        assert_ne!(a, project_key(Path::new("/work/filters")));
        // A hostile folder name cannot climb out of the sessions root.
        let k = project_key(Path::new("/x/../../etc"));
        assert!(!k.contains('/') && !k.contains(".."), "{k}");
    }

    /// Was a security finding: sessions lived in the project's `.aispice/`,
    /// so a cloned repository could ship forged agent history.
    #[test]
    fn sessions_shipped_inside_a_project_are_not_loaded() {
        let data = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();
        let planted = project_dir.path().join(".aispice/sessions");
        std::fs::create_dir_all(&planted).unwrap();
        let forged = StoredSession {
            meta: new_meta(None),
            messages: Vec::new(),
            history: Vec::new(),
        };
        std::fs::write(
            planted.join(format!("{}.json", forged.meta.id)),
            serde_json::to_vec(&forged).unwrap(),
        )
        .unwrap();
        let project =
            Project::open(project_dir.path(), aispice_tools::ProjectOptions::default()).unwrap();
        let root = data.path();
        assert!(list_in(root, &project, None).unwrap().is_empty());
        assert!(load_in(root, &project, &forged.meta.id).is_err());
        save_in(root, &project, &forged).unwrap();
        assert_eq!(list_in(root, &project, None).unwrap().len(), 1);
        assert!(data.path().join(project_key(project.root())).is_dir());
    }
}
