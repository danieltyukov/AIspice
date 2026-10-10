//! Electrical rule checks.
//!
//! Each finding names the rule, the parts and nets involved, and where on the
//! sheet to look, so both a person and the agent can act on it. Rules aim at
//! what makes a simulation fail or lie: no ground, floating nodes, no DC path,
//! shorted sources, missing values or analysis. Readability rules (off-grid,
//! overlapping parts) are reported as information.

use crate::geometry::{GRID, Point};
use crate::netlist::{Connectivity, connect};
use crate::schematic::{Item, Schematic};
use crate::symbol::SymbolLibrary;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    /// Stable rule id, e.g. `no-ground`.
    pub rule: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Point>,
}

impl Finding {
    fn new(severity: Severity, rule: &str, message: impl Into<String>) -> Self {
        Self {
            severity,
            rule: rule.into(),
            message: message.into(),
            parts: vec![],
            nets: vec![],
            at: None,
        }
    }
    fn parts(mut self, p: &[&str]) -> Self {
        self.parts = p.iter().map(|s| s.to_string()).collect();
        self
    }
    fn nets(mut self, n: &[&str]) -> Self {
        self.nets = n.iter().map(|s| s.to_string()).collect();
        self
    }
    fn at(mut self, p: Option<Point>) -> Self {
        self.at = p;
        self
    }
}

/// Every rule id, for documentation and for turning rules off.
pub const RULES: &[(&str, &str)] = &[
    (
        "no-ground",
        "The circuit has no ground (a flag labelled 0).",
    ),
    (
        "unknown-symbol",
        "A symbol could not be found in any library.",
    ),
    ("missing-name", "A component has no instance name."),
    ("duplicate-name", "Two components share an instance name."),
    (
        "missing-value",
        "A resistor, capacitor or inductor has no value.",
    ),
    ("floating-pin", "A pin connects to nothing."),
    ("dangling-wire", "A wire end touches nothing."),
    ("single-pin-net", "An unlabelled net reaches only one pin."),
    (
        "shorted-part",
        "Both terminals of a two-terminal part are on the same net.",
    ),
    (
        "shorted-source",
        "A voltage source's terminals are on the same net.",
    ),
    (
        "parallel-sources",
        "Two voltage sources are connected directly in parallel.",
    ),
    (
        "no-dc-path",
        "A net reaches ground only through capacitors or current sources.",
    ),
    (
        "no-analysis",
        "There is no analysis directive (.tran, .ac, .dc, .op, .noise, .tf).",
    ),
    (
        "multiple-labels",
        "One net carries several different labels.",
    ),
    ("off-grid", "A part or wire is not on the 16-unit grid."),
    ("overlap", "Two parts are drawn on top of each other."),
];

pub struct LintReport {
    pub findings: Vec<Finding>,
    pub connectivity: Connectivity,
}

impl LintReport {
    pub fn errors(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .count()
    }
    pub fn warnings(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
            .count()
    }
}

/// Run every rule.
pub fn lint(sch: &Schematic, lib: &SymbolLibrary) -> LintReport {
    let conn = connect(sch, lib);
    let mut out = Vec::new();

    for w in &conn.warnings {
        let rule = if w.message.contains("no symbol named") {
            "unknown-symbol"
        } else if w.message.contains("no InstName") {
            "missing-name"
        } else {
            "multiple-labels"
        };
        let sev = if rule == "multiple-labels" {
            Severity::Warning
        } else {
            Severity::Error
        };
        out.push(Finding::new(sev, rule, w.message.clone()).at(w.at));
    }

    if !conn.nets.iter().any(|n| n.is_ground()) && !conn.nets.is_empty() {
        out.push(Finding::new(
            Severity::Error,
            "no-ground",
            "No ground. Add a flag labelled 0 on the reference node; SPICE needs one.",
        ));
    }

    // Names.
    let mut seen: HashMap<String, usize> = HashMap::new();
    for p in &conn.placed {
        if !p.inst.is_empty() {
            *seen.entry(p.inst.to_ascii_uppercase()).or_default() += 1;
        }
    }
    let mut dup: Vec<_> = seen.into_iter().filter(|(_, c)| *c > 1).collect();
    dup.sort();
    for (name, count) in dup {
        out.push(
            Finding::new(
                Severity::Error,
                "duplicate-name",
                format!("{count} components are named {name}; instance names must be unique."),
            )
            .parts(&[&name]),
        );
    }

    // Per part checks.
    for p in &conn.placed {
        let Some(def) = &p.def else { continue };
        let Item::Symbol(sym) = &sch.items[p.item] else {
            continue;
        };
        let letter = def
            .prefix()
            .chars()
            .next()
            .unwrap_or('X')
            .to_ascii_uppercase();
        if matches!(letter, 'R' | 'C' | 'L') {
            let value = sym.value().unwrap_or("").trim();
            if value.is_empty() || value.eq_ignore_ascii_case(&letter.to_string()) {
                out.push(
                    Finding::new(
                        Severity::Error,
                        "missing-value",
                        format!("{} has no value.", p.inst),
                    )
                    .parts(&[&p.inst])
                    .at(Some(sym.at)),
                );
            }
        }
        let pins = conn.pin_nets.get(&p.inst.to_ascii_uppercase());
        if let Some(pins) = pins
            && pins.len() == 2
            && pins[0].1 == pins[1].1
        {
            let net = &conn.nets[pins[0].1].name;
            let (rule, sev) = if letter == 'V' {
                ("shorted-source", Severity::Error)
            } else {
                ("shorted-part", Severity::Warning)
            };
            out.push(
                Finding::new(
                    sev,
                    rule,
                    format!("Both terminals of {} are on net {net}.", p.inst),
                )
                .parts(&[&p.inst])
                .nets(&[net])
                .at(Some(sym.at)),
            );
        }
        if !sym.at.on_grid() {
            out.push(
                Finding::new(
                    Severity::Info,
                    "off-grid",
                    format!("{} is at {}, off the 16-unit grid.", p.inst, sym.at),
                )
                .parts(&[&p.inst])
                .at(Some(sym.at)),
            );
        }
    }

    // Floating pins and single-pin nets.
    for net in &conn.nets {
        if net.name.starts_with("NC_") {
            for pin in &net.pins {
                out.push(
                    Finding::new(
                        Severity::Warning,
                        "floating-pin",
                        format!("Pin {} of {} connects to nothing.", pin.pin, pin.inst),
                    )
                    .parts(&[&pin.inst])
                    .at(Some(pin.at)),
                );
            }
        } else if !net.labelled && net.pins.len() == 1 {
            let pin = &net.pins[0];
            out.push(
                Finding::new(
                    Severity::Warning,
                    "single-pin-net",
                    format!(
                        "Pin {} of {} reaches a wire but no other pin.",
                        pin.pin, pin.inst
                    ),
                )
                .parts(&[&pin.inst])
                .nets(&[&net.name])
                .at(Some(pin.at)),
            );
        }
    }

    // Dangling wire ends: an end that touches no other wire, pin or flag.
    let mut touch: HashMap<Point, usize> = HashMap::new();
    for w in sch.wires() {
        *touch.entry(w.a).or_default() += 1;
        *touch.entry(w.b).or_default() += 1;
    }
    let anchors: HashSet<Point> = conn
        .nets
        .iter()
        .flat_map(|n| n.pins.iter().map(|p| p.at))
        .chain(sch.flags().map(|f| f.at))
        .collect();
    let wires: Vec<_> = sch.wires().copied().collect();
    let segs: Vec<(Point, Point)> = wires.iter().map(|w| (w.a, w.b)).collect();
    let (index, _) = crate::geometry::SegmentIndex::new(&segs);
    for w in &wires {
        let horizontal = w.a.y == w.b.y;
        for end in [w.a, w.b] {
            let shared = touch.get(&end).copied().unwrap_or(0) > 1;
            // Another wire covers this end if a perpendicular run passes
            // through it, or if the merged run along this wire extends past it
            // (only another wire can make it extend).
            let c = index.cover(end);
            let (along, across) = if horizontal {
                (c.horizontal, c.vertical)
            } else {
                (c.vertical, c.horizontal)
            };
            let coord = if horizontal { end.x } else { end.y };
            let on_other = across.is_some()
                || along.is_some_and(|s| s.lo < coord && coord < s.hi)
                || c.diagonal.len() > usize::from(w.a.x != w.b.x && w.a.y != w.b.y);
            if !shared && !on_other && !anchors.contains(&end) {
                out.push(
                    Finding::new(
                        Severity::Warning,
                        "dangling-wire",
                        format!("A wire ends at {end} without touching anything."),
                    )
                    .at(Some(end)),
                );
            }
        }
        if !w.a.on_grid() || !w.b.on_grid() {
            out.push(
                Finding::new(
                    Severity::Info,
                    "off-grid",
                    format!("Wire {} to {} is off the 16-unit grid.", w.a, w.b),
                )
                .at(Some(w.a)),
            );
        }
    }

    // Voltage sources in parallel.
    let mut vsrc: HashMap<(usize, usize), Vec<String>> = HashMap::new();
    for p in &conn.placed {
        let Some(def) = &p.def else { continue };
        if def.prefix().to_ascii_uppercase().starts_with('V')
            && let Some(pins) = conn.pin_nets.get(&p.inst.to_ascii_uppercase())
            && pins.len() == 2
            && pins[0].1 != pins[1].1
        {
            let key = (pins[0].1.min(pins[1].1), pins[0].1.max(pins[1].1));
            vsrc.entry(key).or_default().push(p.inst.clone());
        }
    }
    for ((a, b), names) in vsrc {
        if names.len() > 1 {
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            out.push(
                Finding::new(Severity::Error, "parallel-sources", format!("{} are voltage sources in parallel across {} and {}; the circuit has no unique solution.", names.join(" and "), conn.nets[a].name, conn.nets[b].name))
                    .parts(&refs)
                    .nets(&[&conn.nets[a].name, &conn.nets[b].name]),
            );
        }
    }

    // DC path to ground: walk from ground through parts that conduct at DC.
    if let Some(gnd) = conn.nets.iter().position(|n| n.is_ground()) {
        let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
        for p in &conn.placed {
            let Some(def) = &p.def else { continue };
            let letter = def
                .prefix()
                .chars()
                .next()
                .unwrap_or('X')
                .to_ascii_uppercase();
            if matches!(letter, 'C' | 'I') {
                continue;
            }
            let Some(pins) = conn.pin_nets.get(&p.inst.to_ascii_uppercase()) else {
                continue;
            };
            // Controlled sources: only the output pair conducts.
            let take = if matches!(letter, 'E' | 'G' | 'S') {
                2
            } else {
                pins.len()
            };
            let nets: Vec<usize> = pins.iter().take(take).map(|(_, n)| *n).collect();
            for &x in &nets {
                for &y in &nets {
                    if x != y {
                        adj.entry(x).or_default().push(y);
                    }
                }
            }
        }
        let mut reached = HashSet::from([gnd]);
        let mut queue = VecDeque::from([gnd]);
        while let Some(n) = queue.pop_front() {
            for &m in adj.get(&n).into_iter().flatten() {
                if reached.insert(m) {
                    queue.push_back(m);
                }
            }
        }
        for (i, net) in conn.nets.iter().enumerate() {
            if !reached.contains(&i) && !net.name.starts_with("NC_") && net.pins.len() > 1 {
                out.push(
                    Finding::new(Severity::Warning, "no-dc-path", format!("Net {} reaches ground only through capacitors or current sources; add a DC path (for example a large resistor) or the operating point is undefined.", net.name))
                        .nets(&[&net.name])
                        .at(net.pins.first().map(|p| p.at)),
                );
            }
        }
    }

    if !sch
        .directives()
        .flat_map(|t| t.lines())
        .any(|l| crate::netlist::spice::is_analysis(&l))
    {
        out.push(Finding::new(Severity::Warning, "no-analysis", "No analysis directive. Add one of .op, .tran, .ac, .dc, .noise or .tf before simulating."));
    }

    // Overlapping parts.
    let boxes: Vec<(String, crate::geometry::Rect)> = conn
        .placed
        .iter()
        .filter_map(|p| {
            let Item::Symbol(sym) = &sch.items[p.item] else {
                return None;
            };
            let r = p.def.as_ref()?.placed_bounds(sym.at, sym.orient)?;
            Some((p.inst.clone(), r.inflate(-GRID / 2)))
        })
        .collect();
    for i in 0..boxes.len() {
        for j in i + 1..boxes.len() {
            if boxes[i].1.intersects(&boxes[j].1) {
                out.push(
                    Finding::new(
                        Severity::Info,
                        "overlap",
                        format!(
                            "{} and {} are drawn on top of each other.",
                            boxes[i].0, boxes[j].0
                        ),
                    )
                    .parts(&[&boxes[i].0, &boxes[j].0]),
                );
            }
        }
    }

    out.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then(a.rule.cmp(&b.rule))
            .then(a.message.cmp(&b.message))
    });
    LintReport {
        findings: out,
        connectivity: conn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schematic::parse;

    fn rules(src: &str) -> Vec<String> {
        let (sch, _) = parse(src);
        lint(&sch, &SymbolLibrary::builtin_only())
            .findings
            .into_iter()
            .map(|f| f.rule)
            .collect()
    }

    const RC: &str = "Version 4\nSHEET 1 880 680\nWIRE 96 96 32 96\nWIRE 240 96 176 96\nWIRE 240 128 240 96\nFLAG 32 176 0\nFLAG 240 192 0\nFLAG 240 96 out\nSYMBOL voltage 32 80 R0\nSYMATTR InstName V1\nSYMATTR Value SINE(0 1 1k)\nSYMBOL res 192 80 R90\nSYMATTR InstName R1\nSYMATTR Value 1k\nSYMBOL cap 224 128 R0\nSYMATTR InstName C1\nSYMATTR Value 100n\nTEXT 0 232 Left 2 !.tran 5m\n";

    #[test]
    fn clean_circuit_has_no_findings() {
        assert_eq!(rules(RC), Vec::<String>::new());
    }

    #[test]
    fn missing_ground_and_analysis() {
        let src = RC
            .replace("FLAG 32 176 0\n", "FLAG 32 176 gnd2\n")
            .replace("FLAG 240 192 0\n", "FLAG 240 192 gnd2\n")
            .replace("TEXT 0 232 Left 2 !.tran 5m\n", "");
        let r = rules(&src);
        assert!(r.contains(&"no-ground".to_string()), "{r:?}");
        assert!(r.contains(&"no-analysis".to_string()), "{r:?}");
    }

    #[test]
    fn floating_pin_and_missing_value() {
        let src = format!("{RC}SYMBOL res 480 80 R0\nSYMATTR InstName R2\n");
        let r = rules(&src);
        assert_eq!(
            r.iter().filter(|x| *x == "floating-pin").count(),
            2,
            "{r:?}"
        );
        assert!(r.contains(&"missing-value".to_string()));
    }

    #[test]
    fn capacitor_only_node_has_no_dc_path() {
        // C2 in series with C1's top node: the middle node floats at DC.
        let src = "Version 4\nSHEET 1 880 680\nWIRE 16 64 16 96\nFLAG 16 0 in\nFLAG 16 160 0\nSYMBOL cap 0 0 R0\nSYMATTR InstName C1\nSYMATTR Value 1n\nSYMBOL cap 0 96 R0\nSYMATTR InstName C2\nSYMATTR Value 1n\nSYMBOL voltage -128 0 R0\nSYMATTR InstName V1\nSYMATTR Value 1\nFLAG -128 16 in\nFLAG -128 96 0\nTEXT 0 300 Left 2 !.op\n";
        let r = rules(src);
        assert!(r.contains(&"no-dc-path".to_string()), "{r:?}");
    }

    #[test]
    fn duplicate_names() {
        let src = format!("{RC}SYMBOL res 480 80 R0\nSYMATTR InstName R1\nSYMATTR Value 1k\n");
        assert!(rules(&src).contains(&"duplicate-name".to_string()));
    }

    #[test]
    fn parallel_sources() {
        let src = "Version 4\nSHEET 1 880 680\nFLAG 0 16 a\nFLAG 0 96 0\nFLAG 128 16 a\nFLAG 128 96 0\nSYMBOL voltage 0 0 R0\nSYMATTR InstName V1\nSYMATTR Value 1\nSYMBOL voltage 128 0 R0\nSYMATTR InstName V2\nSYMATTR Value 2\nTEXT 0 300 Left 2 !.op\n";
        let r = rules(src);
        assert!(r.contains(&"parallel-sources".to_string()), "{r:?}");
    }

    #[test]
    fn dangling_wire_is_reported() {
        let src = format!("{RC}WIRE 400 400 480 400\n");
        let r = rules(&src);
        assert_eq!(
            r.iter().filter(|x| *x == "dangling-wire").count(),
            2,
            "{r:?}"
        );
    }
}
