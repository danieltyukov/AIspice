//! Choosing where new things go on the sheet.

use super::EditError;
use crate::geometry::{GRID, Orient, Point, Rect, snap};
use crate::schematic::{Item, Schematic};
use crate::symbol::{SymbolDef, SymbolLibrary};

/// Bounds of everything drawn: parts, wires, labels and text anchors.
pub(crate) fn content_bounds(sch: &Schematic, lib: &SymbolLibrary) -> Option<Rect> {
    let mut r: Option<Rect> = None;
    let mut add = |x: Rect| r = Some(r.map_or(x, |cur| cur.union(x)));
    for item in &sch.items {
        match item {
            Item::Symbol(s) => {
                if let Some(b) = lib
                    .resolve(&s.name)
                    .ok()
                    .and_then(|(d, _)| d.placed_bounds(s.at, s.orient))
                {
                    add(b);
                } else {
                    add(Rect::from_points(s.at, s.at));
                }
            }
            Item::Wire(w) => add(Rect::from_points(w.a, w.b)),
            Item::Flag(f) => add(Rect::from_points(f.at, f.at)),
            Item::Text(t) => add(Rect::from_points(t.at, t.at)),
            _ => {}
        }
    }
    r
}

fn occupied(sch: &Schematic, lib: &SymbolLibrary) -> (Vec<Rect>, Vec<(Point, Point)>, Vec<Point>) {
    let mut parts = Vec::new();
    for s in sch.symbols() {
        if let Some(b) = lib
            .resolve(&s.name)
            .ok()
            .and_then(|(d, _)| d.placed_bounds(s.at, s.orient))
        {
            parts.push(b);
        }
    }
    let wires = sch.wires().map(|w| (w.a, w.b)).collect();
    let points = sch.flags().map(|f| f.at).collect();
    (parts, wires, points)
}

fn segment_touches(a: Point, b: Point, r: &Rect) -> bool {
    let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
    let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
    x1 >= r.min.x && x0 <= r.max.x && y1 >= r.min.y && y0 <= r.max.y
}

/// A grid position where `def` fits without overlapping parts or touching any
/// wire or label, so its pins start out unconnected.
pub(crate) fn free_spot(
    sch: &Schematic,
    lib: &SymbolLibrary,
    def: &SymbolDef,
    orient: Orient,
    near: Option<&str>,
) -> Result<Point, EditError> {
    let (parts, wires, points) = occupied(sch, lib);
    let probe = def
        .placed_bounds(Point::new(0, 0), orient)
        .unwrap_or(Rect::from_points(Point::new(0, 0), Point::new(0, 0)));
    let fits = |origin: Point| {
        let r = Rect {
            min: origin + probe.min,
            max: origin + probe.max,
        }
        .inflate(GRID);
        !parts.iter().any(|p| p.intersects(&r))
            && !wires.iter().any(|(a, b)| segment_touches(*a, *b, &r))
            && !points.iter().any(|p| r.contains(*p))
    };
    let anchor = match near {
        Some(name) => {
            let s = sch.symbol(name).ok_or_else(|| {
                EditError::NoSuchComponent(name.to_string(), super::pins::known_names(sch))
            })?;
            lib.resolve(&s.name)
                .ok()
                .and_then(|(d, _)| d.placed_bounds(s.at, s.orient))
                .unwrap_or(Rect::from_points(s.at, s.at))
        }
        None => match content_bounds(sch, lib) {
            Some(b) => {
                Rect::from_points(Point::new(b.max.x, b.min.y), Point::new(b.max.x, b.min.y))
            }
            None => return Ok(Point::new(128, 64)),
        },
    };
    // Spiral outwards from the anchor: right, below, left, above, then wider.
    for ring in 1..40 {
        let d = ring * GRID * 2;
        let candidates = [
            Point::new(anchor.max.x + d - probe.min.x, anchor.min.y - probe.min.y),
            Point::new(anchor.min.x - probe.min.x, anchor.max.y + d - probe.min.y),
            Point::new(anchor.min.x - d - probe.max.x, anchor.min.y - probe.min.y),
            Point::new(anchor.min.x - probe.min.x, anchor.min.y - d - probe.max.y),
            Point::new(
                anchor.max.x + d - probe.min.x,
                anchor.max.y + d - probe.min.y,
            ),
        ];
        for c in candidates {
            let c = Point::new(snap(c.x), snap(c.y));
            if fits(c) {
                return Ok(c);
            }
        }
    }
    let b =
        content_bounds(sch, lib).unwrap_or(Rect::from_points(Point::new(0, 0), Point::new(0, 0)));
    Ok(Point::new(snap(b.max.x + 256), snap(b.min.y)))
}

/// Below the circuit, under any existing text.
pub(crate) fn text_spot(sch: &Schematic, lib: &SymbolLibrary) -> Point {
    let b =
        content_bounds(sch, lib).unwrap_or(Rect::from_points(Point::new(0, 0), Point::new(0, 0)));
    let lowest_text = sch.texts().map(|t| t.at.y).max();
    let y = match lowest_text {
        Some(ty) if ty >= b.max.y - GRID * 4 => ty + 32,
        _ => b.max.y + 48,
    };
    Point::new(snap(b.min.x), snap(y))
}
