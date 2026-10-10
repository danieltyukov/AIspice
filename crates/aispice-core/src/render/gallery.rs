//! A sheet of symbols in all eight orientations, for checking drawings, pin
//! positions and text placement by eye.

use super::{Extras, RenderOptions, render_with};
use crate::geometry::{GRID, Orient, Point};
use crate::schematic::{Attr, Item, Schematic, Shape, ShapeKind, Symbol, Text};
use crate::symbol::SymbolLibrary;

/// One row per symbol, one column per orientation, with a dot on every pin
/// and the symbol's default text windows. Names that do not resolve are drawn
/// as placeholders, so a typo shows up on the sheet.
pub fn symbol_gallery_svg(lib: &SymbolLibrary, names: &[&str]) -> String {
    let defs: Vec<_> = names
        .iter()
        .map(|n| lib.resolve(n).ok().map(|(d, _)| d))
        .collect();
    let extent = defs
        .iter()
        .flatten()
        .filter_map(|d| d.bounds())
        .map(|r| r.width().max(r.height()))
        .max()
        .unwrap_or(64);
    // Room around the drawing for attribute text, rounded to the grid.
    let cell = (extent + 128 + GRID - 1) / GRID * GRID;
    let label_w = 192;
    let header_h = 48;

    let mut sch = Schematic::new();
    for (col, o) in Orient::ALL.iter().enumerate() {
        let x = label_w + col as i32 * cell + cell / 2;
        let mut t = Text::comment(Point::new(x, header_h / 2), o.to_string());
        t.align = "Center".into();
        sch.items.push(Item::Text(t));
    }
    for (row, (name, def)) in names.iter().zip(&defs).enumerate() {
        let y0 = header_h + row as i32 * cell;
        let mut t = Text::comment(Point::new(label_w - 24, y0 + cell / 2), name.to_string());
        t.align = "Right".into();
        sch.items.push(Item::Text(t));
        let letter = def
            .as_ref()
            .and_then(|d| d.prefix().chars().next())
            .unwrap_or('X');
        for (col, o) in Orient::ALL.iter().enumerate() {
            let x0 = label_w + col as i32 * cell;
            sch.items.push(Item::Shape(Shape {
                kind: ShapeKind::Rectangle,
                tokens: vec![
                    "Normal".to_string(),
                    x0.to_string(),
                    y0.to_string(),
                    (x0 + cell).to_string(),
                    (y0 + cell).to_string(),
                    // LTspice dash style 2: dotted.
                    "2".to_string(),
                ],
            }));
            let centre = Point::new(x0 + cell / 2, y0 + cell / 2);
            let at = match def
                .as_ref()
                .and_then(|d| d.placed_bounds(Point::new(0, 0), *o))
            {
                Some(b) => centre - Point::new((b.min.x + b.max.x) / 2, (b.min.y + b.max.y) / 2),
                None => centre - Point::new(32, 32),
            };
            let mut sym = Symbol::new(*name, at, *o);
            sym.attrs.push(Attr {
                key: "InstName".into(),
                value: format!("{letter}{}", col + 1),
            });
            sch.items.push(Item::Symbol(sym));
        }
    }
    let opts = RenderOptions {
        padding: 16,
        standalone: true,
        show_unconnected_pins: false,
        highlight: Vec::new(),
    };
    render_with(&sch, lib, &opts, Extras { pin_dots: true }).svg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gallery_has_eight_placements_per_symbol() {
        let lib = SymbolLibrary::builtin_only();
        let svg = symbol_gallery_svg(&lib, &["res", "npn"]);
        assert_eq!(svg.matches(r#"data-symbol="res""#).count(), 8);
        assert_eq!(svg.matches(r#"data-symbol="npn""#).count(), 8);
        // Two pins on each resistor, three on each transistor.
        assert_eq!(svg.matches(r#"class="pin-dot""#).count(), 8 * 2 + 8 * 3);
        assert!(svg.contains(">M270<"));
    }

    #[test]
    fn unknown_names_show_as_placeholders() {
        let lib = SymbolLibrary::builtin_only();
        let svg = symbol_gallery_svg(&lib, &["nope"]);
        assert_eq!(svg.matches(r#"class="symbol missing""#).count(), 8);
    }
}
