//! The analysis modules used together through the public API, the way the
//! agent's tools will: a netlist is swept, toleranced and sized, each
//! variant is "simulated" (here by the closed-form response of an RC
//! lowpass, read from the variant's own netlist), and the results go through
//! specs, yield statistics and a plot.

use aispice_core::netlist::{Netlist, parse};
use aispice_sim::measure::{Measure, measure_dataset};
use aispice_sim::montecarlo::{self, Distribution, Tolerance};
use aispice_sim::optimize::{
    self, CmaEs, ESeries, NelderMead, Objective, Optimizer, Param, RunOptions, StopReason,
};
use aispice_sim::plot::{PlotRequest, markers_for, plot_svg};
use aispice_sim::spec::{evaluate, parse_specs};
use aispice_sim::sweep::{self, SweepParam, SweepValues};
use aispice_sim::{AnalysisKind, Complex, Dataset, Quantity, Step, Vector, VectorData};
use std::f64::consts::PI;

const DECK: &str = "RC lowpass\nV1 in 0 AC 1\nR1 in out {Rval}\nC1 out 0 10n\n.param Rval=1.6k\n.ac dec 20 10 10Meg\n.end\n";

/// The AC response of the deck's RC, as a simulator would return it.
fn simulate(n: &Netlist) -> Dataset {
    let r = sweep::get_value(n, "Rval").expect("Rval");
    let c = sweep::get_value(n, "C1").expect("C1");
    let f: Vec<f64> = (0..=120)
        .map(|i| 10f64.powf(1.0 + 6.0 * i as f64 / 120.0))
        .collect();
    let mut vout = Vec::new();
    let mut iv1 = Vec::new();
    for &fi in &f {
        let zc = Complex::new(0.0, -1.0 / (2.0 * PI * fi * c));
        let i = Complex::new(1.0, 0.0) / (Complex::new(r, 0.0) + zc);
        vout.push(i * zc);
        iv1.push(i);
    }
    let len = f.len();
    Dataset {
        title: n.title.clone(),
        plotname: "AC Analysis".into(),
        kind: AnalysisKind::Ac,
        axis: Some(0),
        vectors: vec![
            Vector {
                name: "frequency".into(),
                quantity: Quantity::Frequency,
                data: VectorData::Real(f),
            },
            Vector {
                name: "V(out)".into(),
                quantity: Quantity::Voltage,
                data: VectorData::Complex(vout),
            },
            Vector {
                name: "I(V1)".into(),
                quantity: Quantity::Current,
                data: VectorData::Complex(iv1),
            },
        ],
        steps: vec![Step {
            range: 0..len,
            label: String::new(),
        }],
    }
}

fn corner(r: f64, c: f64) -> f64 {
    1.0 / (2.0 * PI * r * c)
}

#[test]
fn sweep_then_specs_then_plot() {
    let deck = parse(DECK);
    let params = vec![SweepParam {
        target: "Rval".into(),
        values: SweepValues::Text("1k 2.2k 4.7k".into()),
    }];
    let variants = sweep::variants(&deck, &params).unwrap();
    assert_eq!(variants.len(), 3);
    let specs =
        parse_specs("bw = bandwidth_3db(V(out)) >= 5k\nzin = value_at(mag(1/I(V1)), 1Meg) >= 1k")
            .unwrap();
    let passing: Vec<bool> = variants
        .iter()
        .map(|(_, n)| evaluate(&specs, &[simulate(n)]).all_pass)
        .collect();
    // Corners: 15.9k, 7.2k and 3.4k.
    assert_eq!(passing, vec![true, true, false]);

    // One result per sweep point, plotted together as a stepped run would be.
    let ds = simulate(&variants[1].1);
    let bw = Measure::parse("bandwidth_3db(V(out))").unwrap();
    let r = measure_dataset("bw", &bw, &ds);
    assert!((r.value.unwrap() - corner(2.2e3, 10e-9)).abs() / corner(2.2e3, 10e-9) < 3e-3);
    let mut req = PlotRequest::new(["V(out)"]);
    req.markers = markers_for(&bw, &r);
    let svg = plot_svg(&[ds], &req).unwrap();
    assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
    assert!(svg.contains("data-pane=\"phase\""));
}

#[test]
fn monte_carlo_yield_and_worst_case() {
    let deck = parse(DECK);
    let tolerances = vec![
        Tolerance {
            target: "Rval".into(),
            tol_pct: 5.0,
            distribution: Distribution::Gaussian,
        },
        Tolerance {
            target: "C*".into(),
            tol_pct: 10.0,
            distribution: Distribution::Uniform,
        },
    ];
    // Nominal corner is 9.95 kHz; the spec leaves about 5% of room.
    let specs = parse_specs("bw = bandwidth_3db(V(out)) in 9.45k..10.45k").unwrap();
    let runs = montecarlo::variants(&deck, &tolerances, 200, 7).unwrap();
    let reports: Vec<_> = runs
        .iter()
        .map(|v| evaluate(&specs, &[simulate(&v.netlist)]))
        .collect();
    let y = montecarlo::summarize(&reports);
    assert_eq!(y.runs, 200);
    assert!(y.yield_pct > 20.0 && y.yield_pct < 80.0, "{}", y.summary);
    let worst = y.worst_run.unwrap();
    assert!(!reports[worst].all_pass);

    // Every corner of +-5% R and +-10% C, against the same spec.
    let corners = montecarlo::corners(&deck, &tolerances, 10).unwrap();
    assert_eq!(corners.len(), 4);
    let corner_pass: Vec<bool> = corners
        .iter()
        .map(|v| evaluate(&specs, &[simulate(&v.netlist)]).all_pass)
        .collect();
    // RC products 0.855, 0.945, 1.045 and 1.155 of nominal: only R low with
    // C high stays inside the 5% window.
    let labels: Vec<&str> = corners.iter().map(|v| v.label.as_str()).collect();
    assert_eq!(
        labels,
        vec!["Rval- C1-", "Rval+ C1-", "Rval- C1+", "Rval+ C1+"]
    );
    assert_eq!(corner_pass, vec![false, false, true, false]);
}

#[test]
fn sizing_with_both_optimizers() {
    let deck = parse(DECK);
    let specs = parse_specs(
        "bw = bandwidth_3db(V(out)) in 19k..21k\n\
         zin = value_at(mag(1/I(V1)), 1Meg) >= 500",
    )
    .unwrap();
    let mut r = Param::new("Rval", 100.0, 1e6);
    r.snap = Some(ESeries::E24);
    let mut c = Param::new("C1", 100e-12, 1e-6);
    c.snap = Some(ESeries::E12);
    let params = vec![r, c];
    let objective = Objective::default();
    let evaluate_batch = |batch: &[Vec<f64>]| -> Vec<optimize::Evaluation> {
        batch
            .iter()
            .map(|x| {
                let n = optimize::apply(&deck, &params, x).unwrap();
                objective.score(&evaluate(&specs, &[simulate(&n)]))
            })
            .collect()
    };
    let mut nm = NelderMead::new(&params, Default::default()).unwrap();
    let mut es = CmaEs::new(&params, Default::default()).unwrap();
    for opt in [&mut nm as &mut dyn Optimizer, &mut es] {
        let result = optimize::run(opt, &params, &RunOptions::default(), evaluate_batch);
        assert_eq!(result.stop, StopReason::Feasible);
        let fc = corner(result.best[0], result.best[1]);
        assert!((19e3..=21e3).contains(&fc), "{fc}");
        let snapped = result.snapped.unwrap();
        assert!(snapped.feasible);
        // The snapped values go back into the netlist as written values.
        let n = optimize::apply(&deck, &params, &snapped.values).unwrap();
        let text = aispice_core::netlist::write(&n);
        assert!(text.contains(".param Rval="), "{text}");
        assert!(evaluate(&specs, &[simulate(&n)]).all_pass);
    }
}
