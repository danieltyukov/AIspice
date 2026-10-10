use std::process::Command;
use tauri::State;

use crate::state::AppState;

/// Find the LTspice exe path in common Wine locations.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
fn home_dir_ltspice_path() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let candidates = [
        format!("{}/.wine/drive_c/Program Files/LTC/LTspiceXVII/XVIIx64.exe", home),
        format!("{}/LTspice64.exe", home),
        format!("{}/.wine/drive_c/Program Files/ADI/LTspice/LTspice.exe", home),
    ];
    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return Some(path.clone());
        }
    }
    None
}

/// Information about a detected LTspice installation.
#[derive(serde::Serialize)]
pub struct LtspiceInfo {
    pub found: bool,
    pub path: String,
    /// "native", "wine", or "not_found"
    pub method: String,
}

/// Detect if LTspice is installed and return its path and method.
#[tauri::command]
pub fn detect_ltspice(state: State<'_, AppState>) -> Result<LtspiceInfo, String> {
    // Check if we already cached the path
    {
        let cached = state
            .ltspice_path
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if let Some(ref path) = *cached {
            let method = if path.contains("wine") || path.contains(".wine") {
                "wine"
            } else {
                "native"
            };
            return Ok(LtspiceInfo {
                found: true,
                path: path.clone(),
                method: method.to_string(),
            });
        }
    }

    let info = detect_ltspice_impl();

    // Cache the result if found
    if info.found {
        if let Ok(mut cached) = state.ltspice_path.lock() {
            *cached = Some(info.path.clone());
        }
    }

    Ok(info)
}

fn detect_ltspice_impl() -> LtspiceInfo {
    let not_found = LtspiceInfo {
        found: false,
        path: String::new(),
        method: "not_found".to_string(),
    };

    #[cfg(target_os = "linux")]
    {
        // 1. Check if XVIIx64.exe is already running via Wine
        if let Ok(output) = Command::new("ps").args(["aux"]).output() {
            let ps_output = String::from_utf8_lossy(&output.stdout);
            for line in ps_output.lines() {
                if line.contains("XVIIx64.exe") || line.contains("LTspice") {
                    // Extract the path from the ps line if possible
                    if let Some(idx) = line.find("XVIIx64.exe") {
                        // Walk backward to find the start of the path
                        let before = &line[..idx + "XVIIx64.exe".len()];
                        if let Some(path_start) = before.rfind('/') {
                            // Walk further back to find the wine prefix path
                            let candidate = &before[..=path_start + "XVIIx64.exe".len()];
                            // Just use the known Wine path
                            let _ = candidate;
                        }
                    }
                }
            }
        }

        // 2. Check common Wine paths
        let home = std::env::var("HOME").unwrap_or_default();
        let wine_paths = [
            format!(
                "{}/.wine/drive_c/Program Files/LTC/LTspiceXVII/XVIIx64.exe",
                home
            ),
            format!(
                "{}/.wine/drive_c/Program Files/ADI/LTspice/LTspice.exe",
                home
            ),
            format!("{}/LTspice64.exe", home),
        ];

        for path in &wine_paths {
            if std::path::Path::new(path).exists() {
                return LtspiceInfo {
                    found: true,
                    path: path.clone(),
                    method: "wine".to_string(),
                };
            }
        }

        // 3. Check if wine is at least available
        if let Ok(output) = Command::new("which").arg("wine").output() {
            if output.status.success() {
                // Wine is available but no LTspice exe found
                return not_found;
            }
        }

        return not_found;
    }

    #[cfg(target_os = "macos")]
    {
        let native_paths = [
            "/Applications/LTspice.app",
            "/Applications/LTspice.app/Contents/MacOS/LTspice",
        ];

        for path in &native_paths {
            if std::path::Path::new(path).exists() {
                return LtspiceInfo {
                    found: true,
                    path: path.to_string(),
                    method: "native".to_string(),
                };
            }
        }

        return not_found;
    }

    #[cfg(target_os = "windows")]
    {
        let native_paths = [
            r"C:\Program Files\LTC\LTspiceXVII\XVIIx64.exe",
            r"C:\Program Files\ADI\LTspice\LTspice.exe",
            r"C:\Program Files (x86)\LTC\LTspiceIV\scad3.exe",
        ];

        for path in &native_paths {
            if std::path::Path::new(path).exists() {
                return LtspiceInfo {
                    found: true,
                    path: path.to_string(),
                    method: "native".to_string(),
                };
            }
        }

        return not_found;
    }

    #[allow(unreachable_code)]
    not_found
}

/// Trigger LTspice to reload / open a file.
#[tauri::command]
pub async fn reload_ltspice(
    file_path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let ltspice_path = {
        let cached = state
            .ltspice_path
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        cached.clone()
    };

    #[cfg(target_os = "linux")]
    {
        // Try xdotool first to send keystroke to existing window
        if let Ok(output) = Command::new("xdotool")
            .args(["search", "--name", "LTspice"])
            .output()
        {
            let window_ids = String::from_utf8_lossy(&output.stdout);
            let window_ids: Vec<&str> = window_ids.trim().lines().collect();
            if !window_ids.is_empty() {
                // Activate the window and send Ctrl+O then the file path
                // Simpler: just re-open the file via wine
                if let Some(ref exe_path) = ltspice_path {
                    let _ = Command::new("wine")
                        .arg(exe_path)
                        .arg(&file_path)
                        .spawn();
                    return Ok(());
                }
            }
        }

        // Fallback: open the file with the detected LTspice path
        if let Some(ref exe_path) = ltspice_path {
            Command::new("wine")
                .arg(exe_path)
                .arg(&file_path)
                .spawn()
                .map_err(|e| format!("Failed to launch LTspice: {}", e))?;
            return Ok(());
        }

        return Err("LTspice path not detected. Run detect_ltspice first.".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let script = format!(
            r#"
            tell application "LTspice"
                activate
                open POSIX file "{}"
            end tell
            "#,
            file_path
        );
        Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| format!("AppleScript failed: {}", e))?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(ref exe_path) = ltspice_path {
            Command::new(exe_path)
                .arg(&file_path)
                .spawn()
                .map_err(|e| format!("Failed to launch LTspice: {}", e))?;
            return Ok(());
        }
        return Err("LTspice path not detected. Run detect_ltspice first.".to_string());
    }

    #[allow(unreachable_code)]
    Err("Unsupported platform".to_string())
}

/// Internal non-async helper to reload LTspice. Used by chat command after edits.
/// Best-effort — silently ignores failures.
pub fn reload_ltspice_internal(file_path: &str) {
    #[cfg(target_os = "linux")]
    {
        // Find existing LTspice window and close+reopen the file within it
        // (avoids spawning a new LTspice session)
        if let Ok(output) = Command::new("xdotool")
            .args(["search", "--name", "LTspice XVII"])
            .output()
        {
            let window_ids = String::from_utf8_lossy(&output.stdout);
            if let Some(wid) = window_ids.trim().lines().next() {
                // Focus the LTspice window
                let _ = Command::new("xdotool")
                    .args(["windowactivate", "--sync", wid])
                    .output();
                std::thread::sleep(std::time::Duration::from_millis(300));

                // Close current file (Ctrl+W), then reopen it (Ctrl+O)
                let _ = Command::new("xdotool")
                    .args(["key", "--clearmodifiers", "ctrl+w"])
                    .output();
                std::thread::sleep(std::time::Duration::from_millis(500));

                // Convert Unix path to Wine path and open via Ctrl+O
                let win_path = if let Ok(wp) = Command::new("winepath")
                    .arg("-w")
                    .arg(file_path)
                    .output()
                {
                    String::from_utf8_lossy(&wp.stdout).trim().to_string()
                } else {
                    file_path.to_string()
                };

                let _ = Command::new("xdotool")
                    .args(["key", "--clearmodifiers", "ctrl+o"])
                    .output();
                std::thread::sleep(std::time::Duration::from_millis(500));

                // Type the file path into the open dialog and press Enter
                let _ = Command::new("xdotool")
                    .args(["type", "--clearmodifiers", &win_path])
                    .output();
                std::thread::sleep(std::time::Duration::from_millis(200));
                let _ = Command::new("xdotool")
                    .args(["key", "Return"])
                    .output();
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let script = format!(
            r#"tell application "LTspice" to open POSIX file "{}""#,
            _file_path
        );
        let _ = Command::new("osascript").arg("-e").arg(&script).output();
    }

    #[cfg(target_os = "windows")]
    {
        // Find LTspice window and send F5
        let _ = Command::new("powershell")
            .arg("-Command")
            .arg(r#"
                Add-Type -AssemblyName System.Windows.Forms
                $wshell = New-Object -ComObject wscript.shell
                $proc = Get-Process -Name "XVIIx64" -ErrorAction SilentlyContinue
                if ($proc) {
                    $wshell.AppActivate($proc.MainWindowTitle)
                    Start-Sleep -Milliseconds 200
                    [System.Windows.Forms.SendKeys]::SendWait("^r")
                }
            "#)
            .output();
    }
}

/// Run LTspice simulation in batch mode.
#[tauri::command]
pub async fn run_ltspice_batch(
    file_path: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let path = std::path::Path::new(&file_path);
    if !path.exists() {
        return Err(format!("File not found: {}", file_path));
    }

    // Get or detect LTspice path
    let exe_path = {
        let cached = state
            .ltspice_path
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        cached.clone()
    };

    let exe_path = match exe_path {
        Some(p) => p,
        None => {
            // Try detection
            let info = detect_ltspice_impl();
            if !info.found {
                return Err("LTspice not found. Install it or configure the path.".to_string());
            }
            // Cache the result
            if let Ok(mut cached) = state.ltspice_path.lock() {
                *cached = Some(info.path.clone());
            }
            info.path
        }
    };

    #[cfg(target_os = "linux")]
    {
        let output = Command::new("wine")
            .arg(&exe_path)
            .arg("-b")
            .arg(&file_path)
            .output()
            .map_err(|e| format!("Failed to run LTspice via Wine: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            return Ok(format!("Simulation complete.\n{}{}", stdout, stderr));
        } else {
            // Wine often returns non-zero even on success, check for log file
            let log_path = file_path.replace(".asc", ".log");
            if std::path::Path::new(&log_path).exists() {
                return Ok(format!(
                    "Simulation finished (Wine exit code non-zero but log generated).\n{}{}",
                    stdout, stderr
                ));
            } else {
                return Err(format!("Simulation failed:\n{}{}", stdout, stderr));
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let output = Command::new(&exe_path)
            .arg("-b")
            .arg(&file_path)
            .output()
            .map_err(|e| format!("Failed to run LTspice: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            return Ok(format!("Simulation complete.\n{}{}", stdout, stderr));
        } else {
            return Err(format!("Simulation failed:\n{}{}", stdout, stderr));
        }
    }

    #[cfg(target_os = "windows")]
    {
        let output = Command::new(&exe_path)
            .arg("-b")
            .arg(&file_path)
            .output()
            .map_err(|e| format!("Failed to run LTspice: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            return Ok(format!("Simulation complete.\n{}{}", stdout, stderr));
        } else {
            return Err(format!("Simulation failed:\n{}{}", stdout, stderr));
        }
    }

    #[allow(unreachable_code)]
    Err("Unsupported platform".to_string())
}

/// Read a simulation log file (.log) that LTspice generates.
#[tauri::command]
pub fn read_simulation_log(file_path: String) -> Result<String, String> {
    let log_path = file_path.replace(".asc", ".log");
    std::fs::read_to_string(&log_path).map_err(|e| format!("Could not read log: {}", e))
}
