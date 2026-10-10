//! Schematic rendering: `.asc` to SVG, and SVG to PNG for multimodal models.
//!
//! The SVG is meant to be themed. Every element carries a class (`wire`,
//! `junction`, `symbol`, `label`, `flag`, `directive`, `comment`, `attr`, ...)
//! and the embedded style sheet reads its colours from CSS custom properties
//! listed in [`THEME_VARIABLES`], so the desktop app and the website switch
//! between light and dark without touching the markup. With
//! [`RenderOptions::standalone`] the fallback colours are written in directly
//! and a background is added, for files viewed on their own and for
//! rasterising.
//!
//! Geometry follows LTspice: symbol drawings and their text windows transform
//! with the placed orientation exactly like pins, arcs run counter-clockwise
//! on screen (so a mirror reverses them), and text is kept upright the way
//! LTspice keeps it readable. Symbol groups carry `data-inst` and
//! `data-symbol`, wires `data-wire`, so a viewer can map clicks back to items.

mod gallery;
mod geom;
mod junction;
#[cfg(feature = "raster")]
mod raster;
mod style;
mod svg;
mod text;

pub use gallery::symbol_gallery_svg;
#[cfg(feature = "raster")]
pub use raster::render_png;
pub use style::{THEME_VARIABLES, ThemeVar};

use crate::geometry::{Orient, Point, Rect};
use crate::schematic::{Flag, Item, Schematic, Shape, ShapeKind, Symbol, Text, TextKind, Wire};
use crate::symbol::{Graphic, SymbolDef, SymbolLibrary};
use geom::{BBox, PathBuilder, Pt};
use junction::{Sides, sides};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::sync::Arc;
use svg::{esc, num};
use text::{Align, HAlign, TextBlock, VAlign, font_size, split_lines, text_width};

/// How a part changed, for highlighting a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HighlightKind {
    Added,
    Removed,
    Changed,
}

impl HighlightKind {
    /// The class added to the symbol's group.
    pub fn class(self) -> &'static str {
        match self {
            HighlightKind::Added => "hl-added",
            HighlightKind::Removed => "hl-removed",
            HighlightKind::Changed => "hl-changed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Highlight {
    /// Instance name, matched case-insensitively as SPICE does.
    pub inst: String,
    pub kind: HighlightKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RenderOptions {
    /// Margin around the drawing, in schematic units (16 per grid step).
    pub padding: i32,
    /// Write fixed colours and a white background instead of CSS custom
    /// properties. Use it for files opened on their own and for PNGs; turn it
    /// off when the SVG is inlined in a themed page.
    pub standalone: bool,
    /// Draw a small marker on every pin that nothing connects to.
    pub show_unconnected_pins: bool,
    /// Parts to emphasise, for showing a diff.
    pub highlight: Vec<Highlight>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            padding: 32,
            standalone: true,
            show_unconnected_pins: false,
            highlight: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RenderOutput {
    pub svg: String,
    /// Bounds of the drawing in schematic coordinates, before padding.
    pub bounds: Rect,
    /// Things the reader should know, such as symbols drawn as placeholders.
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum RenderError {
    #[error("the SVG could not be read: {0}")]
    Svg(String),
    #[error("scale must be a positive number, got {0}")]
    Scale(f32),
    #[error("the image would be {width}x{height} pixels, over the limit of {max} per side")]
    TooLarge { width: u32, height: u32, max: u32 },
    #[error("PNG encoding failed: {0}")]
    Encode(String),
}

/// Render a schematic to SVG.
pub fn render_svg(sch: &Schematic, lib: &SymbolLibrary, opts: &RenderOptions) -> RenderOutput {
    render_with(sch, lib, opts, Extras::default())
}

/// Drawing aids that only the symbol gallery uses.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Extras {
    /// A dot on every pin, connected or not.
    pub pin_dots: bool,
}

/// A symbol instance with its definition, if one was found.
struct Placed<'a> {
    sym: &'a Symbol,
    def: Option<Arc<SymbolDef>>,
}

impl Placed<'_> {
    fn pins(&self) -> Vec<Point> {
        self.def
            .as_ref()
            .map(|d| {
                d.pins
                    .iter()
                    .map(|p| d.pin_position(p, self.sym.at, self.sym.orient))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn centre(&self) -> Point {
        match self
            .def
            .as_ref()
            .and_then(|d| d.placed_bounds(self.sym.at, self.sym.orient))
        {
            Some(r) => Point::new((r.min.x + r.max.x) / 2, (r.min.y + r.max.y) / 2),
            None => self.sym.at,
        }
    }
}

/// Output for one layer plus the area it covers.
#[derive(Default)]
struct Layer {
    svg: String,
    bbox: BBox,
}

/// The renderer behind [`render_svg`] and the symbol gallery.
pub(crate) fn render_with(
    sch: &Schematic,
    lib: &SymbolLibrary,
    opts: &RenderOptions,
    extras: Extras,
) -> RenderOutput {
    let mut warnings = Vec::new();
    let placed: Vec<Placed> = sch
        .symbols()
        .map(|sym| match lib.resolve(&sym.name) {
            Ok((def, _)) => Placed {
                sym,
                def: Some(def),
            },
            Err(_) => {
                warnings.push(format!(
                    "symbol `{}` for {} was not found; drawn as a placeholder box",
                    sym.name,
                    sym.inst_name().unwrap_or("an unnamed part")
                ));
                Placed { sym, def: None }
            }
        })
        .collect();
    for h in &opts.highlight {
        if sch.symbol(&h.inst).is_none() {
            warnings.push(format!(
                "highlighted part {} is not in the schematic",
                h.inst
            ));
        }
    }

    let wires: Vec<Wire> = sch.wires().copied().collect();
    let pin_owners: Vec<(Point, Point)> = placed
        .iter()
        .flat_map(|p| {
            let centre = p.centre();
            p.pins().into_iter().map(move |pin| (pin, centre))
        })
        .collect();
    let pins: Vec<Point> = pin_owners.iter().map(|(p, _)| *p).collect();
    let flag_points: Vec<Point> = sch.flags().map(|f| f.at).collect();
    let conn = junction::analyse(&wires, &pins, &flag_points);

    let mut shapes = Layer::default();
    let mut wire_layer = Layer::default();
    let mut symbols = Layer::default();
    let mut flags = Layer::default();
    let mut dots = Layer::default();
    let mut texts = Layer::default();

    for item in &sch.items {
        match item {
            Item::Shape(s) => draw_shape(&mut shapes, s),
            Item::Wire(w) => draw_wire(&mut wire_layer, w, "wire"),
            Item::BusTap(t) => draw_wire(&mut wire_layer, &Wire::new(t.a, t.b), "bustap"),
            Item::Text(t) => draw_text(&mut texts, t),
            _ => {}
        }
    }
    for p in &placed {
        let hl = p.sym.inst_name().and_then(|n| {
            opts.highlight
                .iter()
                .find(|h| h.inst.eq_ignore_ascii_case(n))
                .map(|h| h.kind)
        });
        draw_symbol(&mut symbols, p, hl);
    }
    for f in sch.flags() {
        let port = sch
            .io_pins()
            .find(|io| io.at == f.at)
            .map(|io| io.direction.as_str());
        draw_flag(&mut flags, f, port, sides(f.at, &wires, &pin_owners));
    }
    for j in &conn.junctions {
        let _ = writeln!(
            dots.svg,
            r#"<circle class="junction" cx="{}" cy="{}" r="4"/>"#,
            j.x, j.y
        );
        dots.bbox
            .include(Pt::new(j.x as f64 - 4.0, j.y as f64 - 4.0));
        dots.bbox
            .include(Pt::new(j.x as f64 + 4.0, j.y as f64 + 4.0));
    }
    if opts.show_unconnected_pins {
        for p in &conn.open_pins {
            let _ = writeln!(
                dots.svg,
                r#"<rect class="pin" x="{}" y="{}" width="6" height="6"/>"#,
                p.x - 3,
                p.y - 3
            );
        }
    }
    if extras.pin_dots {
        for p in &pins {
            let _ = writeln!(
                dots.svg,
                r#"<circle class="pin-dot" cx="{}" cy="{}" r="3"/>"#,
                p.x, p.y
            );
        }
    }

    let layers = [
        ("shapes", &shapes),
        ("wires", &wire_layer),
        ("symbols", &symbols),
        ("flags", &flags),
        ("junctions", &dots),
        ("texts", &texts),
    ];
    let mut content = BBox::EMPTY;
    for (_, l) in &layers {
        content.union(&l.bbox);
    }
    for p in &pins {
        content.include(Pt::from(*p));
    }
    let bounds = content.to_rect();
    let view = bounds.inflate(opts.padding.max(0));
    let (w, h) = (view.width().max(1), view.height().max(1));

    let mut out = String::new();
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" class="aispice-sch" viewBox="{} {} {w} {h}" width="{w}" height="{h}" xml:space="preserve">"#,
        view.min.x, view.min.y
    );
    out.push('\n');
    let _ = writeln!(
        out,
        "<style>\n{}\n</style>",
        style::style_sheet(opts.standalone)
    );
    let _ = writeln!(
        out,
        r#"<rect class="bg" x="{}" y="{}" width="{w}" height="{h}"/>"#,
        view.min.x, view.min.y
    );
    for (class, layer) in layers {
        if !layer.svg.is_empty() {
            let _ = writeln!(out, "<g class=\"{class}\">\n{}</g>", layer.svg);
        }
    }
    out.push_str("</svg>\n");
    RenderOutput {
        svg: out,
        bounds,
        warnings,
    }
}

fn draw_wire(layer: &mut Layer, w: &Wire, class: &str) {
    if w.a == w.b {
        return;
    }
    let (a, b) = (w.a, w.b);
    let _ = writeln!(
        layer.svg,
        r#"<polyline class="{class}" data-wire="{},{},{},{}" points="{},{} {},{}"/>"#,
        a.x, a.y, b.x, b.y, a.x, a.y, b.x, b.y
    );
    layer.bbox.include(Pt::from(a));
    layer.bbox.include(Pt::from(b));
}

/// The attribute a WINDOW index shows.
fn window_attr(index: i32) -> Option<&'static str> {
    Some(match index {
        0 => "InstName",
        3 => "Value",
        123 => "Value2",
        38 => "SpiceModel",
        39 => "SpiceLine",
        40 => "SpiceLine2",
        _ => return None,
    })
}

fn window_class(index: i32) -> &'static str {
    match index {
        0 => "attr name",
        3 => "attr value",
        _ => "attr",
    }
}

/// A text window: position in the symbol's frame, justification and size.
struct Win {
    index: i32,
    at: Point,
    align: String,
    size: i32,
}

/// The windows to draw: the symbol definition's defaults, each replaced by the
/// instance's WINDOW line with the same index, plus any extra instance ones.
fn windows(sym: &Symbol, def: Option<&SymbolDef>) -> Vec<Win> {
    let mut out: Vec<Win> = def
        .map(|d| {
            d.windows
                .iter()
                .map(|w| Win {
                    index: w.index,
                    at: w.at,
                    align: w.align.clone(),
                    size: w.size,
                })
                .collect()
        })
        .unwrap_or_default();
    for w in &sym.windows {
        let win = Win {
            index: w.index,
            at: w.at,
            align: w.align.clone(),
            size: w.size,
        };
        match out.iter_mut().find(|o| o.index == w.index) {
            Some(slot) => *slot = win,
            None => out.push(win),
        }
    }
    out
}

fn window_text(sym: &Symbol, def: Option<&SymbolDef>, index: i32) -> Option<String> {
    let key = window_attr(index)?;
    // LTspice shows the symbol's default for anything the instance leaves
    // unset (an op-amp shows its model name), except the instance name.
    let value = sym.attr(key).or_else(|| {
        (index != 0)
            .then(|| def.and_then(|d| d.attr(key)))
            .flatten()
    })?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn draw_symbol(layer: &mut Layer, p: &Placed, hl: Option<HighlightKind>) {
    let sym = p.sym;
    let (origin, orient) = (sym.at, sym.orient);
    let place = |q: Point| Pt::from(origin + orient.apply(q));
    let mut body = PathBuilder::default();
    let mut fills = String::new();
    let mut geometry = BBox::EMPTY;
    let mut extra = String::new();
    let mut text_box = BBox::EMPTY;

    match p.def.as_deref() {
        Some(def) => {
            for tri in geom::arrowheads(&def.graphics) {
                let [a, b, c] = tri.map(place);
                let _ = write!(
                    fills,
                    "M{} {}L{} {}L{} {}Z",
                    num(a.x),
                    num(a.y),
                    num(b.x),
                    num(b.y),
                    num(c.x),
                    num(c.y)
                );
            }
            for g in &def.graphics {
                if let Some(prim) = geom::place(g, origin, orient) {
                    geometry.union(&prim.bbox());
                    body.push(&prim);
                } else if let Graphic::Text {
                    at,
                    align,
                    size,
                    text,
                } = g
                    && let Some(a) = Align::parse(align)
                {
                    let lines = split_lines(text);
                    let b = TextBlock {
                        class: "sym-text",
                        at: place(*at),
                        align: a.oriented(orient),
                        size: *size,
                        lines: &lines,
                    }
                    .write(&mut extra);
                    text_box.union(&b);
                }
            }
            for pin in &def.pins {
                if pin.justification.eq_ignore_ascii_case("none") || pin.name.is_empty() {
                    continue;
                }
                let Some(a) = Align::parse(&pin.justification) else {
                    continue;
                };
                let (dx, dy) = a.offset_dir();
                let at = pin.at.offset(dx * pin.label_offset, dy * pin.label_offset);
                let lines = vec![pin.name.clone()];
                let b = TextBlock {
                    class: "pin-label",
                    at: place(at),
                    align: a.oriented(orient),
                    size: 2,
                    lines: &lines,
                }
                .write(&mut extra);
                text_box.union(&b);
            }
        }
        None => {
            // Unknown size: a 64-unit box at the origin, turned like a symbol.
            let prim = geom::place(
                &Graphic::Rect {
                    a: Point::new(0, 0),
                    b: Point::new(64, 64),
                },
                origin,
                orient,
            )
            .expect("rectangles always place");
            geometry.union(&prim.bbox());
            body.push(&prim);
            let mut lines = Vec::new();
            if let Some(n) = sym.inst_name() {
                lines.push(n.to_string());
            }
            lines.push(sym.name.clone());
            let b = TextBlock {
                class: "sym-text",
                at: geometry.center(),
                align: Align::horizontal(HAlign::Middle, VAlign::Middle),
                size: 1,
                lines: &lines,
            }
            .write(&mut extra);
            text_box.union(&b);
        }
    }

    let def = p.def.as_deref();
    for w in windows(sym, def) {
        // The placeholder already shows the instance name inside its box.
        if def.is_none() && w.index == 0 {
            continue;
        }
        let (Some(a), Some(value)) = (Align::parse(&w.align), window_text(sym, def, w.index))
        else {
            continue;
        };
        let lines = vec![value];
        let b = TextBlock {
            class: window_class(w.index),
            at: place(w.at),
            align: a.oriented(orient),
            size: w.size,
            lines: &lines,
        }
        .write(&mut extra);
        text_box.union(&b);
    }

    let mut class = String::from("symbol");
    if def.is_none() {
        class.push_str(" missing");
    }
    if let Some(h) = hl {
        class.push(' ');
        class.push_str(h.class());
    }
    let _ = write!(layer.svg, r#"<g class="{class}""#);
    if let Some(n) = sym.inst_name() {
        let _ = write!(layer.svg, r#" data-inst="{}""#, esc(n));
    }
    let _ = write!(layer.svg, r#" data-symbol="{}">"#, esc(&sym.name));
    if hl.is_some() && !geometry.is_empty() {
        let mut b = geometry;
        b.union(&text_box);
        let b = b.inflate(6.0);
        let _ = write!(
            layer.svg,
            r#"<rect class="hl-box" x="{}" y="{}" width="{}" height="{}" rx="6"/>"#,
            num(b.min.x),
            num(b.min.y),
            num(b.max.x - b.min.x),
            num(b.max.y - b.min.y)
        );
    }
    if !body.is_empty() {
        let _ = write!(layer.svg, r#"<path class="body" d="{}"/>"#, body.finish());
    }
    if !fills.is_empty() {
        let _ = write!(layer.svg, r#"<path class="body fill" d="{fills}"/>"#);
    }
    layer.svg.push_str(&extra);
    layer.svg.push_str("</g>\n");
    layer.bbox.union(&geometry.inflate(1.0));
    layer.bbox.union(&text_box);
}

/// Which side of its connection point a net label's text goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Right,
    Left,
    Above,
    Below,
}

/// Put the label where nothing is attached, preferring above.
fn label_side(s: Sides) -> Side {
    let attached = [s.left, s.right, s.up, s.down]
        .iter()
        .filter(|b| **b)
        .count();
    if attached == 1 {
        return if s.left {
            Side::Right
        } else if s.right {
            Side::Left
        } else if s.up {
            Side::Below
        } else {
            Side::Above
        };
    }
    if !s.up {
        Side::Above
    } else if !s.down {
        Side::Below
    } else if !s.right {
        Side::Right
    } else {
        Side::Left
    }
}

fn draw_flag(layer: &mut Layer, f: &Flag, port: Option<&str>, attached: Sides) {
    let (x, y) = (f.at.x as f64, f.at.y as f64);
    if f.is_ground() {
        let _ = writeln!(
            layer.svg,
            r#"<g class="flag ground" data-net="0"><path class="mark" d="M{x} {y}V{y1}M{l1} {y1}H{r1}M{l2} {y2}H{r2}M{l3} {y3}H{r3}"/></g>"#,
            x = num(x),
            y = num(y),
            y1 = num(y + 8.0),
            y2 = num(y + 14.0),
            y3 = num(y + 20.0),
            l1 = num(x - 16.0),
            r1 = num(x + 16.0),
            l2 = num(x - 10.0),
            r2 = num(x + 10.0),
            l3 = num(x - 4.0),
            r3 = num(x + 4.0),
        );
        layer.bbox.include(Pt::new(x - 17.0, y - 1.0));
        layer.bbox.include(Pt::new(x + 17.0, y + 21.0));
        return;
    }

    let side = label_side(attached);
    let fs = font_size(2);
    let lines = vec![f.label.clone()];
    let _ = write!(
        layer.svg,
        r#"<g class="flag{}" data-net="{}">"#,
        if port.is_some() { " port" } else { "" },
        esc(&f.label)
    );
    let (text_at, align) = match port {
        Some(dir) => {
            let (shape, centre) = port_shape(x, y, side, text_width(&f.label, fs), dir);
            let _ = write!(layer.svg, r#"<path class="mark" d="{shape}"/>"#);
            // The text runs along the port outline.
            let mut align = Align::horizontal(HAlign::Middle, VAlign::Middle);
            align.vertical = matches!(side, Side::Above | Side::Below);
            (centre, align)
        }
        None => {
            // A label on a wire or pin needs no mark, as in LTspice; a
            // floating one gets a small box so its connection point shows.
            if attached == Sides::default() {
                let _ = write!(
                    layer.svg,
                    r#"<path class="mark" d="M{} {}h5v5h-5Z"/>"#,
                    num(x - 2.5),
                    num(y - 2.5)
                );
            }
            // As in LTspice, a label at the end of a single vertical wire or
            // pin reads upwards, away from it; otherwise the text is
            // horizontal, beside or above the connection.
            let single = [attached.left, attached.right, attached.up, attached.down]
                .iter()
                .filter(|b| **b)
                .count()
                == 1;
            let gap = 4.0;
            match side {
                Side::Right => (
                    Pt::new(x + gap, y),
                    Align::horizontal(HAlign::Start, VAlign::Middle),
                ),
                Side::Left => (
                    Pt::new(x - gap, y),
                    Align::horizontal(HAlign::End, VAlign::Middle),
                ),
                Side::Above if single => (
                    Pt::new(x, y - gap),
                    Align {
                        vertical: true,
                        h: HAlign::Start,
                        v: VAlign::Middle,
                    },
                ),
                Side::Below if single => (
                    Pt::new(x, y + gap),
                    Align {
                        vertical: true,
                        h: HAlign::End,
                        v: VAlign::Middle,
                    },
                ),
                Side::Above => (
                    Pt::new(x, y - 2.0),
                    Align::horizontal(HAlign::Middle, VAlign::Bottom),
                ),
                Side::Below => (
                    Pt::new(x, y + 2.0),
                    Align::horizontal(HAlign::Middle, VAlign::Top),
                ),
            }
        }
    };
    let b = TextBlock {
        class: "label",
        at: text_at,
        align,
        size: 2,
        lines: &lines,
    }
    .write(&mut layer.svg);
    layer.svg.push_str("</g>\n");
    layer.bbox.union(&b.inflate(2.0));
    layer.bbox.include(Pt::new(x - 3.0, y - 3.0));
    layer.bbox.include(Pt::new(x + 3.0, y + 3.0));
}

/// Outline of a hierarchical port around its label, starting at the
/// connection point and pointing the way the signal flows: an `In` port
/// points at the connection, an `Out` port away from it, `BiDir` both ways.
/// Returns the path and the centre for the text.
fn port_shape(x: f64, y: f64, side: Side, text_w: f64, dir: &str) -> (String, Pt) {
    // Along: away from the connection point. Across: perpendicular to it.
    let (half, tip, body) = (11.0, 10.0, text_w + 12.0);
    let (tip_near, tip_far) = match dir.to_ascii_lowercase().as_str() {
        "in" => (tip, 0.0),
        "out" => (0.0, tip),
        "bidir" => (tip, tip),
        _ => (0.0, 0.0),
    };
    let len = tip_near + body + tip_far;
    let mut local = Vec::new();
    if tip_near > 0.0 {
        local.push((0.0, 0.0));
    }
    local.push((tip_near, -half));
    local.push((len - tip_far, -half));
    if tip_far > 0.0 {
        local.push((len, 0.0));
    }
    local.push((len - tip_far, half));
    local.push((tip_near, half));
    let (ox, oy) = match side {
        Side::Right => (1.0, 0.0),
        Side::Left => (-1.0, 0.0),
        Side::Above => (0.0, -1.0),
        Side::Below => (0.0, 1.0),
    };
    let map = |(u, v): (f64, f64)| Pt::new(x + u * ox - v * oy, y + u * oy + v * ox);
    let mut d = String::new();
    for (i, p) in local.iter().enumerate() {
        let q = map(*p);
        let _ = write!(
            d,
            "{}{} {}",
            if i == 0 { 'M' } else { 'L' },
            num(q.x),
            num(q.y)
        );
    }
    d.push('Z');
    (d, map((tip_near + body / 2.0, 0.0)))
}

fn draw_text(layer: &mut Layer, t: &Text) {
    let Some(align) = Align::parse(&t.align) else {
        return;
    };
    let class = match t.kind {
        TextKind::Directive => "directive",
        TextKind::Comment => "comment",
    };
    let lines = split_lines(&t.content);
    let b = TextBlock {
        class,
        at: Pt::from(t.at),
        align,
        size: t.size,
        lines: &lines,
    }
    .write(&mut layer.svg);
    layer.svg.push('\n');
    layer.bbox.union(&b);
}

/// Sheet drawings: `LINE Normal x1 y1 x2 y2 [style]` and friends. The
/// optional trailing number is LTspice's dash style.
fn draw_shape(layer: &mut Layer, s: &Shape) {
    let nums: Vec<i32> = s
        .tokens
        .iter()
        .skip(1)
        .map_while(|t| t.parse().ok())
        .collect();
    let p = |i: usize| Point::new(nums[i], nums[i + 1]);
    let (graphic, used) = match s.kind {
        ShapeKind::Line if nums.len() >= 4 => (Graphic::Line { a: p(0), b: p(2) }, 4),
        ShapeKind::Rectangle if nums.len() >= 4 => (Graphic::Rect { a: p(0), b: p(2) }, 4),
        ShapeKind::Circle if nums.len() >= 4 => (Graphic::Circle { a: p(0), b: p(2) }, 4),
        ShapeKind::Arc if nums.len() >= 8 => (
            Graphic::Arc {
                a: p(0),
                b: p(2),
                start: p(4),
                end: p(6),
            },
            8,
        ),
        _ => return,
    };
    let Some(prim) = geom::place(&graphic, Point::new(0, 0), Orient::R0) else {
        return;
    };
    let mut path = PathBuilder::default();
    path.push(&prim);
    let dash = match nums.get(used) {
        Some(1) => r#" stroke-dasharray="12 8""#,
        Some(2) => r#" stroke-dasharray="3 6""#,
        Some(3) => r#" stroke-dasharray="12 6 3 6""#,
        Some(4) => r#" stroke-dasharray="12 6 3 6 3 6""#,
        _ => "",
    };
    let _ = writeln!(
        layer.svg,
        r#"<path class="shape" d="{}"{dash}/>"#,
        path.finish()
    );
    layer.bbox.union(&prim.bbox().inflate(1.0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schematic::parse;

    fn render(src: &str, opts: &RenderOptions) -> RenderOutput {
        let (sch, warnings) = parse(src);
        assert!(warnings.is_empty(), "{warnings:?}");
        render_svg(&sch, &SymbolLibrary::builtin_only(), opts)
    }

    const HEAD: &str = "Version 4\nSHEET 1 880 680\n";

    #[test]
    fn instance_windows_override_defaults_by_index() {
        // The default res name window is at (36,40) Left. The override moves
        // it; the value keeps its default.
        let src = format!(
            "{HEAD}SYMBOL res 0 0 R0\nWINDOW 0 -40 40 Right 2\nSYMATTR InstName R7\nSYMATTR Value 4k7\n"
        );
        let out = render(&src, &RenderOptions::default());
        let name = out
            .svg
            .lines()
            .find(|l| l.contains("data-inst=\"R7\""))
            .unwrap();
        assert!(
            name.contains(r#"<text class="attr name" x="-40""#),
            "{name}"
        );
        assert!(name.contains(r#"text-anchor="end""#), "{name}");
        assert!(
            name.contains(r#"<text class="attr value" x="36""#),
            "{name}"
        );
    }

    #[test]
    fn invisible_windows_are_not_drawn() {
        let src = format!(
            "{HEAD}SYMBOL res 0 0 R0\nWINDOW 3 36 76 Invisible 2\nSYMATTR InstName R1\nSYMATTR Value 10k\n"
        );
        let out = render(&src, &RenderOptions::default());
        assert!(!out.svg.contains(">10k<"), "{}", out.svg);
        assert!(out.svg.contains(">R1<"));
    }

    #[test]
    fn size_zero_is_small_but_visible() {
        let src = format!(
            "{HEAD}SYMBOL voltage 0 0 R0\nWINDOW 3 24 96 Left 0\nSYMATTR InstName V1\nSYMATTR Value 5\n"
        );
        let out = render(&src, &RenderOptions::default());
        assert!(out.svg.contains(r#"font-size="8.33">5<"#), "{}", out.svg);
    }

    #[test]
    fn default_values_come_from_the_symbol() {
        let src = format!("{HEAD}SYMBOL OpAmps\\opamp2 0 0 R0\nSYMATTR InstName U1\n");
        let out = render(&src, &RenderOptions::default());
        assert!(out.svg.contains(">opamp2<"), "{}", out.svg);
    }

    #[test]
    fn missing_symbols_become_dashed_boxes_with_a_warning() {
        let src = format!("{HEAD}SYMBOL LT9999 64 64 R0\nSYMATTR InstName U9\n");
        let out = render(&src, &RenderOptions::default());
        assert_eq!(out.warnings.len(), 1);
        assert!(out.warnings[0].contains("LT9999"));
        assert!(out.svg.contains(r#"class="symbol missing" data-inst="U9""#));
        assert!(out.svg.contains(">LT9999<"));
    }

    #[test]
    fn highlights_add_classes() {
        let src = format!(
            "{HEAD}SYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMBOL cap 96 0 R0\nSYMATTR InstName C1\n"
        );
        let opts = RenderOptions {
            highlight: vec![
                Highlight {
                    inst: "r1".into(),
                    kind: HighlightKind::Added,
                },
                Highlight {
                    inst: "C1".into(),
                    kind: HighlightKind::Changed,
                },
                Highlight {
                    inst: "Q5".into(),
                    kind: HighlightKind::Removed,
                },
            ],
            ..RenderOptions::default()
        };
        let out = render(&src, &opts);
        assert!(
            out.svg
                .contains(r#"class="symbol hl-added" data-inst="R1""#)
        );
        assert!(
            out.svg
                .contains(r#"class="symbol hl-changed" data-inst="C1""#)
        );
        assert!(out.svg.contains(r#"class="hl-box""#));
        assert_eq!(
            out.warnings,
            vec!["highlighted part Q5 is not in the schematic"]
        );
    }

    #[test]
    fn wires_carry_their_coordinates() {
        let src = format!("{HEAD}WIRE 16 32 96 32\n");
        let out = render(&src, &RenderOptions::default());
        assert!(
            out.svg.contains(
                r#"<polyline class="wire" data-wire="16,32,96,32" points="16,32 96,32"/>"#
            )
        );
        assert_eq!(
            out.bounds,
            Rect::from_points(Point::new(16, 32), Point::new(96, 32))
        );
        assert!(out.svg.contains(r#"viewBox="-16 0 144 64""#), "{}", out.svg);
    }

    #[test]
    fn view_box_fits_content_plus_padding() {
        let src = format!("{HEAD}WIRE 0 0 160 0\nWIRE 0 0 0 96\n");
        let opts = RenderOptions {
            padding: 10,
            ..RenderOptions::default()
        };
        let out = render(&src, &opts);
        assert!(
            out.svg.contains(r#"viewBox="-10 -10 180 116""#),
            "{}",
            out.svg
        );
    }

    #[test]
    fn junction_dots_appear_at_t_junctions_only() {
        let src = format!(
            "{HEAD}WIRE 0 0 128 0\nWIRE 64 0 64 64\nWIRE 0 128 128 128\nWIRE 64 96 64 160\n"
        );
        let out = render(&src, &RenderOptions::default());
        assert_eq!(out.svg.matches(r#"class="junction""#).count(), 1);
        assert!(out.svg.contains(r#"cx="64" cy="0""#));
    }

    #[test]
    fn unconnected_pins_are_marked_on_request() {
        let src = format!("{HEAD}WIRE 16 16 16 -32\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\n");
        let plain = render(&src, &RenderOptions::default());
        assert!(!plain.svg.contains(r#"class="pin""#));
        let opts = RenderOptions {
            show_unconnected_pins: true,
            ..RenderOptions::default()
        };
        let marked = render(&src, &opts);
        // Pin A has a wire; pin B at (16,96) does not.
        assert_eq!(marked.svg.matches(r#"class="pin""#).count(), 1);
        assert!(marked.svg.contains(r#"x="13" y="93""#));
    }

    #[test]
    fn ground_and_labels() {
        let src = format!("{HEAD}WIRE 0 0 64 0\nFLAG 0 0 0\nFLAG 64 0 out\nIOPIN 64 0 Out\n");
        let out = render(&src, &RenderOptions::default());
        assert!(out.svg.contains(r#"class="flag ground""#));
        assert!(out.svg.contains(r#"class="flag port" data-net="out""#));
        assert!(out.svg.contains(">out</text>"));
    }

    #[test]
    fn label_text_goes_where_nothing_is_attached() {
        let s = |left, right, up, down| Sides {
            left,
            right,
            up,
            down,
        };
        assert_eq!(label_side(s(true, false, false, false)), Side::Right);
        assert_eq!(label_side(s(false, true, false, false)), Side::Left);
        assert_eq!(label_side(s(false, false, false, true)), Side::Above);
        assert_eq!(label_side(s(false, false, true, false)), Side::Below);
        assert_eq!(label_side(s(true, true, false, false)), Side::Above);
        assert_eq!(label_side(s(false, false, true, true)), Side::Right);
        assert_eq!(label_side(Sides::default()), Side::Above);
    }

    #[test]
    fn directives_and_comments_split_lines() {
        let src = format!("{HEAD}TEXT 0 0 Left 2 !.tran 1m\\n.op\nTEXT 0 64 Left 2 ;a note\n");
        let out = render(&src, &RenderOptions::default());
        assert!(out.svg.contains(r#"<text class="directive""#));
        assert!(out.svg.contains(">.tran 1m</tspan>"));
        assert!(out.svg.contains(">.op</tspan>"));
        assert!(out.svg.contains(r#"<text class="comment""#));
    }

    #[test]
    fn sheet_shapes_are_drawn() {
        let src = format!(
            "{HEAD}LINE Normal 0 0 64 0 2\nRECTANGLE Normal 0 0 64 32\nCIRCLE Normal 0 0 32 32\nARC Normal 0 0 32 32 32 16 0 16\n"
        );
        let out = render(&src, &RenderOptions::default());
        assert_eq!(out.svg.matches(r#"class="shape""#).count(), 4);
        assert!(out.svg.contains(r#"stroke-dasharray="3 6""#));
        assert!(out.svg.contains("M32 16A16 16 0 0 0 0 16"));
    }

    #[test]
    fn themed_output_reads_custom_properties() {
        let src = format!("{HEAD}WIRE 0 0 64 0\n");
        let themed = render(
            &src,
            &RenderOptions {
                standalone: false,
                ..RenderOptions::default()
            },
        );
        assert!(themed.svg.contains("var(--sch-wire, #2b6cb0)"));
        let standalone = render(&src, &RenderOptions::default());
        assert!(!standalone.svg.contains("var("));
    }

    #[test]
    fn options_deserialise_with_defaults() {
        let opts: RenderOptions =
            serde_json::from_str(r#"{"highlight":[{"inst":"R1","kind":"removed"}]}"#).unwrap();
        assert_eq!(opts.padding, 32);
        assert_eq!(opts.highlight[0].kind, HighlightKind::Removed);
    }
}
