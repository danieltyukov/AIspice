use super::operations::{EditOperation, EditResponse};
use crate::formats::encoding;
use std::path::Path;

/// Try to parse an AI response as an edit JSON. Returns None if it's plain text.
pub fn try_parse_edit_response(text: &str) -> Option<EditResponse> {
    let trimmed = text.trim();

    // Try direct JSON parse
    if let Ok(resp) = serde_json::from_str::<EditResponse>(trimmed) {
        return Some(resp);
    }

    // Try extracting from ```json fences
    if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            let json_str = after[..end].trim();
            if let Ok(resp) = serde_json::from_str::<EditResponse>(json_str) {
                return Some(resp);
            }
        }
    }

    // Try finding a raw JSON object
    if let Some(start) = trimmed.find('{') {
        // Find the matching closing brace
        let candidate = &trimmed[start..];
        let mut depth = 0;
        let mut end_pos = None;
        for (i, ch) in candidate.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end_pos = Some(i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        if let Some(end) = end_pos {
            let json_str = &candidate[..end];
            if let Ok(resp) = serde_json::from_str::<EditResponse>(json_str) {
                return Some(resp);
            }
        }
    }

    None
}

/// Apply edit operations to an .asc file on disk.
/// Returns a summary of what was done.
pub fn apply_edits_to_file(
    working_dir: &str,
    filename: &str,
    operations: &[EditOperation],
) -> Result<String, String> {
    let path = Path::new(working_dir).join(filename);
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("Failed to read {}: {}", filename, e))?;
    let (content, enc) = encoding::detect_and_decode(&bytes);
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    let mut changes_made = 0;

    for op in operations {
        match op {
            EditOperation::SetValue { target, value } => {
                // Find the SYMBOL block with this InstName and change its Value
                let mut i = 0;
                while i < lines.len() {
                    if lines[i].starts_with("SYMBOL ") {
                        // Scan the following SYMATTR lines
                        let mut j = i + 1;
                        let mut inst_name_match = false;
                        let mut value_line = None;
                        while j < lines.len()
                            && (lines[j].starts_with("SYMATTR ")
                                || lines[j].starts_with("WINDOW "))
                        {
                            if lines[j].starts_with("SYMATTR InstName ")
                                && lines[j][17..].trim() == target.as_str()
                            {
                                inst_name_match = true;
                            }
                            if lines[j].starts_with("SYMATTR Value ") {
                                value_line = Some(j);
                            }
                            j += 1;
                        }
                        if inst_name_match {
                            if let Some(vl) = value_line {
                                lines[vl] = format!("SYMATTR Value {}", value);
                                changes_made += 1;
                            }
                        }
                    }
                    i += 1;
                }
            }

            EditOperation::AddComponent {
                comp_type,
                position,
                rotation,
                name,
                value,
            } => {
                // Add SYMBOL + WINDOW + SYMATTR lines before the first TEXT line (or at end)
                let insert_pos = lines
                    .iter()
                    .position(|l| l.starts_with("TEXT "))
                    .unwrap_or(lines.len());

                let mut new_lines = vec![
                    format!(
                        "SYMBOL {} {} {} {}",
                        comp_type, position[0], position[1], rotation
                    ),
                    format!("SYMATTR InstName {}", name),
                    format!("SYMATTR Value {}", value),
                ];
                // Insert in reverse order at insert_pos
                new_lines.reverse();
                for line in new_lines {
                    lines.insert(insert_pos, line);
                }
                changes_made += 1;
            }

            EditOperation::AddWire { from, to } => {
                // Add WIRE line after existing WIREs
                let insert_pos = lines
                    .iter()
                    .rposition(|l| l.starts_with("WIRE "))
                    .map(|p| p + 1)
                    .unwrap_or_else(|| {
                        // After SHEET line
                        lines
                            .iter()
                            .position(|l| l.starts_with("SHEET "))
                            .map(|p| p + 1)
                            .unwrap_or(2)
                    });
                lines.insert(
                    insert_pos,
                    format!("WIRE {} {} {} {}", from[0], from[1], to[0], to[1]),
                );
                changes_made += 1;
            }

            EditOperation::Remove { target } => {
                // Find and remove the SYMBOL block with this InstName
                let mut i = 0;
                while i < lines.len() {
                    if lines[i].starts_with("SYMBOL ") {
                        let block_start = i;
                        let mut j = i + 1;
                        let mut inst_name_match = false;
                        while j < lines.len()
                            && (lines[j].starts_with("SYMATTR ")
                                || lines[j].starts_with("WINDOW "))
                        {
                            if lines[j].starts_with("SYMATTR InstName ")
                                && lines[j][17..].trim() == target.as_str()
                            {
                                inst_name_match = true;
                            }
                            j += 1;
                        }
                        if inst_name_match {
                            lines.drain(block_start..j);
                            changes_made += 1;
                            continue; // Don't increment i since we removed lines
                        }
                    }
                    i += 1;
                }
            }

            EditOperation::MoveComponent { target, position } => {
                // Find the SYMBOL line with this InstName and change its coordinates
                let mut i = 0;
                while i < lines.len() {
                    if lines[i].starts_with("SYMBOL ") {
                        let sym_line = i;
                        let mut j = i + 1;
                        let mut inst_name_match = false;
                        while j < lines.len()
                            && (lines[j].starts_with("SYMATTR ")
                                || lines[j].starts_with("WINDOW "))
                        {
                            if lines[j].starts_with("SYMATTR InstName ")
                                && lines[j][17..].trim() == target.as_str()
                            {
                                inst_name_match = true;
                            }
                            j += 1;
                        }
                        if inst_name_match {
                            let parts: Vec<&str> =
                                lines[sym_line].split_whitespace().collect();
                            if parts.len() >= 5 {
                                lines[sym_line] = format!(
                                    "SYMBOL {} {} {} {}",
                                    parts[1], position[0], position[1], parts[4]
                                );
                                changes_made += 1;
                            }
                        }
                    }
                    i += 1;
                }
            }

            EditOperation::AddFlag { position, label } => {
                let insert_pos = lines
                    .iter()
                    .rposition(|l| l.starts_with("FLAG "))
                    .map(|p| p + 1)
                    .unwrap_or_else(|| {
                        lines
                            .iter()
                            .rposition(|l| l.starts_with("WIRE "))
                            .map(|p| p + 1)
                            .unwrap_or(2)
                    });
                lines.insert(
                    insert_pos,
                    format!("FLAG {} {} {}", position[0], position[1], label),
                );
                changes_made += 1;
            }

            EditOperation::AddDirective { position, text } => {
                let directive_text = if text.starts_with('!') || text.starts_with('.') {
                    if text.starts_with('.') {
                        format!("!{}", text)
                    } else {
                        text.clone()
                    }
                } else {
                    format!("!{}", text)
                };
                lines.push(format!(
                    "TEXT {} {} Left 2 {}",
                    position[0], position[1], directive_text
                ));
                changes_made += 1;
            }

            EditOperation::RemoveText { text } => {
                // Remove TEXT lines containing the specified text
                let search = text.trim();
                let before = lines.len();
                lines.retain(|l| {
                    if l.starts_with("TEXT ") {
                        // Check if this TEXT line contains the search string
                        !l.contains(search)
                    } else {
                        true
                    }
                });
                let removed = before - lines.len();
                if removed > 0 {
                    changes_made += removed;
                }
            }

            EditOperation::RemoveWire { from, to } => {
                // Remove WIRE lines matching these coordinates
                let wire_str = format!("WIRE {} {} {} {}", from[0], from[1], to[0], to[1]);
                let wire_str_rev = format!("WIRE {} {} {} {}", to[0], to[1], from[0], from[1]);
                let before = lines.len();
                lines.retain(|l| l.trim() != wire_str && l.trim() != wire_str_rev);
                let removed = before - lines.len();
                if removed > 0 {
                    changes_made += removed;
                }
            }
        }
    }

    if changes_made == 0 {
        return Err("No changes were applied.".to_string());
    }

    // Write back
    let output = lines.join("\n") + "\n";
    let encoded = encoding::encode(&output, enc);
    std::fs::write(&path, encoded)
        .map_err(|e| format!("Failed to write {}: {}", filename, e))?;

    Ok(format!("{} edit(s) applied to {}", changes_made, filename))
}
