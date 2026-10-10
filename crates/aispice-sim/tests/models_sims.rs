//! aispice's embedded generic models on the real simulators, with no LTspice
//! library: every model must load, and the operating points must look like
//! the datasheets they approximate.

mod common;

use aispice_core::netlist::parse;
use aispice_sim::backend::{SimId, SimOutput, Simulator};
use aispice_sim::dialect::translate;
use aispice_sim::models::{ModelSource, resolve};
use aispice_sim::{AnalysisKind, Dataset};
use common::*;

const DECK: &str = "embedded models
V1 vcc 0 12
D1 vcc d1 1N4148
Rd1 d1 0 10k
D2 vcc d2 1N4007
Rd2 d2 0 10k
Rb1 vcc b1 1Meg
Rc1 vcc c1 1k
Q1 c1 b1 0 2N3904
Rb2 0 b2 1Meg
Rc2 0 c2 1k
Q2 c2 b2 vcc 2N3906
Rb3 vcc b3 1Meg
Rc3 vcc c3 1k
Q3 c3 b3 0 2N2222
Vg g 0 10
Rm1 vcc dm1 100
M1 dm1 g 0 0 2N7002
Vgp gp 0 2
Rm2 dm2 0 100
M2 dm2 gp vcc vcc BSS84
Rm3 vcc dm3 12
M3 dm3 g 0 0 NMOS_POWER
Vin in 0 AC 1
Ri in inn 1k
Rf inn oa 10k
XU1 inn 0 oa opamp Aol=100K GBW=10Meg
.lib opamp.sub
.op
.end
";

fn op(out: &SimOutput) -> &Dataset {
    out.dataset(AnalysisKind::Op).expect("an operating point")
}

fn v(d: &Dataset, node: &str) -> f64 {
    d.vector(node)
        .unwrap_or_else(|| panic!("no {node} in {:?}", d.names()))
        .data
        .real()[0]
}

async fn run_on(sim: &dyn Simulator, id: SimId) -> SimOutput {
    let r = resolve(&parse(DECK), &[], None);
    assert!(r.missing.is_empty(), "{:?}", r.missing);
    assert!(r.added.iter().all(|a| a.source == ModelSource::Embedded));
    assert_eq!(r.added.len(), 9, "{:?}", r.added);
    let tr = translate(&r.netlist, id.target());
    let out = run(sim, &tr.text, "embedded")
        .await
        .unwrap_or_else(|e| panic!("{id}: {e}\n{}", tr.text));
    assert!(out.errors.is_empty(), "{id}: {:?}", out.errors);
    out
}

fn check_datasheet_levels(id: SimId, d: &Dataset) {
    // Diode forward drops at about 1.1 mA.
    let vf1 = 12.0 - v(d, "d1");
    let vf2 = 12.0 - v(d, "d2");
    eprintln!("{id}: 1N4148 VF {vf1:.4} V, 1N4007 VF {vf2:.4} V");
    assert!((0.55..0.72).contains(&vf1), "{id}: 1N4148 VF {vf1}");
    assert!((0.5..0.7).contains(&vf2), "{id}: 1N4007 VF {vf2}");
    // Current gain at about 2 mA: Ib = (12 - 0.65) / 1Meg.
    for (name, b, c, pnp) in [
        ("2N3904", "b1", "c1", false),
        ("2N3906", "b2", "c2", true),
        ("2N2222", "b3", "c3", false),
    ] {
        let (ib, ic) = if pnp {
            ((v(d, b)) / 1e6, v(d, c) / 1e3)
        } else {
            ((12.0 - v(d, b)) / 1e6, (12.0 - v(d, c)) / 1e3)
        };
        let beta = ic / ib;
        eprintln!("{id}: {name} beta {beta:.1} at Ic {:.3} mA", ic * 1e3);
        assert!((100.0..300.0).contains(&beta), "{id}: {name} beta {beta}");
    }
    // Small-signal MOSFETs fully on, power MOSFET at 1 A.
    let rds = |vd: f64, i: f64| vd / i;
    let i1 = (12.0 - v(d, "dm1")) / 100.0;
    let r1 = rds(v(d, "dm1"), i1);
    let i2 = v(d, "dm2") / 100.0;
    let r2 = rds(12.0 - v(d, "dm2"), i2);
    let i3 = (12.0 - v(d, "dm3")) / 12.0;
    let r3 = rds(v(d, "dm3"), i3);
    eprintln!(
        "{id}: 2N7002 {r1:.3} ohm, BSS84 {r2:.2} ohm, NMOS_POWER {:.1} mohm at {i3:.3} A",
        r3 * 1e3
    );
    assert!((1.0..5.0).contains(&r1), "{id}: 2N7002 {r1}");
    assert!((5.0..20.0).contains(&r2), "{id}: BSS84 {r2}");
    assert!((0.02..0.08).contains(&r3), "{id}: NMOS_POWER {r3}");
}

#[tokio::test]
async fn embedded_models_on_ngspice() {
    let Some(ng) = ngspice().await else { return };
    let out = run_on(&ng, SimId::Ngspice).await;
    check_datasheet_levels(SimId::Ngspice, op(&out));
}

#[tokio::test]
async fn embedded_models_on_xyce() {
    let Some(x) = xyce().await else { return };
    let out = run_on(&x, SimId::Xyce).await;
    check_datasheet_levels(SimId::Xyce, op(&out));
}

/// With LTspice installed its own library comes first: the standard models
/// and `opamp.sub`, read from the install, never copied into aispice.
#[test]
fn ltspice_library_supplies_models_first() {
    let dirs = aispice_sim::backend::Ltspice::default().lib_dirs();
    if dirs.is_empty() {
        eprintln!("skipped: no LTspice library found");
        return;
    }
    let mut all = Vec::new();
    for name in ["ce_amp", "rectifier", "opamp_inv"] {
        let (netlist, libs) = netlist_of(&schematic(name));
        let r = aispice_sim::models::resolve_with(&netlist, &libs, &dirs);
        assert!(r.missing.is_empty(), "{name}: {:?}", r.missing);
        all.extend(r.added);
    }
    let names: Vec<_> = all.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, vec!["2N3904", "1N4148", "opamp"]);
    for a in &all {
        match &a.source {
            ModelSource::Ltspice(p) => eprintln!("{} from {}", a.name, p.display()),
            other => panic!("{} came from {other:?}", a.name),
        }
    }
}

/// The embedded op-amp behaves exactly like LTspice's: same gain and
/// bandwidth on the inverting amplifier schematic.
#[tokio::test]
async fn embedded_opamp_matches_the_ideal_single_pole() {
    let Some(ng) = ngspice().await else { return };
    let (netlist, libs) = netlist_of(&schematic("opamp_inv"));
    let r = resolve(&netlist, &libs, None);
    assert_eq!(r.added[0].source, ModelSource::Embedded);
    let tr = translate(&r.netlist, SimId::Ngspice.target());
    let out = run(&ng, &tr.text, "opamp_embedded").await.unwrap();
    let d = out.dataset(AnalysisKind::Ac).unwrap();
    let f = d.axis_vector().unwrap().data.real();
    let m: Vec<f64> = d
        .vector("out")
        .unwrap()
        .data
        .as_complex()
        .iter()
        .map(|z| z.abs())
        .collect();
    let g = value_at(&f, &m, 1e3);
    assert!(rel(g, 10.0 / (1.0 + 11.0 / 1e5)) < 1e-6, "{g}");
    let bw = crossing(&f, &m, g / 2f64.sqrt()).unwrap();
    eprintln!("embedded opamp: gain {g:.6}, bandwidth {bw:.1} Hz");
    // The same number LTspice's opamp.sub gives on every simulator.
    assert!(rel(bw, 911_976.686_693) < 1e-6, "{bw}");
}
