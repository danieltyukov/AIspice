//! Atomic file writes.

use std::io::{self, Write};
use std::path::Path;

/// Write `contents` to `path` through a temporary file in the same directory
/// and a rename, so a crash never leaves a half-written file. `private`
/// files get mode 0600 (and a 0700 parent if one has to be created) on Unix.
pub(crate) fn write_atomic(path: &Path, contents: &[u8], private: bool) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    create_dir(dir, private)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(contents)?;
    tmp.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if private { 0o600 } else { 0o644 };
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(mode))?;
    }
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

fn create_dir(dir: &Path, private: bool) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(if private { 0o700 } else { 0o755 })
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        let _ = private;
        std::fs::create_dir_all(dir)
    }
}
