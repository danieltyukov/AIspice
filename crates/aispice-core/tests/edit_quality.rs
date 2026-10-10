//! Drawing quality of schematics built one edit at a time, the way the agent
//! builds them: parts added near each other without coordinates, then wired
//! pin to pin and joined to named nets.

use aispice_core::edit::{EditOp, apply};
use aispice_core::layout::{Quality, quality};
use aispice_core::lint::{Severity, lint};
use aispice_core::netlist::{build, connect};
use aispice_core::schematic::Schematic;
use aispice_core::symbol::SymbolLibrary;

fn lib() -> SymbolLibrary {
    SymbolLibrary::builtin_only()
}

fn ops(json: &str) -> Vec<EditOp> {
    serde_json::from_str(json).expect("valid ops json")
}

/// A common-emitter amplifier, placed with `near` and wired with `connect`
/// and `connect_to_net`, exactly as an agent would issue the edits.
fn common_emitter_by_edits() -> Schematic {
    let mut sch = Schematic::new();
    apply(
        &mut sch,
        &lib(),
        &ops(r#"[
            {"op": "add_component", "symbol": "voltage", "name": "VCC", "value": "12"},
            {"op": "add_component", "symbol": "Misc\\signal", "name": "VIN", "value": "SINE(0 10m 1k)", "near": "VCC"},
            {"op": "add_component", "symbol": "cap", "name": "C1", "value": "10u", "orient": "R90", "near": "VIN"},
            {"op": "add_component", "symbol": "res", "name": "R1", "value": "47k", "near": "C1"},
            {"op": "add_component", "symbol": "res", "name": "R2", "value": "10k", "near": "R1"},
            {"op": "add_component", "symbol": "npn", "name": "Q1", "value": "2N3904", "near": "R1"},
            {"op": "add_component", "symbol": "res", "name": "RC", "value": "4.7k", "near": "Q1"},
            {"op": "add_component", "symbol": "res", "name": "RE", "value": "1k", "near": "Q1"},
            {"op": "add_component", "symbol": "cap", "name": "CE", "value": "100u", "near": "RE"},
            {"op": "add_component", "symbol": "cap", "name": "C2", "value": "10u", "orient": "R90", "near": "RC"},
            {"op": "add_component", "symbol": "res", "name": "RL", "value": "10k", "near": "C2"},
            {"op": "connect", "from": "VIN.+", "to": "C1.B"},
            {"op": "connect", "from": "C1.A", "to": "Q1.B"},
            {"op": "connect", "from": "R1.B", "to": "Q1.B"},
            {"op": "connect", "from": "R2.A", "to": "Q1.B"},
            {"op": "connect", "from": "RC.B", "to": "Q1.C"},
            {"op": "connect", "from": "RE.A", "to": "Q1.E"},
            {"op": "connect", "from": "CE.A", "to": "Q1.E"},
            {"op": "connect", "from": "C2.B", "to": "Q1.C"},
            {"op": "connect", "from": "RL.A", "to": "C2.A"},
            {"op": "connect_to_net", "pin": "VCC.+", "net": "vcc"},
            {"op": "connect_to_net", "pin": "R1.A", "net": "vcc"},
            {"op": "connect_to_net", "pin": "RC.A", "net": "vcc"},
            {"op": "connect_to_net", "pin": "VCC.-", "net": "0"},
            {"op": "connect_to_net", "pin": "VIN.-", "net": "0"},
            {"op": "connect_to_net", "pin": "R2.B", "net": "0"},
            {"op": "connect_to_net", "pin": "RE.B", "net": "0"},
            {"op": "connect_to_net", "pin": "CE.B", "net": "0"},
            {"op": "connect_to_net", "pin": "RL.B", "net": "0"},
            {"op": "connect_to_net", "pin": "RL.A", "net": "out"},
            {"op": "add_directive", "text": ".tran 0 5m"}
        ]"#),
    )
    .expect("edits apply");
    sch
}

fn net(sch: &Schematic, inst: &str, pin: &str) -> String {
    connect(sch, &lib())
        .net_of(inst, pin)
        .map(|n| n.name.clone())
        .unwrap_or_default()
}

#[test]
fn common_emitter_built_by_edits_is_connected_correctly() {
    let sch = common_emitter_by_edits();
    // Base node: C1, R1, R2 and Q1's base together.
    let b = net(&sch, "Q1", "B");
    for (inst, pin) in [("C1", "A"), ("R1", "B"), ("R2", "A")] {
        assert_eq!(net(&sch, inst, pin), b, "{inst}.{pin}");
    }
    assert_eq!(net(&sch, "RC", "A"), "vcc");
    assert_eq!(net(&sch, "R1", "A"), "vcc");
    assert_eq!(net(&sch, "RL", "A"), "out");
    assert_eq!(net(&sch, "RE", "B"), "0");
    let findings = lint(&sch, &lib()).findings;
    assert!(
        !findings.iter().any(|f| f.severity == Severity::Error),
        "{findings:#?}"
    );
    let (built, _) = build(&sch, &lib(), "* ce");
    assert_eq!(built.netlist.elements().count(), 11);
}

#[test]
fn common_emitter_built_by_edits_reads_cleanly() {
    let sch = common_emitter_by_edits();
    let q: Quality = quality(&sch, &lib());
    // For looking at the drawing: render it with the layout gallery example.
    if let Some(dir) = std::env::var_os("AISPICE_DUMP_DIR") {
        let path = std::path::Path::new(&dir).join("edit_common_emitter.asc");
        std::fs::write(path, aispice_core::schematic::write(&sch)).unwrap();
    }
    assert_eq!(q.wires_through_bodies, 0, "{q:?}");
    assert_eq!(q.overlapping_parts, 0, "{q:?}");
    assert!(q.crossings <= 2, "{q:?}");
    assert!(q.overlapping_text <= 3, "{q:?}");
}
