//! The real agent loop driving the real tools against a real project folder,
//! with a scripted model in place of a provider.

use aispice_agent::testing::{ScriptedProvider, text_reply, tool_reply};
use aispice_agent::{Agent, AgentEvent, ContentBlock, ToolContext};
use aispice_tools::{Project, ProjectOptions, Workspace, registry};
use serde_json::json;
use std::sync::{Arc, Mutex};

fn workspace() -> (tempfile::TempDir, Arc<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::open(dir.path(), ProjectOptions::default()).unwrap();
    (dir, Arc::new(Workspace::with_project(project)))
}

#[tokio::test]
async fn agent_builds_checks_and_undoes_a_circuit() {
    let (dir, ws) = workspace();
    let provider = ScriptedProvider::new()
        .then_reply(tool_reply([(
            "t1",
            "create_schematic",
            json!({
                "circuit": "rc.asc",
                "edits": [
                    {"op": "add_component", "symbol": "voltage", "name": "V1", "value": "SINE(0 1 1k)", "attrs": {"SpiceLine": "AC 1"}},
                    {"op": "add_component", "symbol": "res", "name": "R1", "value": "1k", "orient": "R90", "near": "V1"},
                    {"op": "add_component", "symbol": "cap", "name": "C1", "value": "100n", "near": "R1"},
                    {"op": "connect", "from": "V1.+", "to": "R1.B"},
                    {"op": "connect", "from": "R1.A", "to": "C1.A"},
                    {"op": "connect_to_net", "pin": "V1.-", "net": "0"},
                    {"op": "connect_to_net", "pin": "C1.B", "net": "0"},
                    {"op": "connect_to_net", "pin": "C1.A", "net": "out"},
                    {"op": "add_directive", "text": ".ac dec 20 10 100k"}
                ]
            }),
        )]))
        .then_reply(tool_reply([("t2", "lint", json!({"circuit": "rc.asc"})), ("t3", "netlist", json!({"circuit": "rc.asc"}))]))
        .then_reply(tool_reply([(
            "t4",
            "edit_schematic",
            json!({"circuit": "rc.asc", "edits": [{"op": "set_value", "name": "R1", "value": "2.2k"}], "reason": "Lower the corner"}),
        )]))
        .then_reply(tool_reply([("t5", "history", json!({"circuit": "rc.asc", "action": "undo"}))]))
        .then_reply(tool_reply([("t6", "read_schematic", json!({"circuit": "rc.asc"}))]))
        .then_reply(text_reply("Done."));
    let provider = Arc::new(provider);
    let agent = Agent::new(provider.clone(), Arc::new(registry(ws.clone())), "scripted");
    let mut history = Vec::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let summary = agent
        .run(
            &mut history,
            vec![ContentBlock::text("Build an RC low-pass")],
            ToolContext::default(),
            &mut move |e| sink.lock().unwrap().push(e),
        )
        .await
        .unwrap();
    assert_eq!(summary.steps, 6);

    let events = events.lock().unwrap();
    let outputs: Vec<(String, String, bool)> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolEnd { name, output, .. } => {
                Some((name.clone(), output.text_content(), output.is_error))
            }
            _ => None,
        })
        .collect();
    for (name, text, is_error) in &outputs {
        assert!(!is_error, "{name} failed: {text}");
    }
    let by = |n: &str| {
        outputs
            .iter()
            .find(|(name, ..)| name == n)
            .map(|(_, t, _)| t.clone())
            .unwrap()
    };
    assert!(
        by("create_schematic").contains("Created rc.asc with 3 component(s)"),
        "{}",
        by("create_schematic")
    );
    assert!(by("lint").contains("no problems found"), "{}", by("lint"));
    assert!(by("netlist").contains("C1 out 0 100n"), "{}", by("netlist"));
    assert!(
        by("edit_schematic").contains("R1: value 1k -> 2.2k"),
        "{}",
        by("edit_schematic")
    );
    assert!(
        by("history").contains("is now at version"),
        "{}",
        by("history")
    );
    // After undo the file is back at 1k.
    assert!(
        by("read_schematic").contains("R1  res      1k"),
        "{}",
        by("read_schematic")
    );
    let on_disk = std::fs::read_to_string(dir.path().join("rc.asc")).unwrap();
    assert!(on_disk.contains("SYMATTR Value 1k"));

    // The edit card payload has the shape the desktop app expects.
    let edit_data = events
        .iter()
        .find_map(|e| match e {
            AgentEvent::ToolEnd { name, output, .. } if name == "edit_schematic" => {
                output.data.clone()
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(edit_data["kind"], "edit");
    assert!(
        edit_data["diff"]
            .as_str()
            .unwrap()
            .contains("+SYMATTR Value 2.2k")
    );
    assert_eq!(edit_data["highlights"][0]["inst"], "R1");
}

#[tokio::test]
async fn tools_refuse_paths_outside_the_project() {
    let (_dir, ws) = workspace();
    let provider = Arc::new(
        ScriptedProvider::new()
            .then_reply(tool_reply([
                ("a", "read_schematic", json!({"circuit": "../../etc/passwd.asc"})),
                ("b", "create_schematic", json!({"circuit": "/tmp/evil.asc"})),
                ("c", "edit_schematic", json!({"circuit": "x.asc", "edits": [{"op": "add_directive", "text": ".control\nshell id\n.endc"}]})),
            ]))
            .then_reply(text_reply("ok")),
    );
    let agent = Agent::new(provider, Arc::new(registry(ws)), "scripted");
    let mut history = Vec::new();
    let mut errors = Vec::new();
    agent
        .run(
            &mut history,
            vec![ContentBlock::text("try")],
            ToolContext::default(),
            &mut |e| {
                if let AgentEvent::ToolEnd { output, .. } = e {
                    errors.push((output.is_error, output.text_content()));
                }
            },
        )
        .await
        .unwrap();
    assert_eq!(errors.len(), 3);
    assert!(errors.iter().all(|(is_err, _)| *is_err), "{errors:?}");
}
