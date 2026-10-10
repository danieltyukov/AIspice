//! The LTspice schematic (`.asc`) model.
//!
//! Parsing is lossless for anything LTspice writes: every line lands in a typed
//! item or, if unrecognised, in [`Item::Other`] verbatim, and writing an
//! unmodified schematic reproduces the file. Items keep their file order, and
//! new ones are inserted where LTspice itself would put them, so a diff of an
//! edit shows only the edit.

mod parse;
mod write;

pub use parse::{ParseWarning, parse, parse_bytes};
pub use write::{write, write_bytes};

use crate::encoding::Encoding;
use crate::geometry::{Orient, Point};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

/// How the file was stored on disk, so it can be written back the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFormat {
    pub encoding: Encoding,
    pub line_ending: LineEnding,
}

/// New files are written the way every LTspice version reads them: one byte
/// per character, CRLF, no byte order mark (LTspice XVII aborts on one).
impl Default for FileFormat {
    fn default() -> Self {
        Self {
            encoding: Encoding::Latin1,
            line_ending: LineEnding::CrLf,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schematic {
    /// The `Version` line's value, `4` or `4.1`.
    pub version: String,
    pub sheet: Sheet,
    pub items: Vec<Item>,
    #[serde(default)]
    pub format: FileFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sheet {
    pub number: i32,
    pub width: i32,
    pub height: i32,
}

impl Default for Sheet {
    fn default() -> Self {
        Self {
            number: 1,
            width: 880,
            height: 680,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Item {
    Wire(Wire),
    Flag(Flag),
    IoPin(IoPin),
    Symbol(Symbol),
    Text(Text),
    Shape(Shape),
    BusTap(BusTap),
    /// A line aispice does not interpret, kept exactly as read.
    Other {
        line: String,
    },
}

impl Item {
    /// Where LTspice groups this kind of item in a file. New items are inserted
    /// after the last existing item of the same rank.
    pub(crate) fn rank(&self) -> u8 {
        match self {
            Item::Wire(_) => 0,
            Item::Flag(_) => 1,
            Item::IoPin(_) => 1,
            Item::BusTap(_) => 2,
            Item::Symbol(_) => 3,
            Item::Text(_) => 4,
            Item::Shape(_) => 5,
            Item::Other { .. } => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Wire {
    pub a: Point,
    pub b: Point,
}

impl Wire {
    pub fn new(a: Point, b: Point) -> Self {
        Self { a, b }
    }

    /// Same segment regardless of direction.
    pub fn same_as(&self, other: &Wire) -> bool {
        (self.a == other.a && self.b == other.b) || (self.a == other.b && self.b == other.a)
    }
}

/// A net label. The label `0` is ground.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Flag {
    pub at: Point,
    pub label: String,
}

impl Flag {
    pub fn is_ground(&self) -> bool {
        self.label == "0"
    }
}

/// Marks the flag at the same position as a port of a hierarchical block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IoPin {
    pub at: Point,
    /// `In`, `Out` or `BiDir`.
    pub direction: String,
}

/// One placed component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    /// Library name as written in the file, e.g. `res` or `OpAmps\opamp2`.
    pub name: String,
    pub at: Point,
    pub orient: Orient,
    pub windows: Vec<Window>,
    pub attrs: Vec<Attr>,
    /// Lines inside the block that are neither WINDOW nor SYMATTR.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<String>,
}

impl Symbol {
    pub fn new(name: impl Into<String>, at: Point, orient: Orient) -> Self {
        Self {
            name: name.into(),
            at,
            orient,
            windows: Vec::new(),
            attrs: Vec::new(),
            extra: Vec::new(),
        }
    }

    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.key.eq_ignore_ascii_case(key))
            .map(|a| a.value.as_str())
    }

    /// Set an attribute, replacing it in place if present so the file keeps
    /// its order, otherwise appending it. An empty value removes it.
    pub fn set_attr(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        if value.is_empty() {
            self.attrs.retain(|a| !a.key.eq_ignore_ascii_case(key));
            return;
        }
        match self
            .attrs
            .iter_mut()
            .find(|a| a.key.eq_ignore_ascii_case(key))
        {
            Some(a) => a.value = value,
            None => self.attrs.push(Attr {
                key: key.to_string(),
                value,
            }),
        }
    }

    pub fn inst_name(&self) -> Option<&str> {
        self.attr("InstName")
    }

    pub fn value(&self) -> Option<&str> {
        self.attr("Value")
    }

    /// The library name with `\` normalised to `/`, lower-cased, which is how
    /// symbol lookups compare names.
    pub fn lookup_name(&self) -> String {
        normalize_symbol_name(&self.name)
    }
}

/// Lower-cased, `/`-separated, with repeated separators collapsed: LTspice
/// writes `OpAmps\\opamp2` with a doubled backslash, and some files use one.
pub fn normalize_symbol_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        let c = if c == '\\' {
            '/'
        } else {
            c.to_ascii_lowercase()
        };
        if c == '/' && out.ends_with('/') {
            continue;
        }
        out.push(c);
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attr {
    pub key: String,
    pub value: String,
}

/// Where one attribute's text is drawn, relative to the symbol origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    /// 0 InstName, 3 Value, 123 Value2, 38 SpiceModel, 39 SpiceLine, 40 SpiceLine2.
    pub index: i32,
    pub at: Point,
    /// `Left`, `Right`, `Center`, `Top`, `Bottom`, the `V` variants, or `Invisible`.
    pub align: String,
    pub size: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextKind {
    /// `!`: a SPICE directive that goes into the netlist.
    Directive,
    /// `;`: a comment shown on the sheet.
    Comment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    pub at: Point,
    pub align: String,
    pub size: i32,
    pub kind: TextKind,
    /// The text without its `!` or `;`. LTspice stores line breaks as the two
    /// characters `\n`; they are kept that way.
    pub content: String,
}

impl Text {
    pub fn directive(at: Point, content: impl Into<String>) -> Self {
        Self {
            at,
            align: "Left".into(),
            size: 2,
            kind: TextKind::Directive,
            content: content.into(),
        }
    }

    pub fn comment(at: Point, content: impl Into<String>) -> Self {
        Self {
            at,
            align: "Left".into(),
            size: 2,
            kind: TextKind::Comment,
            content: content.into(),
        }
    }

    /// The text as separate lines, decoding LTspice's escapes: `\n` is a line
    /// break and `\\` a literal backslash, so a path such as
    /// `C:\\temp\\new.lib` is not split at its `\n`. Blank lines are dropped.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut chars = self.content.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.peek() {
                    Some('n') => {
                        chars.next();
                        out.push(std::mem::take(&mut cur));
                        continue;
                    }
                    Some('\\') => {
                        chars.next();
                        cur.push('\\');
                        continue;
                    }
                    _ => {}
                }
            }
            cur.push(c);
        }
        out.push(cur);
        out.into_iter()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    Line,
    Rectangle,
    Circle,
    Arc,
}

/// A drawing primitive on the sheet: `LINE Normal x1 y1 x2 y2 [style]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shape {
    pub kind: ShapeKind,
    /// Everything after the keyword, kept as tokens.
    pub tokens: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusTap {
    pub a: Point,
    pub b: Point,
}

impl Default for Schematic {
    fn default() -> Self {
        Self::new()
    }
}

impl Schematic {
    pub fn new() -> Self {
        Self {
            version: "4".into(),
            sheet: Sheet::default(),
            items: Vec::new(),
            format: FileFormat::default(),
        }
    }

    pub fn wires(&self) -> impl Iterator<Item = &Wire> {
        self.items.iter().filter_map(|i| match i {
            Item::Wire(w) => Some(w),
            _ => None,
        })
    }

    pub fn flags(&self) -> impl Iterator<Item = &Flag> {
        self.items.iter().filter_map(|i| match i {
            Item::Flag(f) => Some(f),
            _ => None,
        })
    }

    pub fn io_pins(&self) -> impl Iterator<Item = &IoPin> {
        self.items.iter().filter_map(|i| match i {
            Item::IoPin(p) => Some(p),
            _ => None,
        })
    }

    pub fn symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.items.iter().filter_map(|i| match i {
            Item::Symbol(s) => Some(s),
            _ => None,
        })
    }

    pub fn symbols_mut(&mut self) -> impl Iterator<Item = &mut Symbol> {
        self.items.iter_mut().filter_map(|i| match i {
            Item::Symbol(s) => Some(s),
            _ => None,
        })
    }

    pub fn texts(&self) -> impl Iterator<Item = &Text> {
        self.items.iter().filter_map(|i| match i {
            Item::Text(t) => Some(t),
            _ => None,
        })
    }

    pub fn directives(&self) -> impl Iterator<Item = &Text> {
        self.texts().filter(|t| t.kind == TextKind::Directive)
    }

    /// Find a component by instance name, case-insensitively as SPICE does.
    pub fn symbol(&self, inst_name: &str) -> Option<&Symbol> {
        self.symbols().find(|s| {
            s.inst_name()
                .is_some_and(|n| n.eq_ignore_ascii_case(inst_name))
        })
    }

    pub fn symbol_mut(&mut self, inst_name: &str) -> Option<&mut Symbol> {
        self.symbols_mut().find(|s| {
            s.inst_name()
                .is_some_and(|n| n.eq_ignore_ascii_case(inst_name))
        })
    }

    /// Index of the component in `items`.
    pub fn symbol_index(&self, inst_name: &str) -> Option<usize> {
        self.items.iter().position(|i| match i {
            Item::Symbol(s) => s
                .inst_name()
                .is_some_and(|n| n.eq_ignore_ascii_case(inst_name)),
            _ => false,
        })
    }

    /// Insert an item after the last one of the same kind, or where LTspice
    /// would place its kind if there is none yet.
    pub fn insert(&mut self, item: Item) {
        let rank = item.rank();
        let pos = self
            .items
            .iter()
            .rposition(|i| i.rank() <= rank)
            .map(|p| p + 1)
            .unwrap_or(0);
        self.items.insert(pos, item);
    }

    /// Instance names in use, upper-cased.
    pub fn inst_names(&self) -> Vec<String> {
        self.symbols()
            .filter_map(|s| s.inst_name())
            .map(|n| n.to_ascii_uppercase())
            .collect()
    }

    /// The first unused instance name with this prefix: `R1`, `R2`, ...
    pub fn next_inst_name(&self, prefix: &str) -> String {
        let used = self.inst_names();
        let prefix_up = prefix.to_ascii_uppercase();
        (1..)
            .map(|n| format!("{prefix_up}{n}"))
            .find(|name| !used.contains(name))
            .expect("unbounded range")
    }
}
