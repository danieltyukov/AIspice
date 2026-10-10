//! Telling the UI when a circuit changes on disk, whoever changed it.

use notify::{EventKind, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub struct ProjectWatcher {
    _watcher: notify::RecommendedWatcher,
}

/// Watch `root` for circuit files changing and call `on_change` with the
/// changed file, at most once per file every 300 ms.
pub fn watch(
    root: &Path,
    on_change: impl Fn(PathBuf) + Send + 'static,
) -> Result<ProjectWatcher, String> {
    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(tx).map_err(|e| e.to_string())?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;
    let state_dir = root.join(aispice_tools::project::STATE_DIR);
    std::thread::spawn(move || {
        let mut last: std::collections::HashMap<PathBuf, Instant> = Default::default();
        for event in rx.into_iter().flatten() {
            if !matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
            ) {
                continue;
            }
            for path in event.paths {
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                if path.starts_with(&state_dir)
                    || !matches!(ext.as_str(), "asc" | "cir" | "net" | "sp" | "specs")
                {
                    continue;
                }
                let now = Instant::now();
                if last
                    .get(&path)
                    .is_some_and(|t| now.duration_since(*t) < Duration::from_millis(300))
                {
                    continue;
                }
                last.insert(path.clone(), now);
                on_change(path);
            }
        }
    });
    Ok(ProjectWatcher { _watcher: watcher })
}
