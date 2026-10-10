//! Translated decks on the real simulators: an LTspice-flavoured deck must
//! run cleanly on ngspice and Xyce after translation, and give the numbers
//! LTspice gives for the original. Skips per simulator when it is missing;
//! LTspice needs `AISPICE_LTSPICE_TESTS=1`.

mod common;

use aispice_core::netlist::parse;
use aispice_sim::backend::{SimId, SimOutput, Simulator};
use aispice_sim::dialect::translate;
use common::*;

async fn run_everywhere(deck_name: &str) -> Vec<(SimId, SimOutput)> {
    let netlist = parse(&deck(deck_name));
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
        let tr = translate(&netlist, id.target());
        async move {
            assert!(tr.unsupported.is_empty(), "{id}: {:?}", tr.unsupported);
            let out = run(sim.as_ref(), &tr.text, deck_name)
                .await
                .unwrap_or_else(|e| panic!("{id}: {e}\n{}", tr.text));
            assert!(out.errors.is_empty(), "{id}: {:?}\n{}", out.errors, tr.text);
            eprintln!("{id} notes: {:?}", tr.notes);
            (*id, out)
        }
    });
    futures::future::join_all(runs).await
}

fn meas(id: SimId, out: &SimOutput, name: &str) -> f64 {
    out.measurements
        .iter()
        .find(|m| m.name.eq_ignore_ascii_case(name))
        .and_then(|m| m.value)
        .unwrap_or_else(|| {
            panic!(
                "{id}: no value for {name} in {:?}\n{}",
                out.measurements, out.log
            )
        })
}

fn check(name: &str, runs: &[(SimId, SimOutput)], tol: f64) -> Vec<f64> {
    let values: Vec<(SimId, f64)> = runs
        .iter()
        .map(|(id, o)| (*id, meas(*id, o, name)))
        .collect();
    for (id, v) in &values {
        eprintln!("{name}: {id} = {v:.6}");
    }
    for (a, va) in &values {
        for (b, vb) in &values {
            assert!(rel(*va, *vb) <= tol, "{name}: {a} {va} vs {b} {vb}");
        }
    }
    values.into_iter().map(|(_, v)| v).collect()
}

#[tokio::test]
async fn ltspice_flavoured_transient_runs_everywhere() {
    let runs = run_everywhere("lt_flavoured").await;
    if runs.is_empty() {
        return;
    }
    let vmax = check("vmax", &runs, 0.01);
    check("vend", &runs, 0.02);
    check("bmax", &runs, 0.01);
    let half = check("half", &runs, 0.01);
    assert!((half[0] - vmax[0] / 2.0).abs() < 1e-3 * vmax[0]);
}

#[tokio::test]
async fn ltspice_flavoured_ac_runs_everywhere() {
    let runs = run_everywhere("lt_flavoured_ac").await;
    if runs.is_empty() {
        return;
    }
    // |H| = 1000/1060 * x / sqrt(1 + x^2) with x = f / fc reaches 0.7071 a
    // little above the corner, because the passband gain is below one.
    let fc = 1.0 / (2.0 * std::f64::consts::PI * 1060.0 * 1e-6);
    let r = std::f64::consts::FRAC_1_SQRT_2 / (1000.0 / 1060.0);
    let want = fc * r / (1.0 - r * r).sqrt();
    for v in check("fhp", &runs, 0.005) {
        // 20 points per decade, interpolated linearly.
        assert!(rel(v, want) < 0.01, "{v} vs {want}");
    }
    // At 10 kHz the capacitor is nearly a short: 1000 / 1060.
    for v in check("g10k", &runs, 0.005) {
        assert!(rel(v, 1000.0 / 1060.0) < 0.005, "{v}");
    }
}
