//! Joining two pins with wires, safely and tidily.
//!
//! Candidate routes come from two places: a fixed menu of tidy shapes (a
//! straight wire, the two L-shapes, routes that step out from the pins first,
//! routes through the channel between parts) and a maze search on the 16-unit
//! grid. Every candidate is checked against LTspice's connection rules on that
//! grid: it may not run through a part body or within a clearance margin of
//! one (except straight out along its own pin's lead), nor touch another net.
//! The legal candidates are scored on length, corners, crossings and how close
//! they pass to parts and text, and the best few are tried in order of score.
//! Each is then checked by recomputing connectivity: it must join the two nets
//! and nothing else, so the router can never create a short even if the grid
//! model were wrong. When no wire route passes, a pair of net labels joins
//! the pins instead.

use super::pins::{PinLoc, bodies};
use crate::geometry::{GRID, Point, Rect, SegmentIndex};
use crate::layout::maze::{CLEARANCE, COORD_LIMIT, Grid, Start, grown};
use crate::layout::{Dir, core_body, flag_box, pin_facing, place_rect, segment_hits, symbol_texts};
use crate::netlist::{Connectivity, connect};
use crate::schematic::{Flag, Item, Schematic, Wire};
use crate::symbol::SymbolLibrary;
use std::collections::HashMap;

/// How a connection was made, for the edit report.
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    AlreadyConnected,
    Wires(Vec<Wire>),
    Labels {
        label: String,
        items: Vec<Item>,
    },
    /// Neither wires nor labels could join the pins without touching another
    /// net; the schematic is unchanged.
    Failed,
}

type PinKey = (String, String);

/// The instance part of the key under which a net label is recorded in a
/// partition. No instance name can contain a control character.
const LABEL_KEY: &str = "\u{1}label";

/// The partition key of a net label.
pub(crate) fn label_key(label: &str) -> PinKey {
    (LABEL_KEY.to_string(), label.to_ascii_uppercase())
}

/// Net index of every pin, keyed by upper-cased instance and pin name, and
/// of every net label (see [`label_key`]). Labels count as members so that
/// a route which runs into a label with no pins on it (a stray or leftover
/// label) is seen to join that net too.
pub(crate) fn pin_partition(conn: &Connectivity) -> HashMap<PinKey, usize> {
    let mut map = HashMap::new();
    for (inst, pins) in &conn.pin_nets {
        for (pin, net) in pins {
            map.insert((inst.clone(), pin.to_ascii_uppercase()), *net);
        }
    }
    for (i, net) in conn.nets.iter().enumerate() {
        for l in &net.labels {
            map.insert(label_key(l), i);
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

fn path_wires(points: &[Point]) -> Vec<Wire> {
    points
        .windows(2)
        .filter(|w| w[0] != w[1])
        .map(|w| Wire::new(w[0], w[1]))
        .collect()
}

/// The way a pin really faces: along its lead, from the symbol drawing.
/// [`PinLoc::out`] guesses from the body centre, which is wrong for pins set
/// off-centre on their side (a MOSFET gate).
fn facing(sch: &Schematic, lib: &SymbolLibrary, p: &PinLoc) -> Dir {
    let guess = Dir::from_vec(p.out.0, p.out.1).unwrap_or(Dir::Up);
    let Some(sym) = sch.symbol(&p.inst) else {
        return guess;
    };
    let Ok((def, _)) = lib.resolve(&sym.name) else {
        return guess;
    };
    def.pin(&p.pin)
        .map(|pd| pin_facing(&def, pd, sym.orient))
        .unwrap_or(guess)
}

/// Wire routes between two pins, tidiest first, each leaving `a` along `fa`
/// and arriving at `b` along `fb`.
fn candidates(a: Point, fa: Dir, b: Point, fb: Dir) -> Vec<Vec<Point>> {
    let (p, q) = (a, b);
    let (ax, ay) = fa.vec();
    let (bx, by) = fb.vec();
    let mut out = Vec::new();
    if p.x == q.x || p.y == q.y {
        out.push(vec![p, q]);
    }
    out.push(vec![p, Point::new(q.x, p.y), q]);
    out.push(vec![p, Point::new(p.x, q.y), q]);
    let mid_x = crate::geometry::snap((p.x + q.x) / 2);
    let mid_y = crate::geometry::snap((p.y + q.y) / 2);
    for k in [1, 2, 3, 4, 6] {
        let pa = p.offset(ax * GRID * k, ay * GRID * k);
        let qb = q.offset(bx * GRID * k, by * GRID * k);
        out.push(vec![p, pa, Point::new(q.x, pa.y), q]);
        out.push(vec![p, pa, Point::new(pa.x, q.y), q]);
        out.push(vec![p, pa, Point::new(qb.x, pa.y), qb, q]);
        out.push(vec![p, pa, Point::new(pa.x, qb.y), qb, q]);
        // Through the channel between the parts: step out of each pin, run
        // along the gap, step back in. This is what joins the bottom of one
        // vertical part to the top of its neighbour.
        out.push(vec![
            p,
            pa,
            Point::new(mid_x, pa.y),
            Point::new(mid_x, qb.y),
            qb,
            q,
        ]);
        out.push(vec![
            p,
            pa,
            Point::new(pa.x, mid_y),
            Point::new(qb.x, mid_y),
            qb,
            q,
        ]);
    }
    // Wider detours for crowded sheets.
    for k in [2, 3, 6] {
        let pa = p.offset(ax * GRID * k, ay * GRID * k);
        let qb = q.offset(bx * GRID * k, by * GRID * k);
        for off in [
            GRID * 2,
            -GRID * 2,
            GRID * 4,
            -GRID * 4,
            GRID * 6,
            -GRID * 6,
        ] {
            out.push(vec![
                p,
                pa,
                Point::new(mid_x + off, pa.y),
                Point::new(mid_x + off, qb.y),
                qb,
                q,
            ]);
            out.push(vec![
                p,
                pa,
                Point::new(pa.x, mid_y + off),
                Point::new(qb.x, mid_y + off),
                qb,
                q,
            ]);
        }
    }
    // Drop zero-length steps, reversals and duplicate routes.
    let mut seen = Vec::new();
    out.into_iter()
        .map(|path| crate::layout::maze::corners(&path))
        .filter(|path| {
            if seen.contains(path) {
                false
            } else {
                seen.push(path.clone());
                true
            }
        })
        .collect()
}

/// Which of the points that carry connections belong to the two nets being
/// joined, following LTspice's rules the way [`connect`] does: wire ends join,
/// anything on a wire joins it, flags with one label join.
struct Membership {
    root_of: HashMap<Point, usize>,
    parent: Vec<usize>,
}

impl Membership {
    fn new(sch: &Schematic, pins: &[Point]) -> Self {
        let mut m = Membership {
            root_of: HashMap::new(),
            parent: Vec::new(),
        };
        let wires: Vec<Wire> = sch.wires().copied().collect();
        for w in &wires {
            let (a, b) = (m.id(w.a), m.id(w.b));
            m.union(a, b);
        }
        for p in pins {
            m.id(*p);
        }
        let flags: Vec<(Point, String)> = sch
            .flags()
            .map(|f| (f.at, f.label.to_ascii_uppercase()))
            .collect();
        for (p, _) in &flags {
            m.id(*p);
        }
        let segs: Vec<(Point, Point)> = wires.iter().map(|w| (w.a, w.b)).collect();
        let (index, _) = SegmentIndex::new(&segs);
        let points: Vec<Point> = m.root_of.keys().copied().collect();
        for p in points {
            let cover = index.cover(p);
            let id = m.id(p);
            for wi in cover
                .horizontal
                .map(|s| s.wire)
                .into_iter()
                .chain(cover.vertical.map(|s| s.wire))
                .chain(cover.diagonal)
            {
                let other = m.id(wires[wi].a);
                m.union(id, other);
            }
        }
        let mut by_label: HashMap<String, usize> = HashMap::new();
        for (p, label) in &flags {
            let id = m.id(*p);
            match by_label.get(label) {
                Some(&o) => m.union(id, o),
                None => {
                    by_label.insert(label.clone(), id);
                }
            }
        }
        m
    }

    fn id(&mut self, p: Point) -> usize {
        if let Some(&i) = self.root_of.get(&p) {
            return i;
        }
        let i = self.parent.len();
        self.parent.push(i);
        self.root_of.insert(p, i);
        i
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

    fn root(&mut self, p: Point) -> Option<usize> {
        let i = *self.root_of.get(&p)?;
        Some(self.find(i))
    }
}

/// Grid ids for the router: the two nets being joined, and everything else.
const NET_A: u32 = 0;
const NET_B: u32 = 1;
const FOREIGN: u32 = 2;

/// The sheet as the router sees it, for one connection.
struct Scene {
    grid: Grid,
    /// Points of net B a route may end on.
    goals_b: Vec<Point>,
}

/// How far around the two pins the router looks for a way through. Routes
/// that need a wider detour than this are not tidy anyway; labels do better.
const MARGIN: i32 = 24 * GRID;

/// Whether a point is far enough inside `i32` for routing arithmetic.
fn routable(p: Point) -> bool {
    (p.x as i64).abs() <= COORD_LIMIT && (p.y as i64).abs() <= COORD_LIMIT
}

/// Drawn bodies (core drawing, without leads) of every part that has one,
/// with the part's pins and their facings.
fn part_bodies(sch: &Schematic, lib: &SymbolLibrary) -> Vec<(Rect, Vec<(Point, Dir)>)> {
    let drawn: Vec<String> = bodies(sch, lib)
        .into_iter()
        .map(|(n, _)| n.to_ascii_uppercase())
        .collect();
    let mut out = Vec::new();
    for s in sch.symbols() {
        let Ok((def, _)) = lib.resolve(&s.name) else {
            continue;
        };
        let named = s
            .inst_name()
            .is_some_and(|n| drawn.contains(&n.to_ascii_uppercase()));
        let Some(core) = core_body(&def).filter(|_| named) else {
            continue;
        };
        let pins = def
            .pins
            .iter()
            .map(|p| {
                (
                    def.pin_position(p, s.at, s.orient),
                    pin_facing(&def, p, s.orient),
                )
            })
            .collect();
        out.push((place_rect(core, s.at, s.orient), pins));
    }
    out
}

impl Scene {
    /// The routing grid for joining `a` and `b`: the region around the two
    /// pins only, so a part far away on the sheet cannot inflate it. `None`
    /// when even that region is too large for a grid.
    fn new(sch: &Schematic, lib: &SymbolLibrary, a: &PinLoc, b: &PinLoc) -> Option<Scene> {
        if !routable(a.at) || !routable(b.at) {
            return None;
        }
        let region = grown(Rect::from_points(a.at, b.at), MARGIN);
        let mut grid = Grid::new(region)?;
        let near = |r: &Rect| r.intersects(&region);
        let mut pins: Vec<(Point, Dir)> = Vec::new();
        for s in sch.symbols() {
            let Ok((def, _)) = lib.resolve(&s.name) else {
                continue;
            };
            let close = def.placed_bounds(s.at, s.orient).is_some_and(|r| near(&r));
            if !close {
                continue;
            }
            for p in &def.pins {
                pins.push((
                    def.pin_position(p, s.at, s.orient),
                    pin_facing(&def, p, s.orient),
                ));
            }
            for t in symbol_texts(s, &def) {
                if near(&t) {
                    grid.add_cost(grown(t, 2), 30);
                }
            }
        }
        for (body, _) in part_bodies(sch, lib) {
            if near(&grown(body, CLEARANCE + 16)) {
                grid.add_keepout(grown(body, CLEARANCE));
                grid.add_cost(grown(body, CLEARANCE + 16), 3);
            }
        }
        let pin_points: Vec<Point> = pins.iter().map(|(p, _)| *p).collect();
        let mut m = Membership::new(sch, &pin_points);
        let (ra, rb) = (m.root(a.at), m.root(b.at));
        let mut id_of = |p: Point| -> u32 {
            let r = m.root(p);
            if r.is_some() && r == ra {
                NET_A
            } else if r.is_some() && r == rb {
                NET_B
            } else {
                FOREIGN
            }
        };
        let mut goals_b = Vec::new();
        let wires: Vec<Wire> = sch.wires().copied().collect();
        for w in &wires {
            let id = id_of(w.a);
            grid.add_wire(w.a, w.b, id);
            if id == NET_B {
                goals_b.extend(grid.points_on(w.a, w.b));
            }
        }
        let owners: Vec<(Point, Point)> = Vec::new();
        for f in sch.flags() {
            if !region.contains(f.at) {
                continue;
            }
            let id = id_of(f.at);
            grid.add_flag(f.at, id);
            if f.is_ground() {
                grid.add_keepout(Rect::from_points(f.at.offset(-24, 2), f.at.offset(24, 28)));
            } else {
                grid.add_cost(grown(flag_box(f, &wires, &owners), 2), 12);
            }
            if id == NET_B {
                goals_b.push(f.at);
            }
        }
        // Pins last, so their leads open corridors through the keep-outs.
        for (p, f) in &pins {
            let id = id_of(*p);
            grid.add_pin(*p, *f, id);
            if id == NET_B {
                goals_b.push(*p);
            }
        }
        Some(Scene { grid, goals_b })
    }

    fn legal(&self, path: &[Point]) -> bool {
        path.len() >= 2
            && path
                .windows(2)
                .all(|w| w[0] == w[1] || self.grid.segment_free_for(w[0], w[1], &[NET_A, NET_B]))
            && path
                .windows(2)
                .all(|w| w[0].x == w[1].x || w[0].y == w[1].y)
    }

    fn cost(&self, path: &[Point]) -> u32 {
        self.grid.path_cost(path, &[NET_A, NET_B])
    }

    fn maze(&self, a: Point, fa: Dir) -> Option<Vec<Point>> {
        self.grid.route(
            &[Start {
                at: a,
                dirs: vec![fa],
            }],
            &self.goals_b,
            &[NET_A, NET_B],
            40_000,
        )
    }
}

/// Whether a path stays out of every part body and its clearance, except
/// for a segment leaving one of that part's pins straight out along its lead.
fn clear_of_bodies(path: &[Point], parts: &[(Rect, Vec<(Point, Dir)>)]) -> bool {
    path.windows(2).all(|w| {
        parts.iter().all(|(body, pins)| {
            let zone = grown(*body, CLEARANCE);
            !segment_hits(w[0], w[1], &zone)
                || pins.iter().any(|&(pin, f)| {
                    [(w[0], w[1]), (w[1], w[0])].into_iter().any(|(from, to)| {
                        from == pin && Dir::from_vec(to.x - from.x, to.y - from.y) == Some(f)
                    })
                })
        })
    })
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
        let (fa, fb) = (facing(sch, self.lib, a), facing(sch, self.lib, b));
        let options: Vec<(u32, Vec<Point>)> = match Scene::new(sch, self.lib, a, b) {
            Some(scene) => {
                let mut options: Vec<(u32, Vec<Point>)> = candidates(a.at, fa, b.at, fb)
                    .into_iter()
                    .filter(|p| scene.legal(p))
                    .map(|p| (scene.cost(&p), p))
                    .collect();
                if let Some(p) = scene.maze(a.at, fa)
                    && scene.legal(&p)
                {
                    options.push((scene.cost(&p), p));
                }
                options.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.len().cmp(&y.1.len())));
                options
            }
            // No grid for this region: the tidy shapes in their own order,
            // kept off part bodies; connectivity still checks each one.
            None if routable(a.at) && routable(b.at) => {
                let parts = part_bodies(sch, self.lib);
                candidates(a.at, fa, b.at, fb)
                    .into_iter()
                    .filter(|p| clear_of_bodies(p, &parts))
                    .map(|p| (0, p))
                    .collect()
            }
            None => Vec::new(),
        };
        for (_, path) in options.iter().take(16) {
            let wires = path_wires(path);
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
        let dirs = [facing(sch, self.lib, a), facing(sch, self.lib, b)];
        let stubs: &[i32] = if routable(a.at) && routable(b.at) {
            &[2, 1, 0]
        } else {
            &[0]
        };
        for &stub in stubs {
            let mut trial = sch.clone();
            let mut items = Vec::new();
            for (p, d) in [a, b].into_iter().zip(dirs) {
                let end = d.step(p.at, GRID * stub);
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
        // Rare: a label placed exactly on each pin joins only those pins'
        // nets. If even that fails, the schematic is left unchanged.
        Route::Failed
    }
}

/// A label not used anywhere in the schematic yet.
fn unique_label(conn: &Connectivity, hint: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schematic::parse;

    fn lib() -> SymbolLibrary {
        SymbolLibrary::builtin_only()
    }

    #[test]
    fn wires_keep_clear_of_bodies_they_pass() {
        // R2 sits right of R1 with its body between R1.A and the target
        // pin: the route must go around R2 with clearance, not along it.
        let src = "Version 4\nSHEET 1 880 680\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMATTR Value 1k\nSYMBOL res 64 -16 R0\nSYMATTR InstName R2\nSYMATTR Value 1k\nSYMBOL res 192 0 R0\nSYMATTR InstName R3\nSYMATTR Value 1k\n";
        let (mut sch, _) = parse(src);
        let a = crate::edit::locate(&sch, &lib(), "R1.B").unwrap();
        let b = crate::edit::locate(&sch, &lib(), "R3.B").unwrap();
        let route = Router { lib: &lib() }.connect(&mut sch, &a, &b, "x");
        assert!(matches!(route, Route::Wires(_)), "{route:?}");
        let q = crate::layout::quality(&sch, &lib());
        assert_eq!(q.wires_through_bodies, 0, "{q:?}");
        // Nothing within the clearance of R2's drawn body.
        let (r2, _) = lib().resolve("res").unwrap();
        let body = place_rect(
            core_body(&r2).unwrap(),
            Point::new(64, -16),
            crate::Orient::R0,
        )
        .inflate(CLEARANCE - 1);
        for w in sch.wires() {
            assert!(
                !crate::layout::segment_hits(w.a, w.b, &body),
                "{w:?} grazes R2"
            );
        }
    }

    #[test]
    fn the_grid_covers_only_the_region_of_the_route() {
        let src = "Version 4\nSHEET 1 880 680\nWIRE -32 400 2000000000 400\nSYMBOL res 2000000000 2000000000 R0\nSYMATTR InstName RF\nSYMATTR Value 1k\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMATTR Value 1k\nSYMBOL res 128 0 R0\nSYMATTR InstName R2\nSYMATTR Value 1k\n";
        let (sch, _) = parse(src);
        let a = crate::edit::locate(&sch, &lib(), "R1.B").unwrap();
        let b = crate::edit::locate(&sch, &lib(), "R2.B").unwrap();
        let scene = Scene::new(&sch, &lib(), &a, &b).expect("a small region");
        assert!(
            scene.grid.cell_count() < 20_000,
            "{}",
            scene.grid.cell_count()
        );
    }

    #[test]
    fn prefers_fewer_corners_among_legal_routes() {
        // Two pins on one row with nothing between them: one straight wire.
        let src = "Version 4\nSHEET 1 880 680\nSYMBOL res 0 0 R90\nSYMATTR InstName R1\nSYMATTR Value 1k\nSYMBOL res 256 0 R90\nSYMATTR InstName R2\nSYMATTR Value 1k\n";
        let (mut sch, _) = parse(src);
        let a = crate::edit::locate(&sch, &lib(), "R1.A").unwrap();
        let b = crate::edit::locate(&sch, &lib(), "R2.B").unwrap();
        let route = Router { lib: &lib() }.connect(&mut sch, &a, &b, "x");
        match route {
            Route::Wires(w) => assert_eq!(w.len(), 1, "{w:?}"),
            other => panic!("{other:?}"),
        }
    }
}
