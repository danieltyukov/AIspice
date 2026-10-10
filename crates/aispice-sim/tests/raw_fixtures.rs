//! The raw reader against real files written by LTspice XVII, ngspice 42 and
//! Xyce 7.10 for the decks in `fixtures/decks`, checked against the circuits'
//! analytic answers. `fixtures/regenerate.sh` rebuilds the files.

use aispice_sim::raw::{apply_step_labels, read_raw};
use aispice_sim::{AnalysisKind, Dataset, Quantity};
use std::f64::consts::PI;
use std::path::PathBuf;

fn fixture(rel: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn one(rel: &str) -> Dataset {
    let mut ds = read_raw(&fixture(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    assert_eq!(ds.len(), 1, "{rel}: expected one plot");
    ds.remove(0)
}

fn real(ds: &Dataset, name: &str) -> Vec<f64> {
    ds.vector(name)
        .unwrap_or_else(|| panic!("no vector {name} in {:?}", ds.names()))
        .data
        .real()
}

/// First-order RC step response with tau = 1 ms.
fn rc_step(t: f64, tau: f64) -> f64 {
    1.0 - (-t / tau).exp()
}

/// |H(f)| of the RC low-pass with R = 1k, C = 100n.
fn rc_mag(f: f64) -> f64 {
    let fc = 1.0 / (2.0 * PI * 1e3 * 100e-9);
    1.0 / (1.0 + (f / fc).powi(2)).sqrt()
}

fn assert_rc_tran(ds: &Dataset, out: &str, tol: f64) {
    assert_eq!(ds.kind, AnalysisKind::Transient);
    assert_eq!(ds.axis, Some(0));
    let axis = ds.axis_vector().unwrap();
    assert_eq!(axis.name, "time");
    assert_eq!(axis.quantity, Quantity::Time);
    let t = axis.data.real();
    let v = real(ds, out);
    assert!(t.windows(2).all(|w| w[1] > w[0]), "time must increase");
    for (t, v) in t.iter().zip(&v) {
        assert!(
            (v - rc_step(*t, 1e-3)).abs() < tol,
            "V(out)({t}) = {v}, want {}",
            rc_step(*t, 1e-3)
        );
    }
}

fn assert_rc_ac(ds: &Dataset, out: &str, points: usize) {
    assert_eq!(ds.kind, AnalysisKind::Ac);
    assert_eq!(ds.len(), points);
    let f = ds.axis_vector().unwrap();
    assert_eq!(f.name, "frequency");
    assert_eq!(f.quantity, Quantity::Frequency);
    let f = f.data.real();
    assert!((f[0] - 10.0).abs() < 1e-9);
    assert!((f[points - 1] - 100e3).abs() < 1e-6);
    let v = ds.vector(out).unwrap().data.as_complex();
    for (f, v) in f.iter().zip(&v) {
        let want = rc_mag(*f);
        assert!(
            (v.abs() - want).abs() < 1e-6,
            "|V(out)|({f}) = {}, want {want}",
            v.abs()
        );
        let phase = -(f / (1.0 / (2.0 * PI * 1e-4))).atan().to_degrees();
        assert!((v.phase_deg() - phase).abs() < 1e-4);
    }
}

// LTspice

#[test]
fn ltspice_transient_compressed() {
    let ds = one("ltspice/rc_tran.raw");
    assert_eq!(ds.len(), 23);
    assert_eq!(
        ds.names(),
        vec!["time", "V(in)", "V(out)", "I(C1)", "I(R1)", "I(V1)"]
    );
    assert_eq!(ds.vector("I(R1)").unwrap().quantity, Quantity::Current);
    assert_eq!(ds.title, "RC transient");
    assert!(!ds.is_stepped());
    assert_rc_tran(&ds, "out", 2e-3);
}

#[test]
fn ltspice_transient_double_precision() {
    let ds = one("ltspice/rc_tran_double.raw");
    assert_eq!(ds.len(), 80);
    // With 100 us steps the integration error near t = 0 is about 1e-3; the
    // precision of the file itself shows in the exact current check below.
    assert_rc_tran(&ds, "V(out)", 2e-3);
    // R1 carries the capacitor current: (1 - V(out)) / 1k.
    let i = real(&ds, "I(R1)");
    let vin = real(&ds, "V(in)");
    let v = real(&ds, "V(out)");
    for k in 1..ds.len() {
        let want = (vin[k] - v[k]) / 1e3;
        assert!(
            (i[k] - want).abs() < 1e-12,
            "I(R1)[{k}] = {}, want {want}",
            i[k]
        );
    }
}

#[test]
fn ltspice_transient_offset_start_time() {
    let ds = one("ltspice/rc_tran_offset.raw");
    assert_eq!(ds.len(), 251);
    let t = ds.axis_vector().unwrap().data.real();
    assert!((t[0] - 1e-3).abs() < 1e-12, "starts at {}", t[0]);
    assert!((t[t.len() - 1] - 2e-3).abs() < 1e-12);
    assert_rc_tran(&ds, "out", 2e-4);
}

#[test]
fn ltspice_op_files() {
    let ds = one("ltspice/div_op.raw");
    assert_eq!(ds.kind, AnalysisKind::Op);
    assert_eq!(ds.axis, None);
    assert_eq!(ds.len(), 1);
    assert_eq!(real(&ds, "out"), vec![7.5]);
    assert_eq!(real(&ds, "in"), vec![10.0]);
    assert!((real(&ds, "I(R1)")[0] - 2.5e-3).abs() < 1e-9);
    assert!((real(&ds, "I(V1)")[0] + 2.5e-3).abs() < 1e-9);

    // The .op that LTspice writes beside a transient run.
    let op = one("ltspice/op_tran.op.raw");
    assert_eq!(op.kind, AnalysisKind::Op);
    assert_eq!(real(&op, "out"), vec![1.0]);
    let tran = one("ltspice/op_tran.raw");
    assert_eq!(tran.kind, AnalysisKind::Transient);
    assert_eq!(tran.len(), 3);
    assert!(real(&tran, "out").iter().all(|v| (v - 1.0).abs() < 1e-6));
}

#[test]
fn ltspice_ac_binary_and_ascii_agree() {
    let bin = one("ltspice/rc_ac.raw");
    assert_rc_ac(&bin, "V(out)", 41);
    let asc = one("ltspice/rc_ac_ascii.raw");
    assert_rc_ac(&asc, "V(out)", 41);
    let a = bin.vector("out").unwrap().data.as_complex();
    let b = asc.vector("out").unwrap().data.as_complex();
    for (a, b) in a.iter().zip(&b) {
        assert!((*a - *b).abs() < 1e-12);
    }
}

#[test]
fn ltspice_dc_sweep() {
    let ds = one("ltspice/div_dc.raw");
    assert_eq!(ds.kind, AnalysisKind::Dc);
    assert_eq!(ds.len(), 11);
    let axis = ds.axis_vector().unwrap();
    assert_eq!(axis.name, "v1");
    assert_eq!(axis.quantity, Quantity::Sweep);
    let sweep = axis.data.real();
    let out = real(&ds, "out");
    for (s, o) in sweep.iter().zip(&out) {
        assert!((o - 0.75 * s).abs() < 1e-6);
    }
}

#[test]
fn ltspice_stepped_transient_and_fastaccess() {
    let mut ds = one("ltspice/rc_step.raw");
    assert_eq!(ds.len(), 110);
    assert_eq!(ds.steps.len(), 3);
    assert_eq!(ds.steps[0].label, "step 1");
    apply_step_labels(
        &mut ds,
        &["r=1000".into(), "r=2000".into(), "r=4000".into()],
    );
    assert_eq!(ds.steps[2].label, "r=4000");
    let t = ds.axis_vector().unwrap().data.real();
    let v = real(&ds, "out");
    for (step, tau) in ds.steps.iter().zip([1e-3, 2e-3, 4e-3]) {
        assert_eq!(t[step.range.start], 0.0);
        assert!((t[step.range.end - 1] - 5e-3).abs() < 1e-12);
        for i in step.range.clone() {
            assert!((v[i] - rc_step(t[i], tau)).abs() < 2e-3);
        }
    }

    let fast = one("ltspice/rc_step_fast.raw");
    assert_eq!(
        fast.steps,
        ds.steps
            .iter()
            .enumerate()
            .map(|(k, s)| aispice_sim::Step {
                range: s.range.clone(),
                label: format!("step {}", k + 1),
            })
            .collect::<Vec<_>>()
    );
    for (a, b) in fast.vectors.iter().zip(&ds.vectors) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.data, b.data);
    }
}

#[test]
fn ltspice_stepped_op() {
    let ds = one("ltspice/div_step_op.raw");
    assert_eq!(ds.kind, AnalysisKind::Op);
    assert_eq!(ds.axis, None);
    assert_eq!(ds.len(), 2);
    let labels: Vec<_> = ds.steps.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, vec!["r=1k", "r=3k"]);
    assert_eq!(ds.vectors[0].quantity, Quantity::Sweep);
    assert_eq!(real(&ds, "out"), vec![7.5, 5.0]);
}

#[test]
fn ltspice_stepped_ac() {
    let ds = one("ltspice/rc_ac_step.raw");
    assert_eq!(ds.len(), 42);
    assert_eq!(ds.steps.len(), 2);
    assert_eq!(ds.steps[1].range, 21..42);
    let f = ds.axis_vector().unwrap().data.real();
    let v = ds.vector("out").unwrap().data.as_complex();
    for (step, r) in ds.steps.iter().zip([1e3, 2e3]) {
        let fc = 1.0 / (2.0 * PI * r * 100e-9);
        for i in step.range.clone() {
            let want = 1.0 / (1.0 + (f[i] / fc).powi(2)).sqrt();
            assert!((v[i].abs() - want).abs() < 1e-6);
        }
    }
}

#[test]
fn ltspice_ascii_stepped_transient() {
    let ds = one("ltspice/rc_ascii.raw");
    assert_eq!(ds.steps.len(), 2);
    let t = ds.axis_vector().unwrap().data.real();
    let v = real(&ds, "out");
    for (step, tau) in ds.steps.iter().zip([1e-3, 2e-3]) {
        assert!((t[step.range.end - 1] - 1e-3).abs() < 1e-12);
        for i in step.range.clone() {
            assert!((v[i] - rc_step(t[i], tau)).abs() < 1e-3);
        }
    }
}

#[test]
fn ltspice_noise() {
    let ds = one("ltspice/rc_noise.raw");
    assert_eq!(ds.kind, AnalysisKind::Noise);
    assert_eq!(ds.len(), 21);
    assert_eq!(ds.axis_vector().unwrap().name, "frequency");
    assert_eq!(
        ds.names(),
        vec!["frequency", "gain", "V(r1)", "V(onoise)", "V(inoise)"]
    );
    // Thermal noise of 1k at 300.15 K, where the capacitor does not filter.
    let want = (4.0 * 1.380_649e-23 * 300.15 * 1e3f64).sqrt();
    let onoise = real(&ds, "V(onoise)");
    assert!(
        (onoise[0] - want).abs() / want < 1e-3,
        "{} vs {want}",
        onoise[0]
    );
}

#[test]
fn ltspice_transfer_function() {
    let ds = one("ltspice/div_tf.raw");
    assert_eq!(ds.kind, AnalysisKind::TransferFunction);
    assert_eq!(ds.axis, None);
    assert!((real(&ds, "Transfer_function")[0] - 0.75).abs() < 1e-6);
    assert!((real(&ds, "v1#Input_impedance")[0] - 4000.0).abs() < 1e-3);
    assert!((real(&ds, "output_impedance_at_V(out)")[0] - 750.0).abs() < 1e-3);
}

#[test]
fn ltspice_stepped_measurement_raw() {
    let ds = one("ltspice/rc_step.log.raw");
    assert_eq!(ds.kind, AnalysisKind::Other);
    assert_eq!(ds.names(), vec!["r", "vend", "vmax"]);
    assert_eq!(ds.axis_vector().unwrap().quantity, Quantity::Sweep);
    assert_eq!(real(&ds, "r"), vec![1000.0, 2000.0, 4000.0]);
    for (v, r) in real(&ds, "vend").iter().zip([1e3, 2e3, 4e3]) {
        assert!((v - rc_step(2e-3, r * 1e-6)).abs() < 1e-3);
    }

    let ac = one("ltspice/rc_ac_step.log.raw");
    let g = ac.vector("g1k").unwrap().data.as_complex();
    assert!((g[0].abs() - rc_mag(1e3)).abs() < 1e-6);
    let fc = real(&ac, "fc");
    assert!((fc[0] - 1591.55).abs() / 1591.55 < 2e-3);
    assert!((fc[1] - 795.77).abs() / 795.77 < 2e-2);
}

// ngspice

#[test]
fn ngspice_transient() {
    let ds = one("ngspice/rc_tran.raw");
    assert_eq!(ds.len(), 128);
    assert_eq!(ds.names(), vec!["time", "V(in)", "V(out)", "I(v1)"]);
    assert_eq!(ds.title, "rc transient");
    assert_rc_tran(&ds, "out", 2e-3);
}

#[test]
fn ngspice_ac_ignores_junk_imaginary_frequency() {
    let ds = one("ngspice/rc_ac.raw");
    assert_rc_ac(&ds, "out", 41);
}

#[test]
fn ngspice_op_has_no_axis() {
    let ds = one("ngspice/div_op.raw");
    assert_eq!(ds.kind, AnalysisKind::Op);
    assert_eq!(ds.axis, None);
    assert_eq!(ds.names(), vec!["V(in)", "V(out)", "I(v1)"]);
    assert!((real(&ds, "out")[0] - 7.5).abs() < 1e-12);
}

#[test]
fn ngspice_dc_sweep() {
    let ds = one("ngspice/div_dc.raw");
    assert_eq!(ds.kind, AnalysisKind::Dc);
    let axis = ds.axis_vector().unwrap();
    assert_eq!(axis.name, "v-sweep");
    assert_eq!(axis.quantity, Quantity::Sweep);
    let s = axis.data.real();
    let out = real(&ds, "out");
    assert_eq!(s.len(), 11);
    for (s, o) in s.iter().zip(&out) {
        assert!((o - 0.75 * s).abs() < 1e-12);
    }
}

#[test]
fn ngspice_several_plots_in_one_file() {
    for rel in ["ngspice/multi.raw", "ngspice/multi_ascii.raw"] {
        let ds = read_raw(&fixture(rel)).unwrap();
        let kinds: Vec<_> = ds.iter().map(|d| d.kind).collect();
        assert_eq!(
            kinds,
            vec![AnalysisKind::Ac, AnalysisKind::Op, AnalysisKind::Transient],
            "{rel}"
        );
        assert_eq!(ds[0].len(), 9);
        assert_eq!(ds[1].len(), 1);
        assert_eq!(ds[2].len(), 59);
        assert!((real(&ds[1], "out")[0] - 1.0).abs() < 1e-12);
        let f = ds[0].axis_vector().unwrap().data.real();
        let v = ds[0].vector("out").unwrap().data.as_complex();
        for (f, v) in f.iter().zip(&v) {
            assert!((v.abs() - rc_mag(*f)).abs() < 1e-9);
        }
    }
    let bin = read_raw(&fixture("ngspice/multi.raw")).unwrap();
    let asc = read_raw(&fixture("ngspice/multi_ascii.raw")).unwrap();
    for (a, b) in bin.iter().zip(&asc) {
        assert_eq!(a.names(), b.names());
        for (va, vb) in a.vectors.iter().zip(&b.vectors) {
            for (x, y) in va.data.as_complex().iter().zip(vb.data.as_complex()) {
                assert!((*x - y).abs() <= 1e-12 * x.abs().max(1.0));
            }
        }
    }
}

#[test]
fn ngspice_pole_zero() {
    use aispice_sim::polezero::{PoleZero, Shape};
    let ds = one("ngspice/rlc_pz.raw");
    assert_eq!(ds.kind, AnalysisKind::PoleZero);
    assert_eq!(ds.axis, None);
    assert_eq!(ds.names(), vec!["V(pole(1))", "V(pole(2))"]);
    let pz = PoleZero::from_dataset(&ds);
    assert!(pz.zeros.is_empty());
    let (l, c, r) = (10e-3f64, 100e-9f64, 100.0f64);
    for p in &pz.poles {
        let Shape::Complex { f0_hz, q, .. } = p.shape() else {
            panic!("{p:?}");
        };
        let f0 = 1.0 / (2.0 * PI * (l * c).sqrt());
        let q_want = (l / c).sqrt() / r;
        assert!((f0_hz - f0).abs() / f0 < 1e-6, "f0 {f0_hz}");
        assert!((q - q_want).abs() / q_want < 1e-6, "Q {q}");
    }

    let pz = PoleZero::from_dataset(&one("ngspice/lead_pz.raw"));
    assert_eq!(pz.poles.len(), 1);
    assert_eq!(pz.zeros.len(), 1);
    assert!((pz.poles[0].re_hz + 2000.0 / (2.0 * PI)).abs() < 1e-9);
    assert!((pz.zeros[0].re_hz + 1000.0 / (2.0 * PI)).abs() < 1e-9);
    assert_eq!(pz.poles[0].im_hz, 0.0);
}

// Xyce

#[test]
fn xyce_transient_binary_and_ascii() {
    for rel in ["xyce/rc_tran.raw", "xyce/rc_tran_ascii.raw"] {
        let ds = one(rel);
        assert_eq!(ds.len(), 65, "{rel}");
        assert_eq!(ds.names(), vec!["time", "V(IN)", "V(OUT)", "I(V1)"]);
        assert_rc_tran(&ds, "out", 2e-3);
    }
}

#[test]
fn xyce_ac_binary_and_ascii() {
    assert_rc_ac(&one("xyce/rc_ac.raw"), "out", 41);
    // ASCII keeps eight significant digits.
    let ds = one("xyce/rc_ac_ascii.raw");
    let f = ds.axis_vector().unwrap().data.real();
    let v = ds.vector("out").unwrap().data.as_complex();
    for (f, v) in f.iter().zip(&v) {
        assert!((v.abs() - rc_mag(*f)).abs() < 1e-7);
    }
}

#[test]
fn xyce_op_drops_dummy_sweep() {
    for rel in ["xyce/div_op.raw", "xyce/div_op_ascii.raw"] {
        let ds = one(rel);
        assert_eq!(ds.kind, AnalysisKind::Op, "{rel}");
        assert_eq!(ds.axis, None);
        assert_eq!(ds.names(), vec!["V(IN)", "V(OUT)", "I(V1)"]);
        assert!((real(&ds, "out")[0] - 7.5).abs() < 1e-9);
    }
}

#[test]
fn xyce_dc_sweep_names_the_source() {
    // Only the ASCII plot name carries the swept source; the binary one says
    // plain `DC transfer characteristic`, and the Xyce backend renames the
    // axis from the deck's `.dc` line.
    for (rel, name) in [
        ("xyce/div_dc.raw", "sweep"),
        ("xyce/div_dc_ascii.raw", "V1"),
    ] {
        let ds = one(rel);
        assert_eq!(ds.kind, AnalysisKind::Dc);
        let axis = ds.axis_vector().unwrap();
        assert_eq!(axis.name, name);
        assert_eq!(axis.quantity, Quantity::Sweep);
        let s = axis.data.real();
        for (s, o) in s.iter().zip(real(&ds, "out")) {
            assert!((o - 0.75 * s).abs() < 1e-9);
        }
    }
}

#[test]
fn xyce_step_plots_merge_into_one_stepped_dataset() {
    for rel in ["xyce/xyce_step.raw", "xyce/xyce_step_ascii.raw"] {
        let ds = one(rel);
        assert_eq!(ds.kind, AnalysisKind::Dc);
        assert_eq!(ds.len(), 6);
        let labels: Vec<_> = ds.steps.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["R=1k", "R=3k"], "{rel}");
        let out = real(&ds, "out");
        assert!((out[2] - 7.5).abs() < 1e-9);
        assert!((out[5] - 5.0).abs() < 1e-9);
    }
}

#[test]
fn every_committed_raw_file_reads() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut count = 0;
    for sim in ["ltspice", "ngspice", "xyce"] {
        let mut files: Vec<_> = std::fs::read_dir(root.join(sim))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "raw"))
            .collect();
        files.sort();
        for f in files {
            let ds = read_raw(&std::fs::read(&f).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
            for d in &ds {
                assert!(!d.is_empty(), "{}: empty plot", f.display());
                assert!(d.vectors.iter().all(|v| v.data.len() == d.len()));
                assert_eq!(d.steps.last().unwrap().range.end, d.len());
            }
            count += 1;
        }
    }
    assert!(count >= 40, "only {count} raw files");
}
