//! Log and console parsers against real LTspice logs and ngspice/Xyce output
//! for the decks in `fixtures/decks`.

use aispice_sim::log::{
    parse_ltspice_log_bytes, parse_ngspice_output, parse_xyce_mt, parse_xyce_output,
};
use std::path::PathBuf;

fn fixture(rel: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn text(rel: &str) -> String {
    String::from_utf8_lossy(&fixture(rel)).into_owned()
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs()
}

#[test]
fn ltspice_measurements_and_failure() {
    let (_, r) = parse_ltspice_log_bytes(&fixture("ltspice/rc_meas.log"));
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let names: Vec<_> = r.measurements.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["vmax", "trise", "vavg", "never"]);
    let vmax = r.measurement("vmax", None).unwrap();
    assert_eq!(vmax.value, Some(0.993269));
    assert_eq!(vmax.expr, "MAX(v(out))");
    // 10 % to 90 % rise of a 1 ms RC: tau * ln 9.
    let trise = r.measurement("trise", None).unwrap().value.unwrap();
    assert!(close(trise, 1e-3 * 9f64.ln(), 2e-3), "{trise}");
    assert!(r.measurement("never", None).unwrap().failed());
    assert_eq!(r.warnings, vec!["Measurement \"never\" FAIL'ed"]);
    assert_eq!(r.elapsed, Some(0.010));
    assert!(r.step_labels.is_empty());
}

#[test]
fn ltspice_every_measurement_form() {
    let (_, r) = parse_ltspice_log_bytes(&fixture("ltspice/meas_forms.log"));
    let get = |n: &str| r.measurement(n, None).unwrap().value.unwrap();
    assert!(close(get("vend"), 1.0 - (-2f64).exp(), 1e-3));
    // WHEN reports the time: tau * ln 2.
    assert!(close(get("tcross"), 1e-3 * 2f64.ln(), 2e-3));
    assert_eq!(get("vwhen"), 1.0);
    assert_eq!(get("half"), 0.496635);
    assert_eq!(get("vrms"), 0.845484);
    assert!(close(get("vint"), 9.94277e-7, 1e-9));
    assert_eq!(r.measurements.len(), 7);

    let (_, ac) = parse_ltspice_log_bytes(&fixture("ltspice/meas_ac.log"));
    let g1k = ac.measurement("g1k", None).unwrap();
    let z = g1k.complex.unwrap();
    let fc = 1.0 / (2.0 * std::f64::consts::PI * 1e-4);
    let want = 1.0 / (1.0 + (1e3 / fc).powi(2)).sqrt();
    assert!(close(z.abs(), want, 1e-5));
    assert!((z.phase_deg() + (1e3 / fc).atan().to_degrees()).abs() < 1e-3);
    let fcm = ac.measurement("fc", None).unwrap().value.unwrap();
    assert!(close(fcm, fc, 1e-3), "{fcm}");
    let fc2 = ac.measurement("fc2", None).unwrap().value.unwrap();
    assert!(close(fc2, 2.0 * fcm, 1e-5));
}

#[test]
fn ltspice_stepped_tables() {
    let (_, r) = parse_ltspice_log_bytes(&fixture("ltspice/rc_step.log"));
    assert_eq!(r.step_labels, vec!["r=1000", "r=2000", "r=4000"]);
    assert_eq!(r.measurements.len(), 6);
    for (k, tau) in [1e-3f64, 2e-3, 4e-3].into_iter().enumerate() {
        let vend = r.measurement("vend", Some(k)).unwrap();
        assert_eq!(vend.expr, "v(out)");
        assert_eq!(vend.detail, "at 0.002");
        assert!(close(vend.value.unwrap(), 1.0 - (-2e-3 / tau).exp(), 2e-3));
    }
    assert_eq!(
        r.measurement("vmax", Some(2)).unwrap().value,
        Some(0.713513)
    );
    assert_eq!(
        r.measurement("vmax", Some(0)).unwrap().detail,
        "FROM 0 TO 0.005"
    );

    let (_, ac) = parse_ltspice_log_bytes(&fixture("ltspice/rc_ac_step.log"));
    assert_eq!(ac.step_labels, vec!["r=1000", "r=2000"]);
    let g = ac.measurement("g1k", Some(1)).unwrap().complex.unwrap();
    assert!((g.db() + 4.11474).abs() < 1e-6);
    assert!((g.phase_deg() + 51.4881).abs() < 1e-6);
    assert_eq!(ac.measurement("fc", Some(1)).unwrap().value, Some(806.356));

    // LTspice prints 0 for a step where the measurement failed.
    let (_, f) = parse_ltspice_log_bytes(&fixture("ltspice/meas_step_fail.log"));
    assert_eq!(f.measurement("t80", Some(2)).unwrap().value, Some(0.0));
    assert!(close(
        f.measurement("t80", Some(0)).unwrap().value.unwrap(),
        1e-3 * 5f64.ln(),
        1e-2
    ));
}

#[test]
fn ltspice_fatal_error_log_is_utf16() {
    let bytes = fixture("ltspice/bad_subckt.log");
    assert_eq!(&bytes[..4], b"C\0i\0", "fixture should be UTF-16LE");
    let (text, r) = parse_ltspice_log_bytes(&bytes);
    assert!(text.starts_with("Circuit: bad netlist"));
    assert_eq!(
        r.errors,
        vec!["Fatal Error: Unknown subcircuit called in: x1 in out nosuchsub"]
    );
}

#[test]
fn ltspice_warnings_and_fourier() {
    let (_, r) = parse_ltspice_log_bytes(&fixture("ltspice/floating.log"));
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(
        r.warnings,
        vec![
            "WARNING: Node FLOAT is floating.",
            "WARNING: Less than two connections to node FLOAT.  This node is used by C2."
        ]
    );
    let thd = r.measurement("thd(V(out))", None).unwrap();
    assert_eq!(thd.value, Some(28.767977));
}

#[test]
fn ngspice_measurements_and_failure() {
    let r = parse_ngspice_output(&text("ngspice/ng_meas.stdout"));
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let names: Vec<_> = r.measurements.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["never", "vmax", "vend", "trise"]);
    assert!(r.measurement("never", None).unwrap().failed());
    assert_eq!(r.measurement("vmax", None).unwrap().value, Some(0.9932623));
    let vend = r.measurement("vend", None).unwrap().value.unwrap();
    assert!(close(vend, 1.0 - (-2f64).exp(), 1e-3));
    let trise = r.measurement("trise", None).unwrap();
    assert!(close(trise.value.unwrap(), 1e-3 * 9f64.ln(), 2e-3));
    assert!(trise.detail.starts_with("targ="));
    assert_eq!(r.warnings.len(), 2);
    assert_eq!(r.elapsed, Some(0.026));
}

#[test]
fn ngspice_errors() {
    let r = parse_ngspice_output(&text("ngspice/bad_subckt.stdout"));
    assert_eq!(r.errors, vec!["Error: unknown subckt: x1 in out nosuchsub"]);

    let r = parse_ngspice_output(
        "Error on line 3 or its substitute:\n  r1 in out 1k tol=1 pwr=0.25\n  unknown parameter (tol) \n    Simulation interrupted due to error!\n",
    );
    assert_eq!(
        r.errors,
        vec![
            "Error on line 3 or its substitute: r1 in out 1k tol=1 pwr=0.25: unknown parameter (tol)"
        ]
    );

    let r = parse_ngspice_output(
        "doAnalyses: TRAN:  Timestep too small; time = 1e-06, timestep = 1e-21: trouble with node \"out\"\n\ntran simulation(s) aborted\n",
    );
    assert_eq!(r.errors.len(), 1);
    assert!(r.errors[0].contains("Timestep too small"));
}

#[test]
fn xyce_measurements_and_errors() {
    let r = parse_xyce_output(&text("xyce/xyce_meas.stdout"));
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let names: Vec<_> = r.measurements.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["VMAX", "VEND", "TRISE", "NEVER"]);
    assert!(r.measurement("never", None).unwrap().failed());
    assert!(close(
        r.measurement("trise", None).unwrap().value.unwrap(),
        1e-3 * 9f64.ln(),
        2e-3
    ));
    assert!(r.elapsed.unwrap() > 0.0);

    let mt = parse_xyce_mt(&text("xyce/xyce_meas.cir.mt0"));
    assert_eq!(mt.len(), 4);
    assert_eq!(mt[1].value, r.measurements[1].value);
    assert!(mt[3].failed());

    let bad = parse_xyce_output(&text("xyce/bad_subckt.stdout"));
    assert_eq!(
        bad.errors,
        vec![
            "Netlist error in file bad_subckt.cir at or near line 4: Subcircuit NOSUCHSUB has not been defined for instance X1"
        ]
    );
    assert!(bad.warnings.is_empty(), "{:?}", bad.warnings);
}
