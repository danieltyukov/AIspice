//! Measuring how readable a schematic is.
//!
//! The metrics are the things a reviewer circles in red: wires crossing,
//! wires drawn through a part, parts or text on top of each other, and then
//! the softer ones, labels standing in for wires, total wire, corners and the
//! area used. The score folds them into one number so candidate layouts can be
//! ranked, and so edits can be checked for making a drawing worse.

use super::geom::{Dir, core_body, overlaps, pin_facing, place_rect, segment_hits};
use super::text::{flag_box, sheet_texts, symbol_texts};
use crate::geometry::{Point, Rect};
use crate::schematic::{Schematic, Wire};
use crate::symbol::SymbolLibrary;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Readability metrics of a schematic. Lengths are in sheet units (16 per
/// grid step); `area` is the bounding box in square units.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Quality {
    /// Places where two wires cross without connecting.
    pub crossings: usize,
    /// Wire segments that pass through the drawn body of a part.
    pub wires_through_bodies: usize,
    /// Pairs of parts whose bodies overlap.
    pub overlapping_parts: usize,
    /// Text blocks (part names and values, labels, directives) that overlap a
    /// wire, a part or other text.
    pub overlapping_text: usize,
    /// Net labels other than ground.
    pub labels: usize,
    /// Corners: two wires meeting at a right angle with nothing else there.
    pub bends: usize,
    pub wire_length: i64,
    pub area: i64,
    /// 100 for a clean drawing; each defect takes points off.
    pub score: f64,
}

impl Quality {
    /// Hard defects: things that make a drawing wrong to read, not just
    /// untidy.
    pub fn defects(&self) -> usize {
        self.wires_through_bodies + self.overlapping_parts
    }
}

/// A placed part as the checks see it.
struct Part {
    body: Rect,
    texts: Vec<Rect>,
    pins: Vec<Point>,
    facings: Vec<Dir>,
}

/// Measure a schematic.
pub fn quality(sch: &Schematic, lib: &SymbolLibrary) -> Quality {
    let wires: Vec<Wire> = sch.wires().filter(|w| w.a != w.b).copied().collect();
    let mut parts = Vec::new();
    for s in sch.symbols() {
        let Ok((def, _)) = lib.resolve(&s.name) else {
            continue;
        };
        let Some(core) = core_body(&def) else {
            continue;
        };
        let body = place_rect(core, s.at, s.orient);
        let pins = def
            .pins
            .iter()
            .map(|p| def.pin_position(p, s.at, s.orient))
            .collect();
        let facings = def
            .pins
            .iter()
            .map(|p| pin_facing(&def, p, s.orient))
            .collect();
        parts.push(Part {
            body,
            texts: symbol_texts(s, &def),
            pins,
            facings,
        });
    }
    let pin_owners: Vec<(Point, Point)> = parts
        .iter()
        .flat_map(|p| {
            let c = Point::new(
                (p.body.min.x + p.body.max.x) / 2,
                (p.body.min.y + p.body.max.y) / 2,
            );
            p.pins.iter().map(move |&pin| (pin, c))
        })
        .collect();

    let mut q = Quality::default();

    // Crossings: a horizontal and a vertical wire meeting at a point inside
    // both. Collinear overlaps are one conductor, not a crossing.
    let horiz: Vec<&Wire> = wires.iter().filter(|w| w.a.y == w.b.y).collect();
    let vert: Vec<&Wire> = wires.iter().filter(|w| w.a.x == w.b.x).collect();
    let mut crossing_points = std::collections::BTreeSet::new();
    for h in &horiz {
        let (x0, x1) = (h.a.x.min(h.b.x), h.a.x.max(h.b.x));
        for v in &vert {
            let (y0, y1) = (v.a.y.min(v.b.y), v.a.y.max(v.b.y));
            if v.a.x > x0 && v.a.x < x1 && h.a.y > y0 && h.a.y < y1 {
                crossing_points.insert(Point::new(v.a.x, h.a.y));
            }
        }
    }
    q.crossings = crossing_points.len();

    // Wires through bodies. The body is shrunk a little so a wire ending on a
    // pin at the edge of a drawing does not count.
    // A wire leaving one of the part's own pins straight out along its lead
    // is fine even where the pin sits inside the drawing's bounding box (an
    // op-amp's supply pins do).
    let leaves_own_pin = |w: &Wire, p: &Part| {
        p.pins.iter().zip(&p.facings).any(|(&pin, &f)| {
            [(w.a, w.b), (w.b, w.a)].into_iter().any(|(from, to)| {
                from == pin && Dir::from_vec(to.x - from.x, to.y - from.y) == Some(f)
            })
        })
    };
    for w in &wires {
        for p in &parts {
            if segment_hits(w.a, w.b, &p.body.inflate(-1)) && !leaves_own_pin(w, p) {
                q.wires_through_bodies += 1;
            }
        }
    }

    for i in 0..parts.len() {
        for j in i + 1..parts.len() {
            if overlaps(&parts[i].body, &parts[j].body) {
                q.overlapping_parts += 1;
            }
        }
    }

    // Text. Estimates are rough, so boxes are trimmed before testing.
    let mut boxes: Vec<(Rect, Option<usize>, Option<Point>)> = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        for t in &p.texts {
            boxes.push((t.inflate(-2), Some(i), None));
        }
    }
    for f in sch.flags() {
        if f.is_ground() {
            continue;
        }
        q.labels += 1;
        boxes.push((
            flag_box(f, &wires, &pin_owners).inflate(-2),
            None,
            Some(f.at),
        ));
    }
    for t in sheet_texts(sch) {
        boxes.push((t.inflate(-2), None, None));
    }
    let grounds: Vec<Rect> = sch
        .flags()
        .filter(|f| f.is_ground())
        .map(|f| flag_box(f, &wires, &pin_owners).inflate(-2))
        .collect();
    for (k, (b, owner, anchor)) in boxes.iter().enumerate() {
        if b.width() <= 0 || b.height() <= 0 {
            continue;
        }
        let hits_wire = wires.iter().any(|w| {
            // A label's own wire leads up to its text; only count wires that
            // pass through the text itself.
            if anchor.is_some_and(|a| crate::geometry::on_segment(a, w.a, w.b)) {
                return false;
            }
            segment_hits(w.a, w.b, b)
        });
        let hits_part = parts
            .iter()
            .enumerate()
            .any(|(i, p)| Some(i) != *owner && overlaps(b, &p.body));
        let hits_text = boxes
            .iter()
            .enumerate()
            .any(|(m, (o, _, _))| m != k && overlaps(b, o));
        let hits_ground = grounds.iter().any(|g| overlaps(b, g));
        if hits_wire || hits_part || hits_text || hits_ground {
            q.overlapping_text += 1;
        }
    }

    // Corners and length.
    let mut ends: HashMap<Point, Vec<&Wire>> = HashMap::new();
    for w in &wires {
        q.wire_length += ((w.a.x - w.b.x).abs() + (w.a.y - w.b.y).abs()) as i64;
        ends.entry(w.a).or_default().push(w);
        ends.entry(w.b).or_default().push(w);
    }
    let pin_points: std::collections::HashSet<Point> =
        parts.iter().flat_map(|p| p.pins.iter().copied()).collect();
    let flag_points: std::collections::HashSet<Point> = sch.flags().map(|f| f.at).collect();
    for (p, ws) in &ends {
        if ws.len() == 2
            && !pin_points.contains(p)
            && !flag_points.contains(p)
            && (ws[0].a.x == ws[0].b.x) != (ws[1].a.x == ws[1].b.x)
            && !wires
                .iter()
                .any(|w| w.a != *p && w.b != *p && crate::geometry::on_segment(*p, w.a, w.b))
        {
            q.bends += 1;
        }
    }

    // Area of everything drawn.
    let mut bounds: Option<Rect> = None;
    let mut add = |r: Rect| bounds = Some(bounds.map_or(r, |b| b.union(r)));
    for p in &parts {
        add(p.body);
        for t in &p.texts {
            add(*t);
        }
    }
    for w in &wires {
        add(Rect::from_points(w.a, w.b));
    }
    for f in sch.flags() {
        add(flag_box(f, &wires, &pin_owners));
    }
    if let Some(b) = bounds {
        q.area = b.width() as i64 * b.height() as i64;
    }

    q.score = score(&q, parts.len());
    q
}

/// Fold the metrics into 0..100. Hard defects cost the most; untidiness costs
/// a little each, capped so one kind of blemish cannot hide the others.
fn score(q: &Quality, parts: usize) -> f64 {
    let parts = parts.max(1) as f64;
    let mut penalty = 0.0;
    penalty += 15.0 * q.wires_through_bodies as f64;
    penalty += 15.0 * q.overlapping_parts as f64;
    penalty += (6.0 * q.overlapping_text as f64).min(30.0);
    penalty += (5.0 * q.crossings as f64).min(30.0);
    penalty += (0.5 * (q.bends as f64 - parts).max(0.0)).min(8.0);
    // Space: a readable part with its text and spacing needs roughly a
    // 160 by 160 square; much more than that is sprawl.
    let per_part = q.area as f64 / parts / (160.0 * 160.0);
    penalty += ((per_part - 2.0).max(0.0) * 3.0).min(10.0);
    (100.0 - penalty).clamp(0.0, 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schematic::parse;

    #[test]
    fn clean_rc_has_no_defects() {
        let src = "Version 4\nSHEET 1 880 680\nWIRE 32 48 32 96\nWIRE 32 48 112 48\nWIRE 192 48 272 48\nWIRE 272 48 272 96\nFLAG 32 176 0\nFLAG 272 160 0\nSYMBOL voltage 32 80 R0\nSYMATTR InstName V1\nSYMATTR Value 1\nSYMBOL res 208 32 R90\nWINDOW 0 0 56 VBottom 2\nWINDOW 3 32 56 VTop 2\nSYMATTR InstName R1\nSYMATTR Value 1k\nSYMBOL cap 256 96 R0\nSYMATTR InstName C1\nSYMATTR Value 1u\n";
        let (sch, _) = parse(src);
        let q = quality(&sch, &SymbolLibrary::builtin_only());
        assert_eq!(q.crossings, 0, "{q:?}");
        assert_eq!(q.wires_through_bodies, 0, "{q:?}");
        assert_eq!(q.overlapping_parts, 0, "{q:?}");
        assert!(q.score > 80.0, "{q:?}");
    }

    #[test]
    fn crossing_and_through_body_are_counted() {
        let src = "Version 4\nSHEET 1 880 680\nWIRE 0 64 128 64\nWIRE 64 0 64 128\nWIRE -32 56 64 56\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMATTR Value 1k\n";
        let (sch, _) = parse(src);
        let q = quality(&sch, &SymbolLibrary::builtin_only());
        assert_eq!(q.crossings, 1, "{q:?}");
        assert!(q.wires_through_bodies >= 2, "{q:?}");
    }
}
