//! Symbol geometry the layout and the router reason about: which way each pin
//! faces, the drawn body without its pin leads, and placed rectangles.

use crate::geometry::{Orient, Point, Rect};
use crate::symbol::{Graphic, PinDef, SymbolDef};

/// One of the four grid directions, in sheet coordinates (Y down).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];

    pub fn vec(self) -> (i32, i32) {
        match self {
            Dir::Up => (0, -1),
            Dir::Down => (0, 1),
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
        }
    }

    pub fn from_vec(dx: i32, dy: i32) -> Option<Dir> {
        match (dx.signum(), dy.signum()) {
            (0, -1) => Some(Dir::Up),
            (0, 1) => Some(Dir::Down),
            (-1, 0) => Some(Dir::Left),
            (1, 0) => Some(Dir::Right),
            _ => None,
        }
    }

    pub fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
        }
    }

    pub fn horizontal(self) -> bool {
        matches!(self, Dir::Left | Dir::Right)
    }

    /// `p` moved `steps` sheet units this way.
    pub fn step(self, p: Point, units: i32) -> Point {
        let (dx, dy) = self.vec();
        p.offset(dx * units, dy * units)
    }

    pub fn index(self) -> usize {
        match self {
            Dir::Up => 0,
            Dir::Down => 1,
            Dir::Left => 2,
            Dir::Right => 3,
        }
    }
}

/// Which way a pin faces in the symbol's own frame: along its lead line, away
/// from the body. Symbols draw a short lead from every pin to the body, so
/// this is exact where the centre-of-body guess is not (a MOSFET gate sits
/// low on the left of its body but faces left).
pub(crate) fn pin_facing_r0(def: &SymbolDef, pin: &PinDef) -> Dir {
    for g in &def.graphics {
        if let Graphic::Line { a, b } = g {
            let other = if *a == pin.at {
                *b
            } else if *b == pin.at {
                *a
            } else {
                continue;
            };
            let (dx, dy) = (pin.at.x - other.x, pin.at.y - other.y);
            if (dx == 0 || dy == 0)
                && let Some(d) = Dir::from_vec(dx, dy)
            {
                return d;
            }
        }
    }
    // No straight lead: away from the centre of the drawing.
    let b = def.bounds().unwrap_or(Rect::from_points(pin.at, pin.at));
    let (cx, cy) = ((b.min.x + b.max.x) / 2, (b.min.y + b.max.y) / 2);
    let (dx, dy) = (pin.at.x - cx, pin.at.y - cy);
    if dx == 0 && dy == 0 {
        Dir::Up
    } else if dx.abs() >= dy.abs() {
        if dx < 0 { Dir::Left } else { Dir::Right }
    } else if dy < 0 {
        Dir::Up
    } else {
        Dir::Down
    }
}

/// A direction after placing with `orient`.
pub(crate) fn orient_dir(d: Dir, orient: Orient) -> Dir {
    let (dx, dy) = d.vec();
    let p = orient.apply(Point::new(dx, dy));
    Dir::from_vec(p.x, p.y).expect("orientations keep axis directions")
}

/// The facing of `pin` once the symbol is placed with `orient`.
pub(crate) fn pin_facing(def: &SymbolDef, pin: &PinDef, orient: Orient) -> Dir {
    orient_dir(pin_facing_r0(def, pin), orient)
}

/// The drawn body in the symbol's frame, leaving out the lead lines that run
/// to the pins. Wires may come close to a lead (that is where they connect)
/// but must keep clear of the body itself.
pub(crate) fn core_body(def: &SymbolDef) -> Option<Rect> {
    let on_pin = |p: &Point| def.pins.iter().any(|pin| pin.at == *p);
    let mut points: Vec<Point> = Vec::new();
    for g in &def.graphics {
        match g {
            Graphic::Line { a, b } => {
                if on_pin(a) || on_pin(b) {
                    continue;
                }
                points.extend([*a, *b]);
            }
            Graphic::Rect { a, b } | Graphic::Circle { a, b } | Graphic::Arc { a, b, .. } => {
                points.extend([*a, *b]);
            }
            Graphic::Text { .. } => {}
        }
    }
    if points.is_empty() {
        points.extend(def.pins.iter().map(|p| p.at));
    }
    let first = *points.first()?;
    let mut r = Rect::from_points(first, first);
    for p in points {
        r.include(p);
    }
    Some(r)
}

/// A rectangle in a symbol's frame, placed at `origin` with `orient`.
pub(crate) fn place_rect(r: Rect, origin: Point, orient: Orient) -> Rect {
    let a = origin + orient.apply(r.min);
    let b = origin + orient.apply(r.max);
    Rect::from_points(a, b)
}

/// Whether two rectangles share interior area (touching edges do not count).
pub(crate) fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
}

/// Whether an axis-aligned segment passes through a rectangle's interior.
pub(crate) fn segment_hits(a: Point, b: Point, r: &Rect) -> bool {
    let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
    let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
    if x0 == x1 {
        x0 > r.min.x && x0 < r.max.x && y1 > r.min.y && y0 < r.max.y
    } else if y0 == y1 {
        y0 > r.min.y && y0 < r.max.y && x1 > r.min.x && x0 < r.max.x
    } else {
        x1 > r.min.x && x0 < r.max.x && y1 > r.min.y && y0 < r.max.y
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::SymbolLibrary;

    fn facing(sym: &str, pin: &str, o: Orient) -> Dir {
        let lib = SymbolLibrary::builtin_only();
        let (def, _) = lib.resolve(sym).unwrap();
        let p = def.pin(pin).unwrap();
        pin_facing(&def, p, o)
    }

    #[test]
    fn pins_face_along_their_leads() {
        assert_eq!(facing("res", "A", Orient::R0), Dir::Up);
        assert_eq!(facing("res", "A", Orient::R90), Dir::Right);
        assert_eq!(facing("res", "A", Orient::R270), Dir::Left);
        assert_eq!(facing("nmos", "G", Orient::R0), Dir::Left);
        assert_eq!(facing("nmos", "G", Orient::M0), Dir::Right);
        assert_eq!(facing("npn", "C", Orient::R0), Dir::Up);
        assert_eq!(facing("pnp", "E", Orient::M180), Dir::Up);
        assert_eq!(facing("OpAmps/opamp", "out", Orient::R0), Dir::Right);
        assert_eq!(facing("OpAmps/opamp2", "V+", Orient::R0), Dir::Up);
        assert_eq!(facing("voltage", "-", Orient::R0), Dir::Down);
    }

    #[test]
    fn core_body_leaves_out_leads() {
        let lib = SymbolLibrary::builtin_only();
        let (res, _) = lib.resolve("res").unwrap();
        let b = core_body(&res).unwrap();
        assert_eq!((b.min.y, b.max.y), (26, 86));
    }
}
