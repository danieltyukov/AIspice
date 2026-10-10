//! Text: LTspice justifications, upright text on rotated and mirrored parts,
//! font sizes, line breaks and a width estimate for bounds.
//!
//! A WINDOW's justification is given in the symbol's own frame and turns with
//! the part, like its position. `VBottom` text is vertical in the symbol's
//! frame, so on a resistor turned R90 it ends up horizontal, sitting above its
//! anchor. LTspice never draws text upside down or reading downwards: when the
//! turned text would read right to left or top to bottom it is turned a further
//! half turn, and the justification is flipped so the text still covers the
//! same area. Mirroring flips it the same way, which is why `Left` text on an
//! `M0` part grows to the left.

use super::geom::{BBox, Pt};
use super::svg::{esc, num};
use crate::geometry::{Orient, Point};
use std::fmt::Write;

/// Along the reading direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HAlign {
    Start,
    Middle,
    End,
}

/// Across the reading direction, relative to the glyphs' own up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VAlign {
    Top,
    Middle,
    Bottom,
}

impl HAlign {
    fn flipped(self) -> Self {
        match self {
            HAlign::Start => HAlign::End,
            HAlign::Middle => HAlign::Middle,
            HAlign::End => HAlign::Start,
        }
    }

    fn svg(self) -> &'static str {
        match self {
            HAlign::Start => "start",
            HAlign::Middle => "middle",
            HAlign::End => "end",
        }
    }
}

impl VAlign {
    fn flipped(self) -> Self {
        match self {
            VAlign::Top => VAlign::Bottom,
            VAlign::Middle => VAlign::Middle,
            VAlign::Bottom => VAlign::Top,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Align {
    /// Reads bottom to top.
    pub vertical: bool,
    pub h: HAlign,
    pub v: VAlign,
}

const RIGHT: Point = Point::new(1, 0);
const UP: Point = Point::new(0, -1);
const LEFT: Point = Point::new(-1, 0);

impl Align {
    pub const fn horizontal(h: HAlign, v: VAlign) -> Align {
        Align {
            vertical: false,
            h,
            v,
        }
    }

    /// Parse an LTspice justification. `Invisible` gives `None`; anything
    /// unknown reads as `Left`, which is what LTspice falls back to.
    pub fn parse(s: &str) -> Option<Align> {
        let lower = s.trim().to_ascii_lowercase();
        if lower == "invisible" {
            return None;
        }
        let (vertical, rest) = match lower.strip_prefix('v') {
            Some(r) if !r.is_empty() => (true, r),
            _ => (false, lower.as_str()),
        };
        let (h, v) = match rest {
            "left" => (HAlign::Start, VAlign::Middle),
            "right" => (HAlign::End, VAlign::Middle),
            "center" | "centre" => (HAlign::Middle, VAlign::Middle),
            "top" => (HAlign::Middle, VAlign::Top),
            "bottom" => (HAlign::Middle, VAlign::Bottom),
            _ => (HAlign::Start, VAlign::Middle),
        };
        Some(Align { vertical, h, v })
    }

    /// Reading direction and glyph-up direction in the text's own frame.
    fn frame(self) -> (Point, Point) {
        if self.vertical {
            (UP, LEFT)
        } else {
            (RIGHT, UP)
        }
    }

    /// The justification after placing the text with `orient`, kept upright.
    pub fn oriented(self, orient: Orient) -> Align {
        let (d, u) = self.frame();
        let (d1, u1) = (orient.apply(d), orient.apply(u));
        let readable = d1 == RIGHT || d1 == UP;
        let dr = if readable {
            d1
        } else {
            Point::new(-d1.x, -d1.y)
        };
        // Glyph up for upright text: up for horizontal, left for vertical.
        let ur = if dr == RIGHT { UP } else { LEFT };
        Align {
            vertical: dr == UP,
            h: if readable { self.h } else { self.h.flipped() },
            v: if ur == u1 { self.v } else { self.v.flipped() },
        }
    }

    /// Unit offset direction for a pin label at distance `offset` from its
    /// pin: away from the pin along whichever axes the justification anchors.
    pub fn offset_dir(self) -> (i32, i32) {
        let (d, u) = self.frame();
        let along = match self.h {
            HAlign::Start => 1,
            HAlign::Middle => 0,
            HAlign::End => -1,
        };
        let across = match self.v {
            VAlign::Top => -1,
            VAlign::Middle => 0,
            VAlign::Bottom => 1,
        };
        (along * d.x + across * u.x, along * d.y + across * u.y)
    }
}

/// Font size in sheet units for an LTspice size index. Index 2 (LTspice's
/// "1.5x", the default) is 20 units, a little over one grid step, which keeps
/// labels readable without crowding a resistor 80 units long.
pub(crate) fn font_size(size: i32) -> f64 {
    const SCALE: [f64; 8] = [0.625, 1.0, 1.5, 2.0, 2.5, 3.5, 5.0, 7.0];
    const BASE: f64 = 40.0 / 3.0;
    BASE * SCALE[size.clamp(0, 7) as usize]
}

pub(crate) const LINE_HEIGHT: f64 = 1.2;

/// Split stored text into display lines. LTspice stores a line break as the
/// two characters `\n` and a literal backslash as `\\`.
pub(crate) fn split_lines(content: &str) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('n') => {
                    chars.next();
                    lines.push(String::new());
                    continue;
                }
                Some('\\') => {
                    chars.next();
                }
                _ => {}
            }
        }
        lines.last_mut().expect("never empty").push(c);
    }
    lines
}

/// Approximate advance width of `s`, for bounds: a rough per-character
/// table tuned to Arial-like faces. It only has to keep text inside the view
/// box, not measure it exactly.
pub(crate) fn text_width(s: &str, fs: f64) -> f64 {
    s.chars()
        .map(|c| match c {
            'i' | 'j' | 'l' | '.' | ',' | ':' | ';' | '!' | '|' | '\'' => 0.25,
            'f' | 't' | 'r' | 'I' | '(' | ')' | '[' | ']' | ' ' | '-' => 0.36,
            'm' | 'w' | 'M' | 'W' => 0.85,
            'A'..='Z' => 0.68,
            _ => 0.56,
        })
        .sum::<f64>()
        * fs
}

/// One block of text, already justified for the sheet.
pub(crate) struct TextBlock<'a> {
    pub class: &'a str,
    pub at: Pt,
    pub align: Align,
    pub size: i32,
    pub lines: &'a [String],
}

impl TextBlock<'_> {
    /// Write the `<text>` element and return its approximate bounds. The
    /// baseline is placed by hand rather than with `dominant-baseline`, which
    /// renderers disagree on.
    pub fn write(&self, out: &mut String) -> BBox {
        let fs = font_size(self.size);
        let lh = fs * LINE_HEIGHT;
        let n = self.lines.len().max(1) as f64;
        let height = n * lh;
        let top = match self.align.v {
            VAlign::Top => 0.0,
            VAlign::Middle => -height / 2.0,
            VAlign::Bottom => -height,
        };
        // Centre the glyphs (cap height about 0.7 em) in each line box.
        let baseline = |i: usize| top + i as f64 * lh + lh / 2.0 + 0.35 * fs;
        let (ax, ay) = (self.at.x, self.at.y);
        let anchor = self.align.h.svg();
        let _ = write!(
            out,
            r#"<text class="{}" x="{}" y="{}" font-size="{}""#,
            self.class,
            num(ax),
            num(ay + baseline(0)),
            num(fs)
        );
        if anchor != "start" {
            let _ = write!(out, r#" text-anchor="{anchor}""#);
        }
        if self.align.vertical {
            let _ = write!(out, r#" transform="rotate(-90 {} {})""#, num(ax), num(ay));
        }
        out.push('>');
        if self.lines.len() == 1 {
            out.push_str(&esc(&self.lines[0]));
        } else {
            for (i, line) in self.lines.iter().enumerate() {
                let _ = write!(
                    out,
                    r#"<tspan x="{}" y="{}">{}</tspan>"#,
                    num(ax),
                    num(ay + baseline(i)),
                    esc(line)
                );
            }
        }
        out.push_str("</text>");

        let width = self
            .lines
            .iter()
            .map(|l| text_width(l, fs))
            .fold(0.0, f64::max);
        let x0 = match self.align.h {
            HAlign::Start => 0.0,
            HAlign::Middle => -width / 2.0,
            HAlign::End => -width,
        };
        let mut b = BBox::EMPTY;
        for (lx, ly) in [(x0, top), (x0 + width, top + height)] {
            // Local frame to sheet: identity, or a quarter turn
            // counter-clockwise on screen for vertical text.
            let p = if self.align.vertical {
                Pt::new(ax + ly, ay - lx)
            } else {
                Pt::new(ax + lx, ay + ly)
            };
            b.include(p);
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Align {
        Align::parse(s).unwrap()
    }

    #[test]
    fn parses_every_ltspice_justification() {
        assert_eq!(Align::parse("Invisible"), None);
        assert_eq!(a("Left"), Align::horizontal(HAlign::Start, VAlign::Middle));
        assert!(a("VTop").vertical);
        assert_eq!(a("VTop").v, VAlign::Top);
        assert_eq!(a("Center").h, HAlign::Middle);
        assert!(a("VCenter").vertical);
        assert_eq!(a("bogus"), a("Left"));
    }

    #[test]
    fn upright_parts_keep_their_justification() {
        for s in ["Left", "Right", "Top", "Bottom", "VLeft", "VTop", "VBottom"] {
            assert_eq!(a(s).oriented(Orient::R0), a(s), "{s}");
        }
    }

    #[test]
    fn rotated_resistor_windows_become_horizontal() {
        // What LTspice writes for a resistor turned R90: the name above the
        // part and the value below it, both horizontal.
        let name = a("VBottom").oriented(Orient::R90);
        assert_eq!(name, a("Bottom"));
        let value = a("VTop").oriented(Orient::R90);
        assert_eq!(value, a("Top"));
        // R270 swaps which window is on top, so the file swaps them too.
        assert_eq!(a("VTop").oriented(Orient::R270), a("Bottom"));
        assert_eq!(a("VBottom").oriented(Orient::R270), a("Top"));
    }

    #[test]
    fn mirrored_rotations_match_what_ltspice_writes() {
        // LTspice writes the R90 windows for M90 and the R270 ones for M270;
        // both must still put the name above.
        assert_eq!(a("VBottom").oriented(Orient::M90), a("Bottom"));
        assert_eq!(a("VTop").oriented(Orient::M270), a("Bottom"));
    }

    #[test]
    fn horizontal_text_on_a_quarter_turn_reads_upwards() {
        // Default `Left` windows on an R90 part would read downwards; they are
        // drawn reading upwards instead, ending at the anchor.
        let t = a("Left").oriented(Orient::R90);
        assert!(t.vertical);
        assert_eq!(t.h, HAlign::End);
        let t = a("Left").oriented(Orient::R270);
        assert!(t.vertical);
        assert_eq!(t.h, HAlign::Start);
    }

    #[test]
    fn mirroring_flips_left_and_right() {
        assert_eq!(a("Left").oriented(Orient::M0), a("Right"));
        assert_eq!(a("Right").oriented(Orient::M0), a("Left"));
        assert_eq!(a("Left").oriented(Orient::R180), a("Right"));
        assert_eq!(a("Top").oriented(Orient::R180), a("Bottom"));
        // M180 is a vertical flip: left stays left.
        assert_eq!(a("Left").oriented(Orient::M180), a("Left"));
        assert_eq!(a("Top").oriented(Orient::M180), a("Bottom"));
    }

    #[test]
    fn pin_label_offsets_point_away_from_the_pin() {
        assert_eq!(a("Left").offset_dir(), (1, 0));
        assert_eq!(a("Right").offset_dir(), (-1, 0));
        assert_eq!(a("Top").offset_dir(), (0, 1));
        assert_eq!(a("Bottom").offset_dir(), (0, -1));
        assert_eq!(a("VLeft").offset_dir(), (0, -1));
        assert_eq!(a("VTop").offset_dir(), (1, 0));
    }

    #[test]
    fn size_two_is_the_normal_size() {
        assert_eq!(font_size(2), 20.0);
        assert_eq!(font_size(0), 40.0 / 3.0 * 0.625);
        assert_eq!(font_size(99), font_size(7));
    }

    #[test]
    fn line_breaks_and_backslashes() {
        assert_eq!(split_lines(".tran 1m\\n.op"), vec![".tran 1m", ".op"]);
        assert_eq!(split_lines("a\\\\nb"), vec!["a\\nb"]);
        assert_eq!(split_lines("  indented"), vec!["  indented"]);
        assert_eq!(split_lines(""), vec![""]);
    }

    #[test]
    fn vertical_text_bounds_are_turned() {
        let lines = vec!["R1".to_string()];
        let block = TextBlock {
            class: "attr",
            at: Pt::new(0.0, 0.0),
            align: Align {
                vertical: true,
                h: HAlign::Start,
                v: VAlign::Middle,
            },
            size: 2,
            lines: &lines,
        };
        let mut s = String::new();
        let b = block.write(&mut s);
        // Reads upwards from the anchor: all of it above, centred across.
        assert!(b.max.y <= 0.0 && b.min.y < -10.0, "{b:?}");
        assert!((b.min.x + b.max.x).abs() < 1e-9, "{b:?}");
        assert!(s.contains(r#"transform="rotate(-90 0 0)""#), "{s}");
    }
}
