//! Opening a circuit in LTspice, and showing it again after aispice edits it.
//!
//! LTspice has no reload command line switch. Opening the file again brings
//! its window forward with the saved version, which is what "reload" means
//! here. Every process is started with an argument vector, never a shell.

use std::path::{Path, PathBuf};
use std::process::Command;

fn find_exe(configured: Option<&Path>) -> Option<PathBuf> {
    let lt = aispice_sim::backend::ltspice::Ltspice {
        exe: configured.map(Path::to_path_buf),
        ..Default::default()
    };
    lt.find().map(|i| i.exe)
}

pub fn open(file: &Path, configured: Option<&Path>) -> Result<(), String> {
    let exe =
        find_exe(configured).ok_or("LTspice is not installed (or set its path in Settings)")?;
    let mut cmd = if cfg!(target_os = "linux") {
        let mut c = Command::new("wine");
        c.arg(&exe);
        c.env("WINEDEBUG", "-all");
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg("-a").arg(&exe).arg("--args");
        c
    } else {
        Command::new(&exe)
    };
    cmd.arg(file);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start LTspice: {e}"))
}

/// Whether an LTspice process is running, so edits only bring LTspice back
/// when the user already has it open.
pub fn is_running() -> bool {
    #[cfg(target_os = "linux")]
    {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return false;
        };
        for e in entries.flatten() {
            if let Ok(cmd) = std::fs::read(e.path().join("cmdline")) {
                let cmd = String::from_utf8_lossy(&cmd).to_ascii_lowercase();
                if cmd.contains("xviix64.exe") || cmd.contains("ltspice.exe") {
                    return true;
                }
            }
        }
        false
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("pgrep")
            .args(["-x", "LTspice"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "windows")]
    {
        ["LTspice.exe", "XVIIx64.exe"].iter().any(|name| {
            Command::new("tasklist")
                .args(["/FI", &format!("IMAGENAME eq {name}"), "/NH"])
                .output()
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .to_ascii_lowercase()
                        .contains(&name.to_ascii_lowercase())
                })
                .unwrap_or(false)
        })
    }
}
