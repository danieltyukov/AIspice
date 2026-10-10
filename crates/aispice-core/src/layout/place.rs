//! Placing parts so the drawing reads like a textbook schematic.
//!
//! Parts are placed one at a time, growing out from the input source the way
//! the signal travels. For each part every reasonable orientation, text side
//! and nearby grid position is scored, and the cheapest spot that keeps clear
//! of what is already placed wins. The score is the drawing conventions
//! written down: short, straight connections to what is already on the
//! part's nets; ground pins facing down and positive rails facing up; signal
//! running left to right; series parts lying along the path and shunt parts
//! standing between a node and ground; feedback parts above the stage they
//! wrap; no predicted crossings.
//!
//! Each connection the placer expects to draw is remembered as a sketch of
//! straight segments, so a later part can plan to tee into that wire rather
//! than only aiming at pins, which is how a feedback resistor finds the wire
//! into an op-amp input.

use super::circuit::{Circuit, Kind, NetKind, PinRole};
use super::geom::{Dir, core_body, orient_dir, overlaps, place_rect, segment_hits};
use super::symbols::{orientations, symbol, text_can_flip};
use super::text::symbol_texts;
use crate::geometry::{GRID, Orient, Point, Rect, snap};
use std::collections::HashSet;

/// Space kept between the zones (body, text, flags) of neighbouring parts.
const GAP: i32 = 24;

/// Convention weights, in the same unit as wire estimates (grid steps). They
/// are set so a convention outweighs a few steps of wire: a person accepts a
/// slightly longer wire to keep ground at the bottom and the signal flowing
/// right.
const W_SIDEWAYS: f64 = 10.0;
const W_BACKWARD: f64 = 10.0;
const W_UPRIGHT_SERIES: f64 = 10.0;
const W_FEEDBACK_SIDE: f64 = 8.0;
const W_CROSSING: f64 = 6.0;
const W_THROUGH_BODY: f64 = 12.0;
const W_THROUGH_TEXT: f64 = 4.0;

/// Choices a candidate layout makes differently from another.
#[derive(Debug, Clone, Default)]
pub(crate) struct Strategy {
    /// Allowed orientations per device, overriding the defaults.
    pub orient: Vec<Option<Vec<Orient>>>,
    /// Put feedback parts below the stage instead of above.
    pub feedback_below: bool,
    /// Place active parts before the shunt parts on the same node.
    pub actives_first: bool,
    /// Extra spacing between parts, in grid steps.
    pub spread: i32,
    /// Place a feedback part as soon as both its ends are on the sheet, so
    /// the parts hanging off its node are placed around it.
    pub feedback_first: bool,
    /// Start from this device instead of the input source: the core of the
    /// circuit first, its sources fitted around it afterwards.
    pub seed: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Placement {
    pub at: Point,
    pub orient: Orient,
    /// Name and value on the other side of the body.
    pub flip_text: bool,
}

/// A pin already on the sheet.
#[derive(Debug, Clone, Copy)]
struct PinAt {
    p: Point,
    f: Dir,
}

/// Geometry of one device in one orientation, relative to its origin.
#[derive(Debug, Clone)]
struct Shape {
    orient: Orient,
    flip_text: bool,
    pref: f64,
    pins: Vec<(Point, Dir)>,
    zone: Rect,
    body: Rect,
    /// Name and value texts and the flags at its pins: wires should not run
    /// through them.
    texts: Vec<Rect>,
}

struct Placer<'a> {
    c: &'a Circuit,
    st: &'a Strategy,
    shapes: Vec<Vec<Shape>>,
    pos: Vec<Option<Placement>>,
    /// Shape index chosen for each placed device.
    chosen: Vec<usize>,
    /// Devices in the order they were placed.
    order: Vec<usize>,
    zones: Vec<Option<Rect>>,
    bodies: Vec<Option<Rect>>,
    texts: Vec<Vec<Rect>>,
    net_pins: Vec<Vec<PinAt>>,
    /// Planned wires per net, as straight segments.
    net_segs: Vec<Vec<(Point, Point)>>,
}

/// Room a flag needs beside a pin: a ground symbol below it, or a rail label
/// in the direction the pin faces.
fn flag_zone(p: Point, f: Dir, kind: NetKind, name: &str) -> Option<Rect> {
    match kind {
        NetKind::Ground => Some(match f {
            Dir::Down => Rect::from_points(p.offset(-16, 0), p.offset(16, 24)),
            Dir::Left | Dir::Right => {
                let s = f.step(p, 32);
                Rect::from_points(
                    Point::new(p.x.min(s.x) - 16, p.y),
                    Point::new(p.x.max(s.x) + 16, p.y + 40),
                )
            }
            Dir::Up => Rect::from_points(p.offset(-48, -40), p.offset(48, 8)),
        }),
        NetKind::Rail { .. } => {
            let len = (name.chars().count() as i32 * 12).max(24) + 8;
            Some(match f {
                Dir::Up => Rect::from_points(p.offset(-10, -len), p.offset(10, 0)),
                Dir::Down => Rect::from_points(p.offset(-10, 0), p.offset(10, len)),
                Dir::Left => Rect::from_points(p.offset(-len - 16, -10), p.offset(0, 10)),
                Dir::Right => Rect::from_points(p.offset(0, -10), p.offset(len + 16, 10)),
            })
        }
        NetKind::Signal => None,
    }
}

fn translate(r: Rect, by: Point) -> Rect {
    Rect {
        min: r.min + by,
        max: r.max + by,
    }
}

/// Fewest corners on a path that leaves heading `a` and arrives moving `b`,
/// with `d` the offset from start to end in grid steps, plus any detour it
/// forces.
fn bends(a: Dir, b: Dir, d: (i32, i32)) -> (u32, i32) {
    let along = |dir: Dir| {
        let (x, y) = dir.vec();
        x * d.0 + y * d.1
    };
    let perp_zero = |dir: Dir| {
        if dir.horizontal() { d.1 == 0 } else { d.0 == 0 }
    };
    if a == b {
        if along(a) > 0 && perp_zero(a) {
            (0, 0)
        } else if along(a) > 0 {
            (2, 0)
        } else {
            (4, 4)
        }
    } else if a == b.opposite() {
        if perp_zero(a) { (4, 4) } else { (2, 2) }
    } else if along(a) > 0 && along(b) > 0 {
        (1, 0)
    } else {
        (3, 2)
    }
}

/// The path a wire would most likely take from `p` (leaving along `a`) to
/// `q` (arriving moving along `b`), as corner points, when it has at most two
/// corners.
fn sketch_path(p: Point, a: Dir, q: Point, b: Dir, n: u32) -> Vec<Point> {
    match n {
        0 => vec![p, q],
        1 => {
            let corner = if a.horizontal() {
                Point::new(q.x, p.y)
            } else {
                Point::new(p.x, q.y)
            };
            vec![p, corner, q]
        }
        2 if a == b => {
            if a.horizontal() {
                let mx = snap((p.x + q.x) / 2);
                vec![p, Point::new(mx, p.y), Point::new(mx, q.y), q]
            } else {
                let my = snap((p.y + q.y) / 2);
                vec![p, Point::new(p.x, my), Point::new(q.x, my), q]
            }
        }
        2 => {
            if a.horizontal() {
                let x = if a == Dir::Right {
                    p.x.max(q.x) + 2 * GRID
                } else {
                    p.x.min(q.x) - 2 * GRID
                };
                vec![p, Point::new(x, p.y), Point::new(x, q.y), q]
            } else {
                let y = if a == Dir::Down {
                    p.y.max(q.y) + 2 * GRID
                } else {
                    p.y.min(q.y) - 2 * GRID
                };
                vec![p, Point::new(p.x, y), Point::new(q.x, y), q]
            }
        }
        _ => Vec::new(),
    }
}

/// Estimated cost, in grid steps, of a wire from pin `p` facing `fp` to `q`,
/// arriving in one of `arrive` directions: its length, two and a half steps
/// per corner, and any detour. Returns the sketch of the cheapest shape.
fn estimate(p: Point, fp: Dir, q: Point, arrive: &[Dir]) -> (f64, Vec<Point>) {
    let d = ((q.x - p.x) / GRID, (q.y - p.y) / GRID);
    if d == (0, 0) {
        return (4.0, vec![p]);
    }
    let len = d.0.abs() + d.1.abs();
    // Pins touching or nearly touching read as a crowded drawing.
    let short = if len < 2 { 3.0 } else { 0.0 };
    let mut best = (f64::INFINITY, Vec::new());
    for &b in arrive {
        let (n, detour) = bends(fp, b, d);
        // A Z with a tiny offset reads as a crooked wire, not a deliberate
        // route: pins that nearly line up should line up.
        let jog = if n == 2 && fp == b {
            let off = if fp.horizontal() {
                d.1.abs()
            } else {
                d.0.abs()
            };
            if off <= 2 { 4.0 } else { 0.0 }
        } else {
            0.0
        };
        let cost = len as f64 + detour as f64 + 2.5 * n as f64 + short + jog;
        if cost < best.0 {
            best = (cost, sketch_path(p, fp, q, b, n));
        }
    }
    best
}

/// The point of segment `s` nearest to `p`.
fn project(p: Point, s: (Point, Point)) -> Point {
    let (a, b) = s;
    Point::new(
        p.x.clamp(a.x.min(b.x), a.x.max(b.x)),
        p.y.clamp(a.y.min(b.y), a.y.max(b.y)),
    )
}

/// Whether two axis-aligned segments cross at a point inside both.
fn crosses(a: (Point, Point), b: (Point, Point)) -> bool {
    let (h, v) = if a.0.y == a.1.y && b.0.x == b.1.x {
        (a, b)
    } else if b.0.y == b.1.y && a.0.x == a.1.x {
        (b, a)
    } else {
        return false;
    };
    let (x0, x1) = (h.0.x.min(h.1.x), h.0.x.max(h.1.x));
    let (y0, y1) = (v.0.y.min(v.1.y), v.0.y.max(v.1.y));
    v.0.x > x0 && v.0.x < x1 && h.0.y > y0 && h.0.y < y1
}

impl<'a> Placer<'a> {
    fn new(c: &'a Circuit, st: &'a Strategy) -> Self {
        let mut shapes = Vec::new();
        for (i, d) in c.devices.iter().enumerate() {
            let allowed = st.orient.get(i).cloned().flatten();
            let mut list = Vec::new();
            for (o, pref) in orientations(d) {
                if allowed.as_ref().is_some_and(|a| !a.contains(&o)) {
                    continue;
                }
                let flips: &[bool] = if text_can_flip(d, o) {
                    &[false, true]
                } else {
                    &[false]
                };
                for &flip in flips {
                    // The input source sits at the left edge: its text reads
                    // best on the outside, leaving room for the first stage.
                    let side = match (Some(i) == c.input, flip) {
                        (true, true) | (false, false) => 0.0,
                        (true, false) => 0.5,
                        (false, true) => {
                            if d.kind.transistor() {
                                2.0
                            } else {
                                1.0
                            }
                        }
                    };
                    list.push(Self::shape(c, i, o, flip, pref + side));
                }
            }
            shapes.push(list);
        }
        Placer {
            c,
            st,
            shapes,
            pos: vec![None; c.devices.len()],
            chosen: vec![0; c.devices.len()],
            order: Vec::new(),
            zones: vec![None; c.devices.len()],
            bodies: vec![None; c.devices.len()],
            texts: vec![Vec::new(); c.devices.len()],
            net_pins: vec![Vec::new(); c.nets.len()],
            net_segs: vec![Vec::new(); c.nets.len()],
        }
    }

    fn shape(c: &Circuit, i: usize, o: Orient, flip: bool, pref: f64) -> Shape {
        let d = &c.devices[i];
        let origin = Point::new(0, 0);
        let pins: Vec<(Point, Dir)> = d
            .pins
            .iter()
            .map(|p| (o.apply(p.at), orient_dir(p.facing, o)))
            .collect();
        let sym = symbol(d, origin, o, flip);
        let body = core_body(&d.def)
            .map(|b| place_rect(b, origin, o))
            .unwrap_or(Rect::from_points(origin, origin));
        let mut zone = body;
        let mut texts = symbol_texts(&sym, &d.def);
        for (k, &(p, f)) in pins.iter().enumerate() {
            zone.include(p);
            let net = &c.nets[d.pins[k].net];
            if let Some(z) = flag_zone(p, f, net.kind, &net.name) {
                // The flag's own pin sits on the zone's edge; only what lies
                // beyond it should keep wires away.
                texts.push(z);
            }
        }
        for t in &texts {
            zone = zone.union(*t);
        }
        Shape {
            orient: o,
            flip_text: flip,
            pref,
            pins,
            zone,
            body,
            texts,
        }
    }

    fn gap(&self) -> i32 {
        GAP + self.st.spread * GRID
    }

    fn fits(&self, zone: Rect) -> bool {
        let z = zone.inflate(self.gap() / 2);
        !self
            .zones
            .iter()
            .flatten()
            .any(|o| overlaps(&z, &o.inflate(self.gap() / 2)))
    }

    /// The cheapest planned connection from pin `p` to net `net` as placed so
    /// far: to one of its pins, or teeing into one of its planned wires.
    fn best_link(&self, net: usize, p: Point, f: Dir) -> Option<(f64, Vec<Point>)> {
        let mut best: Option<(f64, Vec<Point>)> = None;
        let mut take = |cand: (f64, Vec<Point>)| {
            if best.as_ref().is_none_or(|b| cand.0 < b.0) {
                best = Some(cand);
            }
        };
        for q in &self.net_pins[net] {
            take(estimate(p, f, q.p, &[q.f.opposite()]));
        }
        for &s in &self.net_segs[net] {
            let horizontal = s.0.y == s.1.y;
            let step = f.step(p, 2 * GRID);
            for t in [project(p, s), project(step, s), s.0, s.1] {
                let interior = t != s.0 && t != s.1;
                // Into the middle of a wire only at a right angle.
                let arrive: Vec<Dir> = Dir::ALL
                    .into_iter()
                    .filter(|d| !interior || d.horizontal() != horizontal)
                    .collect();
                let (cost, path) = estimate(p, f, t, &arrive);
                // A tee is a junction, slightly worse than meeting a pin.
                take((cost + 0.5, path));
            }
        }
        best
    }

    /// Crossings with other nets' planned wires and passes through placed
    /// bodies, for a sketched path.
    fn sketch_penalty(
        &self,
        net: usize,
        path: &[Point],
        own_body: Option<Rect>,
        own_texts: &[Rect],
    ) -> f64 {
        let mut pen = 0.0;
        for w in path.windows(2) {
            let seg = (w[0], w[1]);
            for (n, segs) in self.net_segs.iter().enumerate() {
                if n == net {
                    continue;
                }
                for &s in segs {
                    if crosses(seg, s) {
                        pen += W_CROSSING;
                    }
                }
            }
            for b in self.bodies.iter().flatten().chain(own_body.iter()) {
                if segment_hits(w[0], w[1], &b.inflate(4)) {
                    pen += W_THROUGH_BODY;
                }
            }
            for t in self.texts.iter().flatten().chain(own_texts.iter()) {
                if segment_hits(w[0], w[1], &t.inflate(-2)) {
                    pen += W_THROUGH_TEXT;
                }
            }
        }
        pen
    }

    /// Planned wires that would run through a candidate's own texts.
    fn text_penalty(&self, texts: &[Rect], at: Point) -> f64 {
        let mut pen = 0.0;
        for t in texts {
            let t = translate(*t, at).inflate(-2);
            for segs in &self.net_segs {
                for &(a, b) in segs {
                    if segment_hits(a, b, &t) {
                        pen += W_THROUGH_TEXT;
                    }
                }
            }
        }
        pen
    }

    fn commit(&mut self, d: usize, shape: usize, at: Point) {
        let s = &self.shapes[d][shape];
        self.zones[d] = Some(translate(s.zone, at));
        self.bodies[d] = Some(translate(s.body, at));
        self.texts[d] = s.texts.iter().map(|t| translate(*t, at)).collect();
        self.pos[d] = Some(Placement {
            at,
            orient: s.orient,
            flip_text: s.flip_text,
        });
        self.chosen[d] = shape;
        if !self.order.contains(&d) {
            self.order.push(d);
        }
        self.add_pins(d);
    }

    /// Sketch a placed part's connections, then add its pins. Sketching
    /// first makes them aim at what was there already.
    fn add_pins(&mut self, d: usize) {
        let Some(pl) = self.pos[d] else { return };
        let s = self.shapes[d][self.chosen[d]].clone();
        for (k, &(rel, f)) in s.pins.iter().enumerate() {
            let net = self.c.devices[d].pins[k].net;
            if !self.c.is_signal(net) {
                continue;
            }
            if let Some((_, path)) = self.best_link(net, rel + pl.at, f) {
                for w in path.windows(2) {
                    if w[0] != w[1] {
                        self.net_segs[net].push((w[0], w[1]));
                    }
                }
            }
        }
        for (k, &(p, f)) in s.pins.iter().enumerate() {
            let net = self.c.devices[d].pins[k].net;
            self.net_pins[net].push(PinAt { p: p + pl.at, f });
        }
    }

    /// Take a part off the sheet and redo every sketch without it.
    fn remove(&mut self, d: usize) {
        self.zones[d] = None;
        self.bodies[d] = None;
        self.texts[d].clear();
        self.pos[d] = None;
        self.resketch();
    }

    /// Rebuild pins and planned wires by replaying the placed parts in the
    /// order they went down.
    fn resketch(&mut self) {
        for v in self.net_pins.iter_mut() {
            v.clear();
        }
        for v in self.net_segs.iter_mut() {
            v.clear();
        }
        for d in self.order.clone() {
            if self.pos[d].is_some() {
                self.add_pins(d);
            }
        }
    }

    /// Move each part, one at a time, to its best spot with every other part
    /// fixed. Undoes some of the short-sightedness of placing in order: a
    /// mirror transistor placed before its partner can turn to face it.
    fn refine(&mut self, passes: usize) {
        for _ in 0..passes {
            let mut moved = false;
            for d in self.order.clone() {
                let has_signal = self.c.devices[d].nets().any(|n| self.c.is_signal(n));
                let Some(old) = self.pos[d] else { continue };
                if !has_signal {
                    continue;
                }
                let old_shape = self.chosen[d];
                self.remove(d);
                let old_cost = self.cost(d, old_shape, old.at, true);
                match (self.best_spot(d), old_cost) {
                    (Some((c, shape, at)), Some(oc)) if c < oc - 0.5 => {
                        self.commit(d, shape, at);
                        moved = true;
                    }
                    _ => self.commit(d, old_shape, old.at),
                }
                self.resketch();
            }
            if !moved {
                break;
            }
        }
    }

    /// Score a candidate; `None` if it collides. `full` adds the checks for
    /// predicted crossings and wires through bodies.
    fn cost(&self, d: usize, shape: usize, at: Point, full: bool) -> Option<f64> {
        let s = &self.shapes[d][shape];
        if !self.fits(translate(s.zone, at)) {
            return None;
        }
        let dev = &self.c.devices[d];
        let own_body = translate(s.body, at);
        let own_texts: Vec<Rect> = if full {
            s.texts.iter().map(|t| translate(*t, at)).collect()
        } else {
            Vec::new()
        };
        let mut total = self.conventions(d, shape, at);
        for (k, &(rel, f)) in s.pins.iter().enumerate() {
            let p = rel + at;
            let net = dev.pins[k].net;
            if !self.c.is_signal(net) {
                continue;
            }
            if let Some((cost, path)) = self.best_link(net, p, f) {
                total += cost;
                if full {
                    total += self.sketch_penalty(net, &path, Some(own_body), &own_texts);
                }
            }
        }
        if full {
            total += self.text_penalty(&s.texts, at);
        }
        Some(total + self.symmetry_bonus(d, shape, at))
    }

    /// A cheap lower-quality score for ranking candidates: conventions plus
    /// the straight-line distance from each signal pin to its net.
    fn quick_cost(&self, d: usize, shape: usize, at: Point) -> Option<f64> {
        let s = &self.shapes[d][shape];
        if !self.fits(translate(s.zone, at)) {
            return None;
        }
        let dev = &self.c.devices[d];
        let mut total = self.conventions(d, shape, at);
        for (k, &(rel, _)) in s.pins.iter().enumerate() {
            let net = dev.pins[k].net;
            if !self.c.is_signal(net) {
                continue;
            }
            let p = rel + at;
            let near = self.net_pins[net]
                .iter()
                .map(|q| q.p)
                .chain(self.net_segs[net].iter().map(|&s| project(p, s)))
                .map(|q| ((q.x - p.x).abs() + (q.y - p.y).abs()) / GRID)
                .min();
            if let Some(dist) = near {
                total += dist as f64;
            }
        }
        Some(total)
    }

    /// The drawing-convention part of a candidate's score: orientation
    /// preference, which way ground and rail pins face, signal direction,
    /// and how two-terminal parts lie.
    fn conventions(&self, d: usize, shape: usize, at: Point) -> f64 {
        let s = &self.shapes[d][shape];
        let dev = &self.c.devices[d];
        let mut total = s.pref;
        let mut signal_pins = Vec::new();
        let supply_source = dev.kind == Kind::Source && dev.nets().all(|n| !self.c.is_signal(n));
        for (k, &(rel, f)) in s.pins.iter().enumerate() {
            let p = rel + at;
            let pin = &dev.pins[k];
            let net = &self.c.nets[pin.net];
            match net.kind {
                NetKind::Ground => {
                    total += match f {
                        Dir::Down => 0.0,
                        Dir::Left | Dir::Right => W_SIDEWAYS,
                        Dir::Up => 2.0 * W_SIDEWAYS,
                    }
                }
                // A supply source drawn on its own only needs its ground
                // pin at the bottom; its rail label goes wherever the other
                // pin points.
                NetKind::Rail { .. } if supply_source => {}
                NetKind::Rail { positive } => {
                    let toward = if positive { Dir::Up } else { Dir::Down };
                    total += if f == toward {
                        0.0
                    } else if f == toward.opposite() {
                        1.6 * W_SIDEWAYS
                    } else {
                        0.8 * W_SIDEWAYS
                    };
                }
                NetKind::Signal => signal_pins.push((p, f, pin.net, pin.role)),
            }
        }
        // Signal flows left to right: through a part, the deeper net's pin
        // should not be left of the shallower one's.
        for &(p1, _, n1, r1) in &signal_pins {
            for &(p2, f2, n2, r2) in &signal_pins {
                let (Some(d1), Some(d2)) = (self.c.nets[n1].depth, self.c.nets[n2].depth) else {
                    continue;
                };
                if d1 >= d2 || n1 == n2 {
                    continue;
                }
                let active_pair =
                    dev.kind.active() && r1 == PinRole::Input && matches!(r2, PinRole::Output);
                if dev.kind.two_terminal() || active_pair {
                    if p2.x < p1.x {
                        total += W_BACKWARD;
                    }
                    if Some(n2) == self.c.output && f2 == Dir::Left {
                        total += 3.0;
                    }
                }
            }
        }
        // Two-terminal parts: along the path between two signal nets, upright
        // between a node and ground or a rail.
        if dev.kind.two_terminal() && s.pins.len() == 2 {
            let horizontal = s.pins[0].1.horizontal();
            let kinds = (
                self.c.nets[dev.pins[0].net].kind,
                self.c.nets[dev.pins[1].net].kind,
            );
            let both_signal = kinds.0 == NetKind::Signal && kinds.1 == NetKind::Signal;
            if both_signal && !horizontal && !self.c.feedback[d] && dev.kind != Kind::Source {
                total += W_UPRIGHT_SERIES;
            }
            if self.c.feedback[d] {
                // Above (or below) the parts it wraps.
                let ys: Vec<i32> = dev
                    .pins
                    .iter()
                    .flat_map(|p| self.net_pins[p.net].iter().map(|q| q.p.y))
                    .collect();
                if let (Some(&top), Some(&bottom)) = (ys.iter().min(), ys.iter().max()) {
                    let y = s.pins[0].0.y + at.y;
                    if !self.st.feedback_below && y > top {
                        total += W_FEEDBACK_SIDE;
                    }
                    if self.st.feedback_below && y < bottom {
                        total += W_FEEDBACK_SIDE;
                    }
                }
                if !horizontal {
                    total += W_UPRIGHT_SERIES * 0.6;
                }
            }
        }
        total
    }

    /// Candidate origins for `d` in `shape`: positions that put one of its
    /// pins near a placed pin of the same net, or level with one of the
    /// net's planned wires.
    fn candidates(&self, d: usize, shape: usize, reach: i32) -> Vec<Point> {
        let s = &self.shapes[d][shape];
        let dev = &self.c.devices[d];
        let mut out = HashSet::new();
        for (k, &(rel, _)) in s.pins.iter().enumerate() {
            let net = dev.pins[k].net;
            if !self.c.is_signal(net) {
                continue;
            }
            let anchors: Vec<Point> = self.net_pins[net]
                .iter()
                .rev()
                .take(3)
                .map(|q| q.p)
                .chain(self.net_segs[net].iter().rev().take(4).map(|s| s.1))
                .collect();
            for q in anchors {
                // Close by, every grid point.
                for dx in -reach..=reach {
                    for dy in -reach..=reach {
                        out.insert(q.offset(dx * GRID, dy * GRID) - rel);
                    }
                }
                // Farther out, the positions that line the pin up with the
                // anchor (straight wires), and a coarse grid for corners.
                let far = reach * 3;
                for k in -far..=far {
                    out.insert(q.offset(k * GRID, 0) - rel);
                    out.insert(q.offset(0, k * GRID) - rel);
                }
                for dx in (-far..=far).step_by(3) {
                    for dy in (-far..=far).step_by(3) {
                        out.insert(q.offset(dx * GRID, dy * GRID) - rel);
                    }
                }
            }
        }
        // Mirror images of a matched transistor already placed: differential
        // pairs and current mirrors read best drawn symmetrically.
        if dev.kind.transistor() {
            for (i, other) in self.c.devices.iter().enumerate() {
                let Some(pl) = self.pos[i] else { continue };
                if i == d || other.kind != dev.kind || !matched(dev, other) {
                    continue;
                }
                if s.orient != mirror_of(pl.orient) {
                    continue;
                }
                for k in 3..=28 {
                    for sign in [-1, 1] {
                        out.insert(Point::new(pl.at.x + sign * k * GRID, pl.at.y));
                    }
                }
            }
        }
        out.into_iter().collect()
    }

    /// Place `d` at its best spot near what is already placed.
    fn place_near(&mut self, d: usize) -> bool {
        match self.best_spot(d) {
            Some((_, shape, at)) => {
                self.commit(d, shape, at);
                true
            }
            None => false,
        }
    }

    /// The best spot for `d` near what is already placed.
    fn best_spot(&self, d: usize) -> Option<(f64, usize, Point)> {
        let order = |a: &(f64, usize, Point), b: &(f64, usize, Point)| {
            a.0.total_cmp(&b.0)
                .then((a.1, a.2.x, a.2.y).cmp(&(b.1, b.2.x, b.2.y)))
        };
        for reach in [6, 12, 20] {
            // A quick pass ranks every candidate that fits by conventions
            // and straight-line distance; the estimates of wire shapes, and
            // then of crossings, only run on the best of them.
            let mut quick: Vec<(f64, usize, Point)> = Vec::new();
            for shape in 0..self.shapes[d].len() {
                for at in self.candidates(d, shape, reach) {
                    if let Some(cost) = self.quick_cost(d, shape, at) {
                        quick.push((cost, shape, at));
                    }
                }
            }
            if quick.is_empty() {
                continue;
            }
            quick.sort_by(order);
            quick.truncate(160);
            let mut scored: Vec<(f64, usize, Point)> = quick
                .into_iter()
                .filter_map(|(_, shape, at)| self.cost(d, shape, at, false).map(|c| (c, shape, at)))
                .collect();
            scored.sort_by(order);
            scored.truncate(40);
            let best = scored
                .iter()
                .filter_map(|&(_, shape, at)| self.cost(d, shape, at, true).map(|c| (c, shape, at)))
                .min_by(|a, b| {
                    a.0.total_cmp(&b.0)
                        .then((a.2.x, a.2.y).cmp(&(b.2.x, b.2.y)))
                });
            if best.is_some() {
                return best;
            }
        }
        None
    }

    /// A reward for drawing a matched transistor as the mirror image of its
    /// partner at the same height.
    fn symmetry_bonus(&self, d: usize, shape: usize, at: Point) -> f64 {
        let dev = &self.c.devices[d];
        if !dev.kind.transistor() {
            return 0.0;
        }
        let s = &self.shapes[d][shape];
        let own = translate(s.body, at);
        let own_cx = own.min.x + own.max.x;
        let mut bonus = 0.0;
        for (i, other) in self.c.devices.iter().enumerate() {
            let (Some(pl), Some(body)) = (self.pos[i], self.bodies[i]) else {
                continue;
            };
            if i == d || other.kind != dev.kind || !matched(dev, other) {
                continue;
            }
            // In a differential pair the two inputs face away from each
            // other; in a mirror the shared gate or base faces the partner.
            let toward = if body.min.x + body.max.x > own_cx {
                Dir::Right
            } else {
                Dir::Left
            };
            if let Some(k) = dev.pins.iter().position(|p| p.role == PinRole::Input) {
                let f = s.pins[k].1;
                let shared = dev.pins[k].net == other.pins[k].net;
                if f.horizontal() && shared != (f == toward) {
                    bonus += 8.0;
                }
            }
            if pl.at.y == at.y && s.orient == mirror_of(pl.orient) && s.flip_text == pl.flip_text {
                bonus -= 6.0;
            }
        }
        bonus
    }

    /// Bounds of everything placed so far.
    fn bounds(&self) -> Option<Rect> {
        self.zones
            .iter()
            .flatten()
            .copied()
            .reduce(|a, b| a.union(b))
    }

    /// Place a part with nothing placed on its signal nets: to the right of
    /// the drawing, top-aligned, in the orientation it likes best.
    fn place_apart(&mut self, d: usize) {
        let b = self.bounds();
        let mut best: Option<(f64, usize, Point)> = None;
        for shape in 0..self.shapes[d].len() {
            let s = &self.shapes[d][shape];
            let base = match b {
                None => Point::new(0, 0) - Point::new(0, s.zone.min.y),
                Some(b) => Point::new(b.max.x + 96 - s.zone.min.x, b.min.y - s.zone.min.y),
            };
            for row in 0..24 {
                for col in 0..6 {
                    let at =
                        Point::new(snap(base.x + col * 4 * GRID), snap(base.y + row * 3 * GRID));
                    if let Some(cost) = self.cost(d, shape, at, true) {
                        let cost = cost + row as f64 * 0.5 + col as f64 * 2.0;
                        if best.is_none_or(|(c, _, _)| cost < c) {
                            best = Some((cost, shape, at));
                        }
                    }
                }
            }
        }
        let (_, shape, at) = best.unwrap_or_else(|| {
            let b = self
                .bounds()
                .unwrap_or(Rect::from_points(Point::new(0, 0), Point::new(0, 0)));
            (0.0, 0, Point::new(snap(b.max.x + 160), snap(b.max.y + 160)))
        });
        self.commit(d, shape, at);
    }

    /// Which part to place next.
    fn next(&self) -> Option<usize> {
        let c = self.c;
        let unplaced: Vec<usize> = (0..c.devices.len())
            .filter(|&d| self.pos[d].is_none())
            .collect();
        if unplaced.is_empty() {
            return None;
        }
        let placed_net = |n: usize| !self.net_pins[n].is_empty();
        let all_placed = |d: usize| {
            c.devices[d]
                .nets()
                .filter(|&n| c.is_signal(n))
                .all(placed_net)
        };
        let class = |d: usize| -> u32 {
            let dev = &c.devices[d];
            let signal = dev.nets().filter(|&n| c.is_signal(n)).count();
            let base = if c.feedback[d] {
                return if self.st.feedback_first { 0 } else { 4 };
            } else if dev.kind.two_terminal() && signal == 1 {
                u32::from(self.st.actives_first)
            } else if !dev.kind.two_terminal() {
                u32::from(!self.st.actives_first)
            } else {
                2
            };
            base + 1
        };
        let frontier: Vec<usize> = unplaced
            .iter()
            .copied()
            .filter(|&d| {
                let touches = c.devices[d].nets().any(|n| c.is_signal(n) && placed_net(n));
                touches && (!c.feedback[d] || all_placed(d))
            })
            .collect();
        if !frontier.is_empty() {
            return frontier.into_iter().min_by_key(|&d| {
                let depth = c.devices[d]
                    .nets()
                    .filter(|&n| c.is_signal(n) && placed_net(n))
                    .filter_map(|n| c.nets[n].depth)
                    .min()
                    .unwrap_or(u32::MAX);
                (class(d), depth, d)
            });
        }
        // Feedback parts whose nets will never all be placed: place anyway.
        if let Some(&d) = unplaced
            .iter()
            .find(|&&d| c.devices[d].nets().any(|n| c.is_signal(n) && placed_net(n)))
        {
            return Some(d);
        }
        // A new start: the seed or the input first, then whatever is closest
        // to it, with supply sources last.
        if let Some(seed) = self.st.seed
            && self.pos[seed].is_none()
        {
            return Some(seed);
        }
        unplaced.iter().copied().min_by_key(|&d| {
            let dev = &c.devices[d];
            let is_input = Some(d) == c.input;
            let signal = dev.nets().any(|n| c.is_signal(n));
            let depth = dev
                .nets()
                .filter_map(|n| c.nets[n].depth)
                .min()
                .unwrap_or(u32::MAX);
            (!is_input, !signal, depth, d)
        })
    }
}

/// The left-right mirror of a transistor orientation.
fn mirror_of(o: Orient) -> Orient {
    match o {
        Orient::R0 => Orient::M0,
        Orient::M0 => Orient::R0,
        Orient::M180 => Orient::R180,
        Orient::R180 => Orient::M180,
        o => o,
    }
}

/// Two transistors that belong together: a differential pair (shared
/// emitter or source) or a mirror (shared base or gate).
fn matched(a: &super::circuit::Device, b: &super::circuit::Device) -> bool {
    a.pins
        .iter()
        .zip(&b.pins)
        .any(|(x, y)| x.net == y.net && matches!(x.role, PinRole::Common | PinRole::Input))
}

/// Place every device. Returns one placement per device and the total
/// drawing-convention cost of the result, for comparing strategies.
pub(crate) fn place(c: &Circuit, st: &Strategy) -> (Vec<Placement>, f64) {
    let mut p = Placer::new(c, st);
    while let Some(d) = p.next() {
        let anchored = c.devices[d]
            .nets()
            .any(|n| c.is_signal(n) && !p.net_pins[n].is_empty());
        if !(anchored && p.place_near(d)) {
            p.place_apart(d);
        }
    }
    p.refine(2);
    let conventions = (0..c.devices.len())
        .map(|d| p.conventions(d, p.chosen[d], p.pos[d].expect("placed").at))
        .sum();
    (
        p.pos.into_iter().map(|x| x.expect("all placed")).collect(),
        conventions,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_facing_pins_need_no_corners() {
        let e = estimate(
            Point::new(0, 0),
            Dir::Right,
            Point::new(64, 0),
            &[Dir::Right],
        );
        assert_eq!(e.0, 4.0);
        assert_eq!(e.1, vec![Point::new(0, 0), Point::new(64, 0)]);
        // Offset vertically: a Z, two corners.
        let z = estimate(
            Point::new(0, 0),
            Dir::Right,
            Point::new(64, 64),
            &[Dir::Right],
        );
        assert_eq!(z.0, 8.0 + 5.0);
        assert_eq!(z.1.len(), 4);
        // Nearly lined up: the small jog costs extra, so pins line up.
        let jog = estimate(
            Point::new(0, 0),
            Dir::Right,
            Point::new(64, 32),
            &[Dir::Right],
        );
        assert!(jog.0 > 6.0 + 5.0, "{jog:?}");
        // Facing away from each other: a detour.
        let u = estimate(Point::new(0, 0), Dir::Left, Point::new(64, 0), &[Dir::Left]);
        assert!(u.0 > 14.0);
    }

    #[test]
    fn crossings_need_interiors() {
        let h = (Point::new(0, 0), Point::new(64, 0));
        assert!(crosses(h, (Point::new(32, -16), Point::new(32, 16))));
        assert!(!crosses(h, (Point::new(64, -16), Point::new(64, 16))));
    }
}
