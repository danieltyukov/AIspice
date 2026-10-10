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

async fn call(
    ws: &Arc<Workspace>,
    tool: &str,
    input: serde_json::Value,
) -> aispice_agent::ToolOutput {
    let reg = registry(ws.clone());
    reg.get(tool)
        .unwrap()
        .call(&ToolContext::default(), input)
        .await
}

fn text_of(out: &aispice_agent::ToolOutput) -> String {
    out.content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// A netlist becomes a drawn schematic that netlists back to the same
/// circuit, with or without a title line.
#[tokio::test(flavor = "multi_thread")]
async fn create_schematic_draws_a_netlist() {
    let (dir, ws) = workspace();
    for (file, netlist) in [
        (
            "rc.asc",
            "* RC low-pass\nV1 in 0 AC 1\nR1 in out 1k\nC1 out 0 100n\n.ac dec 20 10 100k\n",
        ),
        // No title line: the first element must not be swallowed as one.
        (
            "untitled.asc",
            "V1 in 0 AC 1\nR1 in out 1k\nC1 out 0 100n\n.ac dec 20 10 100k\n",
        ),
    ] {
        let out = call(
            &ws,
            "create_schematic",
            json!({"circuit": file, "netlist": netlist}),
        )
        .await;
        let text = text_of(&out);
        assert!(!out.is_error, "{text}");
        assert!(
            text.contains("Created") && text.contains("3 component"),
            "{text}"
        );
        let out = call(&ws, "netlist", json!({"circuit": file})).await;
        let net = text_of(&out).to_ascii_lowercase();
        for line in [
            "v1 in 0",
            "r1 in out 1k",
            "c1 out 0 100n",
            ".ac dec 20 10 100k",
        ] {
            assert!(net.contains(line), "{file}: missing `{line}` in\n{net}");
        }
    }
    assert!(dir.path().join("rc.asc").is_file());
}

#[tokio::test(flavor = "multi_thread")]
async fn create_schematic_refuses_unsafe_or_huge_netlists() {
    let (dir, ws) = workspace();
    let control = "* x\nR1 a 0 1k\n.control\nshell rm -rf ~\n.endc\n";
    let out = call(
        &ws,
        "create_schematic",
        json!({"circuit": "bad.asc", "netlist": control}),
    )
    .await;
    assert!(out.is_error, "{}", text_of(&out));
    assert!(text_of(&out).contains("not allowed"), "{}", text_of(&out));

    let huge = format!("* big\n{}", "R1 a 0 1k\n".repeat(10_000));
    let out = call(
        &ws,
        "create_schematic",
        json!({"circuit": "big.asc", "netlist": huge}),
    )
    .await;
    assert!(
        out.is_error && text_of(&out).contains("limit"),
        "{}",
        text_of(&out)
    );

    let many: String = (0..200)
        .map(|i| format!("R{i} n{i} n{} 1k\n", i + 1))
        .collect();
    let out = call(
        &ws,
        "create_schematic",
        json!({"circuit": "many.asc", "netlist": format!("* many\n{many}")}),
    )
    .await;
    assert!(
        out.is_error && text_of(&out).contains("elements"),
        "{}",
        text_of(&out)
    );

    for f in ["bad.asc", "big.asc", "many.asc"] {
        assert!(!dir.path().join(f).exists(), "{f} was written");
    }
}

struct Decline;

#[async_trait::async_trait]
impl aispice_tools::Approver for Decline {
    async fn approve(&self, _circuit: &str, _summary: &str, _diff: &str) -> bool {
        false
    }
}

/// Was a gap: creating a file skipped the approval asked for in
/// ask-before-apply mode.
#[tokio::test(flavor = "multi_thread")]
async fn create_schematic_asks_for_approval() {
    let (dir, ws) = workspace();
    ws.set_hooks(aispice_tools::workspace::Hooks {
        approver: Some(Arc::new(Decline)),
        after_save: None,
    });
    let out = call(
        &ws,
        "create_schematic",
        json!({"circuit": "rc.asc", "netlist": "* rc\nV1 in 0 1\nR1 in 0 1k\n.op\n"}),
    )
    .await;
    assert!(text_of(&out).contains("declined"), "{}", text_of(&out));
    assert!(!dir.path().join("rc.asc").exists());
}

/// Was a gap: an operation that could not do what it was asked came back as
/// a note while the rest of the edit was saved. Now nothing is saved and the
/// error names the edit by its position and op.
#[tokio::test(flavor = "multi_thread")]
async fn a_failing_operation_saves_nothing() {
    let (dir, ws) = workspace();
    let netlist = "* amp\nV1 in 0 AC 1\nR1 in fb 1k\nR2 fb 0 1k\nR3 out 0 1k\n.op\n";
    let out = call(
        &ws,
        "create_schematic",
        json!({"circuit": "a.asc", "netlist": netlist}),
    )
    .await;
    assert!(!out.is_error, "{}", text_of(&out));
    let before = std::fs::read(dir.path().join("a.asc")).unwrap();
    let out = call(
        &ws,
        "edit_schematic",
        json!({"circuit": "a.asc", "edits": [
            {"op": "set_value", "name": "R3", "value": "2k"},
            {"op": "connect_to_net", "pin": "R2.A", "net": "out"}
        ]}),
    )
    .await;
    let text = text_of(&out);
    assert!(out.is_error, "{text}");
    assert!(
        text.contains("No changes made. edit 1 (connect_to_net):"),
        "{text}"
    );
    assert_eq!(std::fs::read(dir.path().join("a.asc")).unwrap(), before);
}
