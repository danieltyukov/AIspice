//! Schematic to results on every available simulator: aispice's netlister,
//! model resolution and dialect translation, then ngspice, Xyce and LTspice,
//! compared against analytic values and against each other.
//!
//! ngspice and Xyce run when installed; LTspice needs
//! `AISPICE_LTSPICE_TESTS=1`. Each test prints its numbers per simulator.

mod common;

use aispice_sim::backend::{SimId, SimOutput, Simulator};
use aispice_sim::{AnalysisKind, Dataset};
use common::*;
use std::f64::consts::PI;

/// Run one schematic on every simulator that is available and can run it.
async fn simulate_all(name: &str) -> Vec<(SimId, SimOutput)> {
    let asc = schematic(name);
    let mut sims: Vec<(SimId, Box<dyn Simulator>)> = Vec::new();
    if let Some(ng) = ngspice().await {
        sims.push((SimId::Ngspice, Box::new(ng)));
    }
    if let Some(x) = xyce().await {
        sims.push((SimId::Xyce, Box::new(x)));
    }
    if let Some(lt) = ltspice().await {
        sims.push((SimId::Ltspice, Box::new(lt)));
    }
    let runs = sims.iter().map(|(id, sim)| {
        let deck = deck_for(&asc, *id);
        async move {
            if !deck.unsupported.is_empty() {
                eprintln!(
                    "{name} on {id}: skipped, unsupported: {:?}",
                    deck.unsupported
                );
                return None;
            }
            let out = run(sim.as_ref(), &deck.text, name)
                .await
                .unwrap_or_else(|e| panic!("{name} on {id}: {e}\ndeck:\n{}", deck.text));
            assert!(out.errors.is_empty(), "{name} on {id}: {:?}", out.errors);
            Some((*id, out))
        }
    });
    futures::future::join_all(runs)
        .await
        .into_iter()
        .flatten()
        .collect()
}

fn ds(out: &SimOutput, kind: AnalysisKind) -> &Dataset {
    out.dataset(kind).unwrap_or_else(|| {
        panic!(
            "no {kind:?} dataset in {:?}",
            out.datasets.iter().map(|d| d.kind).collect::<Vec<_>>()
        )
    })
}

fn ac_mag(d: &Dataset, node: &str) -> (Vec<f64>, Vec<f64>) {
    let f = d.axis_vector().unwrap().data.real();
    let m = d
        .vector(node)
        .unwrap_or_else(|| panic!("no {node} in {:?}", d.names()))
        .data
        .as_complex()
        .iter()
        .map(|z| z.abs())
        .collect();
    (f, m)
}

/// Print a metric per simulator and check every pair agrees within `tol`.
fn agree(what: &str, values: &[(SimId, f64)], tol: f64) {
    for (id, v) in values {
        eprintln!("{what}: {id} = {v:.6}");
    }
    for (a, va) in values {
        for (b, vb) in values {
            assert!(
                rel(*va, *vb) <= tol,
                "{what}: {a} {va} vs {b} {vb} differ by {:.3} %",
                100.0 * rel(*va, *vb)
            );
        }
    }
}

#[tokio::test]
async fn rc_lowpass_corner() {
    let runs = simulate_all("rc_lowpass").await;
    let fc = 1.0 / (2.0 * PI * 1e3 * 100e-9);
    let mut values = Vec::new();
    for (id, out) in &runs {
        let (f, m) = ac_mag(ds(out, AnalysisKind::Ac), "out");
        let f3 = crossing(&f, &m, 1.0 / 2f64.sqrt()).unwrap();
        assert!(rel(f3, fc) < 0.01, "{id}: {f3} vs {fc}");
        values.push((*id, f3));
    }
    agree("rc_lowpass -3 dB frequency (Hz)", &values, 0.01);
}

#[tokio::test]
async fn divider_voltage() {
    let runs = simulate_all("divider").await;
    let mut values = Vec::new();
    for (id, out) in &runs {
        let v = ds(out, AnalysisKind::Op).vector("out").unwrap().data.real()[0];
        assert!((v - 7.5).abs() < 1e-9, "{id}: {v}");
        values.push((*id, v));
    }
    agree("divider V(out) (V)", &values, 1e-9);
}

#[tokio::test]
async fn common_emitter_gain() {
    let runs = simulate_all("ce_amp").await;
    let mut values = Vec::new();
    for (id, out) in &runs {
        let (f, m) = ac_mag(ds(out, AnalysisKind::Ac), "col");
        let g = value_at(&f, &m, 1e3);
        // Unbypassed emitter: about Rc / (re + Re) = 4.7k / 1.02k.
        assert!(g > 4.0 && g < 4.7, "{id}: gain {g}");
        values.push((*id, g));
    }
    agree("ce_amp gain at 1 kHz", &values, 0.03);
}

#[tokio::test]
async fn nmos_common_source() {
    let runs = simulate_all("nmos_cs").await;
    // Square law with channel-length modulation: Vds solves
    // Vds = 10 - 2.2k * 1m * (1 + 0.02 Vds).
    let vds = 7.8 / 1.044;
    let id_ = 1e-3 * (1.0 + 0.02 * vds);
    let gm = 2e-3 * (1.0 + 0.02 * vds);
    let ro = 1.0 / (0.02 * 1e-3);
    let gain = gm * (2.2e3 * ro / (2.2e3 + ro));
    eprintln!("nmos_cs analytic: Vds {vds:.4} V, Id {id_:.6} A, gain {gain:.4}");
    let mut values = Vec::new();
    for (id, out) in &runs {
        let (f, m) = ac_mag(ds(out, AnalysisKind::Ac), "drain");
        let g = value_at(&f, &m, 1e3);
        assert!(rel(g, gain) < 0.01, "{id}: gain {g} vs {gain}");
        values.push((*id, g));
    }
    agree("nmos_cs gain at 1 kHz", &values, 0.01);
}

#[tokio::test]
async fn opamp_inverting_amplifier() {
    let runs = simulate_all("opamp_inv").await;
    let want = 10.0 / (1.0 + 11.0 / 1e5);
    let mut gains = Vec::new();
    let mut bws = Vec::new();
    for (id, out) in &runs {
        let (f, m) = ac_mag(ds(out, AnalysisKind::Ac), "out");
        let g = value_at(&f, &m, 1e3);
        assert!(rel(g, want) < 1e-3, "{id}: gain {g} vs {want}");
        let bw = crossing(&f, &m, g / 2f64.sqrt()).unwrap();
        // Closed-loop bandwidth GBW / (1 + Rf/Rin), log-spaced points.
        assert!(rel(bw, 10e6 / 11.0) < 0.05, "{id}: bandwidth {bw}");
        gains.push((*id, g));
        bws.push((*id, bw));
    }
    agree("opamp_inv gain at 1 kHz", &gains, 1e-4);
    agree("opamp_inv -3 dB bandwidth (Hz)", &bws, 0.02);
}

#[tokio::test]
async fn rectifier_peak() {
    let runs = simulate_all("rectifier").await;
    let mut values = Vec::new();
    for (id, out) in &runs {
        let d = ds(out, AnalysisKind::Transient);
        let t = d.axis_vector().unwrap().data.real();
        let v = d.vector("out").unwrap().data.real();
        let peak = t
            .iter()
            .zip(&v)
            .filter(|(t, _)| **t > 40e-3)
            .map(|(_, v)| *v)
            .fold(f64::MIN, f64::max);
        // 5 V peak less one diode drop at a few mA.
        assert!(peak > 4.1 && peak < 4.5, "{id}: peak {peak}");
        values.push((*id, peak));
    }
    agree("rectifier peak V(out) (V)", &values, 0.02);
}

#[tokio::test]
async fn rlc_resonance() {
    let runs = simulate_all("rlc_tjunction").await;
    let f0 = 1.0 / (2.0 * PI * (1e-3f64 * 1e-6).sqrt());
    let mut values = Vec::new();
    for (id, out) in &runs {
        let (f, m) = ac_mag(ds(out, AnalysisKind::Ac), "out");
        let (i, peak) =
            m.iter().enumerate().fold(
                (0, f64::MIN),
                |acc, (i, v)| if *v > acc.1 { (i, *v) } else { acc },
            );
        assert!(rel(f[i], f0) < 2e-3, "{id}: peak at {} vs {f0}", f[i]);
        assert!(peak > 0.99 && peak <= 1.0, "{id}: peak {peak}");
        values.push((*id, f[i]));
    }
    agree("rlc resonance (Hz)", &values, 2e-3);
}

#[tokio::test]
async fn every_orientation_conducts() {
    let runs = simulate_all("orientations").await;
    for k in 0..8 {
        let node = format!("c{k}");
        let mut values = Vec::new();
        for (id, out) in &runs {
            let v = ds(out, AnalysisKind::Op).vector(&node).unwrap().data.real()[0];
            // A diode turned the wrong way would leave the node at 0 V.
            assert!(v > 4.0 && v < 4.6, "{id}: V({node}) = {v}");
            values.push((*id, v));
        }
        agree(&format!("orientations V({node}) (V)"), &values, 5e-3);
    }
}
