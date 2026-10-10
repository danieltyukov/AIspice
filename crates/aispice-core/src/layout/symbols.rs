//! How each part is drawn: the orientations that read naturally for its
//! kind, and where its name and value go so they sit beside the body rather
//! than on a wire.

use super::circuit::{Device, Kind};
use super::geom::{core_body, orient_dir};
use crate::geometry::{Orient, Point};
use crate::schematic::{Symbol, Window};

/// Orientations worth trying for a part, with a small cost for the less
/// conventional ones. Transistors keep the terminal at the higher potential
/// on top: collector or drain for N types, emitter or source for P types.
pub(crate) fn orientations(d: &Device) -> Vec<(Orient, f64)> {
    use Orient::*;
    match d.kind {
        Kind::Passive | Kind::Diode => vec![(R0, 0.0), (M180, 0.0), (R270, 0.0), (R90, 0.0)],
        Kind::Source | Kind::Source2 => vec![(R0, 0.0), (M180, 0.0), (R270, 4.0), (R90, 4.0)],
        Kind::Bjt { p: false } | Kind::Fet { p: false } => {
            vec![(R0, 0.0), (M0, 0.5), (M180, 8.0), (R180, 8.0)]
        }
        Kind::Bjt { p: true } | Kind::Fet { p: true } => {
            vec![(M180, 0.0), (R180, 0.5), (R0, 8.0), (M0, 8.0)]
        }
        Kind::OpAmp => vec![(R0, 0.0), (M180, 1.0)],
        Kind::OpAmp5 => vec![(R0, 0.0)],
        Kind::Controlled => vec![(R0, 0.0), (M180, 2.0)],
        Kind::Switch => vec![
            (R270, 0.0),
            (M90, 0.0),
            (R0, 0.5),
            (R90, 1.0),
            (M270, 1.0),
            (M180, 1.0),
        ],
        Kind::Tline => vec![(R0, 0.0), (M0, 0.0)],
        Kind::Sub => vec![(R0, 0.0), (M0, 1.0)],
    }
}

/// Inverse of an orientation, for turning a desired sheet offset back into
/// the symbol's own frame.
fn inverse(o: Orient) -> Orient {
    Orient::ALL
        .into_iter()
        .find(|&i| {
            let p = Point::new(3, 7);
            i.apply(o.apply(p)) == p
        })
        .expect("every orientation has an inverse")
}

/// The WINDOW lines a part needs in this orientation, or none when the
/// symbol's defaults already read well.
///
/// Two-terminal parts get their name above (or left of) the value, beside
/// the body: LTspice's own convention for a resistor turned sideways is
/// name above, value below. Other parts keep the symbol's windows, with the
/// top-to-bottom order restored if the orientation turned it upside down.
pub(crate) fn windows(d: &Device, orient: Orient, flip_text: bool) -> Vec<Window> {
    let def = &d.def;
    let has_value2 = d
        .attrs
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("Value2"));
    if d.kind.two_terminal() && def.pins.len() == 2 {
        let Some(core) = core_body(def) else {
            return Vec::new();
        };
        let (a, b) = (def.pins[0].at, def.pins[1].at);
        let yc = (a.y + b.y) / 2;
        let xc = (core.min.x + core.max.x) / 2;
        let half_w = (core.max.x - core.min.x) / 2;
        let horizontal = orient_dir(super::geom::Dir::Down, orient).horizontal();
        // In sheet terms, relative to the body centre.
        let centre = Point::new(xc, yc);
        let mut out = Vec::new();
        if horizontal {
            // Name above, value below, centred on the body.
            let gap = half_w + 2;
            let name_at = Point::new(0, -gap);
            let value_at = Point::new(0, gap);
            let to_frame = |sheet: Point| centre + inverse(orient).apply(sheet);
            // A vertical justification in the symbol frame becomes
            // horizontal text once turned a quarter.
            let (na, va) = if orient.apply(Point::new(0, 1)).x < 0 {
                ("VBottom", "VTop")
            } else {
                ("VTop", "VBottom")
            };
            out.push(Window {
                index: 0,
                at: to_frame(name_at),
                align: na.into(),
                size: 2,
            });
            out.push(Window {
                index: 3,
                at: to_frame(value_at),
                align: va.into(),
                size: 2,
            });
            if has_value2 {
                out.push(Window {
                    index: 123,
                    at: to_frame(value_at.offset(0, 24)),
                    align: va.into(),
                    size: 2,
                });
            }
        } else {
            // Beside the body, name above value: on the right unless asked
            // to keep the right side free.
            let (x, align) = if flip_text {
                (-(half_w + 8), "Right")
            } else {
                (half_w + 8, "Left")
            };
            let lines: Vec<(i32, i32)> = if has_value2 {
                vec![(0, -24), (3, 0), (123, 24)]
            } else {
                vec![(0, -16), (3, 16)]
            };
            for (index, dy) in lines {
                out.push(Window {
                    index,
                    at: centre + inverse(orient).apply(Point::new(x, dy)),
                    align: align.into(),
                    size: 2,
                });
            }
        }
        return out;
    }
    // Multi-pin parts: the default windows, moved to the other side of the
    // body on request, and put back in reading order if the orientation
    // turned the drawing upside down.
    let flips = orient.apply(Point::new(0, 1)).y < 0;
    let quarter = orient.quarter_turns() % 2 == 1;
    if def.windows.is_empty() {
        return Vec::new();
    }
    if quarter {
        // Turned a quarter, the symbol's own text would read vertically. Put
        // the windows above the body as horizontal lines instead, name over
        // value: a vertical justification in the symbol's frame turns back to
        // horizontal on the sheet, which is what people do in LTspice.
        let Some(core) =
            core_body(def).map(|b| super::geom::place_rect(b, Point::new(0, 0), orient))
        else {
            return Vec::new();
        };
        let cx = (core.min.x + core.max.x) / 2;
        let align = if orient.apply(Point::new(0, 1)).x < 0 {
            "VBottom"
        } else {
            "VTop"
        };
        let mut rows: Vec<i32> = def.windows.iter().map(|w| w.index).collect();
        rows.sort_by_key(|&i| match i {
            0 => 0,
            3 => 1,
            _ => 2,
        });
        let n = rows.len() as i32;
        return rows
            .into_iter()
            .enumerate()
            .map(|(k, index)| {
                let sheet = Point::new(cx, core.min.y - 4 - (n - 1 - k as i32) * 22);
                Window {
                    index,
                    at: inverse(orient).apply(sheet),
                    align: align.into(),
                    size: 2,
                }
            })
            .collect();
    }
    if !flips && !flip_text {
        return Vec::new();
    }
    let centre = core_body(def)
        .map(|b| Point::new((b.min.x + b.max.x) / 2, (b.min.y + b.max.y) / 2))
        .unwrap_or(Point::new(0, 0));
    let cx = orient.apply(centre).x;
    let mut placed: Vec<(i32, Point, String, i32)> = def
        .windows
        .iter()
        .map(|w| {
            let mut at = orient.apply(w.at);
            let mut align = w.align.clone();
            if flip_text {
                at.x = 2 * cx - at.x;
                align = match align.to_ascii_lowercase().as_str() {
                    "left" => "Right".into(),
                    "right" => "Left".into(),
                    _ => align,
                };
            }
            (w.index, at, align, w.size)
        })
        .collect();
    if flips {
        let ys: Vec<i32> = def.windows.iter().map(|w| w.at.y).collect();
        let mut order: Vec<usize> = (0..ys.len()).collect();
        order.sort_by_key(|&i| ys[i]);
        let mut sheet_ys: Vec<i32> = placed.iter().map(|w| w.1.y).collect();
        sheet_ys.sort();
        for (rank, &i) in order.iter().enumerate() {
            placed[i].1.y = sheet_ys[rank];
        }
    }
    placed
        .into_iter()
        .map(|(index, at, align, size)| Window {
            index,
            at: inverse(orient).apply(at),
            align,
            size,
        })
        .collect()
}

/// Whether moving a part's text to its other side is worth trying: upright
/// two-terminal parts and transistors. Op-amps keep theirs above and below.
pub(crate) fn text_can_flip(d: &Device, orient: Orient) -> bool {
    if d.kind.two_terminal() {
        !orient_dir(super::geom::Dir::Down, orient).horizontal()
    } else {
        d.kind.transistor()
    }
}

/// The symbol record for a placed part.
pub(crate) fn symbol(d: &Device, at: Point, orient: Orient, flip_text: bool) -> Symbol {
    let mut s = Symbol::new(d.symbol.clone(), at, orient);
    s.windows = windows(d, orient, flip_text);
    s.set_attr("InstName", d.inst.clone());
    for (k, v) in &d.attrs {
        s.set_attr(k, v.clone());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::circuit;
    use crate::layout::text::symbol_texts;
    use crate::netlist::parse;
    use crate::symbol::SymbolLibrary;

    #[test]
    fn sideways_resistor_has_name_above_and_value_below() {
        let c = circuit::build(&parse("t\nR1 a b 10k\n"), &SymbolLibrary::builtin_only());
        let d = &c.devices[0];
        for o in [Orient::R90, Orient::R270] {
            let s = symbol(d, Point::new(0, 0), o, false);
            let texts = symbol_texts(&s, &d.def);
            let pin_y = d.def.pin_position(&d.def.pins[0], s.at, o).y;
            assert_eq!(texts.len(), 2, "{o}");
            assert!(texts[0].max.y <= pin_y - 8, "{o}: name {:?}", texts[0]);
            assert!(texts[1].min.y >= pin_y + 8, "{o}: value {:?}", texts[1]);
        }
    }

    #[test]
    fn flipped_parts_keep_name_above_value() {
        let c = circuit::build(
            &parse("t\nC1 a b 1u\nQ1 c b e PNP\n"),
            &SymbolLibrary::builtin_only(),
        );
        for (d, o) in [(&c.devices[0], Orient::M180), (&c.devices[1], Orient::M180)] {
            let s = symbol(d, Point::new(0, 0), o, false);
            let texts = symbol_texts(&s, &d.def);
            assert!(texts.len() >= 2);
            assert!(texts[0].min.y < texts[1].min.y, "{}: {texts:?}", d.inst);
        }
    }
}
