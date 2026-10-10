use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AscFile {
    pub version: i32,
    pub sheet_id: i32,
    pub sheet_width: i32,
    pub sheet_height: i32,
    pub wires: Vec<AscWire>,
    pub flags: Vec<AscFlag>,
    pub symbols: Vec<AscSymbol>,
    pub texts: Vec<AscText>,
    /// Lines that were not recognised by the parser — kept so we can
    /// round-trip a file without silently dropping content.
    #[serde(default)]
    pub unknown_lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AscWire {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AscFlag {
    pub x: i32,
    pub y: i32,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AscSymbol {
    pub symbol_type: String,
    pub x: i32,
    pub y: i32,
    pub rotation: String,
    pub windows: Vec<AscWindow>,
    pub attributes: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AscWindow {
    pub index: i32,
    pub x: i32,
    pub y: i32,
    pub alignment: String,
    pub font_size: i32,
    /// Optional extra text that may appear after the font size.
    #[serde(default)]
    pub extra: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AscText {
    pub x: i32,
    pub y: i32,
    pub alignment: String,
    pub font_size: i32,
    pub content: String,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse an LTspice `.asc` file from its textual content.
pub fn parse_asc(content: &str) -> Result<AscFile, String> {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    let mut version: i32 = 4; // sensible default
    let mut sheet_id: i32 = 1;
    let mut sheet_width: i32 = 880;
    let mut sheet_height: i32 = 580;
    let mut wires: Vec<AscWire> = Vec::new();
    let mut flags: Vec<AscFlag> = Vec::new();
    let mut symbols: Vec<AscSymbol> = Vec::new();
    let mut texts: Vec<AscText> = Vec::new();
    let mut unknown_lines: Vec<String> = Vec::new();

    let mut i = 0;
    while i < total {
        let line = lines[i].trim();

        if line.is_empty() {
            i += 1;
            continue;
        }

        // --- Version ---
        if line.starts_with("Version") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                version = parts[1]
                    .parse()
                    .map_err(|_| format!("Invalid version number on line {}", i + 1))?;
            }
            i += 1;
            continue;
        }

        // --- SHEET ---
        if line.starts_with("SHEET") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                sheet_id = parts[1]
                    .parse()
                    .map_err(|_| format!("Invalid SHEET id on line {}", i + 1))?;
                sheet_width = parts[2]
                    .parse()
                    .map_err(|_| format!("Invalid SHEET width on line {}", i + 1))?;
                sheet_height = parts[3]
                    .parse()
                    .map_err(|_| format!("Invalid SHEET height on line {}", i + 1))?;
            }
            i += 1;
            continue;
        }

        // --- WIRE ---
        if line.starts_with("WIRE") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 5 {
                wires.push(AscWire {
                    x1: parts[1]
                        .parse()
                        .map_err(|_| format!("Invalid WIRE x1 on line {}", i + 1))?,
                    y1: parts[2]
                        .parse()
                        .map_err(|_| format!("Invalid WIRE y1 on line {}", i + 1))?,
                    x2: parts[3]
                        .parse()
                        .map_err(|_| format!("Invalid WIRE x2 on line {}", i + 1))?,
                    y2: parts[4]
                        .parse()
                        .map_err(|_| format!("Invalid WIRE y2 on line {}", i + 1))?,
                });
            }
            i += 1;
            continue;
        }

        // --- FLAG ---
        if line.starts_with("FLAG") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                flags.push(AscFlag {
                    x: parts[1]
                        .parse()
                        .map_err(|_| format!("Invalid FLAG x on line {}", i + 1))?,
                    y: parts[2]
                        .parse()
                        .map_err(|_| format!("Invalid FLAG y on line {}", i + 1))?,
                    label: parts[3..].join(" "),
                });
            }
            i += 1;
            continue;
        }

        // --- SYMBOL (multi-line) ---
        if line.starts_with("SYMBOL") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 5 {
                let mut sym = AscSymbol {
                    symbol_type: parts[1].to_string(),
                    x: parts[2]
                        .parse()
                        .map_err(|_| format!("Invalid SYMBOL x on line {}", i + 1))?,
                    y: parts[3]
                        .parse()
                        .map_err(|_| format!("Invalid SYMBOL y on line {}", i + 1))?,
                    rotation: parts[4].to_string(),
                    windows: Vec::new(),
                    attributes: Vec::new(),
                };

                // Consume subsequent WINDOW / SYMATTR lines
                i += 1;
                while i < total {
                    let sub = lines[i].trim();
                    if sub.starts_with("WINDOW") {
                        let sp: Vec<&str> = sub.split_whitespace().collect();
                        if sp.len() >= 5 {
                            sym.windows.push(AscWindow {
                                index: sp[1].parse().unwrap_or(0),
                                x: sp[2].parse().unwrap_or(0),
                                y: sp[3].parse().unwrap_or(0),
                                alignment: sp[4].to_string(),
                                font_size: if sp.len() > 5 {
                                    sp[5].parse().unwrap_or(0)
                                } else {
                                    0
                                },
                                extra: if sp.len() > 6 {
                                    sp[6..].join(" ")
                                } else {
                                    String::new()
                                },
                            });
                        }
                        i += 1;
                    } else if sub.starts_with("SYMATTR") {
                        let sp: Vec<&str> = sub.split_whitespace().collect();
                        if sp.len() >= 3 {
                            let key = sp[1].to_string();
                            let val = sp[2..].join(" ");
                            sym.attributes.push((key, val));
                        }
                        i += 1;
                    } else {
                        break;
                    }
                }

                symbols.push(sym);
                continue; // i already advanced past the sub-lines
            }
            i += 1;
            continue;
        }

        // --- TEXT ---
        if line.starts_with("TEXT") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 5 {
                // TEXT x y align size rest...
                let text_content = if parts.len() > 5 {
                    // The content may begin with a ';' or '!' prefix — keep it.
                    parts[5..].join(" ")
                } else {
                    String::new()
                };

                texts.push(AscText {
                    x: parts[1]
                        .parse()
                        .map_err(|_| format!("Invalid TEXT x on line {}", i + 1))?,
                    y: parts[2]
                        .parse()
                        .map_err(|_| format!("Invalid TEXT y on line {}", i + 1))?,
                    alignment: parts[3].to_string(),
                    font_size: parts[4]
                        .parse()
                        .map_err(|_| format!("Invalid TEXT font_size on line {}", i + 1))?,
                    content: text_content,
                });
            }
            i += 1;
            continue;
        }

        // --- Unrecognised ---
        unknown_lines.push(line.to_string());
        i += 1;
    }

    Ok(AscFile {
        version,
        sheet_id,
        sheet_width,
        sheet_height,
        wires,
        flags,
        symbols,
        texts,
        unknown_lines,
    })
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Serialise an `AscFile` back to the LTspice `.asc` text format.
#[allow(dead_code)]
pub fn write_asc(file: &AscFile) -> String {
    let mut out = String::new();

    out.push_str(&format!("Version {}\n", file.version));
    out.push_str(&format!(
        "SHEET {} {} {}\n",
        file.sheet_id, file.sheet_width, file.sheet_height
    ));

    for w in &file.wires {
        out.push_str(&format!("WIRE {} {} {} {}\n", w.x1, w.y1, w.x2, w.y2));
    }

    for f in &file.flags {
        out.push_str(&format!("FLAG {} {} {}\n", f.x, f.y, f.label));
    }

    for s in &file.symbols {
        out.push_str(&format!(
            "SYMBOL {} {} {} {}\n",
            s.symbol_type, s.x, s.y, s.rotation
        ));
        for win in &s.windows {
            let extra_part = if win.extra.is_empty() {
                String::new()
            } else {
                format!(" {}", win.extra)
            };
            out.push_str(&format!(
                "WINDOW {} {} {} {} {}{}\n",
                win.index, win.x, win.y, win.alignment, win.font_size, extra_part
            ));
        }
        for (key, val) in &s.attributes {
            out.push_str(&format!("SYMATTR {} {}\n", key, val));
        }
    }

    for t in &file.texts {
        out.push_str(&format!(
            "TEXT {} {} {} {} {}\n",
            t.x, t.y, t.alignment, t.font_size, t.content
        ));
    }

    for u in &file.unknown_lines {
        out.push_str(u);
        out.push('\n');
    }

    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Version 4
SHEET 1 880 580
WIRE 112 160 48 160
WIRE 256 160 192 160
FLAG 48 224 0
SYMBOL res 192 144 R90
WINDOW 0 0 56 VBottom 2
WINDOW 3 32 56 VTop 2
SYMATTR InstName R1
SYMATTR Value 1k
TEXT -32 312 Left 2 !.tran 1m
";

    #[test]
    fn test_parse_basic() {
        let f = parse_asc(SAMPLE).unwrap();
        assert_eq!(f.version, 4);
        assert_eq!(f.sheet_width, 880);
        assert_eq!(f.wires.len(), 2);
        assert_eq!(f.flags.len(), 1);
        assert_eq!(f.flags[0].label, "0");
        assert_eq!(f.symbols.len(), 1);
        assert_eq!(f.symbols[0].symbol_type, "res");
        assert_eq!(f.symbols[0].windows.len(), 2);
        assert_eq!(f.symbols[0].attributes.len(), 2);
        assert_eq!(f.texts.len(), 1);
        assert!(f.texts[0].content.contains(".tran"));
    }

    #[test]
    fn test_roundtrip() {
        let f = parse_asc(SAMPLE).unwrap();
        let written = write_asc(&f);
        let f2 = parse_asc(&written).unwrap();
        assert_eq!(f.version, f2.version);
        assert_eq!(f.wires.len(), f2.wires.len());
        assert_eq!(f.symbols.len(), f2.symbols.len());
        assert_eq!(f.texts.len(), f2.texts.len());
    }
}
