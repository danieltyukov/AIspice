//! Write a gallery of plots from synthetic results to `target/plot-check/`,
//! for checking the look by eye:
//!
//! ```text
//! cargo run -p aispice-sim --example plot_gallery
//! rsvg-convert -w 1520 target/plot-check/bode_loop.svg -o /tmp/bode.png
//! ```

use aispice_sim::measure::{Measure, measure_dataset};
use aispice_sim::plot::{Marker, Pane, PlotKind, PlotRequest, Theme, markers_for, plot_svg};
use aispice_sim::{AnalysisKind, Complex, Dataset, Quantity, Step, Vector, VectorData};
use std::f64::consts::PI;
use std::path::PathBuf;

fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| a + (b - a) * i as f64 / (n - 1) as f64)
        .collect()
}

fn logspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    let (la, lb) = (a.log10(), b.log10());
    (0..n)
        .map(|i| 10f64.powf(la + (lb - la) * i as f64 / (n - 1) as f64))
        .collect()
}

fn vector(name: &str, data: VectorData) -> Vector {
    let quantity = if name.starts_with("I(") {
        Quantity::Current
    } else {
        Quantity::Voltage
    };
    Vector {
        name: name.into(),
        quantity,
        data,
    }
}

/// One step: its label, the independent variable, and named columns.
type StepColumns<'a> = (String, Vec<f64>, Vec<(&'a str, VectorData)>);

/// A dataset from per-step columns.
fn dataset(kind: AnalysisKind, steps: Vec<StepColumns<'_>>) -> Dataset {
    let (axis_name, axis_q, plotname) = match kind {
        AnalysisKind::Ac => ("frequency", Quantity::Frequency, "AC Analysis"),
        _ => ("time", Quantity::Time, "Transient Analysis"),
    };
    let mut vectors = vec![Vector {
        name: axis_name.into(),
        quantity: axis_q,
        data: VectorData::Real(Vec::new()),
    }];
    let mut ranges = Vec::new();
    let mut start = 0;
    for (k, (label, axis, cols)) in steps.into_iter().enumerate() {
        let n = axis.len();
        if let VectorData::Real(v) = &mut vectors[0].data {
            v.extend(axis);
        }
        for (i, (name, data)) in cols.into_iter().enumerate() {
            if k == 0 {
                vectors.push(vector(
                    name,
                    match data {
                        VectorData::Real(_) => VectorData::Real(Vec::new()),
                        VectorData::Complex(_) => VectorData::Complex(Vec::new()),
                    },
                ));
            }
            match (&mut vectors[i + 1].data, data) {
                (VectorData::Real(d), VectorData::Real(s)) => d.extend(s),
                (VectorData::Complex(d), VectorData::Complex(s)) => d.extend(s),
                _ => panic!("mixed vector kinds"),
            }
        }
        ranges.push(Step {
            range: start..start + n,
            label,
        });
        start += n;
    }
    Dataset {
        title: "gallery".into(),
        plotname: plotname.into(),
        kind,
        axis: Some(0),
        vectors,
        steps: ranges,
    }
}

fn poles(f: f64, poles_hz: &[f64], gain: f64) -> Complex {
    poles_hz
        .iter()
        .fold(Complex::new(gain, 0.0), |h, p| h / Complex::new(1.0, f / p))
}

fn second_order_step(t: &[f64], wn: f64, zeta: f64) -> Vec<f64> {
    t.iter()
        .map(|&t| {
            if zeta < 1.0 {
                let wd = wn * (1.0 - zeta * zeta).sqrt();
                let phi = (1.0 - zeta * zeta).sqrt().atan2(zeta);
                1.0 - (-zeta * wn * t).exp() / (1.0 - zeta * zeta).sqrt() * (wd * t + phi).sin()
            } else {
                1.0 - (1.0 + wn * t) * (-wn * t).exp()
            }
        })
        .collect()
}

fn main() {
    let out = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"))
        .join("plot-check");
    std::fs::create_dir_all(&out).expect("create output directory");
    let mut written = Vec::new();
    let mut save = |name: &str, svg: String| {
        let path = out.join(name);
        std::fs::write(&path, &svg).expect("write svg");
        written.push((path, svg.len()));
    };

    // 1. First-order step with rise-time and settling markers.
    let t = linspace(0.0, 6e-3, 1201);
    let rc: Vec<f64> = t.iter().map(|t| 3.3 * (1.0 - (-t / 1e-3).exp())).collect();
    let ds = dataset(
        AnalysisKind::Transient,
        vec![(
            String::new(),
            t.clone(),
            vec![("V(out)", VectorData::Real(rc))],
        )],
    );
    let mut req = PlotRequest::new(["V(out)"]);
    req.title = Some("RC charging, tau = 1ms".into());
    for (name, text) in [
        ("rise", "rise_time(V(out))"),
        ("settle", "settling_time(V(out), 1)"),
    ] {
        let m = Measure::parse(text).unwrap();
        req.markers
            .extend(markers_for(&m, &measure_dataset(name, &m, &ds)));
    }
    save("rc_step.svg", plot_svg(&[ds], &req).unwrap());

    // 2. Second-order step, stepped damping.
    let t = linspace(0.0, 4e-3, 2001);
    let wn = 2.0 * PI * 1e3;
    let steps = [0.15, 0.3, 0.5, 0.7, 1.0]
        .iter()
        .map(|&z| {
            (
                format!("zeta={z}"),
                t.clone(),
                vec![("V(out)", VectorData::Real(second_order_step(&t, wn, z)))],
            )
        })
        .collect();
    let ds = dataset(AnalysisKind::Transient, steps);
    let mut req = PlotRequest::new(["V(out)"]);
    req.title = Some("Second-order step response".into());
    req.markers.push(Marker::Y {
        y: 1.0,
        label: Some("final value".into()),
        pane: Pane::Main,
    });
    save("second_order_steps.svg", plot_svg(&[ds], &req).unwrap());

    // 3. Bode of a three-pole loop gain with margin markers.
    let f = logspace(1.0, 1e9, 481);
    let lg: Vec<Complex> = f
        .iter()
        .map(|&f| poles(f, &[10.0, 1e6, 1e7], 1e4))
        .collect();
    let ds = dataset(
        AnalysisKind::Ac,
        vec![(
            String::new(),
            f.clone(),
            vec![("V(lg)", VectorData::Complex(lg))],
        )],
    );
    let mut req = PlotRequest::new(["V(lg)"]);
    req.title = Some("Loop gain".into());
    for (name, text) in [("pm", "phase_margin(V(lg))"), ("gm", "gain_margin(V(lg))")] {
        let m = Measure::parse(text).unwrap();
        req.markers
            .extend(markers_for(&m, &measure_dataset(name, &m, &ds)));
    }
    req.markers.push(Marker::Y {
        y: 0.0,
        label: None,
        pane: Pane::Main,
    });
    req.markers.push(Marker::Y {
        y: -180.0,
        label: Some("-180°".into()),
        pane: Pane::Phase,
    });
    save(
        "bode_loop.svg",
        plot_svg(std::slice::from_ref(&ds), &req).unwrap(),
    );
    req.theme = Theme::Dark;
    save("bode_loop_dark.svg", plot_svg(&[ds], &req).unwrap());

    // 4. RC lowpass, stepped capacitor, with the corner of the first step.
    let f = logspace(10.0, 10e6, 301);
    let steps: Vec<_> = [1e-9, 2.2e-9, 4.7e-9, 10e-9]
        .iter()
        .map(|&c| {
            let fc = 1.0 / (2.0 * PI * 10e3 * c);
            let h: Vec<Complex> = f.iter().map(|&f| poles(f, &[fc], 1.0)).collect();
            (
                format!("C1={}", aispice_core::units::format(c)),
                f.clone(),
                vec![("V(out)", VectorData::Complex(h))],
            )
        })
        .collect();
    let ds = dataset(AnalysisKind::Ac, steps);
    let mut req = PlotRequest::new(["V(out)"]);
    req.title = Some("RC lowpass, R1 = 10k".into());
    req.markers.push(Marker::Y {
        y: -3.0103,
        label: Some("-3 dB".into()),
        pane: Pane::Main,
    });
    save(
        "bode_stepped.svg",
        plot_svg(std::slice::from_ref(&ds), &req).unwrap(),
    );
    req.kind = PlotKind::Magnitude;
    req.title = Some("RC lowpass magnitude".into());
    save("magnitude_only.svg", plot_svg(&[ds], &req).unwrap());

    // 5. A long switching transient: 600k points, ripple and two spikes.
    let n = 600_000;
    let t = linspace(0.0, 2e-3, n);
    let mut v: Vec<f64> = t
        .iter()
        .map(|&t| {
            let ramp = 5.0 * (1.0 - (-t / 3e-4).exp());
            let phase = (t * 200e3).fract();
            ramp + 0.08
                * if phase < 0.4 {
                    phase / 0.4 - 0.5
                } else {
                    (1.0 - phase) / 0.6 - 0.5
                }
        })
        .collect();
    v[250_000] += 1.2;
    v[420_000] -= 0.9;
    let il: Vec<f64> = t
        .iter()
        .map(|&t| {
            let phase = (t * 200e3).fract();
            let tri = if phase < 0.4 {
                phase / 0.4
            } else {
                (1.0 - phase) / 0.6
            };
            1.0 * (1.0 - (-t / 3e-4).exp()) + 0.6 * (tri - 0.5)
        })
        .collect();
    let ds = dataset(
        AnalysisKind::Transient,
        vec![(
            String::new(),
            t,
            vec![
                ("V(out)", VectorData::Real(v)),
                ("I(L1)", VectorData::Real(il)),
            ],
        )],
    );
    let mut req = PlotRequest::new(["V(out)"]);
    req.title = Some("Buck output, 600k points".into());
    save(
        "big_transient.svg",
        plot_svg(std::slice::from_ref(&ds), &req).unwrap(),
    );
    let mut req = PlotRequest::new(["I(L1)"]);
    req.title = Some("Inductor current, zoomed".into());
    req.x_range = Some([1.0e-3, 1.05e-3]);
    save("zoomed_current.svg", plot_svg(&[ds], &req).unwrap());

    // 6. Two traces: a square wave into an RC.
    let t = linspace(0.0, 4e-3, 4001);
    let tau = 1.5e-4;
    let mut vout = Vec::with_capacity(t.len());
    let mut y = 0.0;
    let vin: Vec<f64> = t
        .iter()
        .map(|&t| if (t * 1e3).fract() < 0.5 { 1.0 } else { -1.0 })
        .collect();
    for i in 0..t.len() {
        if i > 0 {
            let dt = t[i] - t[i - 1];
            y += (vin[i] - y) * (1.0 - (-dt / tau).exp());
        }
        vout.push(y);
    }
    let ds = dataset(
        AnalysisKind::Transient,
        vec![(
            String::new(),
            t,
            vec![
                ("V(in)", VectorData::Real(vin)),
                ("V(out)", VectorData::Real(vout)),
            ],
        )],
    );
    let mut req = PlotRequest::new(["V(in)", "V(out)"]);
    req.title = Some("Square wave through an RC".into());
    let m = Measure::parse("crossing(V(out), 0, rise, 2)").unwrap();
    req.markers
        .extend(markers_for(&m, &measure_dataset("t0", &m, &ds)));
    save("two_traces.svg", plot_svg(&[ds], &req).unwrap());

    // 7. Monte Carlo bundle: 60 runs of a second-order response.
    let t = linspace(0.0, 3e-3, 1501);
    let mut seed = 12345u64;
    let mut uniform = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let steps = (0..60)
        .map(|k| {
            let wn = 2.0 * PI * 1e3 * (1.0 + 0.1 * (uniform() - 0.5));
            let z = 0.45 * (1.0 + 0.3 * (uniform() - 0.5));
            (
                format!("mc{}", k + 1),
                t.clone(),
                vec![("V(out)", VectorData::Real(second_order_step(&t, wn, z)))],
            )
        })
        .collect();
    let ds = dataset(AnalysisKind::Transient, steps);
    let mut req = PlotRequest::new(["V(out)"]);
    req.title = Some("Monte Carlo, 60 runs".into());
    save("monte_carlo.svg", plot_svg(&[ds], &req).unwrap());

    for (path, len) in written {
        println!("{} ({} bytes)", path.display(), len);
    }
}
