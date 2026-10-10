//! Taking a pin, or a whole part, off the nets it is on, together with the
//! wires and labels that served only it.
//!
//! A label at the end of a pin's stub belongs to that pin: left behind, it
//! keeps the old net's name alive on an empty spot, and the next stub drawn
//! there joins the old net to the new one. So when a pin is disconnected or
//! its part removed, every wire piece that reaches no other pin goes, with
//! the labels on it. A piece other pins still use keeps its labels; only the
//! wires ending on the pin go, and the other pins that shared the pin's net
//! are joined back onto it afterwards.

use super::route::{Route, Router};
use super::{EditError, PinLoc, connect_to_net, locate, prune_dangling};
use crate::geometry::{Point, SegmentIndex};
use crate::netlist::{PinRef, connect};
use crate::schematic::{Item, Schematic, Wire};
use crate::symbol::SymbolLibrary;
use std::collections::{BTreeSet, HashSet};

/// The wires of a sheet grouped into the pieces they form, by LTspice's
/// rules (shared ends, an end landing on another wire, overlapping runs) but
/// without joining pieces that share a label.
struct Pieces {
    /// Item index and wire, for every wire.
    wires: Vec<(usize, Wire)>,
    parent: Vec<usize>,
    index: SegmentIndex,
}

impl Pieces {
    fn new(sch: &Schematic) -> Self {
        let wires: Vec<(usize, Wire)> = sch
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| match it {
                Item::Wire(w) => Some((i, *w)),
                _ => None,
            })
            .collect();
        let segs: Vec<(Point, Point)> = wires.iter().map(|(_, w)| (w.a, w.b)).collect();
        let (index, merged) = SegmentIndex::new(&segs);
        let mut p = Pieces {
            parent: (0..wires.len()).collect(),
            wires,
            index,
        };
        for (a, b) in merged {
            p.union(a, b);
        }
        for i in 0..p.wires.len() {
            let w = p.wires[i].1;
            for end in [w.a, w.b] {
                for j in p.covering(end) {
                    p.union(i, j);
                }
            }
        }
        p
    }

    /// Wires that pass through or end at a point.
    fn covering(&self, at: Point) -> Vec<usize> {
        let c = self.index.cover(at);
        c.horizontal
            .map(|s| s.wire)
            .into_iter()
            .chain(c.vertical.map(|s| s.wire))
            .chain(c.diagonal)
            .collect()
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra.max(rb)] = ra.min(rb);
        }
    }

    /// The pieces touching a point.
    fn at(&mut self, p: Point) -> BTreeSet<usize> {
        self.covering(p).into_iter().map(|w| self.find(w)).collect()
    }

    /// Item indices of the wires in a piece.
    fn members(&mut self, root: usize) -> Vec<usize> {
        let mut out = Vec::new();
        for i in 0..self.wires.len() {
            if self.find(i) == root {
                out.push(self.wires[i].0);
            }
        }
        out
    }
}

/// What taking things off the sheet removed.
#[derive(Debug, Default)]
struct Removed {
    wires: usize,
    labels: Vec<String>,
}

impl Removed {
    fn describe(&self) -> String {
        let mut parts = Vec::new();
        if self.wires > 0 {
            parts.push(format!("{} wire(s)", self.wires));
        }
        let mut labels: Vec<&str> = self.labels.iter().map(String::as_str).collect();
        labels.sort_unstable();
        labels.dedup();
        match labels.len() {
            0 => {}
            1 => parts.push(format!("label {}", labels[0])),
            _ => parts.push(format!("labels {}", labels.join(", "))),
        }
        parts.join(" and ")
    }
}

/// Remove wires and flags from the sheet. `flags_at` removes every flag on
/// one of the points.
fn take_out(
    sch: &mut Schematic,
    wires: &HashSet<usize>,
    flags_at: &HashSet<Point>,
    removed: &mut Removed,
) {
    let mut i = 0usize;
    sch.items.retain(|item| {
        let keep = match item {
            Item::Wire(_) => !wires.contains(&i),
            Item::Flag(f) => {
                let hit = flags_at.contains(&f.at);
                if hit {
                    removed.labels.push(f.label.clone());
                }
                !hit
            }
            _ => true,
        };
        if !keep && matches!(item, Item::Wire(_)) {
            removed.wires += 1;
        }
        i += 1;
        keep
    });
}

/// Points where a flag sits on a wire that is about to go, which will be
/// left touching nothing once it has gone.
fn orphaned_flags(sch: &Schematic, gone: &HashSet<usize>, pin_points: &[Point]) -> Vec<Point> {
    let doomed: Vec<Wire> = sch
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| match it {
            Item::Wire(w) if gone.contains(&i) => Some(*w),
            _ => None,
        })
        .collect();
    let kept: Vec<(Point, Point)> = sch
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| match it {
            Item::Wire(w) if !gone.contains(&i) => Some((w.a, w.b)),
            _ => None,
        })
        .collect();
    let (kept_index, _) = SegmentIndex::new(&kept);
    sch.flags()
        .map(|f| f.at)
        .filter(|&at| {
            doomed
                .iter()
                .any(|w| crate::geometry::on_segment(at, w.a, w.b))
        })
        .filter(|&at| {
            let c = kept_index.cover(at);
            c.horizontal.is_none()
                && c.vertical.is_none()
                && c.diagonal.is_empty()
                && !pin_points.contains(&at)
        })
        .collect()
}

/// Every pin on the sheet, from the connectivity.
fn all_pins(sch: &Schematic, lib: &SymbolLibrary) -> Vec<PinRef> {
    connect(sch, lib)
        .nets
        .iter()
        .flat_map(|n| n.pins.iter().cloned())
        .collect()
}

fn same_pin(p: &PinRef, inst: &str, pin: &str) -> bool {
    p.inst.eq_ignore_ascii_case(inst) && p.pin.eq_ignore_ascii_case(pin)
}

/// Clear the wire pieces at `points` that reach no pin outside `mine`, with
/// every label on them, and labels sitting right on those points.
fn clear_private_pieces(
    sch: &mut Schematic,
    lib: &SymbolLibrary,
    points: &[Point],
    is_mine: &dyn Fn(&PinRef) -> bool,
) -> Removed {
    let pins = all_pins(sch, lib);
    let others: Vec<Point> = pins.iter().filter(|p| !is_mine(p)).map(|p| p.at).collect();
    let mut pieces = Pieces::new(sch);
    let mut wires: HashSet<usize> = HashSet::new();
    let mut piece_points: Vec<(Point, Point)> = Vec::new();
    let roots: BTreeSet<usize> = points.iter().flat_map(|p| pieces.at(*p)).collect();
    for root in roots {
        let serves_others = others.iter().any(|q| pieces.at(*q).contains(&root));
        if serves_others {
            continue;
        }
        for item in pieces.members(root) {
            if let Item::Wire(w) = &sch.items[item] {
                piece_points.push((w.a, w.b));
            }
            wires.insert(item);
        }
    }
    let mut flags_at: HashSet<Point> = sch
        .flags()
        .map(|f| f.at)
        .filter(|&at| {
            piece_points
                .iter()
                .any(|(a, b)| crate::geometry::on_segment(at, *a, *b))
        })
        .collect();
    // A label right on one of the points, with no wire of anyone else
    // there, served only these pins.
    for &p in points {
        if !others.contains(&p) && pieces.at(p).is_empty() {
            flags_at.insert(p);
        }
    }
    let mut removed = Removed::default();
    take_out(sch, &wires, &flags_at, &mut removed);
    removed
}

/// Before a part is removed: take away the wire pieces and labels that only
/// its own pins use. Pieces other pins share stay as they are.
pub(crate) fn clear_part(
    sch: &mut Schematic,
    lib: &SymbolLibrary,
    inst: &str,
) -> (usize, Vec<String>) {
    let mine: Vec<Point> = all_pins(sch, lib)
        .iter()
        .filter(|p| p.inst.eq_ignore_ascii_case(inst))
        .map(|p| p.at)
        .collect();
    let is_mine = |p: &PinRef| p.inst.eq_ignore_ascii_case(inst);
    let removed = clear_private_pieces(sch, lib, &mine, &is_mine);
    (removed.wires, removed.labels)
}

/// Disconnect one pin. Returns the sentence for the edit report.
pub(crate) fn disconnect(
    sch: &mut Schematic,
    lib: &SymbolLibrary,
    spec: &str,
) -> Result<String, EditError> {
    let p = locate(sch, lib, spec)?;
    let conn = connect(sch, lib);
    let Some(old) = conn.net_of(&p.inst, &p.pin).cloned() else {
        return Ok(format!("{spec} was not connected"));
    };
    let others: Vec<PinRef> = old
        .pins
        .iter()
        .filter(|q| !same_pin(q, &p.inst, &p.pin))
        .cloned()
        .collect();
    if others.is_empty() && !old.labelled && old.wire_count == 0 {
        return Ok(format!("{spec} was not connected"));
    }

    // Pieces only this pin uses go whole, labels and all.
    let is_me = |q: &PinRef| same_pin(q, &p.inst, &p.pin);
    let mut removed = clear_private_pieces(sch, lib, &[p.at], &is_me);

    // Pieces other pins still use: only the wires ending on the pin go, and
    // labels left touching nothing.
    let pin_points: Vec<Point> = all_pins(sch, lib).iter().map(|q| q.at).collect();
    let gone: HashSet<usize> = sch
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| match it {
            Item::Wire(w) if w.a == p.at || w.b == p.at => Some(i),
            _ => None,
        })
        .collect();
    let mut flags_at: HashSet<Point> = orphaned_flags(sch, &gone, &pin_points)
        .into_iter()
        .filter(|at| *at != p.at)
        .collect();
    // A label right on the pin goes too.
    flags_at.insert(p.at);
    take_out(sch, &gone, &flags_at, &mut removed);
    removed.wires += prune_dangling(sch, lib);

    let after = connect(sch, lib);
    if let Some(n) = after.net_of(&p.inst, &p.pin)
        && (n.pins.len() > 1 || n.labelled)
    {
        let with: Vec<String> = n
            .pins
            .iter()
            .filter(|q| !same_pin(q, &p.inst, &p.pin))
            .map(|q| format!("{}.{}", q.inst, q.pin))
            .collect();
        return Err(EditError::Refused(format!(
            "{spec} is still connected (to {}): its pin sits on another pin or on a wire that runs through it. Move {} or remove that wire with remove_wire",
            if with.is_empty() {
                n.name.clone()
            } else {
                with.join(", ")
            },
            p.inst
        )));
    }

    // Join the other pins that shared the net back together.
    let kept = rejoin(sch, lib, &old, &others)?;
    let mut msg = format!("Disconnected {spec} from {}", old.name);
    let what = removed.describe();
    if !what.is_empty() {
        msg.push_str(&format!(" (removed {what})"));
    }
    if !kept.is_empty() {
        msg.push_str(&format!(
            "; {} kept on {}",
            kept.join(", "),
            if old.labelled {
                old.name.clone()
            } else {
                "their net".into()
            }
        ));
    }
    Ok(msg)
}

/// After a pin left a net, put the net's other pins back together: onto the
/// net's label when it had one, otherwise wired to each other. Returns the
/// pins that needed it.
fn rejoin(
    sch: &mut Schematic,
    lib: &SymbolLibrary,
    old: &crate::netlist::Net,
    others: &[PinRef],
) -> Result<Vec<String>, EditError> {
    let mut fixed = Vec::new();
    if others.is_empty() {
        return Ok(fixed);
    }
    let spec = |q: &PinRef| format!("{}.{}", q.inst, q.pin);
    if old.labelled {
        for q in others {
            let conn = connect(sch, lib);
            let on_it = conn
                .net_of(&q.inst, &q.pin)
                .is_some_and(|n| n.name.eq_ignore_ascii_case(&old.name));
            if !on_it {
                let mut scratch = super::EditReport::default();
                connect_to_net(sch, lib, &spec(q), &old.name, &mut scratch)?;
                fixed.push(spec(q));
            }
        }
        return Ok(fixed);
    }
    let anchor = locate(sch, lib, &spec(&others[0]))?;
    for q in &others[1..] {
        let conn = connect(sch, lib);
        let same = match (
            conn.net_of(&others[0].inst, &others[0].pin),
            conn.net_of(&q.inst, &q.pin),
        ) {
            (Some(a), Some(b)) => a.name == b.name,
            _ => false,
        };
        if same {
            continue;
        }
        let other: PinLoc = locate(sch, lib, &spec(q))?;
        let hint = format!("{}_{}", anchor.inst, anchor.pin);
        match (Router { lib }).connect(sch, &anchor, &other, &hint) {
            Route::Failed => {
                return Err(EditError::Refused(format!(
                    "{} and {} shared a net through the disconnected pin and could not be joined again",
                    spec(&others[0]),
                    spec(q)
                )));
            }
            _ => fixed.push(spec(q)),
        }
    }
    if !fixed.is_empty() {
        fixed.insert(0, spec(&others[0]));
    }
    Ok(fixed)
}
