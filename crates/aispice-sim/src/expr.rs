//! A small expression language over simulation vectors.
//!
//! Measurements, specs and plots all name what they look at with the same
//! syntax, so an agent writes `V(out)/V(in)` once and it means the same thing
//! in a spec, a plot and a measurement. The syntax is what SPICE users already
//! type in a plot window or a `.meas` line: `V(out)`, `V(a,b)`, `I(R1)`, bare
//! node names, numbers with SPICE suffixes, `+ - * /`, parentheses, and a
//! handful of functions. Anything bigger (user functions, conditionals)
//! belongs in the simulator's own `.meas` or in a behavioural source.
//!
//! Evaluation is per point and per step: an expression over AC data yields
//! complex values, over transient data real ones. `d()` and `integ()` work
//! against the dataset's independent variable, so `d(V(out))` is a slew rate
//! in a transient and a group-delay ingredient in an AC sweep.

use crate::dataset::{AnalysisKind, Complex, Dataset, Quantity};
use aispice_core::units;
use std::fmt;
use std::ops::Range;

/// A parsed expression. Build one with [`parse`] or `str::parse`.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    /// A vector by name, as written: `V(out)`, `I(R1)`, `out`, `time`.
    Vector(String),
    /// `V(a,b)`: the voltage of node `a` relative to node `b`.
    Diff(String, String),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Call(Func, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    /// Magnitude: `|x|`.
    Mag,
    /// `20 log10 |x|`.
    Db,
    /// Phase in degrees, unwrapped along the axis so a three-pole roll-off
    /// reads -270 instead of jumping back to +90.
    Ph,
    Re,
    Im,
    Abs,
    Sqrt,
    Log10,
    /// Derivative with respect to the independent variable.
    D,
    /// Running integral over the independent variable, starting at zero.
    Integ,
}

impl Func {
    pub fn name(self) -> &'static str {
        match self {
            Func::Mag => "mag",
            Func::Db => "db",
            Func::Ph => "ph",
            Func::Re => "re",
            Func::Im => "im",
            Func::Abs => "abs",
            Func::Sqrt => "sqrt",
            Func::Log10 => "log10",
            Func::D => "d",
            Func::Integ => "integ",
        }
    }

    fn lookup(name: &str) -> Option<Func> {
        Some(match name.to_ascii_lowercase().as_str() {
            "mag" => Func::Mag,
            "db" => Func::Db,
            "ph" | "phase" => Func::Ph,
            "re" | "real" => Func::Re,
            "im" | "imag" => Func::Im,
            "abs" => Func::Abs,
            "sqrt" => Func::Sqrt,
            "log10" => Func::Log10,
            "d" | "deriv" => Func::D,
            "integ" => Func::Integ,
            _ => return None,
        })
    }
}

const FUNCTION_LIST: &str = "mag, db, ph, re, im, abs, sqrt, log10, d, integ";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ExprError {
    #[error("{message} at column {column} of `{text}`")]
    Syntax {
        text: String,
        column: usize,
        message: String,
    },
    #[error("unknown function `{0}`; known functions: {FUNCTION_LIST}")]
    UnknownFunction(String),
    #[error("{}", unknown_vector_message(.name, .suggestions, .available))]
    UnknownVector {
        name: String,
        /// Names in the dataset that look like what was meant.
        suggestions: Vec<String>,
        /// Every vector name, for when nothing looks close.
        available: Vec<String>,
    },
    #[error("{0}() needs an independent variable, and an operating point has none")]
    NoAxis(&'static str),
    #[error("points {start}..{end} are outside the dataset, which has {len}")]
    BadRange {
        start: usize,
        end: usize,
        len: usize,
    },
}

fn unknown_vector_message(name: &str, suggestions: &[String], available: &[String]) -> String {
    if !suggestions.is_empty() {
        return format!(
            "unknown vector `{name}`; did you mean {}?",
            suggestions.join(" or ")
        );
    }
    const SHOWN: usize = 16;
    let mut list = available
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if available.len() > SHOWN {
        list.push_str(&format!(", and {} more", available.len() - SHOWN));
    }
    if list.is_empty() {
        format!("unknown vector `{name}`; the dataset has no vectors")
    } else {
        format!("unknown vector `{name}`; available: {list}")
    }
}

/// Parse a number the way a SPICE user writes it (`4.7k`, `1Meg`, `10ns`).
///
/// One deliberate departure from SPICE: `MHz` means megahertz. SPICE reads
/// `M` as milli, which is right for `1M` on a resistor line, but nobody who
/// types `1MHz` into a spec means a millihertz, and getting it wrong silently
/// is worse than being inconsistent with a rule nobody applies to hertz.
pub fn parse_number(text: &str) -> Option<f64> {
    let t = text.trim();
    let v = parse_spice(t)?;
    if t.ends_with("MHz") && !t.to_ascii_lowercase().contains("meg") {
        return Some(v * 1e9);
    }
    Some(v)
}

/// A SPICE number with exactly SPICE's meaning, rounded correctly.
///
/// [`units::parse`] multiplies the mantissa by the scale, so `47n` comes out
/// one bit away from the literal `47e-9`. That is harmless in a simulator,
/// but it breaks exact round trips (a value written into a netlist and read
/// back must be the same number), so the decimal is rebuilt as `47e-9` and
/// handed to the float parser instead. `units::parse` still decides what is
/// a number; this only sharpens the result.
pub fn parse_spice(text: &str) -> Option<f64> {
    let t = text.trim();
    let v = units::parse(t)?;
    Some(
        exact_decimal(t)
            .filter(|e| (e - v).abs() <= 1e-9 * v.abs())
            .unwrap_or(v),
    )
}

fn exact_decimal(t: &str) -> Option<f64> {
    let bytes = t.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        end = 1;
    }
    while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
        end += 1;
    }
    let mantissa = &t[..end];
    let mut exp: i32 = 0;
    if end < bytes.len() && matches!(bytes[end], b'e' | b'E') {
        let mut k = end + 1;
        if k < bytes.len() && matches!(bytes[k], b'+' | b'-') {
            k += 1;
        }
        let digits = k;
        while k < bytes.len() && bytes[k].is_ascii_digit() {
            k += 1;
        }
        if k > digits {
            exp = t[end + 1..k].parse().ok()?;
            end = k;
        }
    }
    let rest = &t[end..];
    let lower = rest.to_ascii_lowercase();
    let (scale, used) = if rest.is_empty() {
        (0, 0)
    } else if lower.starts_with("meg") {
        (6, 3)
    } else if lower.starts_with("mil") {
        return None;
    } else {
        let c = rest.chars().next()?;
        let s = match c.to_ascii_lowercase() {
            't' => 12,
            'g' => 9,
            'k' => 3,
            'm' => -3,
            'u' | 'µ' | 'μ' => -6,
            'n' => -9,
            'p' => -12,
            'f' => -15,
            'a' => -18,
            c if c.is_alphabetic() => 0,
            _ => return None,
        };
        (s, if s == 0 { 0 } else { c.len_utf8() })
    };
    // RKM values (4k7) keep units::parse's answer.
    if rest[used..].chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    format!("{mantissa}e{}", exp + scale).parse().ok()
}

/// Format a number so that [`parse_number`] reads back exactly the same
/// value: the engineering form (`4.7k`) when it is exact, otherwise more
/// digits with the same suffix, otherwise scientific notation.
pub fn format_number(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    let short = units::format(v);
    if parse_spice(&short) == Some(v) {
        return short;
    }
    const SCALES: [(f64, &str); 9] = [
        (1e12, "T"),
        (1e9, "G"),
        (1e6, "Meg"),
        (1e3, "k"),
        (1.0, ""),
        (1e-3, "m"),
        (1e-6, "u"),
        (1e-9, "n"),
        (1e-12, "p"),
    ];
    let mag = v.abs();
    if let Some((scale, suffix)) = SCALES.iter().copied().find(|(s, _)| mag >= *s) {
        let text = format!("{}{suffix}", v / scale);
        if parse_spice(&text) == Some(v) {
            return text;
        }
    }
    format!("{v:e}")
}

/// Parse an expression such as `V(out)/V(in)` or `d(V(out))`.
pub fn parse(text: &str) -> Result<Expr, ExprError> {
    let mut p = Parser {
        src: text,
        pos: 0,
        depth: 0,
    };
    p.skip_ws();
    if p.at_end() {
        return Err(p.error("empty expression"));
    }
    let e = p.additive()?;
    p.skip_ws();
    if let Some(c) = p.peek() {
        return Err(p.error(format!("unexpected `{c}`")));
    }
    Ok(e)
}

impl std::str::FromStr for Expr {
    type Err = ExprError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse(s)
    }
}

/// Parentheses nest at most this deep. Real expressions nest three or four
/// levels; the cap keeps hostile input from exhausting the stack.
const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    src: &'a str,
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn error(&self, message: impl Into<String>) -> ExprError {
        ExprError::Syntax {
            text: self.src.to_string(),
            column: self.src[..self.pos].chars().count() + 1,
            message: message.into(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn peek2(&self) -> Option<char> {
        self.src[self.pos..].chars().nth(1)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn expect(&mut self, want: char) -> Result<(), ExprError> {
        self.skip_ws();
        match self.peek() {
            Some(c) if c == want => {
                self.bump();
                Ok(())
            }
            Some(c) => Err(self.error(format!("expected `{want}`, found `{c}`"))),
            None => Err(self.error(format!("expected `{want}` before the end"))),
        }
    }

    fn enter(&mut self) -> Result<(), ExprError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error(format!("nested deeper than {MAX_DEPTH} levels")));
        }
        Ok(())
    }

    fn additive(&mut self) -> Result<Expr, ExprError> {
        let mut left = self.term()?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some('+') => BinOp::Add,
                Some('-') => BinOp::Sub,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.term()?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
    }

    fn term(&mut self) -> Result<Expr, ExprError> {
        let mut left = self.unary()?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some('*') => BinOp::Mul,
                Some('/') => BinOp::Div,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.unary()?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
    }

    fn unary(&mut self) -> Result<Expr, ExprError> {
        self.skip_ws();
        match self.peek() {
            Some('-') => {
                self.bump();
                self.enter()?;
                let inner = self.unary()?;
                self.depth -= 1;
                Ok(Expr::Neg(Box::new(inner)))
            }
            Some('+') => {
                self.bump();
                self.enter()?;
                let inner = self.unary()?;
                self.depth -= 1;
                Ok(inner)
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<Expr, ExprError> {
        self.skip_ws();
        let Some(c) = self.peek() else {
            return Err(self.error("expected a value before the end"));
        };
        if c == '(' {
            self.bump();
            self.enter()?;
            let e = self.additive()?;
            self.expect(')')?;
            self.depth -= 1;
            return Ok(e);
        }
        if c.is_ascii_digit() || (c == '.' && self.peek2().is_some_and(|d| d.is_ascii_digit())) {
            return self.number();
        }
        if c.is_alphabetic() || c == '_' || c == '@' {
            let ident = self.ident();
            let save = self.pos;
            self.skip_ws();
            if self.peek() != Some('(') {
                self.pos = save;
                return Ok(Expr::Vector(ident));
            }
            if let Some(f) = Func::lookup(&ident) {
                self.bump();
                self.enter()?;
                let arg = self.additive()?;
                self.expect(')')?;
                self.depth -= 1;
                return Ok(Expr::Call(f, Box::new(arg)));
            }
            if let Some(kind) = ref_kind(&ident) {
                let args = self.raw_args()?;
                return self.build_ref(kind, &ident, args);
            }
            return Err(ExprError::UnknownFunction(ident));
        }
        Err(self.error(format!("unexpected `{c}`")))
    }

    fn ident(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(|c| {
            c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '#' | '$' | '@' | '[' | ']')
        }) {
            self.bump();
        }
        self.src[start..self.pos].to_string()
    }

    fn number(&mut self) -> Result<Expr, ExprError> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
            self.bump();
        }
        // An exponent only when digits follow, so `1meg` stays a suffix.
        if matches!(self.peek(), Some('e' | 'E')) {
            let save = self.pos;
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.bump();
                }
            } else {
                self.pos = save;
            }
        }
        // Suffix and unit letters, and the digits of an RKM value like 4k7.
        while self.peek().is_some_and(char::is_alphanumeric) {
            self.bump();
        }
        let text = &self.src[start..self.pos];
        match parse_number(text) {
            Some(v) => Ok(Expr::Num(v)),
            None => {
                self.pos = start;
                Err(self.error(format!("`{text}` is not a number")))
            }
        }
    }

    /// The text between the parentheses of `V(...)`, split on top-level
    /// commas. Node names are taken as written (`in+`, `u1:out`, `2`), which
    /// is why this does not go through the expression grammar.
    fn raw_args(&mut self) -> Result<Vec<String>, ExprError> {
        let open = self.pos;
        self.bump(); // '('
        let mut depth = 0usize;
        let mut args = Vec::new();
        let mut cur = String::new();
        loop {
            let Some(c) = self.bump() else {
                self.pos = open;
                return Err(self.error("unclosed `(`"));
            };
            match c {
                '(' => {
                    depth += 1;
                    cur.push(c);
                }
                ')' if depth == 0 => break,
                ')' => {
                    depth -= 1;
                    cur.push(c);
                }
                ',' if depth == 0 => args.push(std::mem::take(&mut cur)),
                c => cur.push(c),
            }
        }
        args.push(cur);
        let args: Vec<String> = args.into_iter().map(|a| a.trim().to_string()).collect();
        if args.iter().any(String::is_empty) {
            self.pos = open;
            return Err(self.error("empty name inside parentheses"));
        }
        Ok(args)
    }

    fn build_ref(&self, kind: RefKind, ident: &str, args: Vec<String>) -> Result<Expr, ExprError> {
        match kind {
            RefKind::Voltage(wrap) => {
                let base = match args.as_slice() {
                    [a] => Expr::Vector(format!("V({a})")),
                    [a, b] => Expr::Diff(a.clone(), b.clone()),
                    _ => return Err(self.error(format!("{ident}() takes one or two node names"))),
                };
                Ok(match wrap {
                    Some(f) => Expr::Call(f, Box::new(base)),
                    None => base,
                })
            }
            RefKind::Current => match args.as_slice() {
                [a] => {
                    let mut head = ident.to_ascii_lowercase();
                    head.replace_range(0..1, "I");
                    Ok(Expr::Vector(format!("{head}({a})")))
                }
                _ => Err(self.error(format!("{ident}() takes one device name"))),
            },
        }
    }
}

#[derive(Clone, Copy)]
enum RefKind {
    /// `V(...)`, or one of the ngspice shorthands `vdb`, `vm`, `vp`, `vr`,
    /// `vi` that wrap it in a function.
    Voltage(Option<Func>),
    /// `I(...)` and the LTspice terminal currents `Ib`, `Ic`, `Id`, `Ix`, ...
    Current,
}

fn ref_kind(ident: &str) -> Option<RefKind> {
    let lower = ident.to_ascii_lowercase();
    Some(match lower.as_str() {
        "v" => RefKind::Voltage(None),
        "vdb" => RefKind::Voltage(Some(Func::Db)),
        "vm" => RefKind::Voltage(Some(Func::Mag)),
        "vp" => RefKind::Voltage(Some(Func::Ph)),
        "vr" => RefKind::Voltage(Some(Func::Re)),
        "vi" => RefKind::Voltage(Some(Func::Im)),
        s if s.starts_with('i') && s.len() <= 3 && s.chars().all(|c| c.is_ascii_alphabetic()) => {
            RefKind::Current
        }
        _ => return None,
    })
}

impl Expr {
    fn precedence(&self) -> u8 {
        match self {
            Expr::Bin(BinOp::Add | BinOp::Sub, ..) => 1,
            Expr::Bin(BinOp::Mul | BinOp::Div, ..) => 2,
            Expr::Neg(_) => 3,
            Expr::Num(v) if *v < 0.0 => 3,
            _ => 4,
        }
    }

    /// Every vector name the expression reads, as written.
    pub fn references(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_refs(&mut out);
        out
    }

    fn collect_refs(&self, out: &mut Vec<String>) {
        match self {
            Expr::Num(_) => {}
            Expr::Vector(n) => out.push(n.clone()),
            Expr::Diff(a, b) => {
                out.push(format!("V({a})"));
                out.push(format!("V({b})"));
            }
            Expr::Neg(e) | Expr::Call(_, e) => e.collect_refs(out),
            Expr::Bin(_, a, b) => {
                a.collect_refs(out);
                b.collect_refs(out);
            }
        }
    }

    /// The outermost function, if the whole expression is one call. Used by
    /// measurements to tell that `db(V(out))` is already in decibels.
    pub fn outer_func(&self) -> Option<Func> {
        match self {
            Expr::Call(f, _) => Some(*f),
            _ => None,
        }
    }

    /// Evaluate over points `range` of the dataset (one step of a stepped
    /// run, or all of it).
    pub fn eval(&self, ds: &Dataset, range: Range<usize>) -> Result<Series, ExprError> {
        let len = ds.len();
        if range.start > range.end || range.end > len {
            return Err(ExprError::BadRange {
                start: range.start,
                end: range.end,
                len,
            });
        }
        let mut ctx = Ctx {
            ds,
            range,
            axis: None,
        };
        self.eval_in(&mut ctx)
    }

    /// Evaluate over every point of the dataset.
    pub fn eval_all(&self, ds: &Dataset) -> Result<Series, ExprError> {
        self.eval(ds, 0..ds.len())
    }

    fn eval_in(&self, ctx: &mut Ctx<'_>) -> Result<Series, ExprError> {
        let n = ctx.range.len();
        Ok(match self {
            Expr::Num(v) => Series::Real(vec![*v; n]),
            Expr::Vector(name) => ctx.vector(name)?,
            Expr::Diff(a, b) => {
                let va = ctx.node(a)?;
                let vb = ctx.node(b)?;
                binary(BinOp::Sub, va, vb)
            }
            Expr::Neg(e) => match e.eval_in(ctx)? {
                Series::Real(v) => Series::Real(v.into_iter().map(|x| -x).collect()),
                Series::Complex(v) => {
                    Series::Complex(v.into_iter().map(|c| Complex::new(-c.re, -c.im)).collect())
                }
            },
            Expr::Bin(op, a, b) => {
                let va = a.eval_in(ctx)?;
                let vb = b.eval_in(ctx)?;
                binary(*op, va, vb)
            }
            Expr::Call(f, e) => {
                let v = e.eval_in(ctx)?;
                apply(*f, v, ctx)?
            }
        })
    }

    /// A best-effort unit for the expression's value: `V`, `A`, `dB`, `°`,
    /// `V/s`, or empty for a ratio or anything it cannot tell.
    pub fn unit(&self, ds: &Dataset) -> String {
        let axis_unit = ds.axis_vector().map(|v| v.quantity.unit()).unwrap_or("");
        match self {
            Expr::Num(_) => String::new(),
            Expr::Vector(name) => ds
                .vector(name)
                .map(|v| v.quantity.unit().to_string())
                .unwrap_or_default(),
            Expr::Diff(..) => "V".into(),
            Expr::Neg(e) => e.unit(ds),
            Expr::Bin(op, a, b) => {
                let (ua, ub) = (a.unit(ds), b.unit(ds));
                let a_num = matches!(**a, Expr::Num(_));
                let b_num = matches!(**b, Expr::Num(_));
                match op {
                    BinOp::Add | BinOp::Sub => {
                        if ua == ub || b_num {
                            ua
                        } else if a_num {
                            ub
                        } else {
                            String::new()
                        }
                    }
                    BinOp::Mul => match (ua.as_str(), ub.as_str()) {
                        (u, "") => u.to_string(),
                        ("", u) => u.to_string(),
                        ("V", "A") | ("A", "V") => "W".into(),
                        _ => String::new(),
                    },
                    BinOp::Div => match (ua.as_str(), ub.as_str()) {
                        (u, "") => u.to_string(),
                        ("V", "A") => "Ω".into(),
                        _ => String::new(),
                    },
                }
            }
            Expr::Call(f, e) => match f {
                Func::Db => "dB".into(),
                Func::Ph => "°".into(),
                Func::Mag | Func::Abs | Func::Re | Func::Im => e.unit(ds),
                Func::Sqrt | Func::Log10 => String::new(),
                Func::D => {
                    let u = e.unit(ds);
                    if axis_unit.is_empty() {
                        u
                    } else if u.is_empty() {
                        format!("1/{axis_unit}")
                    } else {
                        format!("{u}/{axis_unit}")
                    }
                }
                Func::Integ => format!("{}{axis_unit}", e.unit(ds)),
            },
        }
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Num(v) => write!(f, "{}", format_number(*v)),
            Expr::Vector(n) => write!(f, "{n}"),
            Expr::Diff(a, b) => write!(f, "V({a},{b})"),
            Expr::Neg(e) => {
                if e.precedence() < 3 {
                    write!(f, "-({e})")
                } else {
                    write!(f, "-{e}")
                }
            }
            Expr::Bin(op, a, b) => {
                let prec = self.precedence();
                let sym = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                };
                if a.precedence() < prec {
                    write!(f, "({a})")?;
                } else {
                    write!(f, "{a}")?;
                }
                let right_parens = b.precedence() < prec
                    || (b.precedence() == prec && matches!(op, BinOp::Sub | BinOp::Div));
                if right_parens {
                    write!(f, "{sym}({b})")
                } else {
                    write!(f, "{sym}{b}")
                }
            }
            Expr::Call(func, e) => write!(f, "{}({e})", func.name()),
        }
    }
}

/// Values of an expression at each point: real for transient and DC data,
/// complex for AC data.
#[derive(Debug, Clone, PartialEq)]
pub enum Series {
    Real(Vec<f64>),
    Complex(Vec<Complex>),
}

impl Series {
    pub fn len(&self) -> usize {
        match self {
            Series::Real(v) => v.len(),
            Series::Complex(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_complex(&self) -> bool {
        matches!(self, Series::Complex(_))
    }

    /// Real values, or the magnitude of complex ones. This is what a
    /// measurement such as `max` means on AC data.
    pub fn real_or_magnitude(&self) -> Vec<f64> {
        match self {
            Series::Real(v) => v.clone(),
            Series::Complex(v) => v.iter().map(|c| c.abs()).collect(),
        }
    }

    pub fn as_complex(&self) -> Vec<Complex> {
        match self {
            Series::Real(v) => v.iter().map(|&r| Complex::new(r, 0.0)).collect(),
            Series::Complex(v) => v.clone(),
        }
    }

    /// `20 log10 |x|` per point.
    pub fn db(&self) -> Vec<f64> {
        match self {
            Series::Real(v) => v.iter().map(|x| 20.0 * x.abs().log10()).collect(),
            Series::Complex(v) => v.iter().map(|c| c.db()).collect(),
        }
    }

    /// Phase in degrees per point, unwrapped.
    pub fn phase_deg(&self) -> Vec<f64> {
        let mut ph: Vec<f64> = match self {
            Series::Real(v) => v.iter().map(|x| 0f64.atan2(*x).to_degrees()).collect(),
            Series::Complex(v) => v.iter().map(|c| c.phase_deg()).collect(),
        };
        unwrap_degrees(&mut ph);
        ph
    }
}

/// Remove 360 degree jumps between neighbouring points, keeping the first
/// point's principal value. Non-finite points are left alone and skipped.
pub fn unwrap_degrees(ph: &mut [f64]) {
    let mut offset = 0.0;
    let mut prev: Option<f64> = None;
    for p in ph.iter_mut() {
        if !p.is_finite() {
            continue;
        }
        let raw = *p;
        if let Some(last_raw) = prev {
            let jump = raw - last_raw;
            offset -= 360.0 * (jump / 360.0).round();
        }
        prev = Some(raw);
        *p = raw + offset;
    }
}

struct Ctx<'a> {
    ds: &'a Dataset,
    range: Range<usize>,
    axis: Option<Vec<f64>>,
}

impl Ctx<'_> {
    fn slice(&self, v: &crate::dataset::Vector) -> Series {
        let r = self.range.clone();
        match &v.data {
            crate::dataset::VectorData::Real(d) => Series::Real(d[r].to_vec()),
            crate::dataset::VectorData::Complex(d) => Series::Complex(d[r].to_vec()),
        }
    }

    fn vector(&self, name: &str) -> Result<Series, ExprError> {
        match self.ds.vector(name) {
            Some(v) if v.data.len() >= self.range.end => Ok(self.slice(v)),
            Some(v) => Err(ExprError::BadRange {
                start: self.range.start,
                end: self.range.end,
                len: v.data.len(),
            }),
            None => Err(unknown_vector(self.ds, name)),
        }
    }

    /// A node voltage for `V(a,b)`; ground reads as zero.
    fn node(&self, node: &str) -> Result<Series, ExprError> {
        let name = format!("V({node})");
        if self.ds.vector(&name).is_none() && (node == "0" || node.eq_ignore_ascii_case("gnd")) {
            return Ok(Series::Real(vec![0.0; self.range.len()]));
        }
        self.vector(&name)
    }

    fn axis(&mut self, func: &'static str) -> Result<&[f64], ExprError> {
        if self.axis.is_none() {
            let Some(v) = self.ds.axis_vector() else {
                return Err(ExprError::NoAxis(func));
            };
            let all = v.data.real();
            if all.len() < self.range.end {
                return Err(ExprError::BadRange {
                    start: self.range.start,
                    end: self.range.end,
                    len: all.len(),
                });
            }
            self.axis = Some(all[self.range.clone()].to_vec());
        }
        Ok(self.axis.as_deref().unwrap_or(&[]))
    }
}

fn unknown_vector(ds: &Dataset, name: &str) -> ExprError {
    let available: Vec<String> = ds.names().into_iter().map(str::to_string).collect();
    ExprError::UnknownVector {
        name: name.to_string(),
        suggestions: suggest(name, &available),
        available,
    }
}

/// Names within a small edit distance of `name`, closest first. Both the full
/// name and the bare node inside `V(...)` are compared, so `otu` finds
/// `V(out)`.
fn suggest(name: &str, available: &[String]) -> Vec<String> {
    fn inner(s: &str) -> &str {
        match (s.find('('), s.rfind(')')) {
            (Some(a), Some(b)) if b > a => &s[a + 1..b],
            _ => s,
        }
    }
    let want = name.to_ascii_lowercase();
    let want_inner = inner(&want).to_string();
    let mut scored: Vec<(usize, &String)> = available
        .iter()
        .filter_map(|cand| {
            let c = cand.to_ascii_lowercase();
            let d = edit_distance(&want, &c).min(edit_distance(&want_inner, inner(&c)));
            let limit = (want_inner.chars().count() / 3).max(1);
            (d <= limit).then_some((d, cand))
        })
        .collect();
    scored.sort_by_key(|(d, _)| *d);
    scored.into_iter().take(3).map(|(_, c)| c.clone()).collect()
}

/// Edit distance counting a swap of neighbouring letters as one edit
/// (optimal string alignment), since `otu` for `out` is the typical typo.
#[allow(clippy::needless_range_loop)] // the recurrence reads best indexed
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=b.len() {
        d[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[a.len()][b.len()]
}

fn binary(op: BinOp, a: Series, b: Series) -> Series {
    match (a, b) {
        (Series::Real(x), Series::Real(y)) => Series::Real(
            x.iter()
                .zip(&y)
                .map(|(&p, &q)| match op {
                    BinOp::Add => p + q,
                    BinOp::Sub => p - q,
                    BinOp::Mul => p * q,
                    BinOp::Div => p / q,
                })
                .collect(),
        ),
        (a, b) => {
            let x = a.as_complex();
            let y = b.as_complex();
            Series::Complex(
                x.iter()
                    .zip(&y)
                    .map(|(&p, &q)| match op {
                        BinOp::Add => p + q,
                        BinOp::Sub => p - q,
                        BinOp::Mul => p * q,
                        BinOp::Div => p / q,
                    })
                    .collect(),
            )
        }
    }
}

fn complex_sqrt(c: Complex) -> Complex {
    let r = c.abs();
    let re = ((r + c.re) / 2.0).max(0.0).sqrt();
    let im = ((r - c.re) / 2.0).max(0.0).sqrt().copysign(c.im);
    Complex::new(re, im)
}

fn complex_log10(c: Complex) -> Complex {
    Complex::new(c.abs().log10(), c.im.atan2(c.re) / std::f64::consts::LN_10)
}

fn apply(f: Func, v: Series, ctx: &mut Ctx<'_>) -> Result<Series, ExprError> {
    Ok(match f {
        Func::Mag | Func::Abs => Series::Real(match &v {
            Series::Real(x) => x.iter().map(|a| a.abs()).collect(),
            Series::Complex(x) => x.iter().map(|c| c.abs()).collect(),
        }),
        Func::Db => Series::Real(v.db()),
        Func::Ph => Series::Real(v.phase_deg()),
        Func::Re => Series::Real(match &v {
            Series::Real(x) => x.clone(),
            Series::Complex(x) => x.iter().map(|c| c.re).collect(),
        }),
        Func::Im => Series::Real(match &v {
            Series::Real(x) => vec![0.0; x.len()],
            Series::Complex(x) => x.iter().map(|c| c.im).collect(),
        }),
        Func::Sqrt => match v {
            Series::Real(x) => Series::Real(x.iter().map(|a| a.sqrt()).collect()),
            Series::Complex(x) => Series::Complex(x.into_iter().map(complex_sqrt).collect()),
        },
        Func::Log10 => match v {
            Series::Real(x) => Series::Real(x.iter().map(|a| a.log10()).collect()),
            Series::Complex(x) => Series::Complex(x.into_iter().map(complex_log10).collect()),
        },
        Func::D => {
            let axis = ctx.axis("d")?;
            match v {
                Series::Real(y) => Series::Real(derivative(axis, &y)),
                Series::Complex(y) => {
                    let re: Vec<f64> = y.iter().map(|c| c.re).collect();
                    let im: Vec<f64> = y.iter().map(|c| c.im).collect();
                    zip_complex(derivative(axis, &re), derivative(axis, &im))
                }
            }
        }
        Func::Integ => {
            let axis = ctx.axis("integ")?;
            match v {
                Series::Real(y) => Series::Real(cumulative_integral(axis, &y)),
                Series::Complex(y) => {
                    let re: Vec<f64> = y.iter().map(|c| c.re).collect();
                    let im: Vec<f64> = y.iter().map(|c| c.im).collect();
                    zip_complex(
                        cumulative_integral(axis, &re),
                        cumulative_integral(axis, &im),
                    )
                }
            }
        }
    })
}

fn zip_complex(re: Vec<f64>, im: Vec<f64>) -> Series {
    Series::Complex(
        re.into_iter()
            .zip(im)
            .map(|(r, i)| Complex::new(r, i))
            .collect(),
    )
}

/// Derivative on a non-uniform grid: the three-point second-order formula
/// inside, one-sided differences at the ends. Simulators place time points
/// unevenly (dense at edges, sparse when nothing moves), so a plain
/// difference quotient would be first order exactly where it matters.
pub fn derivative(x: &[f64], y: &[f64]) -> Vec<f64> {
    let n = x.len().min(y.len());
    let mut out = vec![f64::NAN; n];
    if n < 2 {
        if n == 1 {
            out[0] = 0.0;
        }
        return out;
    }
    let one_sided = |i: usize, j: usize| {
        let h = x[j] - x[i];
        if h == 0.0 {
            f64::NAN
        } else {
            (y[j] - y[i]) / h
        }
    };
    out[0] = one_sided(0, 1);
    out[n - 1] = one_sided(n - 2, n - 1);
    for i in 1..n - 1 {
        let h0 = x[i] - x[i - 1];
        let h1 = x[i + 1] - x[i];
        out[i] = if h0 == 0.0 {
            one_sided(i, i + 1)
        } else if h1 == 0.0 {
            one_sided(i - 1, i)
        } else {
            (h0 * h0 * y[i + 1] - h1 * h1 * y[i - 1] + (h1 * h1 - h0 * h0) * y[i])
                / (h0 * h1 * (h0 + h1))
        };
    }
    out
}

/// Running trapezoidal integral starting at zero.
pub fn cumulative_integral(x: &[f64], y: &[f64]) -> Vec<f64> {
    let n = x.len().min(y.len());
    let mut out = Vec::with_capacity(n);
    let mut acc = 0.0;
    for i in 0..n {
        if i > 0 {
            acc += (x[i] - x[i - 1]) * (y[i] + y[i - 1]) / 2.0;
        }
        out.push(acc);
    }
    out
}

/// Whether the dataset's independent variable is frequency, so values are
/// interpolated against log frequency.
pub fn is_log_axis(ds: &Dataset) -> bool {
    ds.kind == AnalysisKind::Ac
        || ds
            .axis_vector()
            .is_some_and(|v| v.quantity == Quantity::Frequency)
}

/// Builders for synthetic datasets in tests across the crate.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::dataset::{AnalysisKind, Complex, Dataset, Quantity, Step, Vector, VectorData};

    fn quantity_of(name: &str) -> Quantity {
        if name.starts_with("I(") {
            Quantity::Current
        } else {
            Quantity::Voltage
        }
    }

    pub fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| a + (b - a) * i as f64 / (n - 1) as f64)
            .collect()
    }

    pub fn logspace(a: f64, b: f64, n: usize) -> Vec<f64> {
        let (la, lb) = (a.log10(), b.log10());
        (0..n)
            .map(|i| 10f64.powf(la + (lb - la) * i as f64 / (n - 1) as f64))
            .collect()
    }

    pub fn tran(t: Vec<f64>, vectors: Vec<(&str, Vec<f64>)>) -> Dataset {
        let n = t.len();
        let mut vs = vec![Vector {
            name: "time".into(),
            quantity: Quantity::Time,
            data: VectorData::Real(t),
        }];
        for (name, y) in vectors {
            vs.push(Vector {
                name: name.into(),
                quantity: quantity_of(name),
                data: VectorData::Real(y),
            });
        }
        Dataset {
            title: "test".into(),
            plotname: "Transient Analysis".into(),
            kind: AnalysisKind::Transient,
            axis: Some(0),
            vectors: vs,
            steps: vec![Step {
                range: 0..n,
                label: String::new(),
            }],
        }
    }

    pub fn ac(f: Vec<f64>, vectors: Vec<(&str, Vec<Complex>)>) -> Dataset {
        let n = f.len();
        let mut vs = vec![Vector {
            name: "frequency".into(),
            quantity: Quantity::Frequency,
            data: VectorData::Real(f),
        }];
        for (name, y) in vectors {
            vs.push(Vector {
                name: name.into(),
                quantity: quantity_of(name),
                data: VectorData::Complex(y),
            });
        }
        Dataset {
            title: "test".into(),
            plotname: "AC Analysis".into(),
            kind: AnalysisKind::Ac,
            axis: Some(0),
            vectors: vs,
            steps: vec![Step {
                range: 0..n,
                label: String::new(),
            }],
        }
    }

    /// Concatenate same-shaped datasets into one stepped dataset.
    pub fn stepped(parts: Vec<(&str, Dataset)>) -> Dataset {
        let mut out = parts[0].1.clone();
        for v in &mut out.vectors {
            v.data = match &v.data {
                VectorData::Real(_) => VectorData::Real(Vec::new()),
                VectorData::Complex(_) => VectorData::Complex(Vec::new()),
            };
        }
        out.steps.clear();
        let mut start = 0;
        for (label, ds) in parts {
            let n = ds.len();
            for (dst, src) in out.vectors.iter_mut().zip(ds.vectors) {
                match (&mut dst.data, src.data) {
                    (VectorData::Real(d), VectorData::Real(s)) => d.extend(s),
                    (VectorData::Complex(d), VectorData::Complex(s)) => d.extend(s),
                    _ => panic!("mismatched vector kinds"),
                }
            }
            out.steps.push(Step {
                range: start..start + n,
                label: label.into(),
            });
            start += n;
        }
        out
    }

    /// `H(jw)` of `1 / prod(1 + s/p_k)` at frequency `f`.
    pub fn poles(f: f64, poles_hz: &[f64], gain: f64) -> Complex {
        let mut h = Complex::new(gain, 0.0);
        for p in poles_hz {
            h = h / Complex::new(1.0, f / p);
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * b.abs().max(1.0)
    }

    fn real(s: Series) -> Vec<f64> {
        match s {
            Series::Real(v) => v,
            Series::Complex(_) => panic!("expected real"),
        }
    }

    fn sample() -> Dataset {
        let t = linspace(0.0, 1e-3, 1001);
        let a: Vec<f64> = t.iter().map(|t| 2.0 + t * 1e3).collect();
        let b: Vec<f64> = t.iter().map(|_| 0.5).collect();
        let i: Vec<f64> = t.iter().map(|_| 1e-3).collect();
        tran(t, vec![("V(a)", a), ("V(b)", b), ("I(R1)", i)])
    }

    #[test]
    fn parses_and_prints_canonically() {
        let cases = [
            ("V(out)", "V(out)"),
            ("v(out)/v(in)", "V(out)/V(in)"),
            ("V(a, b)", "V(a,b)"),
            ("-V(out)", "-V(out)"),
            ("-(a+b)", "-(a+b)"),
            ("a-(b-c)", "a-(b-c)"),
            ("(a-b)-c", "a-b-c"),
            ("a/(b*c)", "a/(b*c)"),
            ("a*b/c", "a*b/c"),
            ("2*(V(x)+1k)", "2*(V(x)+1k)"),
            ("db(V(out)/V(in))", "db(V(out)/V(in))"),
            ("vdb(out)", "db(V(out))"),
            ("vp(out)", "ph(V(out))"),
            ("I(r1)", "I(r1)"),
            ("Ib(Q1)", "Ib(Q1)"),
            ("d(integ(out))", "d(integ(out))"),
            ("1e-3", "1m"),
            ("4k7", "4.7k"),
            ("2.5 * phase(x)", "2.5*ph(x)"),
            ("V(u1:out)", "V(u1:out)"),
            ("V(in+)", "V(in+)"),
            ("1.2345678k", "1.2345678k"),
        ];
        for (text, want) in cases {
            let e = parse(text).unwrap_or_else(|err| panic!("{text}: {err}"));
            assert_eq!(e.to_string(), want, "{text}");
            let again = parse(&e.to_string()).unwrap();
            assert_eq!(again.to_string(), want, "round trip of {text}");
        }
    }

    #[test]
    fn precedence_and_unary_minus() {
        let ds = sample();
        let v = real(
            parse("1 + 2 * 3 - -4 / 2")
                .unwrap()
                .eval(&ds, 0..1)
                .unwrap(),
        );
        assert_eq!(v[0], 9.0);
        let v = real(parse("-(1 + 2) * 3").unwrap().eval(&ds, 0..1).unwrap());
        assert_eq!(v[0], -9.0);
        let v = real(parse("2 - 3 - 4").unwrap().eval(&ds, 0..1).unwrap());
        assert_eq!(v[0], -5.0);
        let v = real(parse("12 / 3 / 2").unwrap().eval(&ds, 0..1).unwrap());
        assert_eq!(v[0], 2.0);
    }

    #[test]
    fn spice_suffixes_and_megahertz() {
        assert_eq!(parse_number("1Meg"), Some(1e6));
        assert_eq!(parse_number("1MHz"), Some(1e6));
        assert_eq!(parse_number("1m"), Some(1e-3));
        assert_eq!(parse_number("1MegHz"), Some(1e6));
        assert_eq!(parse_number("10ns"), Some(10e-9));
        let e = parse("1MHz*2").unwrap();
        assert_eq!(
            e,
            Expr::Bin(
                BinOp::Mul,
                Box::new(Expr::Num(1e6)),
                Box::new(Expr::Num(2.0))
            )
        );
    }

    #[test]
    fn format_number_round_trips() {
        for v in [
            1.0,
            4.7e3,
            1234.5678,
            1.5915494309189535e3,
            2.2e-6,
            1e-15,
            3.3e-17,
            -12.5,
            0.1 + 0.2,
            1e300,
            0.0,
        ] {
            let s = format_number(v);
            assert_eq!(parse_number(&s), Some(v), "{v} -> {s}");
        }
        assert_eq!(format_number(4700.0), "4.7k");
        assert_eq!(format_number(47e-9), "47n");
        assert_eq!(format_number(100e-9), "100n");
        assert_eq!(format_number(2.2e-8), "22n");
        assert_eq!(parse_spice("47n"), Some(47e-9));
        assert_eq!(parse_spice("10uF"), Some(10e-6));
        assert_eq!(parse_spice("1.5e3k"), Some(1.5e6));
        assert_eq!(parse_spice("4k7"), aispice_core::units::parse("4k7"));
        assert_eq!(parse_spice("2mil"), aispice_core::units::parse("2mil"));
        assert_eq!(parse_spice("{R}"), None);
    }

    #[test]
    fn evaluates_vectors_differences_and_ground() {
        let ds = sample();
        let d = real(parse("V(a,b)").unwrap().eval(&ds, 0..3).unwrap());
        assert!(close(d[0], 1.5, 1e-12) && close(d[2], 1.502, 1e-12));
        let g = real(parse("V(a,0)").unwrap().eval(&ds, 0..1).unwrap());
        assert_eq!(g[0], 2.0);
        let bare = real(parse("a").unwrap().eval(&ds, 0..1).unwrap());
        assert_eq!(bare[0], 2.0);
        let p = real(parse("V(a)*I(R1)").unwrap().eval(&ds, 0..1).unwrap());
        assert!(close(p[0], 2e-3, 1e-12));
        assert_eq!(parse("V(a)*I(R1)").unwrap().unit(&ds), "W");
        assert_eq!(parse("V(a)/I(R1)").unwrap().unit(&ds), "Ω");
        assert_eq!(parse("V(a)/V(b)").unwrap().unit(&ds), "");
        assert_eq!(parse("d(V(a))").unwrap().unit(&ds), "V/s");
        assert_eq!(parse("integ(I(R1))").unwrap().unit(&ds), "As");
        assert_eq!(parse("2*V(a)+1").unwrap().unit(&ds), "V");
    }

    #[test]
    fn unknown_vectors_suggest_close_names() {
        let ds = sample();
        let err = parse("V(aa)").unwrap().eval(&ds, 0..1).unwrap_err();
        let ExprError::UnknownVector { suggestions, .. } = &err else {
            panic!("{err}");
        };
        assert!(suggestions.contains(&"V(a)".to_string()), "{suggestions:?}");
        let msg = err.to_string();
        assert!(msg.contains("did you mean V(a)"), "{msg}");
        let err = parse("zzzzzz").unwrap().eval(&ds, 0..1).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("available: time, V(a), V(b), I(R1)"), "{msg}");
        assert_eq!(edit_distance("otu", "out"), 1);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        let err = parse("I(R2)").unwrap().eval(&ds, 0..1).unwrap_err();
        assert!(err.to_string().contains("I(R1)"), "{err}");
    }

    #[test]
    fn syntax_errors_point_at_the_problem() {
        for (text, needle) in [
            ("", "empty"),
            ("V(out", "unclosed"),
            ("1 +", "expected a value"),
            ("(1 + 2", "expected `)`"),
            ("V(out) V(in)", "unexpected `V`"),
            ("2 $ 3", "unexpected `$`"),
            ("V()", "empty name"),
            ("I(a,b)", "one device name"),
            ("1x2", "not a number"),
        ] {
            let err = parse(text).unwrap_err().to_string();
            assert!(err.contains(needle), "{text}: {err}");
        }
        let err = parse("foo(V(out))").unwrap_err();
        assert_eq!(err, ExprError::UnknownFunction("foo".into()));
        let deep = format!("{}1{}", "(".repeat(10_000), ")".repeat(10_000));
        assert!(parse(&deep).unwrap_err().to_string().contains("nested"));
        let negs = format!("{}1", "-".repeat(10_000));
        assert!(parse(&negs).is_err());
    }

    #[test]
    fn derivative_and_integral_of_exponential() {
        // y = exp(-t/tau) on an uneven grid: d/dt = -y/tau, integral = tau (1 - y).
        let tau = 1e-3;
        let t: Vec<f64> = (0..400)
            .map(|i| {
                let u = i as f64 / 399.0;
                5e-3 * u * u
            })
            .collect();
        let y: Vec<f64> = t.iter().map(|t| (-t / tau).exp()).collect();
        let ds = tran(t.clone(), vec![("V(out)", y.clone())]);
        let d = real(parse("d(V(out))").unwrap().eval_all(&ds).unwrap());
        for i in 1..t.len() - 1 {
            let want = -y[i] / tau;
            assert!(
                (d[i] - want).abs() < 2e-3 * (1.0 / tau),
                "i={i} {} vs {want}",
                d[i]
            );
        }
        let s = real(parse("integ(V(out))").unwrap().eval_all(&ds).unwrap());
        let want = tau * (1.0 - y[y.len() - 1]);
        assert!(
            close(s[s.len() - 1], want, 1e-4),
            "{} vs {want}",
            s[s.len() - 1]
        );
    }

    #[test]
    fn derivative_of_sine_is_cosine() {
        let f = 1e3;
        let w = 2.0 * std::f64::consts::PI * f;
        let t = linspace(0.0, 2e-3, 2001);
        let y: Vec<f64> = t.iter().map(|t| (w * t).sin()).collect();
        let ds = tran(t.clone(), vec![("V(x)", y)]);
        let d = real(
            parse("d(x)/6.283185307179586k")
                .unwrap()
                .eval_all(&ds)
                .unwrap(),
        );
        for i in 1..t.len() - 1 {
            assert!((d[i] - (w * t[i]).cos()).abs() < 1e-5, "i={i}");
        }
        let rms_sq = real(parse("integ(x*x)").unwrap().eval_all(&ds).unwrap());
        assert!(close(*rms_sq.last().unwrap() / 2e-3, 0.5, 1e-5));
    }

    #[test]
    fn complex_ac_functions() {
        let fc = 1e3;
        let f = logspace(10.0, 1e6, 201);
        let h: Vec<Complex> = f.iter().map(|&f| poles(f, &[fc], 1.0)).collect();
        let vin: Vec<Complex> = f.iter().map(|_| Complex::new(1.0, 0.0)).collect();
        let ds = ac(f.clone(), vec![("V(out)", h.clone()), ("V(in)", vin)]);
        let ratio = parse("V(out)/V(in)").unwrap().eval_all(&ds).unwrap();
        assert!(ratio.is_complex());
        let db = real(parse("db(V(out)/V(in))").unwrap().eval_all(&ds).unwrap());
        let mag = real(parse("mag(V(out))").unwrap().eval_all(&ds).unwrap());
        let ph = real(parse("ph(V(out))").unwrap().eval_all(&ds).unwrap());
        let re = real(parse("re(V(out))").unwrap().eval_all(&ds).unwrap());
        let im = real(parse("im(V(out))").unwrap().eval_all(&ds).unwrap());
        for i in 0..f.len() {
            let x = f[i] / fc;
            assert!(close(db[i], -10.0 * (1.0 + x * x).log10(), 1e-9));
            assert!(close(mag[i], 1.0 / (1.0 + x * x).sqrt(), 1e-12));
            assert!(close(ph[i], -x.atan().to_degrees(), 1e-9));
            assert!(close(re[i], 1.0 / (1.0 + x * x), 1e-12));
            assert!(close(im[i], -x / (1.0 + x * x), 1e-12));
        }
        assert_eq!(parse("db(V(out))").unwrap().unit(&ds), "dB");
        assert_eq!(parse("ph(V(out))").unwrap().unit(&ds), "°");
        // sqrt and log10 are principal-branch complex functions.
        let s = parse("sqrt(V(out)*V(out))")
            .unwrap()
            .eval(&ds, 100..101)
            .unwrap();
        let Series::Complex(s) = s else { panic!() };
        assert!(close(s[0].re, h[100].re, 1e-9) && close(s[0].im, h[100].im, 1e-9));
        let l = parse("log10(V(in)*10)").unwrap().eval(&ds, 0..1).unwrap();
        let Series::Complex(l) = l else { panic!() };
        assert!(close(l[0].re, 1.0, 1e-12) && l[0].im.abs() < 1e-12);
    }

    #[test]
    fn phase_unwraps_past_minus_180() {
        // Three coincident poles: phase heads for -270 degrees.
        let f = logspace(1.0, 1e5, 301);
        let h: Vec<Complex> = f.iter().map(|&f| poles(f, &[100.0; 3], 1.0)).collect();
        let ds = ac(f.clone(), vec![("V(out)", h)]);
        let ph = real(parse("ph(V(out))").unwrap().eval_all(&ds).unwrap());
        for (i, &fi) in f.iter().enumerate() {
            let want = -3.0 * (fi / 100.0).atan().to_degrees();
            assert!(close(ph[i], want, 1e-9), "{fi}: {} vs {want}", ph[i]);
        }
        assert!(ph.last().unwrap() < &-260.0);
    }

    #[test]
    fn steps_are_evaluated_separately() {
        let a = tran(linspace(0.0, 1.0, 11), vec![("V(x)", vec![1.0; 11])]);
        let b = tran(linspace(0.0, 1.0, 11), vec![("V(x)", vec![3.0; 11])]);
        let ds = stepped(vec![("k=1", a), ("k=3", b)]);
        let e = parse("integ(x)").unwrap();
        let s0 = real(e.eval(&ds, ds.steps[0].range.clone()).unwrap());
        let s1 = real(e.eval(&ds, ds.steps[1].range.clone()).unwrap());
        assert!(close(*s0.last().unwrap(), 1.0, 1e-12));
        assert!(close(*s1.last().unwrap(), 3.0, 1e-12));
        assert!(matches!(
            e.eval(&ds, 0..100),
            Err(ExprError::BadRange { .. })
        ));
    }

    #[test]
    fn operating_point_has_no_axis() {
        let mut ds = tran(vec![0.0], vec![("V(x)", vec![1.0])]);
        ds.axis = None;
        ds.kind = AnalysisKind::Op;
        assert_eq!(
            real(parse("2*x").unwrap().eval_all(&ds).unwrap()),
            vec![2.0]
        );
        assert_eq!(
            parse("d(x)").unwrap().eval_all(&ds),
            Err(ExprError::NoAxis("d"))
        );
    }

    #[test]
    fn references_are_listed() {
        let e = parse("V(a,b)/I(R1)+out").unwrap();
        assert_eq!(e.references(), vec!["V(a)", "V(b)", "I(R1)", "out"]);
    }
}
