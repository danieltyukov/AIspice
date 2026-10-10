//! Where to draw junction dots, which pins are left open, and which way a
//! net label's wire runs.
//!
//! A point is a junction when three or more connections meet there. Each wire
//! ending at the point counts once, each pin once, and a wire passing straight
//! through it counts twice (its two halves), so a T made of a wire end on the
//! middle of another wire gets a dot. Two wires that merely cross have no end
//! or pin at the crossing and never get one, matching how LTspice connects
//! wires only at end points.

use crate::geometry::{Point, on_segment};
use crate::schematic::Wire;
use std::collections::BTreeSet;

pub(crate) struct Connectivity {
    pub junctions: Vec<Point>,
    pub open_pins: Vec<Point>,
}

/// How many connections meet at `p`.
fn degree(p: Point, wires: &[Wire], pins: &[Point]) -> usize {
    let mut n = pins.iter().filter(|&&q| q == p).count();
    for w in wires {
        if w.a == p || w.b == p {
            n += 1;
        } else if on_segment(p, w.a, w.b) {
            n += 2;
        }
    }
    n
}

/// Junctions among wire ends and pins, and pins nothing touches. A pin with a
/// flag on it counts as connected.
pub(crate) fn analyse(wires: &[Wire], pins: &[Point], flags: &[Point]) -> Connectivity {
    let wires: Vec<Wire> = wires.iter().filter(|w| w.a != w.b).copied().collect();
    let candidates: BTreeSet<Point> = wires
        .iter()
        .flat_map(|w| [w.a, w.b])
        .chain(pins.iter().copied())
        .collect();
    let junctions = candidates
        .into_iter()
        .filter(|&p| degree(p, &wires, pins) >= 3)
        .collect();
    let mut open_pins: Vec<Point> = pins
        .iter()
        .copied()
        .filter(|&p| degree(p, &wires, pins) == 1 && !flags.contains(&p))
        .collect();
    open_pins.sort();
    open_pins.dedup();
    Connectivity {
        junctions,
        open_pins,
    }
}

/// Which sides of a point have something attached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Sides {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
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
        } else {
            self.down = true;
        }
    }
}

/// The directions in which wires leave `p`, plus the direction of the body
/// of each symbol with a pin at `p` (`pins` pairs a pin with its symbol's
/// centre).
pub(crate) fn sides(p: Point, wires: &[Wire], pins: &[(Point, Point)]) -> Sides {
    let mut s = Sides::default();
    for w in wires.iter().filter(|w| w.a != w.b) {
        if w.a == p {
            s.mark(w.b.x - p.x, w.b.y - p.y);
        } else if w.b == p {
            s.mark(w.a.x - p.x, w.a.y - p.y);
        } else if on_segment(p, w.a, w.b) {
            s.mark(w.a.x - p.x, w.a.y - p.y);
            s.mark(w.b.x - p.x, w.b.y - p.y);
        }
    }
    for &(pin, centre) in pins {
        if pin == p {
            s.mark(centre.x - p.x, centre.y - p.y);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(x1: i32, y1: i32, x2: i32, y2: i32) -> Wire {
        Wire::new(Point::new(x1, y1), Point::new(x2, y2))
    }

    #[test]
    fn t_junction_gets_a_dot() {
        let wires = [w(0, 0, 64, 0), w(32, 0, 32, 48)];
        let c = analyse(&wires, &[], &[]);
        assert_eq!(c.junctions, vec![Point::new(32, 0)]);
    }

    #[test]
    fn four_way_meeting_gets_one_dot() {
        let wires = [
            w(0, 0, 32, 0),
            w(32, 0, 64, 0),
            w(32, -32, 32, 0),
            w(32, 0, 32, 32),
        ];
        let c = analyse(&wires, &[], &[]);
        assert_eq!(c.junctions, vec![Point::new(32, 0)]);
    }

    #[test]
    fn crossing_wires_do_not_connect() {
        let wires = [w(0, 0, 64, 0), w(32, -32, 32, 32)];
        let c = analyse(&wires, &[], &[]);
        assert!(c.junctions.is_empty());
    }

    #[test]
    fn corners_and_pin_ends_are_not_junctions() {
        let wires = [w(0, 0, 32, 0), w(32, 0, 32, 32)];
        let pins = [Point::new(0, 0)];
        let c = analyse(&wires, &pins, &[]);
        assert!(c.junctions.is_empty());
        assert!(c.open_pins.is_empty());
    }

    #[test]
    fn pins_count_towards_junctions() {
        // Two wires ending on a pin: three connections.
        let wires = [w(0, 0, 32, 0), w(32, 0, 32, 32)];
        let pins = [Point::new(32, 0)];
        let c = analyse(&wires, &pins, &[]);
        assert_eq!(c.junctions, vec![Point::new(32, 0)]);
        // A pin on the middle of a wire also makes a T.
        let c = analyse(&[w(0, 64, 64, 64)], &[Point::new(16, 64)], &[]);
        assert_eq!(c.junctions, vec![Point::new(16, 64)]);
    }

    #[test]
    fn open_pins_ignore_flagged_ones() {
        let pins = [Point::new(0, 0), Point::new(0, 64), Point::new(0, 128)];
        let wires = [w(0, 64, 32, 64)];
        let flags = [Point::new(0, 128)];
        let c = analyse(&wires, &pins, &flags);
        assert_eq!(c.open_pins, vec![Point::new(0, 0)]);
    }

    #[test]
    fn zero_length_wires_are_ignored() {
        let wires = [w(0, 0, 64, 0), w(32, 0, 32, 0), w(32, 0, 32, 0)];
        let c = analyse(&wires, &[], &[]);
        assert!(c.junctions.is_empty());
    }

    #[test]
    fn sides_follow_wires_and_symbol_bodies() {
        let wires = [w(0, 0, -48, 0), w(0, 0, 0, 32)];
        let s = sides(Point::new(0, 0), &wires, &[]);
        assert_eq!(
            s,
            Sides {
                left: true,
                down: true,
                ..Sides::default()
            }
        );
        let s = sides(
            Point::new(10, 10),
            &[],
            &[(Point::new(10, 10), Point::new(10, 60))],
        );
        assert!(s.down && !s.up && !s.left && !s.right);
    }
}
