use std::process::Command;

/// Check whether `ngspice` is available on the system PATH.
#[tauri::command]
pub fn check_ngspice() -> Result<bool, String> {
    match Command::new("ngspice").arg("--version").output() {
        Ok(output) => Ok(output.status.success()),
        Err(_) => Ok(false),
    }
}

/// Run an ngspice batch simulation.
///
/// Executes: `ngspice -b -r <output_path> <netlist_path>`
///
/// Returns the combined stdout + stderr output from ngspice.
#[tauri::command]
pub async fn run_simulation(
    netlist_path: String,
    output_path: String,
) -> Result<String, String> {
    let output = tokio::process::Command::new("ngspice")
        .arg("-b")
        .arg("-r")
        .arg(&output_path)
        .arg(&netlist_path)
        .output()
        .await
        .map_err(|e| format!("Failed to run ngspice: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let combined = if stderr.is_empty() {
        stdout
    } else {
        format!("{}\n--- stderr ---\n{}", stdout, stderr)
    };

    if output.status.success() {
        Ok(combined)
    } else {
        Err(format!(
            "ngspice exited with status {}\n{}",
            output.status, combined
        ))
    }
}

/// Parse CSV/TSV waveform data (e.g., exported from ngspice or LTspice).
///
/// Expects the first row to be headers (tab or comma separated) and
/// subsequent rows to be numeric data. Returns a JSON object:
/// ```json
/// {
///   "headers": ["time", "V(out)", ...],
///   "rows": [[0.0, 1.23, ...], [0.001, 1.45, ...], ...]
/// }
/// ```
#[tauri::command]
pub fn parse_waveform_csv(content: String) -> Result<serde_json::Value, String> {
    let mut lines = content.lines();

    // Detect separator from the header line
    let header_line = lines
        .next()
        .ok_or_else(|| "Empty waveform data".to_string())?;

    let separator = if header_line.contains('\t') {
        '\t'
    } else {
        ','
    };

    let headers: Vec<String> = header_line
        .split(separator)
        .map(|h| h.trim().to_string())
        .collect();

    let mut rows: Vec<Vec<f64>> = Vec::new();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let values: Vec<f64> = trimmed
            .split(separator)
            .map(|v| v.trim().parse::<f64>().unwrap_or(f64::NAN))
            .collect();

        rows.push(values);
    }

    Ok(serde_json::json!({
        "headers": headers,
        "rows": rows,
    }))
}
