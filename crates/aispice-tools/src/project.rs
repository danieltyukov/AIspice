//! A project: a folder of circuits that aispice may read and write.
//!
//! Every path a tool receives is resolved here and must land inside the
//! project root after symlinks are followed; anything else is refused before
//! a file is touched. Saves are atomic (write to a temporary file, then
//! rename) and every save records a snapshot, so any edit can be undone, and
//! a change made outside aispice (in LTspice, say) is captured as its own
//! snapshot before the next edit so undo never loses it.

use aispice_core::schematic::{self, Schematic};
use aispice_core::symbol::{SymbolLibrary, SymbolSource};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// The folder aispice keeps its own state in, inside the project.
pub const STATE_DIR: &str = ".aispice";

/// File types listed as circuits.
const CIRCUIT_EXTS: &[&str] = &["asc", "cir", "net", "sp", "spi", "spice", "ckt"];
/// Folders never searched for circuits.
const SKIP_DIRS: &[&str] = &[
    STATE_DIR,
    ".git",
    "node_modules",
    "target",
    "dist",
    ".venv",
    "__pycache__",
];

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("`{0}` is outside the project folder")]
    Outside(String),
    #[error("`{0}` does not exist in the project")]
    Missing(String),
    #[error("`{0}` is not a schematic (.asc)")]
    NotSchematic(String),
    #[error("could not read or write {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("nothing to {0}")]
    NoHistory(&'static str),
    #[error("{0}")]
    Other(String),
}

fn io(path: &Path, source: std::io::Error) -> ProjectError {
    ProjectError::Io {
        path: path.display().to_string(),
        source,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CircuitEntry {
    /// Relative to the project root, `/` separated.
    pub path: String,
    pub name: String,
    /// Unix milliseconds.
    pub modified: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub id: String,
    pub time: u64,
    pub summary: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct HistoryIndex {
    snapshots: Vec<Snapshot>,
    /// Index of the snapshot that matches the file on disk.
    current: usize,
}

/// Settings that shape a project's symbol library and simulators.
#[derive(Debug, Clone, Default)]
pub struct ProjectOptions {
    pub extra_symbol_dirs: Vec<PathBuf>,
    pub ltspice_lib: Option<PathBuf>,
}

pub struct Project {
    root: PathBuf,
    lib: Arc<SymbolLibrary>,
    options: ProjectOptions,
    /// Serialises saves and history updates.
    write_lock: Mutex<()>,
}

impl std::fmt::Debug for Project {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Project").field("root", &self.root).finish()
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Project {
    pub fn open(root: &Path, options: ProjectOptions) -> Result<Self, ProjectError> {
        let root = std::fs::canonicalize(root).map_err(|e| io(root, e))?;
        if !root.is_dir() {
            return Err(ProjectError::Other(format!(
                "{} is not a folder",
                root.display()
            )));
        }
        let lib = Arc::new(SymbolLibrary::for_project(
            Some(&root),
            &options.extra_symbol_dirs,
            options.ltspice_lib.as_deref(),
        ));
        Ok(Self {
            root,
            lib,
            options,
            write_lock: Mutex::new(()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".into())
    }

    pub fn library(&self) -> &Arc<SymbolLibrary> {
        &self.lib
    }

    pub fn options(&self) -> &ProjectOptions {
        &self.options
    }

    /// Folders a simulated deck may read files from.
    pub fn allowed_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.root.clone()];
        dirs.extend(self.options.extra_symbol_dirs.iter().cloned());
        if let Some(l) = &self.options.ltspice_lib {
            dirs.push(l.clone());
            if let Some(parent) = l.parent() {
                dirs.push(parent.join("sub"));
                dirs.push(parent.join("cmp"));
            }
        }
        dirs.extend(
            self.lib
                .dirs()
                .into_iter()
                .filter(|(_, s)| *s != SymbolSource::Project)
                .map(|(d, _)| d),
        );
        dirs
    }

    /// Resolve a project-relative path, refusing anything that escapes the
    /// root, including through symlinks. The file need not exist yet, but its
    /// parent folder must.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf, ProjectError> {
        let rel = rel.trim();
        let p = Path::new(rel);
        if rel.is_empty() || rel.contains('\0') {
            return Err(ProjectError::Outside(rel.to_string()));
        }
        let candidate = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(p)
        };
        if p.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(ProjectError::Outside(rel.to_string()));
        }
        let real = match std::fs::canonicalize(&candidate) {
            Ok(r) => r,
            Err(_) => {
                let parent = candidate
                    .parent()
                    .ok_or_else(|| ProjectError::Outside(rel.to_string()))?;
                let real_parent = std::fs::canonicalize(parent)
                    .map_err(|_| ProjectError::Missing(rel.to_string()))?;
                real_parent.join(
                    candidate
                        .file_name()
                        .ok_or_else(|| ProjectError::Outside(rel.to_string()))?,
                )
            }
        };
        if !real.starts_with(&self.root) || real.starts_with(self.root.join(STATE_DIR)) {
            return Err(ProjectError::Outside(rel.to_string()));
        }
        Ok(real)
    }

    /// The project-relative form of an absolute path inside the project.
    pub fn relative(&self, abs: &Path) -> String {
        abs.strip_prefix(&self.root)
            .unwrap_or(abs)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// Circuit files, newest first, at most four folders deep.
    pub fn circuits(&self) -> Vec<CircuitEntry> {
        let mut out = Vec::new();
        walk(&self.root, &self.root, 4, &mut out);
        out.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.path.cmp(&b.path)));
        out
    }

    pub fn read_text(&self, rel: &str) -> Result<String, ProjectError> {
        let path = self.resolve(rel)?;
        let bytes = std::fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ProjectError::Missing(rel.into())
            } else {
                io(&path, e)
            }
        })?;
        Ok(aispice_core::encoding::decode(&bytes).0)
    }

    pub fn load(
        &self,
        rel: &str,
    ) -> Result<(Schematic, Vec<schematic::ParseWarning>), ProjectError> {
        if !rel.to_ascii_lowercase().ends_with(".asc") {
            return Err(ProjectError::NotSchematic(rel.to_string()));
        }
        let path = self.resolve(rel)?;
        let bytes = std::fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ProjectError::Missing(rel.into())
            } else {
                io(&path, e)
            }
        })?;
        Ok(schematic::parse_bytes(&bytes))
    }

    fn history_dir(&self, rel: &str) -> PathBuf {
        let safe: String = rel
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.root.join(STATE_DIR).join("history").join(safe)
    }

    fn read_index(&self, rel: &str) -> HistoryIndex {
        std::fs::read(self.history_dir(rel).join("index.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn write_index(&self, rel: &str, idx: &HistoryIndex) -> Result<(), ProjectError> {
        let dir = self.history_dir(rel);
        std::fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
        let json =
            serde_json::to_vec_pretty(idx).map_err(|e| ProjectError::Other(e.to_string()))?;
        atomic_write(&dir.join("index.json"), &json)
    }

    fn snapshot_path(&self, rel: &str, id: &str) -> PathBuf {
        self.history_dir(rel).join(format!("{id}.asc"))
    }

    /// Record the file as it is now if it differs from the last snapshot
    /// (a first sight, or a change made outside aispice).
    fn capture_external(&self, rel: &str, idx: &mut HistoryIndex) -> Result<(), ProjectError> {
        let path = self.resolve(rel)?;
        let Ok(on_disk) = std::fs::read(&path) else {
            return Ok(());
        };
        let last = idx
            .snapshots
            .get(idx.current)
            .and_then(|s| std::fs::read(self.snapshot_path(rel, &s.id)).ok());
        if last.as_deref() != Some(on_disk.as_slice()) {
            let summary = if idx.snapshots.is_empty() {
                "Opened"
            } else {
                "Changed outside aispice"
            };
            self.push_snapshot(rel, idx, &on_disk, summary)?;
        }
        Ok(())
    }

    fn push_snapshot(
        &self,
        rel: &str,
        idx: &mut HistoryIndex,
        bytes: &[u8],
        summary: &str,
    ) -> Result<String, ProjectError> {
        if !idx.snapshots.is_empty() {
            idx.snapshots.truncate(idx.current + 1);
        }
        let time = now_ms();
        let id = format!("{time}-{}", idx.snapshots.len());
        let dir = self.history_dir(rel);
        std::fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
        atomic_write(&self.snapshot_path(rel, &id), bytes)?;
        idx.snapshots.push(Snapshot {
            id: id.clone(),
            time,
            summary: summary.to_string(),
        });
        idx.current = idx.snapshots.len() - 1;
        // Keep the history bounded: 200 snapshots per circuit.
        while idx.snapshots.len() > 200 {
            let old = idx.snapshots.remove(0);
            let _ = std::fs::remove_file(self.snapshot_path(rel, &old.id));
            idx.current -= 1;
        }
        Ok(id)
    }

    /// Save a schematic, recording the previous and new states. Returns the
    /// new snapshot id.
    pub fn save(&self, rel: &str, sch: &Schematic, summary: &str) -> Result<String, ProjectError> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| ProjectError::Other("project lock poisoned".into()))?;
        let path = self.resolve(rel)?;
        let mut idx = self.read_index(rel);
        self.capture_external(rel, &mut idx)?;
        let bytes = schematic::write_bytes(sch);
        atomic_write(&path, &bytes)?;
        let id = self.push_snapshot(rel, &mut idx, &bytes, summary)?;
        self.write_index(rel, &idx)?;
        Ok(id)
    }

    /// Create a new, empty schematic file. Refuses to overwrite.
    pub fn create(&self, rel: &str, sch: &Schematic) -> Result<String, ProjectError> {
        if !rel.to_ascii_lowercase().ends_with(".asc") {
            return Err(ProjectError::NotSchematic(rel.to_string()));
        }
        let path = self.resolve(rel)?;
        if path.exists() {
            return Err(ProjectError::Other(format!("{rel} already exists")));
        }
        self.save(rel, sch, "Created")
    }

    pub fn history(&self, rel: &str) -> Result<(Vec<Snapshot>, usize), ProjectError> {
        self.resolve(rel)?;
        let idx = self.read_index(rel);
        Ok((idx.snapshots, idx.current))
    }

    fn move_to(
        &self,
        rel: &str,
        target: impl FnOnce(&HistoryIndex) -> Option<usize>,
        what: &'static str,
    ) -> Result<Snapshot, ProjectError> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| ProjectError::Other("project lock poisoned".into()))?;
        let path = self.resolve(rel)?;
        let mut idx = self.read_index(rel);
        self.capture_external(rel, &mut idx)?;
        let to = target(&idx).ok_or(ProjectError::NoHistory(what))?;
        let snap = idx
            .snapshots
            .get(to)
            .cloned()
            .ok_or(ProjectError::NoHistory(what))?;
        let bytes = std::fs::read(self.snapshot_path(rel, &snap.id)).map_err(|e| io(&path, e))?;
        atomic_write(&path, &bytes)?;
        idx.current = to;
        self.write_index(rel, &idx)?;
        Ok(snap)
    }

    pub fn undo(&self, rel: &str) -> Result<Snapshot, ProjectError> {
        self.move_to(
            rel,
            |i| i.current.checked_sub(1).filter(|_| i.snapshots.len() > 1),
            "undo",
        )
    }

    pub fn redo(&self, rel: &str) -> Result<Snapshot, ProjectError> {
        self.move_to(
            rel,
            |i| (i.current + 1 < i.snapshots.len()).then_some(i.current + 1),
            "redo",
        )
    }

    pub fn restore(&self, rel: &str, id: &str) -> Result<Snapshot, ProjectError> {
        let id = id.to_string();
        self.move_to(
            rel,
            move |i| i.snapshots.iter().position(|s| s.id == id),
            "restore",
        )
    }

    pub fn can_undo_redo(&self, rel: &str) -> (bool, bool) {
        let idx = self.read_index(rel);
        (
            idx.current > 0 && idx.snapshots.len() > 1,
            idx.current + 1 < idx.snapshots.len(),
        )
    }

    /// A private scratch folder for one simulation run, inside the state dir.
    pub fn run_dir(&self, run_id: &str) -> Result<PathBuf, ProjectError> {
        let safe: String = run_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let dir = self.root.join(STATE_DIR).join("runs").join(safe);
        std::fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
        Ok(dir)
    }
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<CircuitEntry>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            if depth > 1 && !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                walk(root, &path, depth - 1, out);
            }
            continue;
        }
        let ext = path
            .extension()
            .map(|x| x.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if CIRCUIT_EXTS.contains(&ext.as_str()) {
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(CircuitEntry {
                path: rel,
                name,
                modified,
            });
        }
    }
}

/// Write a file by writing a sibling temporary file and renaming it over the
/// target, so a crash never leaves a half-written schematic.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ProjectError> {
    let dir = path
        .parent()
        .ok_or_else(|| ProjectError::Other(format!("{} has no parent folder", path.display())))?;
    let tmp = dir.join(format!(
        ".{}.aispice-tmp",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    std::fs::write(&tmp, bytes).map_err(|e| io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io(path, e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aispice_core::edit::{EditOp, apply};

    const RC: &str = "Version 4\nSHEET 1 880 680\nWIRE 96 96 32 96\nFLAG 32 176 0\nSYMBOL voltage 32 80 R0\nSYMATTR InstName V1\nSYMATTR Value 1\nSYMBOL res 192 80 R90\nSYMATTR InstName R1\nSYMATTR Value 1k\nTEXT 0 232 Left 2 !.op\n";

    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("rc.asc"), RC).unwrap();
        std::fs::write(dir.path().join("sub/deck.cir"), "t\nR1 a 0 1\n.op\n").unwrap();
        let p = Project::open(dir.path(), ProjectOptions::default()).unwrap();
        (dir, p)
    }

    #[test]
    fn lists_circuits_and_skips_state() {
        let (_d, p) = project();
        std::fs::create_dir_all(p.root().join(".aispice/history")).unwrap();
        std::fs::write(p.root().join(".aispice/history/x.asc"), RC).unwrap();
        let names: Vec<String> = p.circuits().into_iter().map(|c| c.path).collect();
        assert!(names.contains(&"rc.asc".to_string()));
        assert!(names.contains(&"sub/deck.cir".to_string()));
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[test]
    fn paths_cannot_escape() {
        let (_d, p) = project();
        for bad in [
            "../x.asc",
            "/etc/passwd",
            "sub/../../x",
            ".aispice/history/index.json",
            "",
            "a\0b",
        ] {
            assert!(p.resolve(bad).is_err(), "{bad}");
        }
        assert!(p.resolve("rc.asc").is_ok());
        assert!(p.resolve("new.asc").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_out_of_the_project_are_refused() {
        let (_d, p) = project();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.asc"), RC).unwrap();
        std::os::unix::fs::symlink(outside.path(), p.root().join("link")).unwrap();
        assert!(p.resolve("link/secret.asc").is_err());
    }

    #[test]
    fn save_undo_redo_round_trip() {
        let (_d, p) = project();
        let (mut sch, _) = p.load("rc.asc").unwrap();
        let ops: Vec<EditOp> =
            serde_json::from_str(r#"[{"op":"set_value","name":"R1","value":"2.2k"}]"#).unwrap();
        apply(&mut sch, p.library(), &ops).unwrap();
        p.save("rc.asc", &sch, "R1 to 2.2k").unwrap();
        assert!(p.read_text("rc.asc").unwrap().contains("2.2k"));
        assert_eq!(p.can_undo_redo("rc.asc"), (true, false));
        p.undo("rc.asc").unwrap();
        assert!(p.read_text("rc.asc").unwrap().contains("SYMATTR Value 1k"));
        p.redo("rc.asc").unwrap();
        assert!(p.read_text("rc.asc").unwrap().contains("2.2k"));
        let (snaps, current) = p.history("rc.asc").unwrap();
        assert_eq!(snaps.len(), 2);
        assert_eq!(current, 1);
        assert_eq!(snaps[0].summary, "Opened");
    }

    #[test]
    fn external_changes_are_captured_before_the_next_edit() {
        let (_d, p) = project();
        let (sch, _) = p.load("rc.asc").unwrap();
        p.save("rc.asc", &sch, "first save").unwrap();
        // Someone edits the file in LTspice.
        let edited = RC.replace("SYMATTR Value 1k", "SYMATTR Value 47k");
        std::fs::write(p.root().join("rc.asc"), &edited).unwrap();
        let (mut sch2, _) = p.load("rc.asc").unwrap();
        sch2.symbol_mut("V1").unwrap().set_attr("Value", "5");
        p.save("rc.asc", &sch2, "V1 to 5").unwrap();
        let (snaps, _) = p.history("rc.asc").unwrap();
        let summaries: Vec<&str> = snaps.iter().map(|s| s.summary.as_str()).collect();
        assert!(
            summaries.contains(&"Changed outside aispice"),
            "{summaries:?}"
        );
        p.undo("rc.asc").unwrap();
        assert!(
            p.read_text("rc.asc").unwrap().contains("47k"),
            "undo returns to the outside edit"
        );
    }
}
