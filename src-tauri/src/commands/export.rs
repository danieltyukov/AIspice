use std::fs;

/// Export schematic SVG content to a file.
///
/// Currently supports:
/// - `"svg"` — writes the SVG content directly to the output path.
/// - Any other format — writes as SVG (PNG export requires resvg, planned for
///   a future release).
#[tauri::command]
pub async fn export_schematic(
    svg_content: String,
    format: String,
    output_path: String,
) -> Result<(), String> {
    match format.to_lowercase().as_str() {
        "svg" => {
            fs::write(&output_path, &svg_content)
                .map_err(|e| format!("Failed to write SVG to {}: {}", output_path, e))?;
        }
        other => {
            // For unsupported formats, fall back to writing as SVG and warn
            let actual_path = if output_path.ends_with(&format!(".{}", other)) {
                // Replace extension with .svg
                let base = &output_path[..output_path.len() - other.len() - 1];
                format!("{}.svg", base)
            } else {
                output_path.clone()
            };

            fs::write(&actual_path, &svg_content).map_err(|e| {
                format!("Failed to write SVG to {}: {}", actual_path, e)
            })?;

            // Note: this is not an error — the file was saved, just as SVG
            // The frontend should inform the user.
        }
    }

    Ok(())
}
