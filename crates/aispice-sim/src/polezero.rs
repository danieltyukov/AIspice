//! Poles and zeros from ngspice's pole-zero analysis, and what they mean for
//! the response.
//!
//! `.pz in+ in- out+ out- vol|cur pz` writes one complex point per root, in
//! rad/s, as vectors `pole(1)`, `pole(2)`, ... and `zero(1)`, ... (ngspice
//! types them as voltages, so the raw file names them `v(pole(1))` and the
//! reader shows `V(pole(1))`). Conjugate pairs come one after the other. A
//! transfer function with no finite poles or zeros gives an empty plot.
//! LTspice and Xyce have no pole-zero analysis.
//!
//! Roots are reported in Hz (rad/s divided by 2 pi). A complex pair has the
//! natural frequency `f0 = |p|`, damping ratio `zeta = -Re(p) / |p|` and
//! `Q = 1 / (2 zeta)`; a real root has its corner at `|p|`.

use crate::dataset::{AnalysisKind, Complex, Dataset};
use crate::measure::{plain, spaced};
use std::f64::consts::PI;

/// A pole or zero in Hz.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Root {
    pub re_hz: f64,
    pub im_hz: f64,
}

/// Which half of the s-plane a root lies in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Half {
    Left,
    /// On the imaginary axis, the origin included.
    Axis,
    Right,
}

/// What a root means for the response.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// At s = 0: an integrator (pole) or a blocked DC path (zero).
    Origin,
    /// On the real axis, with its corner frequency.
    Real { corner_hz: f64 },
    /// Complex, with natural frequency, Q and damping ratio. Q is infinite
    /// on the imaginary axis and negative in the right half-plane.
    Complex { f0_hz: f64, q: f64, zeta: f64 },
}

impl Root {
    pub fn new(re_hz: f64, im_hz: f64) -> Self {
        Self { re_hz, im_hz }
    }

    pub fn from_rad_per_s(s: Complex) -> Self {
        Self::new(s.re / (2.0 * PI), s.im / (2.0 * PI))
    }

    /// |s|: the natural frequency of a complex root, the corner of a real one.
    pub fn f0_hz(&self) -> f64 {
        self.re_hz.hypot(self.im_hz)
    }

    /// Parts smaller than this count as zero. ngspice's root finder leaves
    /// residues far below a millionth of the root's size.
    fn tolerance(&self) -> f64 {
        1e-6 * self.f0_hz() + 1e-12
    }

    pub fn half(&self) -> Half {
        let tol = self.tolerance();
        if self.re_hz > tol {
            Half::Right
        } else if self.re_hz < -tol {
            Half::Left
        } else {
            Half::Axis
        }
    }

    pub fn is_real(&self) -> bool {
        self.im_hz.abs() <= self.tolerance()
    }

    pub fn shape(&self) -> Shape {
        let f0 = self.f0_hz();
        if f0 <= 1e-12 {
            return Shape::Origin;
        }
        if self.is_real() {
            return Shape::Real {
                corner_hz: self.re_hz.abs(),
            };
        }
        let zeta = if self.half() == Half::Axis {
            0.0
        } else {
            -self.re_hz / f0
        };
        let q = if zeta == 0.0 {
            f64::INFINITY
        } else {
            0.5 / zeta
        };
        Shape::Complex { f0_hz: f0, q, zeta }
    }

    /// Q of a complex root; `None` for a real root or an undamped pair.
    pub fn q(&self) -> Option<f64> {
        match self.shape() {
            Shape::Complex { q, .. } if q.is_finite() => Some(q),
            _ => None,
        }
    }
}

/// A root as reported: a real root, or one complex root of a pair standing
/// for both (with a positive imaginary part).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Group {
    pub root: Root,
    pub pair: bool,
}

/// Fold conjugate pairs into one entry each, keeping the order.
pub fn group(roots: &[Root]) -> Vec<Group> {
    let mut used = vec![false; roots.len()];
    let mut out = Vec::new();
    for i in 0..roots.len() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let r = roots[i];
        if r.is_real() {
            out.push(Group {
                root: r,
                pair: false,
            });
            continue;
        }
        let conjugate = (i + 1..roots.len()).find(|&j| {
            let c = roots[j];
            let tol = r.tolerance().max(c.tolerance());
            !used[j] && (c.re_hz - r.re_hz).abs() <= tol && (c.im_hz + r.im_hz).abs() <= tol
        });
        match conjugate {
            Some(j) => {
                used[j] = true;
                out.push(Group {
                    root: Root::new(r.re_hz, r.im_hz.abs()),
                    pair: true,
                });
            }
            None => out.push(Group {
                root: r,
                pair: false,
            }),
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stability {
    /// Every pole in the left half-plane.
    Stable,
    /// None on the right, but some on the imaginary axis: an integrator or
    /// an undamped resonance.
    Marginal,
    /// At least one pole in the right half-plane: the response grows without
    /// bound.
    Unstable,
}

pub fn stability(poles: &[Root]) -> Stability {
    if poles.iter().any(|p| p.half() == Half::Right) {
        Stability::Unstable
    } else if poles.iter().any(|p| p.half() == Half::Axis) {
        Stability::Marginal
    } else {
        Stability::Stable
    }
}

/// The result of one pole-zero analysis, in ngspice's order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PoleZero {
    pub poles: Vec<Root>,
    pub zeros: Vec<Root>,
}

impl PoleZero {
    /// The roots of the first pole-zero dataset, if there is one.
    pub fn from_datasets(datasets: &[Dataset]) -> Option<Self> {
        datasets
            .iter()
            .find(|d| d.kind == AnalysisKind::PoleZero)
            .map(Self::from_dataset)
    }

    pub fn from_dataset(ds: &Dataset) -> Self {
        let mut poles = Vec::new();
        let mut zeros = Vec::new();
        for v in &ds.vectors {
            let Some((is_pole, k)) = root_name(&v.name) else {
                continue;
            };
            let Some(s) = v.data.as_complex().first().copied() else {
                continue;
            };
            let list = if is_pole { &mut poles } else { &mut zeros };
            list.push((k, Root::from_rad_per_s(s)));
        }
        poles.sort_by_key(|(k, _)| *k);
        zeros.sort_by_key(|(k, _)| *k);
        Self {
            poles: poles.into_iter().map(|(_, r)| r).collect(),
            zeros: zeros.into_iter().map(|(_, r)| r).collect(),
        }
    }

    pub fn stability(&self) -> Stability {
        stability(&self.poles)
    }

    /// The verdict, then every pole and zero with what it means. Numbers
    /// have four significant digits.
    pub fn report(&self) -> String {
        let right = self
            .poles
            .iter()
            .filter(|p| p.half() == Half::Right)
            .count();
        let axis = self.poles.iter().filter(|p| p.half() == Half::Axis).count();
        let mut t = match self.stability() {
            Stability::Unstable => format!(
                "UNSTABLE: {right} of {} poles in the right half-plane; any disturbance grows without bound.\n",
                self.poles.len()
            ),
            Stability::Marginal => format!(
                "Marginally stable: no pole in the right half-plane, but {axis} on the imaginary axis (an integrator or an undamped resonance).\n"
            ),
            Stability::Stable if self.poles.is_empty() => "Stable: no finite poles.\n".into(),
            Stability::Stable => "Stable: every pole is in the left half-plane.\n".into(),
        };
        if self.poles.is_empty() && self.zeros.is_empty() {
            t.push_str(
                "No finite poles or zeros: the response is flat at every frequency (a resistive path, for example).\n",
            );
            return t;
        }
        for (what, roots, is_pole) in [("Poles", &self.poles, true), ("Zeros", &self.zeros, false)]
        {
            if roots.is_empty() {
                t.push_str(&format!("{what}: none\n"));
                continue;
            }
            t.push_str(&format!("{what} ({}):\n", roots.len()));
            for g in group(roots) {
                t.push_str(&format!("  {}\n", describe(&g, is_pole)));
            }
        }
        t
    }
}

/// `pole(3)` or `zero(1)`, bare or as ngspice's `v(pole(3))`.
fn root_name(name: &str) -> Option<(bool, usize)> {
    let lower = name.trim().to_ascii_lowercase();
    let inner = lower
        .strip_prefix("v(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(&lower);
    let (is_pole, rest) = match inner.strip_prefix("pole(") {
        Some(rest) => (true, rest),
        None => (false, inner.strip_prefix("zero(")?),
    };
    let k = rest.strip_suffix(')')?.parse().ok()?;
    Some((is_pole, k))
}

fn hz(v: f64) -> String {
    spaced(v, "Hz")
}

/// A real part with its sign, so a root on the right reads as such.
fn re_hz(r: &Root) -> String {
    if r.half() == Half::Right {
        format!("+{}", hz(r.re_hz))
    } else {
        hz(r.re_hz)
    }
}

/// One line for a root or pair: where it is, what it means, and a loud
/// warning for a pole in the right half-plane.
fn describe(g: &Group, is_pole: bool) -> String {
    let r = g.root;
    let at = if g.pair {
        format!("{} ± j{}", re_hz(&r), hz(r.im_hz))
    } else if r.is_real() {
        re_hz(&r)
    } else {
        let sign = if r.im_hz < 0.0 { '-' } else { '+' };
        format!("{} {sign} j{}", re_hz(&r), hz(r.im_hz.abs()))
    };
    let mut line = match r.shape() {
        Shape::Origin if is_pole => format!("{at}: at the origin (an integrator)"),
        Shape::Origin => format!("{at}: at the origin (DC is blocked)"),
        Shape::Real { corner_hz } => format!("{at}: real, corner at {}", hz(corner_hz)),
        Shape::Complex { f0_hz, q, zeta } => {
            let what = if g.pair { "complex pair" } else { "complex" };
            let q = if q.is_finite() {
                plain(q)
            } else {
                "infinite (undamped)".into()
            };
            format!(
                "{at}: {what}, f0 = {}, Q = {q}, damping ratio = {}",
                hz(f0_hz),
                plain(zeta)
            )
        }
    };
    match (r.half(), is_pole, r.shape()) {
        (Half::Right, true, _) => line.push_str(". UNSTABLE: right half-plane pole"),
        (Half::Axis, true, Shape::Complex { .. }) => {
            line.push_str(". Marginal: on the imaginary axis, it rings forever")
        }
        (Half::Right, false, _) => line.push_str(" (right half-plane zero: non-minimum phase)"),
        _ => {}
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{Quantity, Step, Vector, VectorData};

    fn close(a: f64, b: f64, rel: f64) -> bool {
        (a - b).abs() <= rel * b.abs()
    }

    /// A pole-zero dataset as the raw reader gives it for ngspice's file.
    fn pz_dataset(roots: &[(&str, f64, f64)]) -> Dataset {
        Dataset {
            title: "* test".into(),
            plotname: "Pole-Zero Analysis".into(),
            kind: AnalysisKind::PoleZero,
            axis: None,
            vectors: roots
                .iter()
                .map(|(name, re, im)| Vector {
                    name: (*name).into(),
                    quantity: Quantity::Voltage,
                    data: VectorData::Complex(vec![Complex::new(*re, *im)]),
                })
                .collect(),
            steps: vec![Step {
                range: 0..1,
                label: String::new(),
            }],
        }
    }

    #[test]
    fn series_rlc_pair_has_the_analytic_f0_and_q() {
        // R = 100, L = 10m, C = 100n: f0 = 1/(2 pi sqrt(LC)) = 5032.9 Hz,
        // Q = sqrt(L/C)/R = 3.1623. ngspice reports -5000 +/- j31225 rad/s.
        let (r, l, c) = (100.0f64, 10e-3f64, 100e-9f64);
        let w0 = 1.0 / (l * c).sqrt();
        let alpha = r / (2.0 * l);
        let wd = (w0 * w0 - alpha * alpha).sqrt();
        let p = Root::from_rad_per_s(Complex::new(-alpha, wd));
        let Shape::Complex { f0_hz, q, zeta } = p.shape() else {
            panic!("{:?}", p.shape());
        };
        let f0_want = 1.0 / (2.0 * PI * (l * c).sqrt());
        let q_want = (l / c).sqrt() / r;
        assert!(close(f0_hz, f0_want, 1e-12), "{f0_hz} vs {f0_want}");
        assert!(close(q, q_want, 1e-12), "{q} vs {q_want}");
        assert!(close(zeta, 1.0 / (2.0 * q_want), 1e-12));
        assert_eq!(p.q(), Some(q));
        assert_eq!(p.half(), Half::Left);
    }

    #[test]
    fn real_roots_origin_and_axis() {
        // RC low-pass, R = 1k, C = 159.15n: one pole at -1 kHz.
        let p = Root::from_rad_per_s(Complex::new(-1.0 / (1e3 * 159.15e-9), 0.0));
        assert!(p.is_real());
        let Shape::Real { corner_hz } = p.shape() else {
            panic!();
        };
        assert!(close(corner_hz, 1000.0, 1e-4), "{corner_hz}");
        assert_eq!(p.q(), None);
        assert!(close(p.f0_hz(), corner_hz, 1e-12));

        assert_eq!(Root::new(0.0, 0.0).shape(), Shape::Origin);
        assert_eq!(Root::new(0.0, 0.0).half(), Half::Axis);
        // A lossless LC: exactly on the axis, Q infinite.
        let lc = Root::new(0.0, 5032.9);
        assert_eq!(lc.half(), Half::Axis);
        assert!(
            matches!(lc.shape(), Shape::Complex { q, zeta, .. } if q.is_infinite() && zeta == 0.0)
        );
        assert_eq!(lc.q(), None);
        // Residue a billionth of the root's size is still on the axis.
        assert_eq!(Root::new(5e-6, 5032.9).half(), Half::Axis);
        // A growing pair has negative damping and a negative Q.
        let grow = Root::new(100.0, 1000.0);
        assert_eq!(grow.half(), Half::Right);
        assert!(grow.q().unwrap() < 0.0);
    }

    #[test]
    fn stability_classes() {
        let lhp = Root::new(-1000.0, 0.0);
        let rhp = Root::new(143.2, 0.0);
        let axis = Root::new(0.0, 0.0);
        assert_eq!(stability(&[]), Stability::Stable);
        assert_eq!(stability(&[lhp]), Stability::Stable);
        assert_eq!(stability(&[lhp, axis]), Stability::Marginal);
        assert_eq!(stability(&[lhp, axis, rhp]), Stability::Unstable);
        // A right half-plane zero does not make a circuit unstable.
        let pz = PoleZero {
            poles: vec![lhp],
            zeros: vec![rhp],
        };
        assert_eq!(pz.stability(), Stability::Stable);
        assert!(pz.report().contains("non-minimum phase"), "{}", pz.report());
    }

    #[test]
    fn conjugates_fold_into_pairs() {
        let roots = [
            Root::new(-795.8, 4969.6),
            Root::new(-795.8, -4969.6),
            Root::new(-100.0, 0.0),
            Root::new(-50.0, 20.0),
        ];
        let g = group(&roots);
        assert_eq!(g.len(), 3);
        assert!(g[0].pair && g[0].root.im_hz > 0.0);
        assert!(!g[1].pair && g[1].root.is_real());
        // A complex root without its conjugate stays on its own.
        assert!(!g[2].pair && !g[2].root.is_real());
    }

    #[test]
    fn roots_come_from_the_named_vectors_in_index_order() {
        let ds = pz_dataset(&[
            ("V(pole(2))", -5000.0, -31225.0),
            ("V(pole(1))", -5000.0, 31225.0),
            ("V(zero(1))", -1000.0, 0.0),
            ("pole(3)", -2000.0, 0.0),
            ("V(in)", 1.0, 0.0),
        ]);
        let pz = PoleZero::from_datasets(std::slice::from_ref(&ds)).unwrap();
        assert_eq!(pz.poles.len(), 3);
        assert!(pz.poles[0].im_hz > 0.0 && pz.poles[1].im_hz < 0.0);
        assert!(close(pz.poles[2].re_hz, -2000.0 / (2.0 * PI), 1e-12));
        assert_eq!(pz.zeros.len(), 1);
        assert!(close(pz.zeros[0].re_hz, -1000.0 / (2.0 * PI), 1e-12));
        assert_eq!(root_name("v(zero(12))"), Some((false, 12)));
        assert_eq!(root_name("V(out)"), None);
        assert_eq!(root_name("pole(x)"), None);
        let mut other = ds.clone();
        other.kind = AnalysisKind::Ac;
        assert!(PoleZero::from_datasets(&[other]).is_none());
    }

    #[test]
    fn ngspice_raw_file_to_report() {
        // ngspice 42's raw file for `.pz in 0 out 0 vol pz` on a series RLC
        // (R = 100, L = 10m, C = 100n), parsed by the raw reader.
        let mut bytes = b"Title: * rlc\nDate: Sat Oct 10 16:30:44  2026\nPlotname: Pole-Zero Analysis\nFlags: complex\nNo. Variables: 2\nNo. Points: 1       \nVariables:\n\t0\tv(pole(1))\tvoltage\n\t1\tv(pole(2))\tvoltage\nBinary:\n".to_vec();
        for v in [-5000.0f64, 31224.99, -5000.0, -31224.99] {
            bytes.extend(v.to_le_bytes());
        }
        let ds = crate::raw::read_raw(&bytes).unwrap();
        let pz = PoleZero::from_datasets(&ds).unwrap();
        assert_eq!(pz.poles.len(), 2);
        assert!(pz.zeros.is_empty());
        let report = pz.report();
        assert!(report.starts_with("Stable: every pole"), "{report}");
        assert!(
            report.contains("-795.8 Hz ± j4.97 kHz: complex pair, f0 = 5.033 kHz, Q = 3.162, damping ratio = 0.1581"),
            "{report}"
        );
        assert!(report.contains("Zeros: none"), "{report}");
    }

    #[test]
    fn reports_say_unstable_loudly() {
        let pz = PoleZero {
            poles: vec![Root::from_rad_per_s(Complex::new(900.0, 0.0))],
            zeros: vec![],
        };
        let report = pz.report();
        assert!(report.starts_with("UNSTABLE: 1 of 1 poles"), "{report}");
        assert!(
            report
                .contains("  +143.2 Hz: real, corner at 143.2 Hz. UNSTABLE: right half-plane pole"),
            "{report}"
        );
        let flat = PoleZero::default();
        assert!(flat.report().contains("No finite poles or zeros"));
        let hp = PoleZero {
            poles: vec![Root::new(-159.2, 0.0)],
            zeros: vec![Root::new(0.0, 0.0)],
        };
        assert!(hp.report().contains("0 Hz: at the origin (DC is blocked)"));
    }
}
