//! Joining two pins with wires, safely.
//!
//! Candidate routes are tried from tidiest to most robust: a straight wire,
//! the two L-shapes, routes that step out from the pins first, and finally a
//! pair of net labels. Every candidate is checked by recomputing connectivity:
//! it must join the two nets and nothing else. A route that would touch a
//! third net, even at a single point, is rejected, so the router can never
//! create a short.

use super::pins::{PinLoc, bodies};
use crate::geometry::{GRID, Point, Rect};
use crate::netlist::{Connectivity, connect};
use crate::schematic::{Flag, Item, Schematic, Wire};
use crate::symbol::SymbolLibrary;
use std::collections::HashMap;

/// How a connection was made, for the edit report.
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    AlreadyConnected,
    Wires(Vec<Wire>),
    Labels { label: String, items: Vec<Item> },
}

type PinKey = (String, String);

/// Net index of every pin, keyed by upper-cased instance and pin name.
pub(crate) fn pin_partition(conn: &Connectivity) -> HashMap<PinKey, usize> {
    let mut map = HashMap::new();
    for (inst, pins) in &conn.pin_nets {
        for (pin, net) in pins {
            map.insert((inst.clone(), pin.to_ascii_uppercase()), *net);
        }
    }
    map
}

fn key(p: &PinLoc) -> PinKey {
    (p.inst.to_ascii_uppercase(), p.pin.to_ascii_uppercase())
}

/// Whether `after` differs from `before` only by joining the nets that the
/// pins in `joined` were on.
pub(crate) fn only_joins(
    before: &HashMap<PinKey, usize>,
    after: &HashMap<PinKey, usize>,
    joined: &[PinKey],
) -> bool {
    let allowed: Vec<usize> = joined
        .iter()
        .filter_map(|k| before.get(k).copied())
        .collect();
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, b) in before {
        let Some(a) = after.get(k) else { return false };
        let g = groups.entry(*a).or_default();
        if !g.contains(b) {
            g.push(*b);
        }
    }
    groups
        .values()
        .all(|g| g.len() == 1 || g.iter().all(|n| allowed.contains(n)))
}

fn segment_crosses(a: Point, b: Point, r: &Rect) -> bool {
    // Axis-aligned segment against a rectangle's open interior.
    let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
    let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
    x1 > r.min.x && x0 < r.max.x && y1 > r.min.y && y0 < r.max.y
}

fn path_wires(points: &[Point]) -> Vec<Wire> {
    points
        .windows(2)
        .filter(|w| w[0] != w[1])
        .map(|w| Wire::new(w[0], w[1]))
        .collect()
}

/// Wire routes between two pins, tidiest first.
fn candidates(a: &PinLoc, b: &PinLoc) -> Vec<Vec<Point>> {
    let (p, q) = (a.at, b.at);
    let mut out = Vec::new();
    if p.x == q.x || p.y == q.y {
        out.push(vec![p, q]);
    }
    out.push(vec![p, Point::new(q.x, p.y), q]);
    out.push(vec![p, Point::new(p.x, q.y), q]);
    for k in [2, 3, 4, 6] {
        let pa = p.offset(a.out.0 * GRID * k, a.out.1 * GRID * k);
        let qb = q.offset(b.out.0 * GRID * k, b.out.1 * GRID * k);
        out.push(vec![p, pa, Point::new(q.x, pa.y), q]);
        out.push(vec![p, pa, Point::new(pa.x, q.y), q]);
        out.push(vec![p, pa, Point::new(qb.x, pa.y), qb, q]);
        out.push(vec![p, pa, Point::new(pa.x, qb.y), qb, q]);
    }
    out
}

pub(crate) struct Router<'a> {
    pub lib: &'a SymbolLibrary,
}

impl Router<'_> {
    /// Connect two pins. `label_hint` is used if labels are needed and neither
    /// net is labelled yet.
    pub fn connect(&self, sch: &mut Schematic, a: &PinLoc, b: &PinLoc, label_hint: &str) -> Route {
        let before_conn = connect(sch, self.lib);
        let before = pin_partition(&before_conn);
        let (ka, kb) = (key(a), key(b));
        if before.contains_key(&ka) && before.get(&ka) == before.get(&kb) {
            return Route::AlreadyConnected;
        }
        let blockers: Vec<Rect> = bodies(sch, self.lib).into_iter().map(|(_, r)| r).collect();
        for path in candidates(a, b) {
            let wires = path_wires(&path);
            if wires
                .iter()
                .any(|w| blockers.iter().any(|r| segment_crosses(w.a, w.b, r)))
            {
                continue;
            }
            let mut trial = sch.clone();
            for w in &wires {
                trial.insert(Item::Wire(*w));
            }
            let after = pin_partition(&connect(&trial, self.lib));
            if after.get(&ka) == after.get(&kb)
                && only_joins(&before, &after, &[ka.clone(), kb.clone()])
            {
                *sch = trial;
                return Route::Wires(wires);
            }
        }
        self.label(sch, a, b, &before_conn, label_hint)
    }

    /// Join two pins with a pair of net labels on short stubs, or labels right
    /// on the pins if stubs would touch something else.
    fn label(
        &self,
        sch: &mut Schematic,
        a: &PinLoc,
        b: &PinLoc,
        conn: &Connectivity,
        hint: &str,
    ) -> Route {
        let net_a = conn.net_of(&a.inst, &a.pin);
        let net_b = conn.net_of(&b.inst, &b.pin);
        let label = [net_a, net_b]
            .into_iter()
            .flatten()
            .find(|n| n.is_ground())
            .or_else(|| [net_a, net_b].into_iter().flatten().find(|n| n.labelled))
            .map(|n| n.name.clone())
            .unwrap_or_else(|| unique_label(conn, hint));
        let before = pin_partition(conn);
        let keys = [key(a), key(b)];
        for stub in [2, 1, 0] {
            let mut trial = sch.clone();
            let mut items = Vec::new();
            for p in [a, b] {
                let end = p.at.offset(p.out.0 * GRID * stub, p.out.1 * GRID * stub);
                if end != p.at {
                    let w = Item::Wire(Wire::new(p.at, end));
                    trial.insert(w.clone());
                    items.push(w);
                }
                let f = Item::Flag(Flag {
                    at: end,
                    label: label.clone(),
                });
                trial.insert(f.clone());
                items.push(f);
            }
            let after = pin_partition(&connect(&trial, self.lib));
            if after.get(&keys[0]) == after.get(&keys[1]) && only_joins(&before, &after, &keys) {
                *sch = trial;
                return Route::Labels { label, items };
            }
        }
        // Unreachable in practice: a label placed exactly on each pin joins
        // only those pins' nets. Keep the schematic unchanged if it ever fails.
        Route::Labels {
            label,
            items: Vec::new(),
        }
    }
}

/// A label not used anywhere in the schematic yet.
pub(crate) fn unique_label(conn: &Connectivity, hint: &str) -> String {
    let base: String = hint
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    let base = if base.is_empty() {
        "net".to_string()
    } else {
        base.to_ascii_lowercase()
    };
    let taken = |n: &str| {
        conn.nets.iter().any(|x| {
            x.name.eq_ignore_ascii_case(n) || x.labels.iter().any(|l| l.eq_ignore_ascii_case(n))
        })
    };
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|i| format!("{base}{i}"))
        .find(|n| !taken(n))
        .expect("unbounded")
}
