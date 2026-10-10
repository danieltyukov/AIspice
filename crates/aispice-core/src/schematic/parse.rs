use super::*;
use crate::encoding;

/// A line that was kept verbatim because it did not parse as what its keyword
/// promised. Parsing never fails on content; it reports instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseWarning {
    pub line: usize,
    pub message: String,
}

/// Decode and parse the bytes of an `.asc` file, remembering the encoding and
/// line endings for writing back.
pub fn parse_bytes(bytes: &[u8]) -> (Schematic, Vec<ParseWarning>) {
    let (text, enc) = encoding::decode(bytes);
    let (mut sch, warnings) = parse(&text);
    sch.format.encoding = enc;
    (sch, warnings)
}

/// Parse `.asc` text.
pub fn parse(text: &str) -> (Schematic, Vec<ParseWarning>) {
    let mut sch = Schematic::new();
    sch.format.line_ending = if text.contains("\r\n") {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    };
    let mut warnings = Vec::new();
    let mut seen_sheet = false;
    let mut current: Option<Symbol> = None;

    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (keyword, rest) = split_keyword(trimmed);

        // WINDOW and SYMATTR belong to the symbol above them.
        if matches!(keyword, "WINDOW" | "SYMATTR")
            && let Some(sym) = current.as_mut()
        {
            if keyword == "SYMATTR" {
                let (key, value) = split_keyword(rest);
                sym.attrs.push(Attr {
                    key: key.to_string(),
                    value: value.to_string(),
                });
            } else {
                match parse_window(rest) {
                    Some(w) => sym.windows.push(w),
                    None => {
                        warnings.push(warn(line_no, "unreadable WINDOW line kept as is"));
                        sym.extra.push(line.to_string());
                    }
                }
            }
            continue;
        }
        if let Some(sym) = current.take() {
            sch.items.push(Item::Symbol(sym));
        }

        let parsed = match keyword {
            "Version" => {
                sch.version = rest.to_string();
                continue;
            }
            "SHEET" if !seen_sheet => match ints::<3>(rest) {
                Some([number, width, height]) => {
                    seen_sheet = true;
                    sch.sheet = Sheet {
                        number,
                        width,
                        height,
                    };
                    continue;
                }
                None => None,
            },
            "WIRE" => ints::<4>(rest).map(|[x1, y1, x2, y2]| {
                Item::Wire(Wire::new(Point::new(x1, y1), Point::new(x2, y2)))
            }),
            "FLAG" => point_and_rest(rest).map(|(at, label)| {
                Item::Flag(Flag {
                    at,
                    label: label.to_string(),
                })
            }),
            "IOPIN" => point_and_rest(rest).map(|(at, dir)| {
                Item::IoPin(IoPin {
                    at,
                    direction: dir.to_string(),
                })
            }),
            "BUSTAP" => ints::<4>(rest).map(|[x1, y1, x2, y2]| {
                Item::BusTap(BusTap {
                    a: Point::new(x1, y1),
                    b: Point::new(x2, y2),
                })
            }),
            "SYMBOL" => {
                let mut parts = rest.split_whitespace();
                let sym = (|| {
                    let name = parts.next()?;
                    let x = parts.next()?.parse().ok()?;
                    let y = parts.next()?.parse().ok()?;
                    let orient = parts.next().unwrap_or("R0").parse().ok()?;
                    Some(Symbol::new(name, Point::new(x, y), orient))
                })();
                match sym {
                    Some(s) => {
                        current = Some(s);
                        continue;
                    }
                    None => None,
                }
            }
            "TEXT" => parse_text(rest).map(Item::Text),
            "LINE" | "RECTANGLE" | "CIRCLE" | "ARC" => {
                let kind = match keyword {
                    "LINE" => ShapeKind::Line,
                    "RECTANGLE" => ShapeKind::Rectangle,
                    "CIRCLE" => ShapeKind::Circle,
                    _ => ShapeKind::Arc,
                };
                Some(Item::Shape(Shape {
                    kind,
                    tokens: rest.split_whitespace().map(str::to_string).collect(),
                }))
            }
            _ => Some(Item::Other {
                line: line.to_string(),
            }),
        };
        match parsed {
            Some(item) => sch.items.push(item),
            None => {
                warnings.push(warn(
                    line_no,
                    &format!("unreadable {keyword} line kept as is"),
                ));
                sch.items.push(Item::Other {
                    line: line.to_string(),
                });
            }
        }
    }
    if let Some(sym) = current.take() {
        sch.items.push(Item::Symbol(sym));
    }
    (sch, warnings)
}

fn warn(line: usize, message: &str) -> ParseWarning {
    ParseWarning {
        line,
        message: message.to_string(),
    }
}

/// Split off the first whitespace-delimited word. The remainder keeps its
/// inner spacing, which matters for labels and directive text.
fn split_keyword(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

fn ints<const N: usize>(s: &str) -> Option<[i32; N]> {
    let mut out = [0; N];
    let mut parts = s.split_whitespace();
    for slot in out.iter_mut() {
        *slot = parts.next()?.parse().ok()?;
    }
    Some(out)
}

fn point_and_rest(s: &str) -> Option<(Point, &str)> {
    let (x, rest) = split_keyword(s);
    let (y, rest) = split_keyword(rest);
    let at = Point::new(x.parse().ok()?, y.parse().ok()?);
    (!rest.is_empty()).then_some((at, rest))
}

fn parse_window(s: &str) -> Option<Window> {
    let mut parts = s.split_whitespace();
    let index = parts.next()?.parse().ok()?;
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    let align = parts.next()?.to_string();
    let size = parts.next().map(|v| v.parse().ok()).unwrap_or(Some(2))?;
    Some(Window {
        index,
        at: Point::new(x, y),
        align,
        size,
    })
}

/// `TEXT x y Left 2 !.tran 1m`
fn parse_text(s: &str) -> Option<Text> {
    let (x, rest) = split_keyword(s);
    let (y, rest) = split_keyword(rest);
    let (align, rest) = split_keyword(rest);
    let (size, body) = split_keyword(rest);
    let at = Point::new(x.parse().ok()?, y.parse().ok()?);
    let size = size.parse().ok()?;
    let (kind, content) = if let Some(c) = body.strip_prefix('!') {
        (TextKind::Directive, c)
    } else if let Some(c) = body.strip_prefix(';') {
        (TextKind::Comment, c)
    } else {
        // Older files sometimes omit the marker on comments.
        (TextKind::Comment, body)
    };
    Some(Text {
        at,
        align: align.to_string(),
        size,
        kind,
        content: content.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RC: &str = "Version 4\nSHEET 1 880 680\nWIRE 96 96 32 96\nWIRE 240 96 176 96\nFLAG 32 176 0\nFLAG 240 96 out\nSYMBOL voltage 32 80 R0\nWINDOW 123 0 0 Left 0\nWINDOW 39 0 0 Left 0\nSYMATTR InstName V1\nSYMATTR Value SINE(0 1 1k)\nSYMBOL res 192 80 R90\nWINDOW 0 0 56 VBottom 2\nWINDOW 3 32 56 VTop 2\nSYMATTR InstName R1\nSYMATTR Value 1k\nTEXT 0 232 Left 2 !.ac dec 20 10 100k\nTEXT 0 264 Left 2 ;RC low-pass\n";

    #[test]
    fn parses_items_in_order() {
        let (sch, warnings) = parse(RC);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(sch.version, "4");
        assert_eq!(
            sch.sheet,
            Sheet {
                number: 1,
                width: 880,
                height: 680
            }
        );
        assert_eq!(sch.wires().count(), 2);
        assert_eq!(sch.flags().count(), 2);
        let r1 = sch.symbol("r1").unwrap();
        assert_eq!(r1.name, "res");
        assert_eq!(r1.orient, Orient::R90);
        assert_eq!(r1.value(), Some("1k"));
        assert_eq!(r1.windows.len(), 2);
        let v1 = sch.symbol("V1").unwrap();
        assert_eq!(v1.value(), Some("SINE(0 1 1k)"));
        let texts: Vec<_> = sch.texts().collect();
        assert_eq!(texts[0].kind, TextKind::Directive);
        assert_eq!(texts[0].content, ".ac dec 20 10 100k");
        assert_eq!(texts[1].kind, TextKind::Comment);
    }

    #[test]
    fn round_trips_exactly() {
        let (sch, _) = parse(RC);
        assert_eq!(write(&sch), RC);
    }

    #[test]
    fn crlf_and_unknown_lines_survive() {
        let src = "Version 4.1\r\nSHEET 1 1200 900\r\nDATAFLAG 64 64 \"\"\r\nWIRE 0 0 64 0\r\n";
        let (sch, warnings) = parse(src);
        assert!(warnings.is_empty());
        assert_eq!(sch.format.line_ending, LineEnding::CrLf);
        assert_eq!(write(&sch), src);
    }

    #[test]
    fn bad_lines_are_kept_and_reported() {
        let src = "Version 4\nSHEET 1 880 680\nWIRE 1 2 three 4\n";
        let (sch, warnings) = parse(src);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].line, 3);
        assert_eq!(write(&sch), src);
    }

    #[test]
    fn utf16_file_round_trips_bytes() {
        let bytes = crate::encoding::encode(RC, crate::encoding::Encoding::Utf16Le { bom: false });
        let (sch, _) = parse_bytes(&bytes);
        assert_eq!(write_bytes(&sch), bytes);
    }

    #[test]
    fn labels_keep_spaces_and_multiline_directives_split() {
        let src =
            "Version 4\nSHEET 1 880 680\nFLAG 0 0 V out\nTEXT 0 0 Left 2 !.param a=1\\n.tran 1m\n";
        let (sch, _) = parse(src);
        assert_eq!(sch.flags().next().unwrap().label, "V out");
        let lines: Vec<_> = sch.texts().next().unwrap().lines().collect();
        assert_eq!(lines, vec![".param a=1", ".tran 1m"]);
    }
}
