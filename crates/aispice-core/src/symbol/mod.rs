//! Component symbols (`.asy`): pins, SPICE attributes and drawing.
//!
//! Pin positions are what make a schematic connect, so they come from real
//! symbol files rather than from anyone's memory: the project folder first,
//! then any extra folders the user configures, then an LTspice installation if
//! one is found, and finally aispice's own built-in set. The built-in symbols
//! are original drawings whose pins, pin order and attributes match LTspice's,
//! so files move between the two without rewiring.

mod asy;
mod builtin;
mod library;

pub use asy::parse_asy;
pub use library::{LibraryError, Resolved, SymbolHit, SymbolLibrary, SymbolSource};

use crate::geometry::{Orient, Point, Rect};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SymbolType {
    /// A primitive or subcircuit instance.
    #[default]
    Cell,
    /// A hierarchical block whose contents are another schematic.
    Block,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PinDef {
    pub at: Point,
    pub name: String,
    /// Position in the SPICE instance line, starting at 1.
    pub spice_order: u32,
    /// Label placement: `NONE`, `LEFT`, `RIGHT`, `TOP`, `BOTTOM`, `VLEFT`, ...
    pub justification: String,
    pub label_offset: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Graphic {
    Line {
        a: Point,
        b: Point,
    },
    Rect {
        a: Point,
        b: Point,
    },
    /// An ellipse inscribed in the box from `a` to `b`.
    Circle {
        a: Point,
        b: Point,
    },
    /// Part of the ellipse in the box `a`..`b`, drawn counter-clockwise from
    /// the direction of `start` to the direction of `end`, as LTspice does.
    Arc {
        a: Point,
        b: Point,
        start: Point,
        end: Point,
    },
    Text {
        at: Point,
        align: String,
        size: i32,
        text: String,
    },
}

/// One attribute-text placement in a symbol's default drawing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowDef {
    pub index: i32,
    pub at: Point,
    pub align: String,
    pub size: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SymbolDef {
    pub kind: SymbolType,
    pub pins: Vec<PinDef>,
    pub graphics: Vec<Graphic>,
    pub windows: Vec<WindowDef>,
    /// SYMATTR defaults: Prefix, Value, Value2, SpiceModel, SpiceLine,
    /// SpiceLine2, Description, ModelFile.
    pub attrs: Vec<(String, String)>,
}

impl SymbolDef {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    /// The SPICE prefix, e.g. `R`, `QN`, `X`. Its first letter is the device
    /// letter in the netlist.
    pub fn prefix(&self) -> &str {
        self.attr("Prefix").unwrap_or("X")
    }

    pub fn description(&self) -> Option<&str> {
        self.attr("Description")
    }

    /// Pins sorted by SPICE order, the order nodes appear in the netlist.
    pub fn pins_in_spice_order(&self) -> Vec<&PinDef> {
        let mut pins: Vec<&PinDef> = self.pins.iter().collect();
        pins.sort_by_key(|p| p.spice_order);
        pins
    }

    pub fn pin(&self, name: &str) -> Option<&PinDef> {
        self.pins.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Absolute pin position for a symbol placed at `origin` with `orient`.
    pub fn pin_position(&self, pin: &PinDef, origin: Point, orient: Orient) -> Point {
        origin + orient.apply(pin.at)
    }

    /// Bounding box of the drawing and pins in the symbol's own coordinates.
    pub fn bounds(&self) -> Option<Rect> {
        let mut points = self
            .pins
            .iter()
            .map(|p| p.at)
            .chain(self.graphics.iter().flat_map(|g| match g {
                Graphic::Line { a, b } | Graphic::Rect { a, b } | Graphic::Circle { a, b } => {
                    vec![*a, *b]
                }
                Graphic::Arc { a, b, .. } => vec![*a, *b],
                Graphic::Text { at, .. } => vec![*at],
            }));
        let first = points.next()?;
        let mut r = Rect::from_points(first, first);
        for p in points {
            r.include(p);
        }
        Some(r)
    }

    /// Bounding box after placement.
    pub fn placed_bounds(&self, origin: Point, orient: Orient) -> Option<Rect> {
        let b = self.bounds()?;
        let corners = [
            b.min,
            Point::new(b.max.x, b.min.y),
            b.max,
            Point::new(b.min.x, b.max.y),
        ];
        let mut r = Rect::from_points(
            origin + orient.apply(corners[0]),
            origin + orient.apply(corners[0]),
        );
        for c in corners {
            r.include(origin + orient.apply(c));
        }
        Some(r)
    }
}
