//! Schematic coordinates and the eight LTspice orientations.
//!
//! LTspice draws with X to the right and Y down, on a 16-unit grid. A symbol
//! placed with orientation `R90` is its `R0` drawing turned a quarter turn
//! clockwise as seen on screen. `M` orientations rotate first and then mirror
//! left to right; this order was verified against LTspice's own netlister for
//! all eight orientations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// The LTspice grid pitch. Pins sit on multiples of it.
pub const GRID: i32 = 16;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    pub fn offset(self, dx: i32, dy: i32) -> Self {
        Self::new(self.x + dx, self.y + dy)
    }

    pub fn snapped(self) -> Self {
        Self::new(snap(self.x), snap(self.y))
    }

    pub fn on_grid(self) -> bool {
        self.x % GRID == 0 && self.y % GRID == 0
    }
}

impl std::ops::Add for Point {
    type Output = Point;
    fn add(self, rhs: Point) -> Point {
        Point::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl std::ops::Sub for Point {
    type Output = Point;
    fn sub(self, rhs: Point) -> Point {
        Point::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

/// Round to the nearest grid line.
pub fn snap(v: i32) -> i32 {
    (v as f64 / GRID as f64).round() as i32 * GRID
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
pub enum Orient {
    #[default]
    R0,
    R90,
    R180,
    R270,
    M0,
    M90,
    M180,
    M270,
}

impl Orient {
    pub const ALL: [Orient; 8] = [
        Orient::R0,
        Orient::R90,
        Orient::R180,
        Orient::R270,
        Orient::M0,
        Orient::M90,
        Orient::M180,
        Orient::M270,
    ];

    pub fn is_mirrored(self) -> bool {
        matches!(self, Orient::M0 | Orient::M90 | Orient::M180 | Orient::M270)
    }

    /// Quarter turns clockwise, 0 to 3.
    pub fn quarter_turns(self) -> u8 {
        match self {
            Orient::R0 | Orient::M0 => 0,
            Orient::R90 | Orient::M90 => 1,
            Orient::R180 | Orient::M180 => 2,
            Orient::R270 | Orient::M270 => 3,
        }
    }

    pub fn from_parts(quarter_turns: u8, mirrored: bool) -> Self {
        match (quarter_turns % 4, mirrored) {
            (0, false) => Orient::R0,
            (1, false) => Orient::R90,
            (2, false) => Orient::R180,
            (3, false) => Orient::R270,
            (0, true) => Orient::M0,
            (1, true) => Orient::M90,
            (2, true) => Orient::M180,
            _ => Orient::M270,
        }
    }

    /// Map a point given relative to a symbol's origin in its `R0` drawing to
    /// where it lands relative to the origin after this orientation.
    pub fn apply(self, p: Point) -> Point {
        let r = match self.quarter_turns() {
            0 => p,
            1 => Point::new(-p.y, p.x),
            2 => Point::new(-p.x, -p.y),
            _ => Point::new(p.y, -p.x),
        };
        if self.is_mirrored() {
            Point::new(-r.x, r.y)
        } else {
            r
        }
    }

    /// The orientation reached by turning this one a further quarter turn
    /// clockwise.
    pub fn rotated_cw(self) -> Self {
        Self::from_parts(self.quarter_turns() + 1, self.is_mirrored())
    }
}

impl fmt::Display for Orient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Orient::R0 => "R0",
            Orient::R90 => "R90",
            Orient::R180 => "R180",
            Orient::R270 => "R270",
            Orient::M0 => "M0",
            Orient::M90 => "M90",
            Orient::M180 => "M180",
            Orient::M270 => "M270",
        };
        f.write_str(s)
    }
}

impl FromStr for Orient {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Orient::ALL
            .into_iter()
            .find(|o| o.to_string().eq_ignore_ascii_case(s.trim()))
            .ok_or_else(|| format!("unknown orientation `{s}` (expected R0, R90, R180, R270, M0, M90, M180 or M270)"))
    }
}

/// An axis-aligned rectangle, inclusive of its edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}

impl Rect {
    pub fn from_points(a: Point, b: Point) -> Self {
        Self {
            min: Point::new(a.x.min(b.x), a.y.min(b.y)),
            max: Point::new(a.x.max(b.x), a.y.max(b.y)),
        }
    }

    pub fn include(&mut self, p: Point) {
        self.min = Point::new(self.min.x.min(p.x), self.min.y.min(p.y));
        self.max = Point::new(self.max.x.max(p.x), self.max.y.max(p.y));
    }

    pub fn union(self, other: Rect) -> Rect {
        let mut r = self;
        r.include(other.min);
        r.include(other.max);
        r
    }

    pub fn width(&self) -> i32 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> i32 {
        self.max.y - self.min.y
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.min.x <= other.max.x
            && other.min.x <= self.max.x
            && self.min.y <= other.max.y
            && other.min.y <= self.max.y
    }

    pub fn inflate(self, by: i32) -> Rect {
        Rect {
            min: self.min.offset(-by, -by),
            max: self.max.offset(by, by),
        }
    }
}

/// Whether `p` lies on the segment from `a` to `b`, endpoints included.
/// LTspice wires are axis-aligned in practice, but diagonal ones are legal and
/// handled exactly with integer arithmetic.
pub fn on_segment(p: Point, a: Point, b: Point) -> bool {
    let cross = (b.x - a.x) as i64 * (p.y - a.y) as i64 - (b.y - a.y) as i64 * (p.x - a.x) as i64;
    cross == 0
        && p.x >= a.x.min(b.x)
        && p.x <= a.x.max(b.x)
        && p.y >= a.y.min(b.y)
        && p.y <= a.y.max(b.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotations_are_clockwise_on_screen() {
        // A pin to the right of the origin ends up below it after R90.
        assert_eq!(Orient::R90.apply(Point::new(16, 0)), Point::new(0, 16));
        assert_eq!(Orient::R180.apply(Point::new(16, 0)), Point::new(-16, 0));
        assert_eq!(Orient::R270.apply(Point::new(16, 0)), Point::new(0, -16));
    }

    #[test]
    fn rotate_then_mirror_matches_ltspice() {
        // Verified with LTspice XVII's netlister using npn pins.
        let p = Point::new(16, 96);
        assert_eq!(Orient::M0.apply(p), Point::new(-16, 96));
        assert_eq!(Orient::M90.apply(p), Point::new(96, 16));
        assert_eq!(Orient::M180.apply(p), Point::new(16, -96));
        assert_eq!(Orient::M270.apply(p), Point::new(-96, -16));
    }

    #[test]
    fn orientation_text_round_trips() {
        for o in Orient::ALL {
            assert_eq!(o.to_string().parse::<Orient>().unwrap(), o);
        }
        assert!("R45".parse::<Orient>().is_err());
    }

    #[test]
    fn four_quarter_turns_is_identity() {
        for o in Orient::ALL {
            assert_eq!(o.rotated_cw().rotated_cw().rotated_cw().rotated_cw(), o);
        }
    }

    #[test]
    fn segment_index_merges_and_finds() {
        let wires = [
            (Point::new(0, 0), Point::new(64, 0)),
            (Point::new(48, 0), Point::new(128, 0)),
            (Point::new(200, 0), Point::new(256, 0)),
            (Point::new(32, -32), Point::new(32, 32)),
        ];
        let (idx, merged) = SegmentIndex::new(&wires);
        assert_eq!(merged, vec![(0, 1)]);
        let c = idx.cover(Point::new(100, 0));
        assert_eq!(c.horizontal.map(|s| (s.lo, s.hi)), Some((0, 128)));
        assert!(idx.cover(Point::new(150, 0)).horizontal.is_none());
        let c = idx.cover(Point::new(32, 0));
        assert!(c.horizontal.is_some() && c.vertical.is_some());
        assert_eq!(
            idx.cover(Point::new(256, 0)).horizontal.map(|s| s.wire),
            Some(2)
        );
    }

    #[test]
    fn segment_membership() {
        let a = Point::new(0, 0);
        let b = Point::new(64, 0);
        assert!(on_segment(Point::new(32, 0), a, b));
        assert!(on_segment(a, a, b));
        assert!(!on_segment(Point::new(80, 0), a, b));
        assert!(!on_segment(Point::new(32, 16), a, b));
    }
}

/// Answers "which wires pass through this point" in logarithmic time.
///
/// Collinear horizontal (or vertical) wires that overlap or touch are merged
/// into one span per line, because they are electrically one conductor.
/// A query is a binary search on its row and its column. Diagonal wires, which
/// LTspice allows but nobody draws, are checked one by one.
#[derive(Debug, Clone, Default)]
pub struct SegmentIndex {
    rows: std::collections::HashMap<i32, Vec<Span>>,
    cols: std::collections::HashMap<i32, Vec<Span>>,
    diagonal: Vec<(Point, Point, usize)>,
}

/// A merged run of collinear wires. `wire` is one of them, as a representative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub lo: i32,
    pub hi: i32,
    pub wire: usize,
}

/// Which wires cover a point, by orientation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cover {
    pub horizontal: Option<Span>,
    pub vertical: Option<Span>,
    pub diagonal: Vec<usize>,
}

impl SegmentIndex {
    /// Build the index. Also returns pairs of wire indices that were merged
    /// because they overlap on the same line.
    pub fn new(wires: &[(Point, Point)]) -> (Self, Vec<(usize, usize)>) {
        let mut rows: std::collections::HashMap<i32, Vec<Span>> = Default::default();
        let mut cols: std::collections::HashMap<i32, Vec<Span>> = Default::default();
        let mut diagonal = Vec::new();
        for (i, &(a, b)) in wires.iter().enumerate() {
            if a.y == b.y {
                rows.entry(a.y).or_default().push(Span {
                    lo: a.x.min(b.x),
                    hi: a.x.max(b.x),
                    wire: i,
                });
            } else if a.x == b.x {
                cols.entry(a.x).or_default().push(Span {
                    lo: a.y.min(b.y),
                    hi: a.y.max(b.y),
                    wire: i,
                });
            } else {
                diagonal.push((a, b, i));
            }
        }
        let mut merged_pairs = Vec::new();
        for spans in rows.values_mut().chain(cols.values_mut()) {
            spans.sort_by_key(|s| (s.lo, s.hi));
            let mut out: Vec<Span> = Vec::with_capacity(spans.len());
            for s in spans.drain(..) {
                match out.last_mut() {
                    Some(cur) if s.lo <= cur.hi => {
                        merged_pairs.push((cur.wire, s.wire));
                        cur.hi = cur.hi.max(s.hi);
                    }
                    _ => out.push(s),
                }
            }
            *spans = out;
        }
        (
            Self {
                rows,
                cols,
                diagonal,
            },
            merged_pairs,
        )
    }

    /// The wires covering `p`, endpoints included.
    pub fn cover(&self, p: Point) -> Cover {
        let find = |spans: Option<&Vec<Span>>, v: i32| -> Option<Span> {
            let spans = spans?;
            let idx = spans.partition_point(|s| s.lo <= v);
            let s = *spans.get(idx.checked_sub(1)?)?;
            (s.hi >= v).then_some(s)
        };
        Cover {
            horizontal: find(self.rows.get(&p.y), p.x),
            vertical: find(self.cols.get(&p.x), p.y),
            diagonal: self
                .diagonal
                .iter()
                .filter(|(a, b, _)| on_segment(p, *a, *b))
                .map(|(_, _, i)| *i)
                .collect(),
        }
    }
}
