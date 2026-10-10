use super::*;
use crate::encoding;
use std::borrow::Cow;
use std::fmt::Write as _;

/// Every record is one line. Any line break inside a field, from whatever
/// code built the item, is flattened to a space here so it can never start a
/// new record in LTspice (which also breaks on a lone carriage return) or in
/// aispice's own parser.
fn one_line(s: &str) -> Cow<'_, str> {
    if s.contains(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}']) {
        Cow::Owned(s.replace(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}'], " "))
    } else {
        Cow::Borrowed(s)
    }
}

/// Write the schematic as `.asc` text with its original line endings.
pub fn write(sch: &Schematic) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Version {}", sch.version);
    let _ = writeln!(
        out,
        "SHEET {} {} {}",
        sch.sheet.number, sch.sheet.width, sch.sheet.height
    );
    for item in &sch.items {
        write_item(&mut out, item);
    }
    match sch.format.line_ending {
        LineEnding::Lf => out,
        LineEnding::CrLf => out.replace('\n', "\r\n"),
    }
}

/// Write and encode the schematic the way it was stored. A one-byte file that
/// now holds a character outside Latin-1 (an Ohm sign, say) is written as
/// UTF-16LE without a byte order mark, which is what LTspice XVII does itself.
pub fn write_bytes(sch: &Schematic) -> Vec<u8> {
    let text = write(sch);
    let enc = match sch.format.encoding {
        encoding::Encoding::Latin1 if text.chars().any(|c| c as u32 > 0xFF) => {
            encoding::Encoding::Utf16Le { bom: false }
        }
        e => e,
    };
    encoding::encode(&text, enc)
}

fn write_item(out: &mut String, item: &Item) {
    match item {
        Item::Wire(w) => {
            let _ = writeln!(out, "WIRE {} {} {} {}", w.a.x, w.a.y, w.b.x, w.b.y);
        }
        Item::Flag(f) => {
            let _ = writeln!(out, "FLAG {} {} {}", f.at.x, f.at.y, one_line(&f.label));
        }
        Item::IoPin(p) => {
            let _ = writeln!(
                out,
                "IOPIN {} {} {}",
                p.at.x,
                p.at.y,
                one_line(&p.direction)
            );
        }
        Item::BusTap(b) => {
            let _ = writeln!(out, "BUSTAP {} {} {} {}", b.a.x, b.a.y, b.b.x, b.b.y);
        }
        Item::Symbol(s) => {
            let _ = writeln!(
                out,
                "SYMBOL {} {} {} {}",
                one_line(&s.name),
                s.at.x,
                s.at.y,
                s.orient
            );
            for w in &s.windows {
                let _ = writeln!(
                    out,
                    "WINDOW {} {} {} {} {}",
                    w.index,
                    w.at.x,
                    w.at.y,
                    one_line(&w.align),
                    w.size
                );
            }
            for a in &s.attrs {
                if a.value.is_empty() {
                    let _ = writeln!(out, "SYMATTR {}", one_line(&a.key));
                } else {
                    let _ = writeln!(out, "SYMATTR {} {}", one_line(&a.key), one_line(&a.value));
                }
            }
            for line in &s.extra {
                let _ = writeln!(out, "{}", one_line(line));
            }
        }
        Item::Text(t) => {
            let marker = match t.kind {
                TextKind::Directive => '!',
                TextKind::Comment => ';',
            };
            let _ = writeln!(
                out,
                "TEXT {} {} {} {} {}{}",
                t.at.x,
                t.at.y,
                one_line(&t.align),
                t.size,
                marker,
                one_line(&t.content)
            );
        }
        Item::Shape(s) => {
            let keyword = match s.kind {
                ShapeKind::Line => "LINE",
                ShapeKind::Rectangle => "RECTANGLE",
                ShapeKind::Circle => "CIRCLE",
                ShapeKind::Arc => "ARC",
            };
            let _ = writeln!(out, "{keyword} {}", one_line(&s.tokens.join(" ")));
        }
        Item::Other { line } => {
            let _ = writeln!(out, "{}", one_line(line));
        }
    }
}
