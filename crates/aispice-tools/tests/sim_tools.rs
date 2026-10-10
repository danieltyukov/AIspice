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
