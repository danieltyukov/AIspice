//! Simulation tools end to end on a real simulator (ngspice), driven through
//! the agent loop with a scripted model. Skipped when ngspice is missing.

use aispice_agent::testing::{ScriptedProvider, text_reply, tool_reply};
use aispice_agent::{Agent, AgentEvent, ContentBlock, ToolContext};
use aispice_tools::{Project, ProjectOptions, Workspace, registry};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

fn have_ngspice() -> bool {
    std::process::Command::new("ngspice")
        .arg("--version")
        .output()
        .is_ok()
}

async fn run_script(
    ws: Arc<Workspace>,
    steps: Vec<aispice_agent::provider::ChatResponse>,
) -> Vec<(String, String, bool, Option<Value>)> {
    let mut provider = ScriptedProvider::new();
    for s in steps {
        provider = provider.then_reply(s);
    }
    let agent =
        Agent::new(Arc::new(provider), Arc::new(registry(ws)), "scripted").with_max_steps(20);
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let mut history = Vec::new();
    agent
        .run(
            &mut history,
            vec![ContentBlock::text("go")],
            ToolContext::default(),
            &mut move |e| {
                if let AgentEvent::ToolEnd { name, output, .. } = e {
                    sink.lock().unwrap().push((
                        name,
                        output.text_content(),
                        output.is_error,
                        output.data.clone(),
                    ));
                }
            },
        )
        .await
        .unwrap();

    events.lock().unwrap().clone()
}

const RC: &str = r#"[
    {"op": "add_component", "symbol": "voltage", "name": "V1", "value": "0", "attrs": {"SpiceLine": "AC 1"}},
    {"op": "add_component", "symbol": "res", "name": "R1", "value": "1k", "orient": "R90", "near": "V1"},
    {"op": "add_component", "symbol": "cap", "name": "C1", "value": "100n", "near": "R1"},
    {"op": "connect", "from": "V1.+", "to": "R1.B"},
    {"op": "connect", "from": "R1.A", "to": "C1.A"},
    {"op": "connect_to_net", "pin": "V1.-", "net": "0"},
    {"op": "connect_to_net", "pin": "C1.B", "net": "0"},
    {"op": "connect_to_net", "pin": "C1.A", "net": "out"},
    {"op": "add_directive", "text": ".ac dec 50 10 1Meg"}
]"#;

#[tokio::test(flavor = "multi_thread")]
async fn simulate_check_specs_and_optimize_on_ngspice() {
    if !have_ngspice() {
        eprintln!("skipped: ngspice not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let project = Project::open(dir.path(), ProjectOptions::default()).unwrap();
    let ws = Arc::new(Workspace::with_project(project));
    let edits: Value = serde_json::from_str(RC).unwrap();
    let results = run_script(
        ws.clone(),
        vec![
            tool_reply([("a", "create_schematic", json!({"circuit": "rc.asc", "edits": edits}))]),
            tool_reply([("b", "simulate", json!({"circuit": "rc.asc", "simulator": "ngspice", "measurements": ["f3db = bandwidth_3db(V(out))", "g = gain_db_at(V(out), 100)"]}))]),
            tool_reply([("c", "check_specs", json!({"circuit": "rc.asc", "specs": "bw = bandwidth_3db(V(out)) in 1.5k..1.7k"}))]),
            tool_reply([("d", "optimize", json!({"circuit": "rc.asc", "params": [{"name": "R1", "min": "100", "max": "100k", "snap": "E24"}], "specs": "bw = bandwidth_3db(V(out)) in 9.5k..10.5k", "apply": true, "max_evals": 60}))]),
            tool_reply([("e", "check_specs", json!({"circuit": "rc.asc"}))]),
            tool_reply([("f", "plot", json!({"circuit": "rc.asc", "traces": ["V(out)"]}))]),
            tool_reply([("g", "sweep", json!({"circuit": "rc.asc", "params": [{"target": "C1", "values": [1e-8, 1e-7, 1e-6]}], "measurements": ["f3db = bandwidth_3db(V(out))"]}))]),
            tool_reply([("h", "monte_carlo", json!({"circuit": "rc.asc", "tolerances": [{"target": "R*", "tol_pct": 5}, {"target": "C*", "tol_pct": 10}], "runs": 30, "seed": 7}))]),
            text_reply("done"),
        ],
    )
    .await;
    for (name, text, is_error, _) in &results {
        assert!(!is_error, "{name} failed:\n{text}");
    }
    let get = |n: &str| {
        results
            .iter()
            .filter(|r| r.0 == n)
            .map(|r| r.1.clone())
            .collect::<Vec<_>>()
    };
    let sim = &get("simulate")[0];
    assert!(sim.contains("with ngspice"), "{sim}");
    // 1/(2 pi 1k 100n) = 1591.5 Hz
    let f = results
        .iter()
        .find(|r| r.0 == "simulate")
        .unwrap()
        .3
        .clone()
        .unwrap();
    let f3db = f["run"]["measurements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "f3db")
        .unwrap()["value"]
        .as_f64()
        .unwrap();
    assert!((f3db - 1591.5).abs() / 1591.5 < 0.01, "f3db {f3db}");
    let specs1 = &get("check_specs")[0];
    assert!(specs1.contains("PASS"), "{specs1}");
    let opt = &get("optimize")[0];
    assert!(opt.contains("All specs pass"), "{opt}");
    assert!(opt.contains("Applied to the schematic"), "{opt}");
    // R for 10 kHz with 100 nF is 159 ohm; E24 neighbours are 150 and 160.
    let on_disk = std::fs::read_to_string(dir.path().join("rc.asc")).unwrap();
    assert!(
        on_disk.contains("SYMATTR Value 160") || on_disk.contains("SYMATTR Value 150"),
        "{on_disk}"
    );
    let specs2 = &get("check_specs")[1];
    assert!(
        specs2.contains("PASS") && !specs2.contains("FAIL"),
        "saved specs re-checked: {specs2}"
    );
    assert!(dir.path().join("rc.specs").exists());
    let sweep = &get("sweep")[0];
    assert_eq!(sweep.lines().count(), 4, "{sweep}");
    let mc = &get("monte_carlo")[0];
    assert!(mc.contains("yield") || mc.contains("Yield"), "{mc}");
}

/// RC low-pass with R = 1k, C = 159.15n: one real pole at -1/(2 pi R C) =
/// -1 kHz and no zeros. The `.meas` and `.save` lines would stop ngspice's
/// `.pz` if they reached it.
const RC_PZ: &str = r#"[
    {"op": "add_component", "symbol": "voltage", "name": "V1", "value": "0", "attrs": {"SpiceLine": "AC 1"}},
    {"op": "add_component", "symbol": "res", "name": "R1", "value": "1k", "orient": "R90", "near": "V1"},
    {"op": "add_component", "symbol": "cap", "name": "C1", "value": "159.15n", "near": "R1"},
    {"op": "connect", "from": "V1.+", "to": "R1.B"},
    {"op": "connect", "from": "R1.A", "to": "C1.A"},
    {"op": "connect_to_net", "pin": "V1.-", "net": "0"},
    {"op": "connect_to_net", "pin": "C1.B", "net": "0"},
    {"op": "connect_to_net", "pin": "C1.A", "net": "out"},
    {"op": "connect_to_net", "pin": "V1.+", "net": "in"},
    {"op": "add_directive", "text": ".ac dec 50 10 1Meg"},
    {"op": "add_directive", "text": ".save V(out) V(in)"},
    {"op": "add_directive", "text": ".meas ac peak MAX mag(V(out))"}
]"#;

/// Series RLC low-pass with a complex pair: f0 = 1/(2 pi sqrt(LC)) and
/// Q = sqrt(L/C)/R. The transient, `.step` and `.meas` lines must be left
/// out of the pole-zero run.
const RLC_PZ: &str = "series RLC\nV1 in 0 PULSE(0 1 0 1n 1n 1m 2m) AC 1\nR1 in a 100\nL1 a out 10m\nC1 out 0 {C}\n.param C=100n\n.tran 0 2m\n.step param C list 100n 220n\n.meas tran vmax MAX V(out)\n.end\n";

/// Positive feedback through an ideal amplifier: C dV/dt = (Vin - V)/10k +
/// (2V - V)/1k puts a pole at +(1/1k - 1/10k)/C = +900 rad/s (143.2 Hz).
const UNSTABLE_PZ: &str =
    "positive feedback\nV1 in 0 0 AC 1\nRin in x 10k\nC1 x 0 1u\nRf y x 1k\nE1 y 0 x 0 2\n.end\n";

/// A current into a parallel RC: Z(s) = R/(1 + sRC), one pole at -1 kHz.
const CUR_PZ: &str = "transimpedance\nI1 0 x 0 AC 1\nR1 x 0 1k\nC1 x 0 159.15n\n.op\n.end\n";

#[tokio::test(flavor = "multi_thread")]
async fn poles_zeros_on_ngspice() {
    if !have_ngspice() {
        eprintln!("skipped: ngspice not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("rlc.cir"), RLC_PZ).unwrap();
    std::fs::write(dir.path().join("unstable.cir"), UNSTABLE_PZ).unwrap();
    std::fs::write(dir.path().join("cur.cir"), CUR_PZ).unwrap();
    let project = Project::open(dir.path(), ProjectOptions::default()).unwrap();
    let ws = Arc::new(Workspace::with_project(project));
    let edits: Value = serde_json::from_str(RC_PZ).unwrap();
    let pz = "poles_zeros";
    let results = run_script(
        ws.clone(),
        vec![
            tool_reply([("a", "create_schematic", json!({"circuit": "rc.asc", "edits": edits}))]),
            tool_reply([("b", "simulate", json!({"circuit": "rc.asc", "simulator": "ngspice"}))]),
            tool_reply([("c", pz, json!({"circuit": "rc.asc", "input": "in", "output": "out"}))]),
            tool_reply([("d", "measure", json!({"circuit": "rc.asc", "measurements": ["f3db = bandwidth_3db(V(out))"]}))]),
            tool_reply([("e", pz, json!({"circuit": "rc.asc", "input": "in", "output": "out", "params": {"R1": "2k"}}))]),
            tool_reply([("f", pz, json!({"circuit": "rlc.cir", "input": "in", "input_neg": "0", "output": "V(out)", "transfer": "vol"}))]),
            tool_reply([("g", pz, json!({"circuit": "unstable.cir", "input": "in", "output": "y"}))]),
            tool_reply([("h", pz, json!({"circuit": "cur.cir", "input": "x", "output": "x", "transfer": "cur"}))]),
            tool_reply([("i", pz, json!({"circuit": "rc.asc", "input": "nosuch", "output": "out"}))]),
            text_reply("done"),
        ],
    )
    .await;
    assert_eq!(results.len(), 9);
    for (name, text, is_error, _) in &results[..8] {
        assert!(!is_error, "{name} failed:\n{text}");
    }
    let data = |i: usize| results[i].3.clone().unwrap();
    let close = |a: f64, b: f64| (a - b).abs() / b.abs() < 0.005;

    // RC: one real pole at -1 kHz, no zeros, stable.
    let (text, d) = (&results[2].1, data(2));
    assert_eq!(d["kind"], "poles_zeros");
    assert_eq!(d["stable"], true, "{text}");
    assert_eq!(d["zeros"].as_array().unwrap().len(), 0, "{text}");
    let poles = d["poles"].as_array().unwrap();
    assert_eq!(poles.len(), 1, "{text}");
    let re = poles[0]["re_hz"].as_f64().unwrap();
    assert!(close(re, -1000.0), "pole {re} Hz\n{text}");
    assert_eq!(poles[0]["im_hz"].as_f64().unwrap(), 0.0);
    assert!(close(poles[0]["f0_hz"].as_f64().unwrap(), 1000.0));
    assert!(poles[0]["q"].is_null());
    assert!(text.contains("V(out)/V(in)"), "{text}");
    assert!(text.contains("real, corner at 1 kHz"), "{text}");
    assert!(text.starts_with("Poles and zeros"), "{text}");

    // The pole-zero run is not kept: measure still sees the AC run.
    assert!(results[3].1.contains("f3db"), "{}", results[3].1);

    // The same circuit with R1 doubled for this run only.
    let re2 = data(4)["poles"][0]["re_hz"].as_f64().unwrap();
    assert!(close(re2, -500.0), "pole with R1=2k: {re2}");
    assert!(results[4].1.contains("with R1=2k"), "{}", results[4].1);
    let on_disk = std::fs::read_to_string(dir.path().join("rc.asc")).unwrap();
    assert!(
        on_disk.contains("SYMATTR Value 1k"),
        "the file must not change"
    );

    // Series RLC: a complex pair with the analytic f0 and Q.
    let (text, d) = (&results[5].1, data(5));
    let (r, l, c) = (100.0f64, 10e-3f64, 100e-9f64);
    let f0 = 1.0 / (2.0 * std::f64::consts::PI * (l * c).sqrt());
    let q = (l / c).sqrt() / r;
    let poles = d["poles"].as_array().unwrap();
    assert_eq!(poles.len(), 2, "{text}");
    for p in poles {
        let got_f0 = p["f0_hz"].as_f64().unwrap();
        let got_q = p["q"].as_f64().unwrap();
        assert!(close(got_f0, f0), "f0 {got_f0} vs {f0}\n{text}");
        assert!(close(got_q, q), "Q {got_q} vs {q}\n{text}");
    }
    assert!(poles[0]["im_hz"].as_f64().unwrap() > 0.0);
    assert!(poles[1]["im_hz"].as_f64().unwrap() < 0.0);
    assert_eq!(d["stable"], true);
    assert!(
        text.contains("complex pair, f0 = 5.033 kHz, Q = 3.162"),
        "{text}"
    );

    // Positive feedback: a right half-plane pole, reported loudly.
    let (text, d) = (&results[6].1, data(6));
    assert_eq!(d["stable"], false, "{text}");
    let re = d["poles"][0]["re_hz"].as_f64().unwrap();
    assert!(close(re, 900.0 / (2.0 * std::f64::consts::PI)), "{re}");
    assert!(text.contains("UNSTABLE: 1 of 1 poles"), "{text}");
    assert!(text.contains("+143.2 Hz: real"), "{text}");

    // Transimpedance of a current-driven parallel RC.
    let (text, d) = (&results[7].1, data(7));
    assert!(text.contains("V(x)/I(x)"), "{text}");
    let re = d["poles"][0]["re_hz"].as_f64().unwrap();
    assert!(close(re, -1000.0), "{re}");

    // An unknown node is refused before anything runs, with the real ones.
    let (text, is_error) = (&results[8].1, results[8].2);
    assert!(is_error, "{text}");
    assert!(
        text.contains("no node `nosuch`") && text.contains("out"),
        "{text}"
    );
}

struct DenyAll;

#[async_trait::async_trait]
impl aispice_tools::Approver for DenyAll {
    async fn approve(&self, _circuit: &str, _summary: &str, _diff: &str) -> bool {
        false
    }
}

/// In ask-before-apply mode, a declined approval means no tool writes
/// anything: edits, undo, saved specs, optimized values.
#[tokio::test(flavor = "multi_thread")]
async fn declined_approval_blocks_every_write() {
    if !have_ngspice() {
        eprintln!("skipped: ngspice not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let project = Project::open(dir.path(), ProjectOptions::default()).unwrap();
    let ws = Arc::new(Workspace::with_project(project));
    // Build the circuit before turning approval on.
    let edits: Value = serde_json::from_str(RC).unwrap();
    run_script(
        ws.clone(),
        vec![
            tool_reply([(
                "a",
                "create_schematic",
                json!({"circuit": "rc.asc", "edits": edits}),
            )]),
            text_reply("ok"),
        ],
    )
    .await;
    let before = std::fs::read(dir.path().join("rc.asc")).unwrap();
    ws.set_hooks(aispice_tools::Hooks {
        approver: Some(Arc::new(DenyAll)),
        after_save: None,
    });
    let results = run_script(
        ws.clone(),
        vec![
            tool_reply([("b", "edit_schematic", json!({"circuit": "rc.asc", "edits": [{"op": "set_value", "name": "R1", "value": "2k"}]}))]),
            tool_reply([("c", "history", json!({"circuit": "rc.asc", "action": "undo"}))]),
            tool_reply([("d", "check_specs", json!({"circuit": "rc.asc", "specs": "bw = bandwidth_3db(V(out)) >= 1k"}))]),
            tool_reply([("e", "optimize", json!({"circuit": "rc.asc", "params": [{"name": "R1", "min": "100", "max": "10k"}], "specs": "bw = bandwidth_3db(V(out)) in 4k..6k", "apply": true, "max_evals": 20}))]),
            text_reply("done"),
        ],
    )
    .await;
    assert_eq!(
        std::fs::read(dir.path().join("rc.asc")).unwrap(),
        before,
        "schematic must be unchanged"
    );
    assert!(
        !dir.path().join("rc.specs").exists(),
        "specs must not be saved"
    );
    let text: Vec<String> = results.iter().map(|r| r.1.clone()).collect();
    assert!(text[0].contains("declined"), "{}", text[0]);
    assert!(text[1].contains("declined"), "{}", text[1]);
    assert!(text[3].contains("declined"), "{}", text[3]);
}

/// The desktop app shows a spec's value with its unit and the margin as the
/// distance to the nearest limit in that unit.
#[test]
fn spec_report_in_the_app_shape() {
    use aispice_sim::spec::{SpecReport, SpecRow};
    let row = |name: &str, value: f64, min: Option<f64>, max: Option<f64>, pass: bool| SpecRow {
        name: name.into(),
        value: Some(value),
        unit: "Hz".into(),
        min,
        max,
        target: None,
        pass,
        margin: None,
        worst_step: None,
        note: None,
        display: "a sentence for people".into(),
    };
    let report = SpecReport {
        rows: vec![
            row("bw", 1591.0, Some(1500.0), Some(1700.0), true),
            row("low", 900.0, Some(1000.0), None, false),
        ],
        all_pass: false,
        summary: "1 of 2 specs fail".into(),
    };
    let ui = aispice_tools::tools::ui_spec_report(&report);
    assert_eq!(ui["rows"][0]["display"], "1.591 kHz");
    assert_eq!(ui["rows"][0]["margin"], 91.0);
    assert_eq!(ui["rows"][1]["margin"], -100.0);
    assert_eq!(ui["all_pass"], false);
}
