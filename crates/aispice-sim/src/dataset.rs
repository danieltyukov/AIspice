//! Simulation results in one shape, whichever simulator produced them.
//!
//! Vector names are normalised on the way in: node voltages are `V(name)`,
//! branch currents `I(name)`, the independent variable keeps its own name
//! (`time`, `frequency`, or the swept source). Lookups are case-insensitive
//! and accept the bare node name for a voltage, so `out`, `v(out)` and
//! `V(OUT)` all find the same vector.

use serde::{Deserialize, Serialize};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    pub const fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }
    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
    /// Phase in degrees.
    pub fn phase_deg(self) -> f64 {
        self.im.atan2(self.re).to_degrees()
    }
    pub fn db(self) -> f64 {
        20.0 * self.abs().log10()
    }
}

impl std::ops::Div for Complex {
    type Output = Complex;
    fn div(self, rhs: Complex) -> Complex {
        let d = rhs.re * rhs.re + rhs.im * rhs.im;
        Complex::new(
            (self.re * rhs.re + self.im * rhs.im) / d,
            (self.im * rhs.re - self.re * rhs.im) / d,
        )
    }
}

impl std::ops::Mul for Complex {
    type Output = Complex;
    fn mul(self, rhs: Complex) -> Complex {
        Complex::new(
            self.re * rhs.re - self.im * rhs.im,
            self.re * rhs.im + self.im * rhs.re,
        )
    }
}

impl std::ops::Add for Complex {
    type Output = Complex;
    fn add(self, rhs: Complex) -> Complex {
        Complex::new(self.re + rhs.re, self.im + rhs.im)
    }
}

impl std::ops::Sub for Complex {
    type Output = Complex;
    fn sub(self, rhs: Complex) -> Complex {
        Complex::new(self.re - rhs.re, self.im - rhs.im)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisKind {
    Transient,
    Ac,
    Dc,
    Op,
    Noise,
    TransferFunction,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quantity {
    Time,
    Frequency,
    Voltage,
    Current,
    /// A swept parameter or source value.
    Sweep,
    Other,
}

impl Quantity {
    pub fn unit(self) -> &'static str {
        match self {
            Quantity::Time => "s",
            Quantity::Frequency => "Hz",
            Quantity::Voltage => "V",
            Quantity::Current => "A",
            Quantity::Sweep | Quantity::Other => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "values", rename_all = "snake_case")]
pub enum VectorData {
    Real(Vec<f64>),
    Complex(Vec<Complex>),
}

impl VectorData {
    pub fn len(&self) -> usize {
        match self {
            VectorData::Real(v) => v.len(),
            VectorData::Complex(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Real part (or the value itself for real data).
    pub fn real(&self) -> Vec<f64> {
        match self {
            VectorData::Real(v) => v.clone(),
            VectorData::Complex(v) => v.iter().map(|c| c.re).collect(),
        }
    }
    pub fn as_complex(&self) -> Vec<Complex> {
        match self {
            VectorData::Real(v) => v.iter().map(|&r| Complex::new(r, 0.0)).collect(),
            VectorData::Complex(v) => v.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vector {
    pub name: String,
    pub quantity: Quantity,
    pub data: VectorData,
}

/// One run of a stepped simulation (`.step`, sweeps, Monte Carlo), as a range
/// of point indices with a human label such as `R1=2k`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub range: Range<usize>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dataset {
    pub title: String,
    /// The simulator's plot name, e.g. `Transient Analysis`.
    pub plotname: String,
    pub kind: AnalysisKind,
    /// Index of the independent variable in `vectors`; `None` for an
    /// operating point.
    pub axis: Option<usize>,
    pub vectors: Vec<Vector>,
    /// Always at least one step covering every point.
    pub steps: Vec<Step>,
}

/// Normalise a simulator's vector name to aispice's convention.
pub fn normalize_name(raw: &str) -> String {
    let s = raw.trim();
    let lower = s.to_ascii_lowercase();
    // ngspice branch currents: v1#branch
    if let Some(dev) = lower.strip_suffix("#branch") {
        return format!("I({})", s[..dev.len()].to_ascii_uppercase());
    }
    for (prefix, head) in [("v(", "V"), ("i(", "I"), ("ix(", "Ix"), ("iy(", "Iy")] {
        if lower.starts_with(prefix) && lower.ends_with(')') {
            return format!("{head}({})", &s[prefix.len()..s.len() - 1]);
        }
    }
    s.to_string()
}

impl Dataset {
    pub fn axis_vector(&self) -> Option<&Vector> {
        self.axis.and_then(|i| self.vectors.get(i))
    }

    pub fn len(&self) -> usize {
        self.vectors.first().map(|v| v.data.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Find a vector by name: exact (case-insensitive), then `V(name)` for a
    /// bare node name, then `I(name)` for a bare device name.
    pub fn vector(&self, name: &str) -> Option<&Vector> {
        let want = normalize_name(name);
        let eq = |v: &&Vector| v.name.eq_ignore_ascii_case(&want);
        self.vectors
            .iter()
            .find(eq)
            .or_else(|| {
                self.vectors
                    .iter()
                    .find(|v| v.name.eq_ignore_ascii_case(&format!("V({want})")))
            })
            .or_else(|| {
                self.vectors
                    .iter()
                    .find(|v| v.name.eq_ignore_ascii_case(&format!("I({want})")))
            })
    }

    pub fn names(&self) -> Vec<&str> {
        self.vectors.iter().map(|v| v.name.as_str()).collect()
    }

    pub fn is_stepped(&self) -> bool {
        self.steps.len() > 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_normalise() {
        assert_eq!(normalize_name("v(out)"), "V(out)");
        assert_eq!(normalize_name("V(OUT)"), "V(OUT)");
        assert_eq!(normalize_name("i(r1)"), "I(r1)");
        assert_eq!(normalize_name("v1#branch"), "I(V1)");
        assert_eq!(normalize_name("Ix(u1:out)"), "Ix(u1:out)");
        assert_eq!(normalize_name("time"), "time");
    }

    #[test]
    fn lookup_accepts_bare_node_names() {
        let ds = Dataset {
            title: String::new(),
            plotname: String::new(),
            kind: AnalysisKind::Transient,
            axis: Some(0),
            vectors: vec![
                Vector {
                    name: "time".into(),
                    quantity: Quantity::Time,
                    data: VectorData::Real(vec![0.0, 1.0]),
                },
                Vector {
                    name: "V(out)".into(),
                    quantity: Quantity::Voltage,
                    data: VectorData::Real(vec![0.0, 2.0]),
                },
                Vector {
                    name: "I(R1)".into(),
                    quantity: Quantity::Current,
                    data: VectorData::Real(vec![0.0, 1e-3]),
                },
            ],
            steps: vec![Step {
                range: 0..2,
                label: String::new(),
            }],
        };
        assert!(ds.vector("out").is_some());
        assert!(ds.vector("v(OUT)").is_some());
        assert!(ds.vector("r1").is_some());
        assert!(ds.vector("missing").is_none());
    }

    #[test]
    fn complex_math() {
        let a = Complex::new(1.0, 1.0);
        assert!((a.abs() - 2f64.sqrt()).abs() < 1e-12);
        assert!((a.phase_deg() - 45.0).abs() < 1e-12);
        let q = a / Complex::new(0.0, 1.0);
        assert!((q.re - 1.0).abs() < 1e-12 && (q.im + 1.0).abs() < 1e-12);
    }
}
