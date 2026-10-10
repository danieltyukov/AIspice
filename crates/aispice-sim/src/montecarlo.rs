//! Tolerance analysis: Monte Carlo yield and worst-case corners.
//!
//! A design that meets its specs at nominal values can still fail on the
//! bench with 1% resistors and 10% capacitors, and this is exactly where a
//! language model reasoning alone is weakest. This module writes one netlist
//! per run with each toleranced value drawn at random (or pushed to a
//! corner), leaves running them to the caller, and turns the per-run spec
//! reports into a yield with a confidence interval.
//!
//! Sampling is deterministic for a seed and stable under edits: each target
//! draws from its own stream, seeded from the run seed and the target's name,
//! so adding a tolerance on `C1` does not change the values drawn for `R1`,
//! and asking for more runs keeps the first ones identical.
//!
//! The random number generator is a small xoshiro256** written out here
//! rather than a dependency, so a version bump can never change which
//! circuits a saved seed reproduces.

use crate::spec::SpecReport;
use crate::sweep::{self, SweepError};
use aispice_core::netlist::Netlist;
use aispice_core::units;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Distribution {
    /// Equally likely anywhere within plus or minus the tolerance.
    #[default]
    Uniform,
    /// Normal with three standard deviations equal to the tolerance, as
    /// component tolerances are usually specified.
    Gaussian,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Tolerance {
    /// An element or `.param` name, or a glob over element names such as
    /// `R*`, `C?` or `*`. Exact names take precedence over globs, and
    /// earlier globs over later ones.
    pub target: String,
    /// Tolerance in percent of the nominal value.
    pub tol_pct: f64,
    #[serde(default)]
    pub distribution: Distribution,
}

/// A tolerance matched to one value in the netlist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Resolved {
    pub target: String,
    pub nominal: f64,
    pub tol_pct: f64,
    pub distribution: Distribution,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sample {
    pub target: String,
    pub nominal: f64,
    pub value: f64,
}

/// One netlist to simulate, with the values that went into it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variant {
    pub label: String,
    pub netlist: Netlist,
    pub samples: Vec<Sample>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum McError {
    #[error(transparent)]
    Netlist(#[from] SweepError),
    #[error("tolerance for {target} is {tol_pct}%; it must be finite and between 0 and 100")]
    BadTolerance { target: String, tol_pct: f64 },
    #[error("no toleranced values: none of {0} matched an element or .param with a numeric value")]
    NothingMatched(String),
    #[error(
        "{count} toleranced values give 2^{count} corners, over the limit of 2^{max}; use sensitivity_probes and ranked_corners"
    )]
    TooManyForCorners { count: usize, max: usize },
    #[error("expected {expected} metrics (nominal first, then one per probe), got {got}")]
    MetricCount { expected: usize, got: usize },
}

/// A seedable xoshiro256** generator.
#[derive(Debug, Clone)]
pub struct Rng {
    s: [u64; 4],
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut st = seed;
        Rng {
            s: [
                splitmix(&mut st),
                splitmix(&mut st),
                splitmix(&mut st),
                splitmix(&mut st),
            ],
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Standard normal, by the polar method.
    pub fn normal(&mut self) -> f64 {
        loop {
            let u = 2.0 * self.next_f64() - 1.0;
            let v = 2.0 * self.next_f64() - 1.0;
            let s = u * u + v * v;
            if s > 0.0 && s < 1.0 {
                return u * (-2.0 * s.ln() / s).sqrt();
            }
        }
    }
}

fn fnv1a(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.to_ascii_uppercase().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn is_glob(s: &str) -> bool {
    s.contains(['*', '?'])
}

/// Case-insensitive glob with `*` and `?`.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_ascii_uppercase().chars().collect();
    let n: Vec<char> = name.to_ascii_uppercase().chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ni));
            pi += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// Match tolerances to netlist values. Globs skip elements without a
/// numeric value (a diode's model name, a `SINE(...)` source); an exact name
/// that is not numeric is an error, since it was asked for by name.
pub fn resolve(netlist: &Netlist, tolerances: &[Tolerance]) -> Result<Vec<Resolved>, McError> {
    for t in tolerances {
        if !(t.tol_pct.is_finite() && (0.0..=100.0).contains(&t.tol_pct)) {
            return Err(McError::BadTolerance {
                target: t.target.clone(),
                tol_pct: t.tol_pct,
            });
        }
    }
    let mut out: Vec<Resolved> = Vec::new();
    let taken =
        |out: &Vec<Resolved>, name: &str| out.iter().any(|r| r.target.eq_ignore_ascii_case(name));
    for t in tolerances.iter().filter(|t| !is_glob(&t.target)) {
        if taken(&out, &t.target) {
            continue;
        }
        let nominal = sweep::get_value(netlist, &t.target)?;
        let name = netlist
            .element(&t.target)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| t.target.clone());
        out.push(Resolved {
            target: name,
            nominal,
            tol_pct: t.tol_pct,
            distribution: t.distribution,
        });
    }
    for t in tolerances.iter().filter(|t| is_glob(&t.target)) {
        for e in netlist.elements() {
            if !glob_match(&t.target, &e.name) || taken(&out, &e.name) {
                continue;
            }
            if let Ok(nominal) = sweep::get_value(netlist, &e.name) {
                out.push(Resolved {
                    target: e.name.clone(),
                    nominal,
                    tol_pct: t.tol_pct,
                    distribution: t.distribution,
                });
            }
        }
    }
    if out.is_empty() {
        let names: Vec<&str> = tolerances.iter().map(|t| t.target.as_str()).collect();
        return Err(McError::NothingMatched(if names.is_empty() {
            "(no tolerances)".into()
        } else {
            names.join(", ")
        }));
    }
    Ok(out)
}

/// Draw `n` values for one target from its own stream.
fn draws(r: &Resolved, n: usize, seed: u64) -> Vec<f64> {
    let mut rng = Rng::new(seed ^ fnv1a(&r.target));
    let t = r.tol_pct / 100.0;
    (0..n)
        .map(|_| match r.distribution {
            Distribution::Uniform => r.nominal * (1.0 + t * (2.0 * rng.next_f64() - 1.0)),
            Distribution::Gaussian => loop {
                // Untruncated, except that a draw which would flip the sign
                // of the value (possible only for very wide tolerances) is
                // drawn again.
                let k = 1.0 + t / 3.0 * rng.normal();
                if k > 0.0 {
                    break r.nominal * k;
                }
            },
        })
        .collect()
}

fn build(
    netlist: &Netlist,
    label: String,
    values: &[(&Resolved, f64)],
) -> Result<Variant, McError> {
    let mut n = netlist.clone();
    let mut samples = Vec::with_capacity(values.len());
    for (r, v) in values {
        sweep::set_value(&mut n, &r.target, *v)?;
        samples.push(Sample {
            target: r.target.clone(),
            nominal: r.nominal,
            value: *v,
        });
    }
    Ok(Variant {
        label,
        netlist: n,
        samples,
    })
}

/// `n` Monte Carlo variants, labelled `mc1` to `mcN`.
pub fn variants(
    netlist: &Netlist,
    tolerances: &[Tolerance],
    n: usize,
    seed: u64,
) -> Result<Vec<Variant>, McError> {
    let resolved = resolve(netlist, tolerances)?;
    let columns: Vec<Vec<f64>> = resolved.iter().map(|r| draws(r, n, seed)).collect();
    (0..n)
        .map(|run| {
            let values: Vec<(&Resolved, f64)> = resolved
                .iter()
                .zip(&columns)
                .map(|(r, col)| (r, col[run]))
                .collect();
            build(netlist, format!("mc{}", run + 1), &values)
        })
        .collect()
}

fn corner_label(signs: &[(&Resolved, i8)]) -> String {
    let parts: Vec<String> = signs
        .iter()
        .filter(|(_, s)| *s != 0)
        .map(|(r, s)| format!("{}{}", r.target, if *s > 0 { '+' } else { '-' }))
        .collect();
    if parts.is_empty() {
        "nominal".into()
    } else {
        parts.join(" ")
    }
}

fn at_sign(r: &Resolved, sign: i8) -> f64 {
    r.nominal * (1.0 + f64::from(sign) * r.tol_pct / 100.0)
}

/// Every combination of plus and minus tolerance: `2^k` variants for `k`
/// toleranced values, refused beyond `2^max_params`.
pub fn corners(
    netlist: &Netlist,
    tolerances: &[Tolerance],
    max_params: usize,
) -> Result<Vec<Variant>, McError> {
    let resolved = resolve(netlist, tolerances)?;
    let k = resolved.len();
    if k > max_params || k >= 63 {
        return Err(McError::TooManyForCorners {
            count: k,
            max: max_params,
        });
    }
    enumerate_corners(netlist, &resolved, &(0..k).collect::<Vec<_>>(), &vec![0; k])
}

fn enumerate_corners(
    netlist: &Netlist,
    resolved: &[Resolved],
    free: &[usize],
    fixed: &[i8],
) -> Result<Vec<Variant>, McError> {
    let mut out = Vec::with_capacity(1 << free.len());
    for mask in 0u64..(1u64 << free.len()) {
        let mut signs = fixed.to_vec();
        for (bit, &i) in free.iter().enumerate() {
            signs[i] = if mask & (1 << bit) != 0 { 1 } else { -1 };
        }
        let pairs: Vec<(&Resolved, i8)> = resolved.iter().zip(signs.iter().copied()).collect();
        let values: Vec<(&Resolved, f64)> =
            pairs.iter().map(|(r, s)| (*r, at_sign(r, *s))).collect();
        out.push(build(netlist, corner_label(&pairs), &values)?);
    }
    Ok(out)
}

/// The nominal circuit followed by one variant per toleranced value at its
/// plus tolerance, everything else nominal. Simulate them, reduce each to
/// one number where larger is better (the smallest spec margin is the usual
/// choice), and pass the numbers to [`ranked_corners`].
pub fn sensitivity_probes(
    netlist: &Netlist,
    tolerances: &[Tolerance],
) -> Result<Vec<Variant>, McError> {
    let resolved = resolve(netlist, tolerances)?;
    let mut out = Vec::with_capacity(resolved.len() + 1);
    let nominal: Vec<(&Resolved, f64)> = resolved.iter().map(|r| (r, r.nominal)).collect();
    out.push(build(netlist, "nominal".into(), &nominal)?);
    for i in 0..resolved.len() {
        let values: Vec<(&Resolved, f64)> = resolved
            .iter()
            .enumerate()
            .map(|(j, r)| (r, if i == j { at_sign(r, 1) } else { r.nominal }))
            .collect();
        out.push(build(netlist, format!("{}+", resolved[i].target), &values)?);
    }
    Ok(out)
}

/// Worst-case corners for many toleranced values. The `max_params` most
/// sensitive values (by the probe results) are enumerated at both signs;
/// every other one is held at the sign that made the metric worse in its
/// probe, which is the worst case to first order.
pub fn ranked_corners(
    netlist: &Netlist,
    tolerances: &[Tolerance],
    metrics: &[f64],
    max_params: usize,
) -> Result<Vec<Variant>, McError> {
    let resolved = resolve(netlist, tolerances)?;
    let k = resolved.len();
    if metrics.len() != k + 1 {
        return Err(McError::MetricCount {
            expected: k + 1,
            got: metrics.len(),
        });
    }
    let nominal = metrics[0];
    let sens: Vec<f64> = metrics[1..].iter().map(|m| m - nominal).collect();
    let mut order: Vec<usize> = (0..k).collect();
    order.sort_by(|&a, &b| {
        let (sa, sb) = (sens[a].abs(), sens[b].abs());
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });
    let m = max_params.min(k).min(20);
    let mut free: Vec<usize> = order[..m].to_vec();
    free.sort_unstable();
    let fixed: Vec<i8> = (0..k)
        .map(|i| {
            let s = sens[i];
            if !s.is_finite() || s == 0.0 {
                0
            } else if s > 0.0 {
                -1
            } else {
                1
            }
        })
        .collect();
    enumerate_corners(netlist, &resolved, &free, &fixed)
}

/// Statistics of one spec across runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SpecStats {
    pub name: String,
    pub unit: String,
    /// Runs with a value.
    pub measured: usize,
    pub passed: usize,
    pub mean: Option<f64>,
    /// Sample standard deviation.
    pub std: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// The run (0-based) with the smallest margin, or with no value.
    pub worst_run: Option<usize>,
    pub worst_value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct YieldReport {
    pub runs: usize,
    pub passed: usize,
    pub yield_pct: f64,
    /// 95% Wilson interval on the yield, in percent: 100 of 100 passing
    /// still only shows the yield is very likely above 96%.
    pub yield_ci95: (f64, f64),
    pub specs: Vec<SpecStats>,
    /// The run (0-based) closest to failing, or failing worst.
    pub worst_run: Option<usize>,
    pub summary: String,
}

fn wilson(passed: usize, runs: usize) -> (f64, f64) {
    if runs == 0 {
        return (0.0, 100.0);
    }
    let n = runs as f64;
    let p = passed as f64 / n;
    let z = 1.959_963_984_540_054;
    let denom = 1.0 + z * z / n;
    let center = (p + z * z / (2.0 * n)) / denom;
    let half = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / denom;
    (
        ((center - half) * 100.0).max(0.0),
        ((center + half) * 100.0).min(100.0),
    )
}

/// Yield and per-spec statistics from the spec report of every run.
pub fn summarize(reports: &[SpecReport]) -> YieldReport {
    let runs = reports.len();
    let passed = reports.iter().filter(|r| r.all_pass).count();
    let yield_pct = if runs == 0 {
        0.0
    } else {
        passed as f64 / runs as f64 * 100.0
    };
    let mut specs = Vec::new();
    if let Some(first) = reports.first() {
        for row in &first.rows {
            let name = &row.name;
            let mut values = Vec::new();
            let mut pass_count = 0;
            let mut worst: Option<(usize, f64)> = None;
            for (i, rep) in reports.iter().enumerate() {
                let Some(r) = rep.row(name) else { continue };
                if r.pass {
                    pass_count += 1;
                }
                let key = match (r.value, r.margin) {
                    (None, _) => f64::NEG_INFINITY,
                    (Some(_), Some(m)) => m,
                    (Some(_), None) => f64::INFINITY,
                };
                if let Some(v) = r.value {
                    values.push(v);
                }
                if worst.is_none_or(|(_, k)| key < k) {
                    worst = Some((i, key));
                }
            }
            let n = values.len();
            let mean = (n > 0).then(|| values.iter().sum::<f64>() / n as f64);
            let std = mean.filter(|_| n > 1).map(|m| {
                (values.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt()
            });
            let worst_run = worst
                .filter(|(_, k)| k.is_finite() || *k == f64::NEG_INFINITY)
                .map(|(i, _)| i);
            specs.push(SpecStats {
                name: name.clone(),
                unit: row.unit.clone(),
                measured: n,
                passed: pass_count,
                mean,
                std,
                min: values.iter().copied().reduce(f64::min),
                max: values.iter().copied().reduce(f64::max),
                worst_run,
                worst_value: worst_run.and_then(|i| reports[i].row(name).and_then(|r| r.value)),
            });
        }
    }
    let worst_run = reports
        .iter()
        .enumerate()
        .map(|(i, r)| (i, r.worst_margin().unwrap_or(f64::NEG_INFINITY)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|(_, m)| m.is_finite() || *m == f64::NEG_INFINITY)
        .map(|(i, _)| i);
    let ci = wilson(passed, runs);
    let mut summary = format!(
        "yield {}% ({passed} of {runs} runs pass; 95% interval {}% to {}%)",
        round1(yield_pct),
        round1(ci.0),
        round1(ci.1)
    );
    let mut weakest: Vec<&SpecStats> = specs.iter().filter(|s| s.passed < runs).collect();
    weakest.sort_by_key(|s| s.passed);
    if let Some(s) = weakest.first() {
        summary.push_str(&format!(
            "; weakest spec: {} ({} of {runs} pass",
            s.name, s.passed
        ));
        if let (Some(m), Some(sd)) = (s.mean, s.std) {
            summary.push_str(&format!(
                ", mean {}, std {}",
                units::format_with_unit(m, &s.unit),
                units::format_with_unit(sd, &s.unit)
            ));
        }
        summary.push(')');
    }
    YieldReport {
        runs,
        passed,
        yield_pct,
        yield_ci95: ci,
        specs,
        worst_run,
        summary,
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Spec, evaluate, parse_specs};
    use crate::{dataset::Complex, expr::test_support::*};
    use aispice_core::netlist::parse;

    const DECK: &str = "RC\nV1 in 0 DC 1 AC 1\nR1 in out 1k\nR2 out 0 1Meg\nC1 out 0 100n\nD1 out 0 1N4148\n.param gain=2\n.end\n";

    fn tol(target: &str, pct: f64, d: Distribution) -> Tolerance {
        Tolerance {
            target: target.into(),
            tol_pct: pct,
            distribution: d,
        }
    }

    #[test]
    fn rng_is_deterministic_and_well_spread() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut r = Rng::new(7);
        let n = 200_000;
        let (mut sum, mut sq, mut usum) = (0.0, 0.0, 0.0);
        for _ in 0..n {
            let z = r.normal();
            sum += z;
            sq += z * z;
            usum += r.next_f64();
        }
        assert!((sum / n as f64).abs() < 0.01);
        assert!((sq / n as f64 - 1.0).abs() < 0.01);
        assert!((usum / n as f64 - 0.5).abs() < 0.005);
        // Pinned outputs: a change here means saved seeds no longer
        // reproduce the same circuits.
        let mut p = Rng::new(0);
        assert_eq!(p.next_u64(), 0x99ec_5f36_cb75_f2b4);
    }

    #[test]
    fn globs() {
        assert!(glob_match("R*", "R1"));
        assert!(glob_match("r*", "Rload"));
        assert!(glob_match("C?", "C1"));
        assert!(!glob_match("C?", "C12"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*1", "XU1"));
        assert!(glob_match("R*a*", "Rload"));
        assert!(!glob_match("R*", "C1"));
        assert!(glob_match("R1", "r1"));
    }

    #[test]
    fn resolves_globs_exact_names_and_params() {
        let n = parse(DECK);
        let r = resolve(
            &n,
            &[
                tol("R*", 1.0, Distribution::Uniform),
                tol("R2", 0.1, Distribution::Gaussian),
                tol("*", 10.0, Distribution::Uniform),
                tol("gain", 5.0, Distribution::Uniform),
            ],
        )
        .unwrap();
        let names: Vec<(&str, f64)> = r.iter().map(|r| (r.target.as_str(), r.tol_pct)).collect();
        // Exact names first, then globs in order; D1 has no numeric value.
        assert_eq!(
            names,
            vec![
                ("R2", 0.1),
                ("gain", 5.0),
                ("R1", 1.0),
                ("V1", 10.0),
                ("C1", 10.0)
            ]
        );
        assert!(matches!(
            resolve(&n, &[tol("D1", 5.0, Distribution::Uniform)]),
            Err(McError::Netlist(SweepError::NotNumeric { .. }))
        ));
        assert!(matches!(
            resolve(&n, &[tol("L*", 5.0, Distribution::Uniform)]),
            Err(McError::NothingMatched(_))
        ));
        assert!(matches!(
            resolve(&n, &[tol("R1", -1.0, Distribution::Uniform)]),
            Err(McError::BadTolerance { .. })
        ));
    }

    #[test]
    fn sampling_is_seeded_bounded_and_stable() {
        let n = parse(DECK);
        let tols = [
            tol("R1", 5.0, Distribution::Uniform),
            tol("C1", 10.0, Distribution::Gaussian),
        ];
        let a = variants(&n, &tols, 2000, 1).unwrap();
        let b = variants(&n, &tols, 2000, 1).unwrap();
        assert_eq!(a, b);
        let c = variants(&n, &tols, 2000, 2).unwrap();
        assert_ne!(a[0].samples, c[0].samples);
        assert_eq!(a[0].label, "mc1");
        let r1: Vec<f64> = a.iter().map(|v| v.samples[0].value).collect();
        let c1: Vec<f64> = a.iter().map(|v| v.samples[1].value).collect();
        assert!(r1.iter().all(|v| (950.0..=1050.0).contains(v)));
        let mean = r1.iter().sum::<f64>() / r1.len() as f64;
        assert!((mean - 1000.0).abs() < 5.0);
        // Uniform on +-5%: standard deviation 50 / sqrt(3).
        let sd = (r1.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / r1.len() as f64).sqrt();
        assert!((sd - 50.0 / 3f64.sqrt()).abs() < 2.0, "{sd}");
        // Gaussian with 3 sigma = 10%: sigma is 3.33% of 100n.
        let cm = c1.iter().sum::<f64>() / c1.len() as f64;
        let csd = (c1.iter().map(|v| (v - cm).powi(2)).sum::<f64>() / c1.len() as f64).sqrt();
        assert!((csd / 100e-9 - 0.1 / 3.0).abs() < 0.003, "{csd}");
        // The netlist carries the sampled value, read back exactly.
        assert_eq!(sweep::get_value(&a[7].netlist, "R1").unwrap(), r1[7]);
        // Adding a tolerance or more runs leaves existing draws alone.
        let more = variants(
            &n,
            &[
                tols[0].clone(),
                tols[1].clone(),
                tol("R2", 1.0, Distribution::Uniform),
            ],
            2500,
            1,
        )
        .unwrap();
        for i in 0..2000 {
            assert_eq!(more[i].samples[0].value, r1[i]);
            assert_eq!(more[i].samples[1].value, c1[i]);
        }
    }

    #[test]
    fn full_corners() {
        let n = parse(DECK);
        let tols = [
            tol("R1", 1.0, Distribution::Uniform),
            tol("C1", 10.0, Distribution::Uniform),
        ];
        let v = corners(&n, &tols, 10).unwrap();
        let labels: Vec<&str> = v.iter().map(|v| v.label.as_str()).collect();
        assert_eq!(labels, vec!["R1- C1-", "R1+ C1-", "R1- C1+", "R1+ C1+"]);
        assert_eq!(v[1].samples[0].value, 1010.0);
        assert!((v[1].samples[1].value - 90e-9).abs() < 1e-20);
        let many: Vec<Tolerance> = (0..3)
            .map(|_| tol("*", 1.0, Distribution::Uniform))
            .collect();
        assert!(matches!(
            corners(&n, &many, 2),
            Err(McError::TooManyForCorners { count: 4, max: 2 })
        ));
    }

    /// Lowpass corner of the RC as the "simulation": larger is better when
    /// the spec is a minimum bandwidth.
    fn corner_hz(v: &Variant) -> f64 {
        let r = sweep::get_value(&v.netlist, "R1").unwrap();
        let c = sweep::get_value(&v.netlist, "C1").unwrap();
        1.0 / (2.0 * std::f64::consts::PI * r * c)
    }

    #[test]
    fn ranked_corners_find_the_worst_case() {
        // Twelve resistors in a deck, but only R1 and C1 matter.
        let mut text = String::from("t\nR1 in out 1k\nC1 out 0 100n\n");
        for i in 0..10 {
            text.push_str(&format!("RX{i} n{i} 0 1k\n"));
        }
        let n = parse(&text);
        let tols = [tol("*", 5.0, Distribution::Uniform)];
        assert!(corners(&n, &tols, 10).is_err());
        let probes = sensitivity_probes(&n, &tols).unwrap();
        assert_eq!(probes.len(), 13);
        assert_eq!(probes[0].label, "nominal");
        let metrics: Vec<f64> = probes.iter().map(corner_hz).collect();
        let v = ranked_corners(&n, &tols, &metrics, 2).unwrap();
        assert_eq!(v.len(), 4);
        // The worst corner (lowest bandwidth) has R1 and C1 both high.
        let worst = v
            .iter()
            .min_by(|a, b| corner_hz(a).total_cmp(&corner_hz(b)))
            .unwrap();
        assert!(worst.label.starts_with("R1+ C1+"), "{}", worst.label);
        let want = 1.0 / (2.0 * std::f64::consts::PI * 1050.0 * 105e-9);
        assert!((corner_hz(worst) - want).abs() < 1e-6);
        // Insensitive values stay at nominal.
        assert!(!worst.label.contains("RX"));
        assert!(matches!(
            ranked_corners(&n, &tols, &metrics[..3], 2),
            Err(McError::MetricCount { .. })
        ));
    }

    #[test]
    fn yield_from_spec_reports() {
        // Monte Carlo of an RC lowpass against a bandwidth spec, with the
        // closed-form response standing in for the simulator.
        let n = parse(DECK);
        let tols = [
            tol("R1", 5.0, Distribution::Gaussian),
            tol("C1", 10.0, Distribution::Gaussian),
        ];
        let runs = variants(&n, &tols, 400, 99).unwrap();
        let specs: Vec<Spec> = parse_specs("bw = bandwidth_3db(V(out)) >= 1.55k").unwrap();
        let f = logspace(10.0, 1e6, 121);
        let reports: Vec<SpecReport> = runs
            .iter()
            .map(|v| {
                let fc = corner_hz(v);
                let h: Vec<Complex> = f.iter().map(|&f| poles(f, &[fc], 1.0)).collect();
                evaluate(&specs, &[ac(f.clone(), vec![("V(out)", h)])])
            })
            .collect();
        let y = summarize(&reports);
        assert_eq!(y.runs, 400);
        // Nominal corner 1.5915k; the spec sits about 0.75 sigma below it
        // (sigma of R*C is sqrt(1.667^2 + 3.333^2) = 3.73%), so roughly 77%
        // of runs pass.
        assert!(y.yield_pct > 65.0 && y.yield_pct < 88.0, "{}", y.yield_pct);
        assert!(y.yield_ci95.0 < y.yield_pct && y.yield_pct < y.yield_ci95.1);
        let bw = &y.specs[0];
        assert_eq!(bw.measured, 400);
        assert!((bw.mean.unwrap() - 1591.5).abs() < 15.0, "{:?}", bw.mean);
        let worst = y.worst_run.unwrap();
        assert_eq!(bw.worst_run, Some(worst));
        let min = bw.min.unwrap();
        assert_eq!(bw.worst_value, Some(min));
        assert!(y.summary.starts_with("yield "), "{}", y.summary);
        assert!(y.summary.contains("weakest spec: bw"), "{}", y.summary);
        let empty = summarize(&[]);
        assert_eq!(
            (empty.runs, empty.yield_pct, empty.yield_ci95),
            (0, 0.0, (0.0, 100.0))
        );
        let (lo, hi) = wilson(100, 100);
        assert!((lo - 96.3).abs() < 0.1 && hi == 100.0, "{lo}");
    }
}
