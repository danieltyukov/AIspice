use super::*;
use crate::encoding;
use std::fmt::Write as _;

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

/// Write and encode the schematic the way it was stored.
pub fn write_bytes(sch: &Schematic) -> Vec<u8> {
    encoding::encode(&write(sch), sch.format.encoding)
}

fn write_item(out: &mut String, item: &Item) {
    match item {
        Item::Wire(w) => {
            let _ = writeln!(out, "WIRE {} {} {} {}", w.a.x, w.a.y, w.b.x, w.b.y);
        }
        Item::Flag(f) => {
            let _ = writeln!(out, "FLAG {} {} {}", f.at.x, f.at.y, f.label);
        }
        Item::IoPin(p) => {
            let _ = writeln!(out, "IOPIN {} {} {}", p.at.x, p.at.y, p.direction);
        }
        Item::BusTap(b) => {
            let _ = writeln!(out, "BUSTAP {} {} {} {}", b.a.x, b.a.y, b.b.x, b.b.y);
        }
        Item::Symbol(s) => {
            let _ = writeln!(out, "SYMBOL {} {} {} {}", s.name, s.at.x, s.at.y, s.orient);
            for w in &s.windows {
                let _ = writeln!(
                    out,
                    "WINDOW {} {} {} {} {}",
                    w.index, w.at.x, w.at.y, w.align, w.size
                );
            }
            for a in &s.attrs {
                if a.value.is_empty() {
                    let _ = writeln!(out, "SYMATTR {}", a.key);
                } else {
                    let _ = writeln!(out, "SYMATTR {} {}", a.key, a.value);
                }
            }
            for line in &s.extra {
                let _ = writeln!(out, "{line}");
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
                t.at.x, t.at.y, t.align, t.size, marker, t.content
            );
        }
        Item::Shape(s) => {
            let keyword = match s.kind {
                ShapeKind::Line => "LINE",
                ShapeKind::Rectangle => "RECTANGLE",
                ShapeKind::Circle => "CIRCLE",
                ShapeKind::Arc => "ARC",
            };
            let _ = writeln!(out, "{keyword} {}", s.tokens.join(" "));
        }
        Item::Other { line } => {
            let _ = writeln!(out, "{line}");
        }
    }
}
