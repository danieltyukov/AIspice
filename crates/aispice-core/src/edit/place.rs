//! Choosing where new things go on the sheet.
//!
//! A part placed without coordinates gets a spot near the part it is meant to
//! go with, clear of everything already drawn: other parts and their name and
//! value text, wires, labels and ground symbols. Room is also kept for what
//! comes next: the stubs, labels and ground symbols its own pins will need,
//! and a routing channel between it and its neighbours, so the wires added
//! afterwards can run between parts instead of along their edges.

use super::EditError;
use crate::geometry::{GRID, Orient, Point, Rect, snap};
use crate::layout::{Dir, flag_box, pin_facing, symbol_texts};
use crate::schematic::{Item, Schematic, Symbol, Wire};
use crate::symbol::{SymbolDef, SymbolLibrary};

/// Space kept free around a new part, beyond its own text, for the wires and
/// flags its pins will need.
const CHANNEL: i32 = 2 * GRID;

/// How far beyond a pin, along its lead, room is kept for the stub and the
/// label or ground symbol it is likely to get.
const PIN_HALO: i32 = 3 * GRID;

/// The room in front of a pin facing `f`.
fn pin_halo(at: Point, f: Dir) -> Rect {
    let end = f.step(at, PIN_HALO);
    let side = if f.horizontal() {
        Point::new(0, 12)
    } else {
        Point::new(12, 0)
    };
    Rect::from_points(
        Point::new(at.x.min(end.x), at.y.min(end.y)) - side,
        Point::new(at.x.max(end.x), at.y.max(end.y)) + side,
    )
}

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

/// What is already on the sheet, as areas to keep out of.
struct Occupied {
    /// Parts with their text.
    zones: Vec<Rect>,
    wires: Vec<(Point, Point)>,
    /// Labels and ground symbols.
    flags: Vec<Rect>,
}

fn occupied(sch: &Schematic, lib: &SymbolLibrary) -> Occupied {
    let mut zones = Vec::new();
    let mut pin_owners = Vec::new();
    for s in sch.symbols() {
        let Ok((def, _)) = lib.resolve(&s.name) else {
            zones.push(Rect::from_points(s.at, s.at.offset(64, 64)));
            continue;
        };
        if let Some(b) = def.placed_bounds(s.at, s.orient) {
            let mut z = b;
            for t in symbol_texts(s, &def) {
                z = z.union(t);
            }
            let c = Point::new((b.min.x + b.max.x) / 2, (b.min.y + b.max.y) / 2);
            for p in &def.pins {
                let at = def.pin_position(p, s.at, s.orient);
                pin_owners.push((at, c));
                z = z.union(pin_halo(at, pin_facing(&def, p, s.orient)));
            }
            zones.push(z);
        }
    }
    let wire_list: Vec<Wire> = sch.wires().copied().collect();
    let wires = wire_list.iter().map(|w| (w.a, w.b)).collect();
    let flags = sch
        .flags()
        .map(|f| flag_box(f, &wire_list, &pin_owners))
        .collect();
    Occupied {
        zones,
        wires,
        flags,
    }
}

fn segment_touches(a: Point, b: Point, r: &Rect) -> bool {
    let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
    let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
    x1 >= r.min.x && x0 <= r.max.x && y1 >= r.min.y && y0 <= r.max.y
}

/// The area a new part will need relative to its origin: its drawing, its
/// name and value text (estimated with typical text, since the value is set
/// after placement), and the channel around it.
fn footprint(def: &SymbolDef, orient: Orient) -> Rect {
    let origin = Point::new(0, 0);
    let body = def
        .placed_bounds(origin, orient)
        .unwrap_or(Rect::from_points(origin, origin));
    let mut probe = Symbol::new("probe", origin, orient);
    let prefix = def.prefix().chars().next().unwrap_or('X');
    probe.set_attr("InstName", format!("{prefix}10"));
    if def.attr("Value").is_none() {
        probe.set_attr("Value", "10k");
    }
    let mut r = body;
    for t in symbol_texts(&probe, def) {
        r = r.union(t);
    }
    // Pins need room for a stub and a flag or ground symbol.
    for p in &def.pins {
        let at = def.pin_position(p, origin, orient);
        r = r.union(pin_halo(at, pin_facing(def, p, orient)));
    }
    r
}

/// A grid position where `def` fits clear of parts, text, wires and labels,
/// with a routing channel around it, so its pins start out unconnected and
/// there is room to wire them.
pub(crate) fn free_spot(
    sch: &Schematic,
    lib: &SymbolLibrary,
    def: &SymbolDef,
    orient: Orient,
    near: Option<&str>,
) -> Result<Point, EditError> {
    let occ = occupied(sch, lib);
    let probe = footprint(def, orient);
    let fits = |origin: Point| {
        let r = Rect {
            min: origin + probe.min,
            max: origin + probe.max,
        };
        let channel = r.inflate(CHANNEL);
        !occ.zones.iter().any(|z| z.intersects(&channel))
            && !occ
                .wires
                .iter()
                .any(|(a, b)| segment_touches(*a, *b, &channel))
            && !occ.flags.iter().any(|f| f.intersects(&channel))
    };
    let anchor = match near {
        Some(name) => {
            let s = sch.symbol(name).ok_or_else(|| {
                EditError::NoSuchComponent(name.to_string(), super::pins::known_names(sch))
            })?;
            lib.resolve(&s.name)
                .ok()
                .and_then(|(d, _)| {
                    let mut z = d.placed_bounds(s.at, s.orient)?;
                    for t in symbol_texts(s, &d) {
                        z = z.union(t);
                    }
                    Some(z)
                })
                .unwrap_or(Rect::from_points(s.at, s.at))
        }
        None => match content_bounds(sch, lib) {
            Some(b) => {
                Rect::from_points(Point::new(b.max.x, b.min.y), Point::new(b.max.x, b.min.y))
            }
            None => return Ok(Point::new(128, 64)),
        },
    };
    // Rings of candidate spots around the anchor, nearest first. Within the
    // first rings that have room, the spot to the right (where the signal
    // goes) or below is preferred, then left, then above; spots that keep the
    // new part level with the anchor or in line with it read best.
    let mut best: Option<(i32, Point)> = None;
    let mut found_at: Option<i32> = None;
    for ring in 1..48 {
        if found_at.is_some_and(|f| ring > f + 2) {
            break;
        }
        let d = ring * GRID;
        let candidates = [
            (
                0,
                Point::new(anchor.max.x + d - probe.min.x, anchor.min.y - probe.min.y),
            ),
            (
                1,
                Point::new(anchor.min.x - probe.min.x, anchor.max.y + d - probe.min.y),
            ),
            (
                2,
                Point::new(anchor.min.x - d - probe.max.x, anchor.min.y - probe.min.y),
            ),
            (
                3,
                Point::new(anchor.min.x - probe.min.x, anchor.min.y - d - probe.max.y),
            ),
            (
                4,
                Point::new(
                    anchor.max.x + d - probe.min.x,
                    anchor.max.y + d - probe.min.y,
                ),
            ),
            (
                5,
                Point::new(
                    anchor.max.x + d - probe.min.x,
                    anchor.min.y - d - probe.max.y,
                ),
            ),
        ];
        for (side, c) in candidates {
            let c = Point::new(snap(c.x), snap(c.y));
            if !fits(c) {
                continue;
            }
            found_at.get_or_insert(ring);
            let score = ring * 4 + side * 3;
            if best.is_none_or(|(b, _)| score < b) {
                best = Some((score, c));
            }
        }
    }
    if let Some((_, c)) = best {
        return Ok(c);
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

/// Where a part asked for at an explicit position actually goes: snapped to
/// the grid, and moved to the nearest spot clear of other parts if it would
/// land on one. Wires and labels are not avoided here, since a part may be
/// placed on a wire on purpose. Returns a note when the position changed.
pub(crate) fn requested_spot(
    sch: &Schematic,
    lib: &SymbolLibrary,
    def: &SymbolDef,
    orient: Orient,
    at: Point,
) -> (Point, Option<String>) {
    let snapped = at.snapped();
    let parts = occupied(sch, lib).zones;
    let probe = def
        .placed_bounds(Point::new(0, 0), orient)
        .unwrap_or(Rect::from_points(Point::new(0, 0), Point::new(0, 0)));
    let clear = |origin: Point| {
        let r = Rect {
            min: origin + probe.min,
            max: origin + probe.max,
        }
        .inflate(GRID / 2);
        !parts.iter().any(|p| p.intersects(&r))
    };
    if clear(snapped) {
        let note = (snapped != at).then(|| format!("snapped {at} to the grid at {snapped}"));
        return (snapped, note);
    }
    // Search outward ring by ring, nearest first.
    for ring in 1..64 {
        let d = ring * GRID;
        let mut ring_points = Vec::new();
        for k in -ring..=ring {
            let o = k * GRID;
            ring_points.extend([
                Point::new(snapped.x + o, snapped.y - d),
                Point::new(snapped.x + o, snapped.y + d),
                Point::new(snapped.x - d, snapped.y + o),
                Point::new(snapped.x + d, snapped.y + o),
            ]);
        }
        ring_points.sort_by_key(|p| (p.x - snapped.x).pow(2) + (p.y - snapped.y).pow(2));
        if let Some(p) = ring_points.into_iter().find(|p| clear(*p)) {
            return (
                p,
                Some(format!(
                    "moved from {at} to {p} so it does not overlap another part"
                )),
            );
        }
    }
    (
        snapped,
        Some(format!(
            "{snapped} overlaps another part; no free spot was found nearby"
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::place_rect;
    use crate::schematic::parse;

    #[test]
    fn new_parts_keep_a_channel_from_text_and_wires() {
        // R1 with its text to the right and a wire below it.
        let src = "Version 4\nSHEET 1 880 680\nWIRE -64 160 400 160\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMATTR Value 4.7k\n";
        let (sch, _) = parse(src);
        let lib = SymbolLibrary::builtin_only();
        let (def, _) = lib.resolve("res").unwrap();
        let at = free_spot(&sch, &lib, &def, Orient::R0, Some("R1")).unwrap();
        let new = place_rect(def.bounds().unwrap(), at, Orient::R0);
        let r1 = sch.symbol("R1").unwrap();
        let text = symbol_texts(r1, &def)
            .into_iter()
            .reduce(|a, b| a.union(b))
            .unwrap();
        assert!(
            new.min.x >= text.max.x + CHANNEL,
            "{new:?} vs text {text:?}"
        );
        assert!(!segment_touches(
            Point::new(-64, 160),
            Point::new(400, 160),
            &new.inflate(CHANNEL - 1)
        ));
    }
}
