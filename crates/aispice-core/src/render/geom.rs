//! Placing drawing primitives: the orientation transform for every graphic
//! kind, LTspice arcs as SVG path arcs, path data and bounding boxes.
//!
//! LTspice inherits the Windows GDI conventions: a CIRCLE is the ellipse
//! inscribed in its box, and an ARC is part of that ellipse running
//! counter-clockwise on screen from the ray through its start point to the ray
//! through its end point. The start and end points only give directions; they
//! need not lie on the ellipse. A mirror turns counter-clockwise into
//! clockwise, so a mirrored arc swaps its start and end to keep the same
//! stretch of ellipse.

use super::svg::num;
use crate::geometry::{Orient, Point, Rect};
use crate::symbol::Graphic;
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::fmt::Write;

const EPS: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn close_to(self, o: Pt, tol: f64) -> bool {
        (self.x - o.x).abs() <= tol && (self.y - o.y).abs() <= tol
    }
}

impl From<Point> for Pt {
    fn from(p: Point) -> Self {
        Pt::new(p.x as f64, p.y as f64)
    }
}

/// A floating-point bounding box that starts empty.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BBox {
    pub min: Pt,
    pub max: Pt,
}

impl Default for BBox {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl BBox {
    pub const EMPTY: BBox = BBox {
        min: Pt {
            x: f64::INFINITY,
            y: f64::INFINITY,
        },
        max: Pt {
            x: f64::NEG_INFINITY,
            y: f64::NEG_INFINITY,
        },
    };

    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y
    }

    pub fn include(&mut self, p: Pt) {
        self.min.x = self.min.x.min(p.x);
        self.min.y = self.min.y.min(p.y);
        self.max.x = self.max.x.max(p.x);
        self.max.y = self.max.y.max(p.y);
    }

    pub fn union(&mut self, o: &BBox) {
        if !o.is_empty() {
            self.include(o.min);
            self.include(o.max);
        }
    }

    pub fn inflate(self, by: f64) -> BBox {
        if self.is_empty() {
            return self;
        }
        BBox {
            min: Pt::new(self.min.x - by, self.min.y - by),
            max: Pt::new(self.max.x + by, self.max.y + by),
        }
    }

    pub fn center(&self) -> Pt {
        Pt::new(
            (self.min.x + self.max.x) / 2.0,
            (self.min.y + self.max.y) / 2.0,
        )
    }

    /// The smallest integer rectangle that contains the box.
    pub fn to_rect(self) -> Rect {
        if self.is_empty() {
            return Rect::from_points(Point::new(0, 0), Point::new(0, 0));
        }
        Rect::from_points(
            Point::new(self.min.x.floor() as i32, self.min.y.floor() as i32),
            Point::new(self.max.x.ceil() as i32, self.max.y.ceil() as i32),
        )
    }
}

/// A primitive in sheet coordinates, ready to write as SVG.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Prim {
    Line {
        a: Pt,
        b: Pt,
    },
    Rect {
        min: Pt,
        max: Pt,
    },
    Ellipse {
        c: Pt,
        rx: f64,
        ry: f64,
    },
    /// Counter-clockwise on screen from `from` to `to`, covering `span`
    /// radians. A span of a full turn is a closed ellipse.
    Arc {
        c: Pt,
        rx: f64,
        ry: f64,
        from: Pt,
        to: Pt,
        span: f64,
    },
}

/// Place one symbol graphic at `origin` with `orient`. Text is handled by the
/// text module and returns `None` here, as do degenerate arcs.
pub(crate) fn place(g: &Graphic, origin: Point, orient: Orient) -> Option<Prim> {
    let p = |q: Point| Pt::from(origin + orient.apply(q));
    match g {
        Graphic::Line { a, b } => Some(Prim::Line { a: p(*a), b: p(*b) }),
        Graphic::Rect { a, b } => {
            let (a, b) = (p(*a), p(*b));
            Some(Prim::Rect {
                min: Pt::new(a.x.min(b.x), a.y.min(b.y)),
                max: Pt::new(a.x.max(b.x), a.y.max(b.y)),
            })
        }
        Graphic::Circle { a, b } => {
            let (a, b) = (p(*a), p(*b));
            Some(Prim::Ellipse {
                c: Pt::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0),
                rx: (b.x - a.x).abs() / 2.0,
                ry: (b.y - a.y).abs() / 2.0,
            })
        }
        Graphic::Arc { a, b, start, end } => {
            let (mut s, mut e) = (p(*start), p(*end));
            if orient.is_mirrored() {
                std::mem::swap(&mut s, &mut e);
            }
            arc(p(*a), p(*b), s, e)
        }
        Graphic::Text { .. } => None,
    }
}

/// The arc of the ellipse inscribed in the box `a`..`b`, counter-clockwise on
/// screen from the ray through `start` to the ray through `end`. Equal rays
/// give the whole ellipse, as GDI does.
pub(crate) fn arc(a: Pt, b: Pt, start: Pt, end: Pt) -> Option<Prim> {
    let c = Pt::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
    let rx = (b.x - a.x).abs() / 2.0;
    let ry = (b.y - a.y).abs() / 2.0;
    if rx < EPS || ry < EPS {
        return None;
    }
    let from = on_ellipse(c, rx, ry, start);
    let to = on_ellipse(c, rx, ry, end);
    let d = (screen_angle(c, to) - screen_angle(c, from)).rem_euclid(TAU);
    let span = if d < 1e-9 || TAU - d < 1e-9 { TAU } else { d };
    Some(Prim::Arc {
        c,
        rx,
        ry,
        from,
        to,
        span,
    })
}

/// Angle of `p` around `c`, counter-clockwise on screen (Y points down).
fn screen_angle(c: Pt, p: Pt) -> f64 {
    (c.y - p.y).atan2(p.x - c.x)
}

/// Where the ray from the centre through `dir` meets the ellipse.
fn on_ellipse(c: Pt, rx: f64, ry: f64, dir: Pt) -> Pt {
    let (dx, dy) = (dir.x - c.x, dir.y - c.y);
    if dx.abs() < EPS && dy.abs() < EPS {
        return Pt::new(c.x + rx, c.y);
    }
    let t = 1.0 / ((dx / rx).powi(2) + (dy / ry).powi(2)).sqrt();
    Pt::new(c.x + t * dx, c.y + t * dy)
}

fn at_angle(c: Pt, rx: f64, ry: f64, angle: f64) -> Pt {
    on_ellipse(c, rx, ry, Pt::new(c.x + angle.cos(), c.y - angle.sin()))
}

impl Prim {
    /// The point halfway along an arc, by angle. Rotations and mirrors keep
    /// angles, so this point must transform like any other; tests use it to
    /// check that a mirrored arc stays on the correct side.
    #[cfg(test)]
    pub fn arc_midpoint(&self) -> Option<Pt> {
        match self {
            Prim::Arc {
                c,
                rx,
                ry,
                from,
                span,
                ..
            } => Some(at_angle(*c, *rx, *ry, screen_angle(*c, *from) + span / 2.0)),
            _ => None,
        }
    }

    pub fn bbox(&self) -> BBox {
        let mut b = BBox::EMPTY;
        match self {
            Prim::Line { a, b: e } => {
                b.include(*a);
                b.include(*e);
            }
            Prim::Rect { min, max } => {
                b.include(*min);
                b.include(*max);
            }
            Prim::Ellipse { c, rx, ry } => {
                b.include(Pt::new(c.x - rx, c.y - ry));
                b.include(Pt::new(c.x + rx, c.y + ry));
            }
            Prim::Arc {
                c,
                rx,
                ry,
                from,
                to,
                span,
            } => {
                b.include(*from);
                b.include(*to);
                let start = screen_angle(*c, *from);
                for k in 0..4 {
                    let angle = k as f64 * FRAC_PI_2;
                    if (angle - start).rem_euclid(TAU) <= *span + 1e-9 {
                        b.include(at_angle(*c, *rx, *ry, angle));
                    }
                }
            }
        }
        b
    }
}

/// Small closed triangles made of three LINE graphics, in the symbol's own
/// coordinates. Symbol files can only stroke, so arrowheads (on emitters,
/// current sources, MOSFET bodies) are drawn as triangle outlines; filling
/// them makes them read as arrows at any zoom. Size limits keep diode bodies
/// and logic-gate outlines hollow.
pub(crate) fn arrowheads(graphics: &[Graphic]) -> Vec<[Point; 3]> {
    use std::collections::{BTreeSet, HashSet};
    let key = |a: Point, b: Point| if a <= b { (a, b) } else { (b, a) };
    let segments: HashSet<(Point, Point)> = graphics
        .iter()
        .filter_map(|g| match g {
            Graphic::Line { a, b } if a != b => Some(key(*a, *b)),
            _ => None,
        })
        .collect();
    let mut found = BTreeSet::new();
    for &(a, b) in &segments {
        for &(p, q) in &segments {
            let c = if p == b && q != a {
                q
            } else if q == b && p != a {
                p
            } else {
                continue;
            };
            if !segments.contains(&key(c, a)) {
                continue;
            }
            let side = |u: Point, v: Point| {
                let (dx, dy) = ((u.x - v.x) as f64, (u.y - v.y) as f64);
                (dx * dx + dy * dy).sqrt()
            };
            let longest = side(a, b).max(side(b, c)).max(side(c, a));
            let area = (((b.x - a.x) as i64 * (c.y - a.y) as i64
                - (b.y - a.y) as i64 * (c.x - a.x) as i64)
                .abs() as f64)
                / 2.0;
            if area > 0.0 && area <= 200.0 && longest <= 32.0 {
                let mut tri = [a, b, c];
                tri.sort();
                found.insert(tri);
            }
        }
    }
    found.into_iter().collect()
}

/// Builds one SVG path from many primitives, joining lines that continue
/// from the previous end point so a zigzag is one run of `L` commands.
#[derive(Default)]
pub(crate) struct PathBuilder {
    d: String,
    last: Option<Pt>,
}

impl PathBuilder {
    pub fn is_empty(&self) -> bool {
        self.d.is_empty()
    }

    fn move_to(&mut self, p: Pt) {
        if !self.last.is_some_and(|l| l.close_to(p, 1e-6)) {
            let _ = write!(self.d, "M{} {}", num(p.x), num(p.y));
        }
    }

    pub fn push(&mut self, prim: &Prim) {
        match prim {
            Prim::Line { a, b } => {
                if self.last.is_some_and(|l| l.close_to(*b, 1e-6))
                    && !self.last.is_some_and(|l| l.close_to(*a, 1e-6))
                {
                    let _ = write!(self.d, "L{} {}", num(a.x), num(a.y));
                    self.last = Some(*a);
                } else {
                    self.move_to(*a);
                    let _ = write!(self.d, "L{} {}", num(b.x), num(b.y));
                    self.last = Some(*b);
                }
            }
            Prim::Rect { min, max } => {
                let _ = write!(
                    self.d,
                    "M{} {}H{}V{}H{}Z",
                    num(min.x),
                    num(min.y),
                    num(max.x),
                    num(max.y),
                    num(min.x)
                );
                self.last = None;
            }
            Prim::Ellipse { c, rx, ry } => {
                self.full_ellipse(Pt::new(c.x - rx, c.y), *c, *rx, *ry);
                self.last = None;
            }
            Prim::Arc {
                c,
                rx,
                ry,
                from,
                to,
                span,
            } => {
                if *span >= TAU - 1e-9 {
                    self.full_ellipse(*from, *c, *rx, *ry);
                    self.last = None;
                } else {
                    self.move_to(*from);
                    let large = if *span > PI { 1 } else { 0 };
                    // Sweep flag 0 is counter-clockwise on screen in SVG.
                    let _ = write!(
                        self.d,
                        "A{} {} 0 {} 0 {} {}",
                        num(*rx),
                        num(*ry),
                        large,
                        num(to.x),
                        num(to.y)
                    );
                    self.last = Some(*to);
                }
            }
        }
    }

    fn full_ellipse(&mut self, from: Pt, c: Pt, rx: f64, ry: f64) {
        let opposite = Pt::new(2.0 * c.x - from.x, 2.0 * c.y - from.y);
        let _ = write!(
            self.d,
            "M{} {}A{} {} 0 1 0 {} {}A{} {} 0 1 0 {} {}Z",
            num(from.x),
            num(from.y),
            num(rx),
            num(ry),
            num(opposite.x),
            num(opposite.y),
            num(rx),
            num(ry),
            num(from.x),
            num(from.y)
        );
    }

    pub fn finish(self) -> String {
        self.d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64) -> Pt {
        Pt::new(x, y)
    }

    fn mapped(o: Orient, origin: Point, p: Point) -> Pt {
        Pt::from(origin + o.apply(p))
    }

    #[test]
    fn lines_follow_the_pin_transform() {
        let g = Graphic::Line {
            a: Point::new(16, 16),
            b: Point::new(16, 96),
        };
        let origin = Point::new(100, 200);
        for o in Orient::ALL {
            let Some(Prim::Line { a, b }) = place(&g, origin, o) else {
                panic!("line expected");
            };
            assert_eq!(a, mapped(o, origin, Point::new(16, 16)), "{o}");
            assert_eq!(b, mapped(o, origin, Point::new(16, 96)), "{o}");
        }
    }

    #[test]
    fn rectangles_stay_axis_aligned_with_ordered_corners() {
        let g = Graphic::Rect {
            a: Point::new(-8, 4),
            b: Point::new(24, 40),
        };
        for o in Orient::ALL {
            let Some(Prim::Rect { min, max }) = place(&g, Point::new(0, 0), o) else {
                panic!("rect expected");
            };
            assert!(min.x < max.x && min.y < max.y, "{o}");
            let c1 = mapped(o, Point::new(0, 0), Point::new(-8, 4));
            let c2 = mapped(o, Point::new(0, 0), Point::new(24, 40));
            assert_eq!(min, pt(c1.x.min(c2.x), c1.y.min(c2.y)), "{o}");
            assert_eq!(max, pt(c1.x.max(c2.x), c1.y.max(c2.y)), "{o}");
        }
    }

    #[test]
    fn ellipses_swap_radii_on_quarter_turns() {
        let g = Graphic::Circle {
            a: Point::new(-32, 24),
            b: Point::new(32, 56),
        };
        for o in Orient::ALL {
            let Some(Prim::Ellipse { c, rx, ry }) = place(&g, Point::new(10, 10), o) else {
                panic!("ellipse expected");
            };
            assert_eq!(c, mapped(o, Point::new(10, 10), Point::new(0, 40)), "{o}");
            if o.quarter_turns() % 2 == 0 {
                assert_eq!((rx, ry), (32.0, 16.0), "{o}");
            } else {
                assert_eq!((rx, ry), (16.0, 32.0), "{o}");
            }
        }
    }

    #[test]
    fn arcs_run_counter_clockwise_on_screen() {
        // Top half of a circle: from the right point, counter-clockwise on
        // screen, passes over the top (smaller Y) to the left point.
        let p = arc(
            pt(-10.0, -10.0),
            pt(10.0, 10.0),
            pt(10.0, 0.0),
            pt(-10.0, 0.0),
        )
        .unwrap();
        let mid = p.arc_midpoint().unwrap();
        assert!(mid.close_to(pt(0.0, -10.0), 1e-9), "{mid:?}");
        let mut path = PathBuilder::default();
        path.push(&p);
        assert_eq!(path.finish(), "M10 0A10 10 0 0 0 -10 0");
    }

    #[test]
    fn arc_end_points_are_projected_onto_the_ellipse() {
        // Direction points far outside the ellipse still give points on it.
        let p = arc(
            pt(0.0, 0.0),
            pt(40.0, 20.0),
            pt(100.0, 10.0),
            pt(20.0, -50.0),
        )
        .unwrap();
        let Prim::Arc { from, to, span, .. } = p else {
            unreachable!()
        };
        assert!(from.close_to(pt(40.0, 10.0), 1e-9));
        assert!(to.close_to(pt(20.0, 0.0), 1e-9));
        assert!((span - FRAC_PI_2).abs() < 1e-9);
    }

    #[test]
    fn large_arcs_set_the_large_arc_flag() {
        // From the left point counter-clockwise to the top: three quarters.
        let p = arc(
            pt(-10.0, -10.0),
            pt(10.0, 10.0),
            pt(-10.0, 0.0),
            pt(0.0, -10.0),
        )
        .unwrap();
        let mut path = PathBuilder::default();
        path.push(&p);
        assert_eq!(path.finish(), "M-10 0A10 10 0 1 0 0 -10");
        let b = p.bbox();
        assert_eq!((b.min, b.max), (pt(-10.0, -10.0), pt(10.0, 10.0)));
    }

    #[test]
    fn equal_rays_draw_the_whole_ellipse() {
        let p = arc(pt(0.0, 0.0), pt(20.0, 20.0), pt(30.0, 10.0), pt(25.0, 10.0)).unwrap();
        let Prim::Arc { span, .. } = p else {
            unreachable!()
        };
        assert_eq!(span, TAU);
    }

    #[test]
    fn mirrored_arcs_keep_their_side() {
        // An off-centre arc in a non-square box, like a coil turn. Its
        // midpoint must land where the orientation sends the R0 midpoint, in
        // every orientation. Without swapping start and end under a mirror
        // the arc would cover the other three quarters of the ellipse.
        let g = Graphic::Arc {
            a: Point::new(6, 24),
            b: Point::new(26, 40),
            start: Point::new(16, 40),
            end: Point::new(20, 24),
        };
        let origin = Point::new(64, 32);
        let r0 = place(&g, Point::new(0, 0), Orient::R0).unwrap();
        let r0_mid = r0.arc_midpoint().unwrap();
        let Prim::Arc { span: r0_span, .. } = r0 else {
            unreachable!()
        };
        for o in Orient::ALL {
            let placed = place(&g, origin, o).unwrap();
            let mid = placed.arc_midpoint().unwrap();
            // Map the R0 midpoint by hand: it is not on the integer grid.
            let m = mapped_f(o, r0_mid);
            let expected = pt(m.x + origin.x as f64, m.y + origin.y as f64);
            assert!(mid.close_to(expected, 1e-6), "{o}: {mid:?} vs {expected:?}");
            let Prim::Arc { span, .. } = placed else {
                unreachable!()
            };
            assert!((span - r0_span).abs() < 1e-9, "{o}");
        }
    }

    #[test]
    fn mirroring_without_the_swap_would_be_wrong() {
        // Guard against a refactor that drops the swap: the unswapped arc's
        // midpoint is on the opposite side.
        let g = Graphic::Arc {
            a: Point::new(-10, -10),
            b: Point::new(10, 10),
            start: Point::new(10, 0),
            end: Point::new(-10, 0),
        };
        let placed = place(&g, Point::new(0, 0), Orient::M180).unwrap();
        // M180 flips vertically: the top half becomes the bottom half.
        let mid = placed.arc_midpoint().unwrap();
        assert!(mid.close_to(pt(0.0, 10.0), 1e-9), "{mid:?}");
    }

    fn mapped_f(o: Orient, p: Pt) -> Pt {
        // Orient::apply on unit vectors gives the linear map's columns.
        let ex = o.apply(Point::new(1, 0));
        let ey = o.apply(Point::new(0, 1));
        pt(
            ex.x as f64 * p.x + ey.x as f64 * p.y,
            ex.y as f64 * p.x + ey.y as f64 * p.y,
        )
    }

    #[test]
    fn path_builder_joins_continuing_lines() {
        let mut path = PathBuilder::default();
        for (a, b) in [((0, 0), (10, 0)), ((10, 0), (10, 10)), ((0, 10), (10, 10))] {
            path.push(&Prim::Line {
                a: Pt::from(Point::new(a.0, a.1)),
                b: Pt::from(Point::new(b.0, b.1)),
            });
        }
        assert_eq!(path.finish(), "M0 0L10 0L10 10L0 10");
    }

    #[test]
    fn small_closed_triangles_are_arrowheads() {
        let line = |a: (i32, i32), b: (i32, i32)| Graphic::Line {
            a: Point::new(a.0, a.1),
            b: Point::new(b.0, b.1),
        };
        let graphics = [
            // A current-source arrowhead.
            line((0, 56), (-6, 46)),
            line((-6, 46), (6, 46)),
            line((6, 46), (0, 56)),
            // A diode body: closed but too big to fill.
            line((2, 19), (30, 19)),
            line((30, 19), (16, 43)),
            line((16, 43), (2, 19)),
            // Two sides of a triangle only.
            line((40, 0), (50, 10)),
            line((50, 10), (40, 10)),
        ];
        let found = arrowheads(&graphics);
        assert_eq!(
            found,
            vec![[Point::new(-6, 46), Point::new(0, 56), Point::new(6, 46)]]
        );
    }

    #[test]
    fn bbox_rounds_outward() {
        let mut b = BBox::EMPTY;
        assert!(b.is_empty());
        b.include(pt(-1.5, 2.2));
        b.include(pt(3.1, 4.0));
        let r = b.to_rect();
        assert_eq!((r.min, r.max), (Point::new(-2, 2), Point::new(4, 4)));
    }
}
