use super::*;
use crate::lint::{Severity, lint};
use crate::netlist::connect;
use crate::schematic::parse;

fn lib() -> SymbolLibrary {
    SymbolLibrary::builtin_only()
}

fn ops(json: &str) -> Vec<EditOp> {
    serde_json::from_str(json).expect("valid ops json")
}

fn net(sch: &Schematic, pin: &str) -> String {
    let (inst, p) = split_pin_spec(pin).unwrap();
    connect(sch, &lib())
        .net_of(inst, p)
        .map(|n| n.name.clone())
        .unwrap_or_default()
}

/// Build an RC low-pass from nothing using only circuit-level edits.
#[test]
fn builds_rc_lowpass_from_scratch() {
    let mut sch = Schematic::new();
    let report = apply(
        &mut sch,
        &lib(),
        &ops(r#"[
            {"op": "add_component", "symbol": "voltage", "name": "V1", "value": "SINE(0 1 1k)", "attrs": {"SpiceLine": "AC 1"}},
            {"op": "add_component", "symbol": "res", "name": "R1", "value": "1k", "orient": "R90", "near": "V1"},
            {"op": "add_component", "symbol": "cap", "name": "C1", "value": "100n", "near": "R1"},
            {"op": "connect", "from": "V1.+", "to": "R1.B"},
            {"op": "connect", "from": "R1.A", "to": "C1.A"},
            {"op": "connect_to_net", "pin": "V1.-", "net": "0"},
            {"op": "connect_to_net", "pin": "C1.B", "net": "0"},
            {"op": "connect_to_net", "pin": "C1.A", "net": "out"},
            {"op": "add_directive", "text": ".ac dec 20 10 100k"}
        ]"#),
    )
    .unwrap();
    assert_eq!(report.applied.len(), 9, "{:#?}", report);
    assert_eq!(net(&sch, "V1.+"), net(&sch, "R1.B"));
    assert_eq!(net(&sch, "R1.A"), "out");
    assert_eq!(net(&sch, "C1.A"), "out");
    assert_eq!(net(&sch, "V1.-"), "0");
    assert_eq!(net(&sch, "C1.B"), "0");
    let findings = lint(&sch, &lib()).findings;
    assert!(
        findings.iter().all(|f| f.severity == Severity::Info),
        "{findings:#?}"
    );
    let (built, _) = crate::netlist::build(&sch, &lib(), "* rc");
    let text = crate::netlist::write(&built.netlist);
    assert!(text.contains("C1 out 0 100n"), "{text}");
    assert!(
        text.contains("V1 N001 0 SINE(0 1 1k) AC 1")
            || text.contains("V1 P001 0 SINE(0 1 1k) AC 1"),
        "{text}"
    );
}

#[test]
fn router_never_shorts_other_nets() {
    // R1 and R2 side by side with a wire on net `x` running between them.
    // Connecting R1.A to R2.B must not touch `x`.
    let src = "Version 4\nSHEET 1 880 680\nWIRE 64 -64 64 160\nFLAG 64 -64 x\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMATTR Value 1k\nSYMBOL res 128 0 R0\nSYMATTR InstName R2\nSYMATTR Value 1k\n";
    let (mut sch, _) = parse(src);
    apply(
        &mut sch,
        &lib(),
        &ops(r#"[{"op": "connect", "from": "R1.A", "to": "R2.B"}]"#),
    )
    .unwrap();
    assert_eq!(net(&sch, "R1.A"), net(&sch, "R2.B"));
    assert_ne!(net(&sch, "R1.A"), "x");
    let c = connect(&sch, &lib());
    assert!(c.net("x").unwrap().pins.is_empty(), "net x picked up a pin");
}

#[test]
fn failed_batch_leaves_schematic_untouched() {
    let src =
        "Version 4\nSHEET 1 880 680\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMATTR Value 1k\n";
    let (mut sch, _) = parse(src);
    let before = sch.clone();
    let err = apply(&mut sch, &lib(), &ops(r#"[{"op": "set_value", "name": "R1", "value": "2k"}, {"op": "set_value", "name": "R9", "value": "1"}]"#)).unwrap_err();
    assert!(err.to_string().contains("R9"), "{err}");
    assert!(
        err.to_string().contains("R1"),
        "error lists known names: {err}"
    );
    assert_eq!(sch, before);
}

#[test]
fn remove_prunes_loose_wires() {
    let src = "Version 4\nSHEET 1 880 680\nWIRE 16 96 16 160\nWIRE 16 160 128 160\nFLAG 128 160 0\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMATTR Value 1k\n";
    let (mut sch, _) = parse(src);
    let report = apply(
        &mut sch,
        &lib(),
        &ops(r#"[{"op": "remove", "name": "R1"}]"#),
    )
    .unwrap();
    // Both wires led only to R1, so both go; the flag stays.
    assert_eq!(sch.wires().count(), 0, "{report:?}");
    assert_eq!(sch.flags().count(), 1);
}

#[test]
fn replace_symbol_keeps_position_and_connections() {
    let src = "Version 4\nSHEET 1 880 680\nWIRE 16 0 16 -64\nFLAG 16 -64 a\nFLAG 16 64 0\nSYMBOL cap 0 0 R0\nSYMATTR InstName C1\nSYMATTR Value 1u\n";
    let (mut sch, _) = parse(src);
    apply(
        &mut sch,
        &lib(),
        &ops(r#"[{"op": "replace_symbol", "name": "C1", "symbol": "polcap"}]"#),
    )
    .unwrap();
    let c1 = sch.symbol("C1").unwrap();
    assert_eq!(c1.name, "polcap");
    assert_eq!(c1.value(), Some("1u"));
    assert_eq!(net(&sch, "C1.A"), "a");
    assert_eq!(net(&sch, "C1.B"), "0");
}

#[test]
fn directives_are_escaped_and_replaceable() {
    let mut sch = Schematic::new();
    apply(&mut sch, &lib(), &ops(r#"[{"op": "add_directive", "text": ".tran 1m"}, {"op": "replace_directive", "matching": ".tran", "text": ".tran 5m"}]"#)).unwrap();
    let lines: Vec<String> = sch.directives().flat_map(|t| t.lines()).collect();
    assert_eq!(lines, vec![".tran 5m"]);
    let err = apply(
        &mut sch,
        &lib(),
        &ops(r#"[{"op": "remove_directive", "matching": ".ac"}]"#),
    )
    .unwrap_err();
    assert!(matches!(err, EditError::InBatch { .. }));
}

#[test]
fn bad_pin_reports_valid_pins() {
    let src = "Version 4\nSHEET 1 880 680\nSYMBOL npn 0 0 R0\nSYMATTR InstName Q1\n";
    let (mut sch, _) = parse(src);
    let err = apply(
        &mut sch,
        &lib(),
        &ops(r#"[{"op": "connect_to_net", "pin": "Q1.G", "net": "0"}]"#),
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("C (1)") && msg.contains("B (2)") && msg.contains("E (3)"),
        "{msg}"
    );
}

#[test]
fn auto_placed_parts_start_unconnected() {
    let src = "Version 4\nSHEET 1 880 680\nWIRE -100 16 400 16\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMATTR Value 1k\n";
    let (mut sch, _) = parse(src);
    let report = apply(
        &mut sch,
        &lib(),
        &ops(r#"[{"op": "add_component", "symbol": "res", "value": "2k", "near": "R1"}]"#),
    )
    .unwrap();
    assert_eq!(report.added, vec!["R2"]);
    assert!(
        net(&sch, "R2.A").starts_with("NC_"),
        "{}",
        net(&sch, "R2.A")
    );
    assert!(net(&sch, "R2.B").starts_with("NC_"));
}

#[test]
fn line_breaks_cannot_inject_records() {
    let mut sch = Schematic::new();
    apply(
        &mut sch,
        &lib(),
        &ops(r#"[{"op": "add_component", "symbol": "res", "name": "R1", "value": "1k"}]"#),
    )
    .unwrap();
    let before = sch.clone();
    for bad in [
        r#"[{"op": "set_value", "name": "R1", "value": "1k\nTEXT 0 0 Left 2 !.control"}]"#,
        r#"[{"op": "set_attr", "name": "R1", "key": "SpiceLine", "value": "a\rb"}]"#,
        r#"[{"op": "add_label", "at": [0, 0], "label": "out\nWIRE 0 0 1 1"}]"#,
        r#"[{"op": "rename", "name": "R1", "new_name": "R 2"}]"#,
        r#"[{"op": "add_component", "symbol": "res\nTEXT", "value": "1"}]"#,
        r#"[{"op": "connect_to_net", "pin": "R1.A", "net": "a b"}]"#,
    ] {
        let err = apply(&mut sch, &lib(), &ops(bad)).unwrap_err();
        assert!(err.to_string().contains("not allowed"), "{err}");
        assert_eq!(sch, before);
    }
    // A multi-line directive stays one TEXT record.
    apply(
        &mut sch,
        &lib(),
        &ops(r#"[{"op": "add_directive", "text": ".param a=1\n.op"}]"#),
    )
    .unwrap();
    let text = crate::schematic::write(&sch);
    assert_eq!(text.matches("TEXT ").count(), 1, "{text}");
}

#[test]
fn unsafe_attributes_and_directives_are_refused() {
    let mut sch = Schematic::new();
    apply(&mut sch, &lib(), &ops(r#"[{"op": "add_component", "symbol": "res", "name": "R1", "value": "1k"}, {"op": "add_component", "symbol": "res", "name": "R2", "value": "1k"}]"#)).unwrap();
    let before = sch.clone();
    for bad in [
        r#"[{"op": "set_attr", "name": "R1", "key": "InstName", "value": "R1 x"}]"#,
        r#"[{"op": "set_attr", "name": "R1", "key": "InstName", "value": "R2"}]"#,
        r#"[{"op": "set_attr", "name": "R1", "key": "InstName", "value": ""}]"#,
        r#"[{"op": "set_attr", "name": "R1", "key": "SpiceModel", "value": "/etc/passwd"}]"#,
        r#"[{"op": "set_attr", "name": "R1", "key": "SpiceModel", "value": "../../x.lib"}]"#,
        r#"[{"op": "set_value", "name": "R1", "value": "1k\u2028TEXT"}]"#,
        r#"[{"op": "add_directive", "text": ".control\nshell id\n.endc"}]"#,
        r#"[{"op": "add_directive", "text": ".ferret http://example.com/m.lib"}]"#,
        r#"[{"op": "add_directive", "text": ".include /etc/passwd"}]"#,
        r#"[{"op": "replace_directive", "matching": "x", "text": ".lib ../../secret"}]"#,
    ] {
        assert!(apply(&mut sch, &lib(), &ops(bad)).is_err(), "{bad}");
        assert_eq!(sch, before);
    }
}

#[test]
fn writer_never_emits_a_record_break_inside_a_field() {
    let mut sch = Schematic::new();
    // Built directly, bypassing edit validation, as other code might.
    let mut s = crate::schematic::Symbol::new("res", Point::new(0, 0), Orient::R0);
    s.set_attr("InstName", "R1");
    s.set_attr("Value", "1k\r.control\u{2028}x");
    sch.items.push(Item::Symbol(s));
    sch.items
        .push(Item::Text(Text::comment(Point::new(0, 0), "a\rb\nc")));
    let text = crate::schematic::write(&sch);
    let records = text.split(['\n', '\r']).filter(|l| !l.is_empty()).count();
    assert_eq!(records, 2 + 3 + 1, "{text:?}");
}

#[test]
fn side_by_side_vertical_parts_get_a_wire_not_labels() {
    let mut sch = Schematic::new();
    let report = apply(
        &mut sch,
        &lib(),
        &ops(r#"[
            {"op": "add_component", "symbol": "voltage", "name": "V1", "value": "10"},
            {"op": "add_component", "symbol": "res", "name": "R1", "value": "10k", "near": "V1"},
            {"op": "add_component", "symbol": "res", "name": "R2", "value": "10k", "near": "R1"},
            {"op": "connect", "from": "V1.+", "to": "R1.A"},
            {"op": "connect", "from": "R1.B", "to": "R2.A"}
        ]"#),
    )
    .unwrap();
    assert!(
        report.applied.iter().all(|a| !a.contains("net label")),
        "{:#?}",
        report.applied
    );
    assert_eq!(net(&sch, "R1.B"), net(&sch, "R2.A"));
}

#[test]
fn naming_a_labelled_net_renames_instead_of_stacking() {
    let src = "Version 4\nSHEET 1 880 680\nWIRE 16 96 16 128\nFLAG 16 128 n7\nFLAG 400 400 n7\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMATTR Value 1k\nSYMBOL res 384 304 R0\nSYMATTR InstName R2\nSYMATTR Value 1k\n";
    let (mut sch, _) = parse(src);
    apply(
        &mut sch,
        &lib(),
        &ops(r#"[{"op": "connect_to_net", "pin": "R1.B", "net": "mid"}]"#),
    )
    .unwrap();
    assert_eq!(net(&sch, "R1.B"), "mid");
    assert_eq!(sch.flags().filter(|f| f.label == "mid").count(), 2);
    let findings = lint(&sch, &lib()).findings;
    assert!(
        !findings.iter().any(|f| f.rule == "multiple-labels"),
        "{findings:#?}"
    );
}

#[test]
fn explicit_positions_snap_and_avoid_overlaps() {
    let mut sch = Schematic::new();
    let report = apply(
        &mut sch,
        &lib(),
        &ops(r#"[
            {"op": "add_component", "symbol": "voltage", "name": "V1", "value": "1", "at": [0, 0]},
            {"op": "add_component", "symbol": "res", "name": "R1", "value": "1k", "at": [2, 0]},
            {"op": "add_component", "symbol": "cap", "name": "C1", "value": "1n", "at": [0, 7]}
        ]"#),
    )
    .unwrap();
    assert_eq!(report.warnings.len(), 2, "{:#?}", report.warnings);
    for s in sch.symbols() {
        assert!(s.at.on_grid(), "{} at {}", s.inst_name().unwrap(), s.at);
    }
    let findings = lint(&sch, &lib()).findings;
    assert!(
        !findings.iter().any(|f| f.rule == "overlap"),
        "{findings:#?}"
    );
}
