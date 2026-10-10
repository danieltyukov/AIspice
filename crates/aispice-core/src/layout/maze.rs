//! A maze router on the 16-unit grid that knows LTspice's connection rules.
//!
//! LTspice joins a wire to anything that lands on it: another wire's end, a
//! pin, a flag. So a route for one net may cross another net's wire at a right
//! angle, but may never run along it, turn on it, end on it, or pass over a
//! pin, flag or wire end of another net. Part bodies are kept out with a
//! clearance margin; a pin inside that margin may only be left straight out
//! along its lead, which is how a person draws it. Within those rules the
//! search minimises length, corners and crossings, and keeps away from text
//! and from other wires where it can.

use super::geom::Dir;
use crate::geometry::{GRID, Point, Rect};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Clearance between a part's drawn body and any wire not leaving one of its
/// pins.
pub(crate) const CLEARANCE: i32 = 12;

/// Search costs, in tenths of a grid step.
const STEP: u32 = 10;
const BEND: u32 = 40;
const CROSS: u32 = 80;

#[derive(Debug, Clone, Copy, Default)]
struct Cell {
    /// Inside a part's clearance zone.
    keepout: bool,
    /// Net (plus one) that may pass through the keep-out here, along a pin
    /// lead, and whether that lead runs horizontally.
    corridor: u32,
    corridor_h: bool,
    /// Net (plus one) with a pin, flag or wire end here. Other nets may not
    /// touch the point at all.
    owner: u32,
    /// Net (plus one) of a wire running straight through.
    hwire: u32,
    vwire: u32,
    /// Extra cost of passing here: near bodies, through text, beside wires.
    cost: u32,
}

/// Where a route may start: a point and the directions it may leave in.
#[derive(Debug, Clone)]
pub(crate) struct Start {
    pub at: Point,
    pub dirs: Vec<Dir>,
}

pub(crate) struct Grid {
    origin: Point,
    w: i32,
    h: i32,
    cells: Vec<Cell>,
    /// Net (plus one) of a wire along the edge to the right of / below a cell.
    hedge: Vec<u32>,
    vedge: Vec<u32>,
}

impl Grid {
    /// A grid covering `area`, extended to grid lines.
    pub fn new(area: Rect) -> Grid {
        let min = Point::new(
            area.min.x.div_euclid(GRID) * GRID,
            area.min.y.div_euclid(GRID) * GRID,
        );
        let w = (area.max.x - min.x) / GRID + 2;
        let h = (area.max.y - min.y) / GRID + 2;
        let n = (w * h).max(1) as usize;
        Grid {
            origin: min,
            w,
            h,
            cells: vec![Cell::default(); n],
            hedge: vec![0; n],
            vedge: vec![0; n],
        }
    }

    fn idx(&self, p: Point) -> Option<usize> {
        let dx = p.x - self.origin.x;
        let dy = p.y - self.origin.y;
        if dx < 0 || dy < 0 || dx % GRID != 0 || dy % GRID != 0 {
            return None;
        }
        let (gx, gy) = (dx / GRID, dy / GRID);
        (gx < self.w && gy < self.h).then(|| (gy * self.w + gx) as usize)
    }

    fn point(&self, i: usize) -> Point {
        let i = i as i32;
        Point::new(
            self.origin.x + (i % self.w) * GRID,
            self.origin.y + (i / self.w) * GRID,
        )
    }

    fn neighbour(&self, i: usize, d: Dir) -> Option<usize> {
        let p = self.point(i);
        self.idx(d.step(p, GRID))
    }

    /// Grid points strictly inside `r`.
    fn inside(&self, r: Rect) -> Vec<usize> {
        let mut out = Vec::new();
        let x0 = (r.min.x).div_euclid(GRID) * GRID;
        let y0 = (r.min.y).div_euclid(GRID) * GRID;
        let mut y = y0;
        while y < r.max.y {
            let mut x = x0;
            while x < r.max.x {
                if x > r.min.x
                    && y > r.min.y
                    && let Some(i) = self.idx(Point::new(x, y))
                {
                    out.push(i);
                }
                x += GRID;
            }
            y += GRID;
        }
        out
    }

    /// Keep wires out of a part body (already including its clearance).
    pub fn add_keepout(&mut self, r: Rect) {
        for i in self.inside(r) {
            self.cells[i].keepout = true;
        }
    }

    /// Make passing through `r` cost extra.
    pub fn add_cost(&mut self, r: Rect, cost: u32) {
        for i in self.inside(r) {
            self.cells[i].cost = self.cells[i].cost.saturating_add(cost);
        }
    }

    /// A pin of `net` facing `facing`. If it sits in a keep-out, its lead
    /// becomes a corridor that only this net may use, straight out.
    pub fn add_pin(&mut self, at: Point, facing: Dir, net: u32) {
        let Some(i) = self.idx(at) else { return };
        self.cells[i].owner = net + 1;
        let mut j = Some(i);
        while let Some(k) = j {
            if !self.cells[k].keepout {
                break;
            }
            self.cells[k].corridor = net + 1;
            self.cells[k].corridor_h = facing.horizontal();
            j = self.neighbour(k, facing);
        }
    }

    /// A flag of `net` at `at`, and the area its symbol or text covers.
    pub fn add_flag(&mut self, at: Point, net: u32) {
        if let Some(i) = self.idx(at) {
            self.cells[i].owner = net + 1;
        }
    }

    /// A wire of `net`. Its ends and corners belong to the net; its interior
    /// may be crossed by other nets but not followed.
    pub fn add_wire(&mut self, a: Point, b: Point, net: u32) {
        let tag = net + 1;
        for p in [a, b] {
            if let Some(i) = self.idx(p) {
                self.cells[i].owner = tag;
            }
        }
        let Some(d) = Dir::from_vec(b.x - a.x, b.y - a.y) else {
            return;
        };
        let mut p = a;
        while p != b {
            let q = d.step(p, GRID);
            if let (Some(i), Some(j)) = (self.idx(p), self.idx(q)) {
                match d {
                    Dir::Right => self.hedge[i] = tag,
                    Dir::Left => self.hedge[j] = tag,
                    Dir::Down => self.vedge[i] = tag,
                    Dir::Up => self.vedge[j] = tag,
                }
                if q != b {
                    if d.horizontal() {
                        self.cells[j].hwire = tag;
                    } else {
                        self.cells[j].vwire = tag;
                    }
                }
            }
            p = q;
        }
        // Keep other wires a track away where they can be.
        let (dx, dy) = d.vec();
        let side = Point::new(dy.abs() * GRID, dx.abs() * GRID);
        let lo = Point::new(a.x.min(b.x), a.y.min(b.y));
        let hi = Point::new(a.x.max(b.x), a.y.max(b.y));
        let r = Rect::from_points(
            lo.offset(-side.x - 1, -side.y - 1),
            hi.offset(side.x + 1, side.y + 1),
        );
        self.add_cost(r, 3);
    }

    /// Whether a flag or label of `net` may sit at `at`.
    pub fn free_for(&self, at: Point, net: u32) -> bool {
        let Some(i) = self.idx(at) else { return false };
        let c = &self.cells[i];
        let tag = net + 1;
        !c.keepout
            && (c.owner == 0 || c.owner == tag)
            && (c.hwire == 0 || c.hwire == tag)
            && (c.vwire == 0 || c.vwire == tag)
    }

    /// Whether a straight wire for `net` from `a` to `b` is legal, with `a`
    /// and `b` allowed to be the net's own points.
    pub fn segment_free(&self, a: Point, b: Point, net: u32) -> bool {
        self.segment_free_for(a, b, &[net])
    }

    /// [`Grid::segment_free`] for a wire that may touch any of the nets in
    /// `own` (a route joining two nets may touch both).
    pub fn segment_free_for(&self, a: Point, b: Point, own: &[u32]) -> bool {
        let tags: Vec<u32> = own.iter().map(|n| n + 1).collect();
        let mine = |t: u32| t == 0 || tags.contains(&t);
        let Some(d) = Dir::from_vec(b.x - a.x, b.y - a.y) else {
            return a == b && own.iter().any(|&n| self.free_for(a, n));
        };
        let mut p = a;
        loop {
            let Some(i) = self.idx(p) else { return false };
            let c = &self.cells[i];
            if !mine(c.owner) {
                return false;
            }
            let along_corridor = tags.contains(&c.corridor) && c.corridor_h == d.horizontal();
            if c.keepout && !along_corridor {
                return false;
            }
            let (along, across) = if d.horizontal() {
                (c.hwire, c.vwire)
            } else {
                (c.vwire, c.hwire)
            };
            if !mine(along) {
                return false;
            }
            // Ends of the new wire may not land on another net's wire.
            if (p == a || p == b) && !mine(across) {
                return false;
            }
            if p == b {
                return true;
            }
            let q = d.step(p, GRID);
            let Some(j) = self.idx(q) else { return false };
            let e = match d {
                Dir::Right => self.hedge[i],
                Dir::Left => self.hedge[j],
                Dir::Down => self.vedge[i],
                Dir::Up => self.vedge[j],
            };
            if !mine(e) {
                return false;
            }
            p = q;
        }
    }

    /// Like [`Grid::segment_free`] for a new stub leaving `a`, which must also
    /// stay off every wire and point already drawn, the net's own included: a
    /// stub laid over an existing wire reads as a junction that is not there.
    pub fn stub_free(&self, a: Point, b: Point, net: u32) -> bool {
        if !self.segment_free(a, b, net) {
            return false;
        }
        let Some(d) = Dir::from_vec(b.x - a.x, b.y - a.y) else {
            return a == b;
        };
        let mut p = a;
        while p != b {
            let q = d.step(p, GRID);
            let (Some(i), Some(j)) = (self.idx(p), self.idx(q)) else {
                return false;
            };
            let e = match d {
                Dir::Right => self.hedge[i],
                Dir::Left => self.hedge[j],
                Dir::Down => self.vedge[i],
                Dir::Up => self.vedge[j],
            };
            let c = &self.cells[j];
            if e != 0 || c.owner != 0 || c.hwire != 0 || c.vwire != 0 {
                return false;
            }
            p = q;
        }
        true
    }

    /// Cheapest legal route for nets `own` from one of `starts` to any of
    /// `goals`. Returns the corner points, start first.
    pub fn route(
        &self,
        starts: &[Start],
        goals: &[Point],
        own: &[u32],
        limit: u32,
    ) -> Option<Vec<Point>> {
        let tags: Vec<u32> = own.iter().map(|n| n + 1).collect();
        let mine = |t: u32| t == 0 || tags.contains(&t);
        let mut is_goal = vec![false; self.cells.len()];
        let mut gbox: Option<Rect> = None;
        for g in goals {
            if let Some(i) = self.idx(*g) {
                is_goal[i] = true;
                let r = Rect::from_points(*g, *g);
                gbox = Some(gbox.map_or(r, |b| b.union(r)));
            }
        }
        let gbox = gbox?;
        let h = |i: usize| -> u32 {
            let p = self.point(i);
            let dx = (gbox.min.x - p.x).max(p.x - gbox.max.x).max(0);
            let dy = (gbox.min.y - p.y).max(p.y - gbox.max.y).max(0);
            ((dx + dy) / GRID) as u32 * STEP
        };
        let n = self.cells.len() * 5;
        let mut best = vec![u32::MAX; n];
        let mut parent = vec![usize::MAX; n];
        let mut heap = BinaryHeap::new();
        for s in starts {
            let Some(i) = self.idx(s.at) else { continue };
            let st = i * 5 + 4;
            best[st] = 0;
            heap.push(Reverse((h(i), 0u32, st)));
        }
        let mut found = None;
        while let Some(Reverse((_, g, st))) = heap.pop() {
            if g > best[st] {
                continue;
            }
            let (i, din) = (st / 5, st % 5);
            if din != 4 && is_goal[i] {
                found = Some(st);
                break;
            }
            if g > limit {
                break;
            }
            let start_dirs: Option<&Vec<Dir>> = if din == 4 {
                starts
                    .iter()
                    .find(|s| self.idx(s.at) == Some(i))
                    .map(|s| &s.dirs)
            } else {
                None
            };
            let ci = &self.cells[i];
            for d in Dir::ALL {
                if let Some(dirs) = start_dirs {
                    if !dirs.contains(&d) {
                        continue;
                    }
                } else if din != 4 {
                    let prev = Dir::ALL[din];
                    if d == prev.opposite() {
                        continue;
                    }
                    // No corner on another net's wire, nor inside a corridor.
                    if d != prev {
                        if !mine(ci.hwire) || !mine(ci.vwire) {
                            continue;
                        }
                        if ci.keepout {
                            continue;
                        }
                    }
                }
                // Leaving a corridor cell only along the corridor.
                if ci.keepout && din != 4 && ci.corridor_h != d.horizontal() {
                    continue;
                }
                let Some(j) = self.neighbour(i, d) else {
                    continue;
                };
                let edge = match d {
                    Dir::Right => self.hedge[i],
                    Dir::Left => self.hedge[j],
                    Dir::Down => self.vedge[i],
                    Dir::Up => self.vedge[j],
                };
                if !mine(edge) {
                    continue;
                }
                let cj = &self.cells[j];
                if !mine(cj.owner) {
                    continue;
                }
                if cj.keepout && !(tags.contains(&cj.corridor) && cj.corridor_h == d.horizontal()) {
                    continue;
                }
                let (along, across) = if d.horizontal() {
                    (cj.hwire, cj.vwire)
                } else {
                    (cj.vwire, cj.hwire)
                };
                if !mine(along) {
                    continue;
                }
                let mut cost = STEP + cj.cost;
                if !mine(across) {
                    // Crossing: legal only straight through, never ending here.
                    if is_goal[j] {
                        continue;
                    }
                    cost += CROSS;
                }
                if din != 4 && Dir::ALL[din] != d {
                    cost += BEND;
                }
                let ng = g + cost;
                let ns = j * 5 + d.index();
                if ng < best[ns] {
                    best[ns] = ng;
                    parent[ns] = st;
                    heap.push(Reverse((ng + h(j), ng, ns)));
                }
            }
        }
        let mut st = found?;
        let mut cells = vec![st / 5];
        while parent[st] != usize::MAX {
            st = parent[st];
            cells.push(st / 5);
        }
        cells.reverse();
        let pts: Vec<Point> = cells.into_iter().map(|i| self.point(i)).collect();
        Some(corners(&pts))
    }

    /// The cost of a finished route, for comparing alternatives.
    pub fn path_cost(&self, path: &[Point], own: &[u32]) -> u32 {
        let tags: Vec<u32> = own.iter().map(|n| n + 1).collect();
        let mut cost = 0;
        for (k, w) in path.windows(2).enumerate() {
            let Some(d) = Dir::from_vec(w[1].x - w[0].x, w[1].y - w[0].y) else {
                continue;
            };
            if k > 0 {
                cost += BEND;
            }
            let mut p = w[0];
            while p != w[1] {
                p = d.step(p, GRID);
                if let Some(i) = self.idx(p) {
                    let c = &self.cells[i];
                    cost += STEP + c.cost;
                    let across = if d.horizontal() { c.vwire } else { c.hwire };
                    if across != 0 && !tags.contains(&across) {
                        cost += CROSS;
                    }
                }
            }
        }
        cost
    }
}

/// Drop the points of a grid path that are not corners.
pub(crate) fn corners(pts: &[Point]) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::new();
    for &p in pts {
        if out.last() == Some(&p) {
            continue;
        }
        if out.len() >= 2 {
            let a = out[out.len() - 2];
            let b = out[out.len() - 1];
            let collinear = (a.x == b.x && b.x == p.x) || (a.y == b.y && b.y == p.y);
            if collinear {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::from_points(Point::new(-160, -160), Point::new(320, 320))
    }

    #[test]
    fn straight_route_when_clear() {
        let g = Grid::new(area());
        let path = g
            .route(
                &[Start {
                    at: Point::new(0, 0),
                    dirs: vec![Dir::Right],
                }],
                &[Point::new(128, 0)],
                &[0],
                10_000,
            )
            .unwrap();
        assert_eq!(path, vec![Point::new(0, 0), Point::new(128, 0)]);
    }

    #[test]
    fn crosses_but_never_follows_or_touches_another_net() {
        let mut g = Grid::new(area());
        // Net 1 runs vertically through the straight line.
        g.add_wire(Point::new(64, -64), Point::new(64, 64), 1);
        let path = g
            .route(
                &[Start {
                    at: Point::new(0, 0),
                    dirs: vec![Dir::Right],
                }],
                &[Point::new(128, 0)],
                &[0],
                10_000,
            )
            .unwrap();
        // Crossing straight through is allowed and cheaper than detouring.
        assert_eq!(path, vec![Point::new(0, 0), Point::new(128, 0)]);
        // A goal sitting on the other wire is unreachable.
        assert!(
            g.route(
                &[Start {
                    at: Point::new(0, 0),
                    dirs: vec![Dir::Right],
                }],
                &[Point::new(64, 0)],
                &[0],
                10_000,
            )
            .is_none()
        );
    }

    #[test]
    fn keepout_forces_a_detour_but_corridors_let_pins_out() {
        let mut g = Grid::new(area());
        g.add_keepout(Rect::from_points(Point::new(40, -40), Point::new(88, 40)));
        g.add_pin(Point::new(48, 0), Dir::Left, 0);
        let path = g
            .route(
                &[Start {
                    at: Point::new(48, 0),
                    dirs: vec![Dir::Left],
                }],
                &[Point::new(-64, 0)],
                &[0],
                10_000,
            )
            .unwrap();
        assert_eq!(path, vec![Point::new(48, 0), Point::new(-64, 0)]);
        let around = g
            .route(
                &[Start {
                    at: Point::new(0, 0),
                    dirs: vec![Dir::Right],
                }],
                &[Point::new(128, 0)],
                &[1],
                10_000,
            )
            .unwrap();
        assert!(around.len() > 2, "{around:?}");
    }
}
