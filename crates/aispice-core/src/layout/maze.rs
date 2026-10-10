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

/// Most cells a grid may have. A large real schematic is well under a
/// million at the 16-unit grid; a sheet with a stray item far away must not
/// make the router allocate gigabytes.
pub(crate) const MAX_CELLS: i64 = 2_000_000;

/// Largest coordinate magnitude a grid may cover. Keeping well inside `i32`
/// means stepping from any cell of a grid can never overflow.
pub(crate) const COORD_LIMIT: i64 = 1 << 28;

/// `r` grown by `by` on every side, saturating instead of overflowing, for
/// rectangles that come from untrusted files.
pub(crate) fn grown(r: Rect, by: i32) -> Rect {
    Rect {
        min: Point::new(r.min.x.saturating_sub(by), r.min.y.saturating_sub(by)),
        max: Point::new(r.max.x.saturating_add(by), r.max.y.saturating_add(by)),
    }
}

impl Grid {
    /// A grid covering `area`, extended to grid lines, or `None` if the area
    /// is beyond [`COORD_LIMIT`] or would need more than [`MAX_CELLS`] cells.
    pub fn new(area: Rect) -> Option<Grid> {
        let g = GRID as i64;
        let (x0, y0) = (area.min.x as i64, area.min.y as i64);
        let (x1, y1) = (area.max.x as i64, area.max.y as i64);
        if [x0, y0, x1, y1].iter().any(|v| v.abs() > COORD_LIMIT) || x1 < x0 || y1 < y0 {
            return None;
        }
        let (mx, my) = (x0.div_euclid(g) * g, y0.div_euclid(g) * g);
        let w = (x1 - mx) / g + 2;
        let h = (y1 - my) / g + 2;
        let n = w.checked_mul(h)?;
        if n > MAX_CELLS {
            return None;
        }
        let n = n.max(1) as usize;
        Some(Grid {
            origin: Point::new(mx as i32, my as i32),
            w: w as i32,
            h: h as i32,
            cells: vec![Cell::default(); n],
            hedge: vec![0; n],
            vedge: vec![0; n],
        })
    }

    /// Number of cells, for tests that check a grid stayed small.
    #[cfg(test)]
    pub(crate) fn cell_count(&self) -> usize {
        self.cells.len()
    }

    fn idx(&self, p: Point) -> Option<usize> {
        let g = GRID as i64;
        let dx = p.x as i64 - self.origin.x as i64;
        let dy = p.y as i64 - self.origin.y as i64;
        if dx < 0 || dy < 0 || dx % g != 0 || dy % g != 0 {
            return None;
        }
        let (gx, gy) = (dx / g, dy / g);
        (gx < self.w as i64 && gy < self.h as i64).then(|| (gy * self.w as i64 + gx) as usize)
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

    /// The grid lines (as coordinates) from `lo` to `hi` inclusive, clipped to
    /// the grid along one axis whose origin is `origin` and length `cells`.
    fn lines(origin: i32, cells: i32, lo: i64, hi: i64) -> std::ops::RangeInclusive<i64> {
        let g = GRID as i64;
        let first = origin as i64;
        let last = first + (cells as i64 - 1) * g;
        let lo = lo.max(first);
        let hi = hi.min(last);
        let start = first + (lo - first + g - 1).div_euclid(g) * g;
        let end = first + (hi - first).div_euclid(g) * g;
        start..=end
    }

    /// Grid points strictly inside `r`, clipped to the grid.
    fn inside(&self, r: Rect) -> Vec<usize> {
        let mut out = Vec::new();
        let g = GRID as usize;
        let xs = Self::lines(
            self.origin.x,
            self.w,
            r.min.x as i64 + 1,
            r.max.x as i64 - 1,
        );
        let ys = Self::lines(
            self.origin.y,
            self.h,
            r.min.y as i64 + 1,
            r.max.y as i64 - 1,
        );
        for y in ys.step_by(g) {
            for x in xs.clone().step_by(g) {
                if let Some(i) = self.idx(Point::new(x as i32, y as i32)) {
                    out.push(i);
                }
            }
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

    /// The grid points on segment `a`-`b` that lie inside the grid, in order
    /// from the lower coordinate. Wires off the grid's lines have none, and a
    /// wire running far outside the grid costs only its part inside.
    pub fn points_on(&self, a: Point, b: Point) -> Vec<Point> {
        let g = GRID as usize;
        if a.y == b.y {
            if (a.y as i64 - self.origin.y as i64).rem_euclid(GRID as i64) != 0 {
                return Vec::new();
            }
            let (lo, hi) = (a.x.min(b.x) as i64, a.x.max(b.x) as i64);
            Self::lines(self.origin.x, self.w, lo, hi)
                .step_by(g)
                .map(|x| Point::new(x as i32, a.y))
                .filter(|p| self.idx(*p).is_some())
                .collect()
        } else if a.x == b.x {
            if (a.x as i64 - self.origin.x as i64).rem_euclid(GRID as i64) != 0 {
                return Vec::new();
            }
            let (lo, hi) = (a.y.min(b.y) as i64, a.y.max(b.y) as i64);
            Self::lines(self.origin.y, self.h, lo, hi)
                .step_by(g)
                .map(|y| Point::new(a.x, y as i32))
                .filter(|p| self.idx(*p).is_some())
                .collect()
        } else {
            Vec::new()
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
        let horizontal = a.y == b.y;
        if !horizontal && a.x != b.x {
            return;
        }
        let pts = self.points_on(a, b);
        for (k, p) in pts.iter().enumerate() {
            let Some(i) = self.idx(*p) else { continue };
            if *p != a && *p != b {
                if horizontal {
                    self.cells[i].hwire = tag;
                } else {
                    self.cells[i].vwire = tag;
                }
            }
            // The edge to the next grid point, when both lie on the wire.
            if let Some(next) = pts.get(k + 1)
                && (next.x - p.x).abs() + (next.y - p.y).abs() == GRID
            {
                if horizontal {
                    self.hedge[i] = tag;
                } else {
                    self.vedge[i] = tag;
                }
            }
        }
        // Keep other wires a track away where they can be.
        let side = if horizontal {
            Point::new(0, GRID)
        } else {
            Point::new(GRID, 0)
        };
        let lo = Point::new(a.x.min(b.x), a.y.min(b.y));
        let hi = Point::new(a.x.max(b.x), a.y.max(b.y));
        let r = grown(Rect::from_points(lo, hi), 1);
        let r = Rect {
            min: Point::new(
                r.min.x.saturating_sub(side.x),
                r.min.y.saturating_sub(side.y),
            ),
            max: Point::new(
                r.max.x.saturating_add(side.x),
                r.max.y.saturating_add(side.y),
            ),
        };
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
        // Each state is settled once; the budget only stops a search that
        // would otherwise wander a huge or pathological maze.
        let mut budget = n.saturating_mul(2);
        while let Some(Reverse((_, g, st))) = heap.pop() {
            if g > best[st] {
                continue;
            }
            if budget == 0 {
                break;
            }
            budget -= 1;
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
        let mut cost: u32 = 0;
        for (k, w) in path.windows(2).enumerate() {
            let Some(d) = Dir::from_vec(w[1].x - w[0].x, w[1].y - w[0].y) else {
                continue;
            };
            if k > 0 {
                cost = cost.saturating_add(BEND);
            }
            for p in self.points_on(w[0], w[1]) {
                if p == w[0] {
                    continue;
                }
                if let Some(i) = self.idx(p) {
                    let c = &self.cells[i];
                    cost = cost.saturating_add(STEP + c.cost);
                    let across = if d.horizontal() { c.vwire } else { c.hwire };
                    if across != 0 && !tags.contains(&across) {
                        cost = cost.saturating_add(CROSS);
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
        let g = Grid::new(area()).unwrap();
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
        let mut g = Grid::new(area()).unwrap();
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
        let mut g = Grid::new(area()).unwrap();
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

    #[test]
    fn oversized_or_far_areas_get_no_grid() {
        let far = Rect::from_points(Point::new(0, 0), Point::new(2_000_000_000, 2_000_000_000));
        assert!(Grid::new(far).is_none());
        let wide = Rect::from_points(Point::new(0, 0), Point::new(16 * 3_000, 16 * 3_000));
        assert!(Grid::new(wide).is_none());
        let edge = Rect::from_points(Point::new(i32::MAX - 64, 0), Point::new(i32::MAX, 64));
        assert!(Grid::new(edge).is_none());
        assert!(Grid::new(area()).is_some());
    }

    #[test]
    fn off_grid_and_far_wires_are_cheap_to_add() {
        let mut g = Grid::new(area()).unwrap();
        // Off the grid's lines: must not loop looking for the far end.
        g.add_wire(Point::new(8, 0), Point::new(100, 0), 1);
        g.add_wire(Point::new(3, 5), Point::new(3, 77), 1);
        // Reaching far outside the grid: only the part inside counts.
        g.add_wire(Point::new(0, 32), Point::new(2_000_000_000, 32), 2);
        g.add_wire(Point::new(i32::MIN, 64), Point::new(i32::MAX, 64), 2);
        g.add_keepout(Rect::from_points(
            Point::new(i32::MIN, i32::MIN),
            Point::new(i32::MAX, -100),
        ));
        assert!(
            g.points_on(Point::new(0, 32), Point::new(2_000_000_000, 32))
                .len()
                < 64
        );
        assert!(!g.segment_free(Point::new(0, 32), Point::new(64, 32), 0));
    }
}
