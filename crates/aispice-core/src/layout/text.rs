//! Where text lands on the sheet, for keeping it clear of parts and wires.
//!
//! The renderer owns the exact drawing; this is the same model in integer
//! sheet units, kept separate so layout and quality checks do not depend on
//! rendering internals. LTspice keeps text upright: a window turned so it
//! would read right to left or downwards is turned a further half turn with
//! its justification flipped, so it still covers the same area.

use crate::geometry::{Orient, Point, Rect};
use crate::schematic::{Flag, Schematic, Symbol, Wire};
use crate::symbol::SymbolDef;

/// Font size in sheet units for an LTspice size index, matching the renderer.
pub(crate) fn font_size(size: i32) -> f64 {
    const SCALE: [f64; 8] = [0.625, 1.0, 1.5, 2.0, 2.5, 3.5, 5.0, 7.0];
    40.0 / 3.0 * SCALE[size.clamp(0, 7) as usize]
}

/// Approximate advance width of a line, tuned to Arial-like faces.
pub(crate) fn text_width(s: &str, fs: f64) -> f64 {
    s.chars()
        .map(|c| match c {
            'i' | 'j' | 'l' | '.' | ',' | ':' | ';' | '!' | '|' | '\'' => 0.25,
            'f' | 't' | 'r' | 'I' | '(' | ')' | '[' | ']' | ' ' | '-' => 0.36,
            'm' | 'w' | 'M' | 'W' => 0.85,
            'A'..='Z' => 0.68,
            _ => 0.56,
        })
        .sum::<f64>()
        * fs
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H {
    Start,
    Middle,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum V {
    Top,
    Middle,
    Bottom,
}

/// A justification: reading direction (vertical reads bottom to top) and
/// where the anchor sits along and across it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Just {
    vertical: bool,
    h: H,
    v: V,
}

impl Just {
    /// Parse an LTspice justification; `Invisible` gives `None`.
    pub fn parse(s: &str) -> Option<Just> {
        let lower = s.trim().to_ascii_lowercase();
        if lower == "invisible" {
            return None;
        }
        let (vertical, rest) = match lower.strip_prefix('v') {
            Some(r) if !r.is_empty() => (true, r),
            _ => (false, lower.as_str()),
        };
        let (h, v) = match rest {
            "right" => (H::End, V::Middle),
            "center" | "centre" => (H::Middle, V::Middle),
            "top" => (H::Middle, V::Top),
            "bottom" => (H::Middle, V::Bottom),
            _ => (H::Start, V::Middle),
        };
        Some(Just { vertical, h, v })
    }

    /// The justification after placing with `orient`, kept upright.
    fn oriented(self, orient: Orient) -> Just {
        let (d, u) = if self.vertical {
            (Point::new(0, -1), Point::new(-1, 0))
        } else {
            (Point::new(1, 0), Point::new(0, -1))
        };
        let (d1, u1) = (orient.apply(d), orient.apply(u));
        let readable = d1 == Point::new(1, 0) || d1 == Point::new(0, -1);
        let dr = if readable {
            d1
        } else {
            Point::new(-d1.x, -d1.y)
        };
        let ur = if dr == Point::new(1, 0) {
            Point::new(0, -1)
        } else {
            Point::new(-1, 0)
        };
        let flip_h = |h| match h {
            H::Start => H::End,
            H::End => H::Start,
            H::Middle => H::Middle,
        };
        let flip_v = |v| match v {
            V::Top => V::Bottom,
            V::Bottom => V::Top,
            V::Middle => V::Middle,
        };
        Just {
            vertical: dr == Point::new(0, -1),
            h: if readable { self.h } else { flip_h(self.h) },
            v: if ur == u1 { self.v } else { flip_v(self.v) },
        }
    }
}

/// Bounds of a block of text lines anchored at `at`.
pub(crate) fn block(at: Point, just: Just, size: i32, lines: &[&str]) -> Rect {
    let fs = font_size(size);
    let height = lines.len().max(1) as f64 * fs * 1.2;
    let width = lines.iter().map(|l| text_width(l, fs)).fold(0.0, f64::max);
    let top = match just.v {
        V::Top => 0.0,
        V::Middle => -height / 2.0,
        V::Bottom => -height,
    };
    let x0 = match just.h {
        H::Start => 0.0,
        H::Middle => -width / 2.0,
        H::End => -width,
    };
    let (ax, ay) = (at.x as f64, at.y as f64);
    let corners = [(x0, top), (x0 + width, top + height)];
    let pts: Vec<(f64, f64)> = corners
        .iter()
        .map(|&(lx, ly)| {
            if just.vertical {
                (ax + ly, ay - lx)
            } else {
                (ax + lx, ay + ly)
            }
        })
        .collect();
    let (xa, xb) = (pts[0].0.min(pts[1].0), pts[0].0.max(pts[1].0));
    let (ya, yb) = (pts[0].1.min(pts[1].1), pts[0].1.max(pts[1].1));
    Rect::from_points(
        Point::new(xa.floor() as i32, ya.floor() as i32),
        Point::new(xb.ceil() as i32, yb.ceil() as i32),
    )
}

/// The attribute a WINDOW index shows.
fn window_attr(index: i32) -> Option<&'static str> {
    Some(match index {
        0 => "InstName",
        3 => "Value",
        123 => "Value2",
        38 => "SpiceModel",
        39 => "SpiceLine",
        40 => "SpiceLine2",
        _ => return None,
    })
}

/// Text boxes of a placed symbol's attribute windows: the definition's
/// defaults replaced by the instance's own WINDOW lines, showing the
/// instance's attribute or the symbol default (never for the name).
pub(crate) fn symbol_texts(sym: &Symbol, def: &SymbolDef) -> Vec<Rect> {
    let mut wins: Vec<(i32, Point, String, i32)> = def
        .windows
        .iter()
        .map(|w| (w.index, w.at, w.align.clone(), w.size))
        .collect();
    for w in &sym.windows {
        let win = (w.index, w.at, w.align.clone(), w.size);
        match wins.iter_mut().find(|o| o.0 == w.index) {
            Some(slot) => *slot = win,
            None => wins.push(win),
        }
    }
    let mut out = Vec::new();
    for (index, at, align, size) in wins {
        let Some(key) = window_attr(index) else {
            continue;
        };
        let value = sym
            .attr(key)
            .or_else(|| (index != 0).then(|| def.attr(key)).flatten())
            .map(str::trim)
            .unwrap_or("");
        if value.is_empty() {
            continue;
        }
        let Some(just) = Just::parse(&align) else {
            continue;
        };
        let lines: Vec<&str> = value.split("\\n").collect();
        out.push(block(
            sym.at + sym.orient.apply(at),
            just.oriented(sym.orient),
            size,
            &lines,
        ));
    }
    out
}

/// Which sides of a flag have a wire or pin attached, as the renderer works
/// it out, so the label text is modelled on the side it is drawn.
#[derive(Debug, Clone, Copy, Default)]
struct Sides {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
}

impl Sides {
    fn mark(&mut self, dx: i32, dy: i32) {
        if dx.abs() >= dy.abs() {
            if dx < 0 {
                self.left = true;
            } else if dx > 0 {
                self.right = true;
            }
        } else if dy < 0 {
            self.up = true;
        } else if dy > 0 {
            self.down = true;
        }
    }

    fn count(&self) -> usize {
        [self.left, self.right, self.up, self.down]
            .iter()
            .filter(|b| **b)
            .count()
    }
}

/// The box a net label's text covers. Ground symbols get their drawn shape.
/// `pins` pairs each pin position with the centre of its part.
pub(crate) fn flag_box(f: &Flag, wires: &[Wire], pins: &[(Point, Point)]) -> Rect {
    let p = f.at;
    if f.is_ground() {
        return Rect::from_points(p.offset(-16, 0), p.offset(16, 20));
    }
    let mut s = Sides::default();
    for w in wires.iter().filter(|w| w.a != w.b) {
        if w.a == p {
            s.mark(w.b.x - p.x, w.b.y - p.y);
        } else if w.b == p {
            s.mark(w.a.x - p.x, w.a.y - p.y);
        } else if crate::geometry::on_segment(p, w.a, w.b) {
            s.mark(w.a.x - p.x, w.a.y - p.y);
            s.mark(w.b.x - p.x, w.b.y - p.y);
        }
    }
    for &(pin, centre) in pins {
        if pin == p {
            s.mark(centre.x - p.x, centre.y - p.y);
        }
    }
    let single = s.count() == 1;
    let (at, just) = if single {
        if s.left {
            (p.offset(4, 0), "Left")
        } else if s.right {
            (p.offset(-4, 0), "Right")
        } else if s.up {
            (p.offset(0, 4), "VRight")
        } else {
            (p.offset(0, -4), "VLeft")
        }
    } else if !s.up {
        (p.offset(0, -2), "Bottom")
    } else if !s.down {
        (p.offset(0, 2), "Top")
    } else if !s.right {
        (p.offset(4, 0), "Left")
    } else {
        (p.offset(-4, 0), "Right")
    };
    let just = Just::parse(just).expect("visible");
    block(at, just, 2, &[f.label.as_str()]).union(Rect::from_points(p, p))
}

/// Text boxes of every directive and comment on the sheet.
pub(crate) fn sheet_texts(sch: &Schematic) -> Vec<Rect> {
    sch.texts()
        .filter_map(|t| {
            let just = Just::parse(&t.align)?;
            let lines: Vec<String> = t.lines();
            let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
            Some(block(t.at, just, t.size, &refs))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotated_resistor_name_sits_above_the_body() {
        // LTspice's own windows for a resistor turned R90.
        let j = Just::parse("VBottom").unwrap().oriented(Orient::R90);
        let r = block(Point::new(-56, 0), j, 2, &["R1"]);
        assert!(r.max.y <= 0 && r.min.y < -10, "{r:?}");
        assert!(r.min.x < -56 && r.max.x > -56, "{r:?}");
    }

    #[test]
    fn left_text_grows_right() {
        let r = block(
            Point::new(36, 40),
            Just::parse("Left").unwrap(),
            2,
            &["10k"],
        );
        assert_eq!(r.min.x, 36);
        assert!(r.max.x > 50 && r.min.y < 40 && r.max.y > 40);
    }
}
