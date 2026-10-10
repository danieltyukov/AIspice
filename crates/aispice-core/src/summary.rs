//! A schematic described in circuit terms: parts, the net on every pin, nets
//! with their members, and directives. This is what the agent reads instead of
//! raw `.asc` coordinates, and what the desktop app lists beside the drawing.

use crate::geometry::{Orient, Point};
use crate::lint::{Finding, lint};
use crate::netlist::build::{effective_attr, subckt_call};
use crate::netlist::spice::is_analysis;
use crate::schematic::{Schematic, TextKind};
use crate::symbol::SymbolLibrary;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinInfo {
    pub pin: String,
    pub net: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentInfo {
    pub name: String,
    pub symbol: String,
    /// What set_value changes. For a part that calls a subcircuit (an
    /// op-amp), this is the subcircuit's name, from the symbol when the
    /// instance does not set it.
    pub value: Option<String>,
    /// SpiceLine and SpiceLine2 as they reach the netlist (the instance's,
    /// else the symbol's defaults): parameters such as `GBW=10Meg` or
    /// `AC 1`, changed with set_attr.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
    /// Other attributes set on the instance (Value2, SpiceModel...).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, String>,
    pub at: Point,
    pub orient: Orient,
    pub pins: Vec<PinInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetInfo {
    pub name: String,
    pub labelled: bool,
    /// `R1.A` style pin references.
    pub members: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchematicSummary {
    pub components: Vec<ComponentInfo>,
    pub nets: Vec<NetInfo>,
    pub directives: Vec<String>,
    pub comments: Vec<String>,
    /// The first analysis directive, if any.
    pub analysis: Option<String>,
    pub findings: Vec<Finding>,
}

pub fn summarize(sch: &Schematic, lib: &SymbolLibrary) -> SchematicSummary {
    let report = lint(sch, lib);
    let conn = &report.connectivity;
    let mut components = Vec::new();
    for p in &conn.placed {
        let crate::schematic::Item::Symbol(sym) = &sch.items[p.item] else {
            continue;
        };
        let pins = conn
            .pin_nets
            .get(&p.inst.to_ascii_uppercase())
            .map(|v| {
                v.iter()
                    .map(|(pin, n)| PinInfo {
                        pin: pin.clone(),
                        net: conn.nets[*n].name.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let is_param =
            |k: &str| k.eq_ignore_ascii_case("SpiceLine") || k.eq_ignore_ascii_case("SpiceLine2");
        let attrs = sym
            .attrs
            .iter()
            .filter(|a| {
                !a.key.eq_ignore_ascii_case("InstName")
                    && !a.key.eq_ignore_ascii_case("Value")
                    && !is_param(&a.key)
            })
            .map(|a| (a.key.clone(), a.value.clone()))
            .collect();
        let mut params = BTreeMap::new();
        let mut value = sym.value().map(str::to_string);
        if let Some(def) = &p.def {
            for key in ["SpiceLine", "SpiceLine2"] {
                if let Some(v) = effective_attr(sym, def, key) {
                    params.insert(key.to_string(), v.to_string());
                }
            }
            // A subcircuit call's value is the subcircuit's name, which the
            // symbol supplies when the instance does not.
            if value.is_none() && subckt_call(sym, def).is_some() {
                value = effective_attr(sym, def, "Value").map(str::to_string);
            }
        }
        components.push(ComponentInfo {
            name: p.inst.clone(),
            symbol: sym.name.clone(),
            value,
            params,
            attrs,
            at: sym.at,
            orient: sym.orient,
            pins,
            description: p
                .def
                .as_ref()
                .and_then(|d| d.description().map(str::to_string)),
        });
    }
    let nets = conn
        .nets
        .iter()
        .map(|n| NetInfo {
            name: n.name.clone(),
            labelled: n.labelled,
            members: n
                .pins
                .iter()
                .map(|p| format!("{}.{}", p.inst, p.pin))
                .collect(),
        })
        .collect();
    let directives: Vec<String> = sch
        .texts()
        .filter(|t| t.kind == TextKind::Directive)
        .flat_map(|t| t.lines())
        .collect();
    let comments = sch
        .texts()
        .filter(|t| t.kind == TextKind::Comment)
        .map(|t| t.content.replace("\\n", " "))
        .collect();
    let analysis = directives.iter().find(|d| is_analysis(d)).cloned();
    SchematicSummary {
        components,
        nets,
        directives,
        comments,
        analysis,
        findings: report.findings,
    }
}

impl SchematicSummary {
    /// Compact text for a language model: one line per part, one per net.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "Components ({}):", self.components.len());
        let name_w = self
            .components
            .iter()
            .map(|c| c.name.len())
            .max()
            .unwrap_or(2)
            .max(2);
        let sym_w = self
            .components
            .iter()
            .map(|c| c.symbol.len())
            .max()
            .unwrap_or(3)
            .max(3);
        for c in &self.components {
            // The value column is what set_value changes; bracketed entries
            // are attributes, changed with set_attr.
            let mut value = c.value.clone().unwrap_or_default();
            for (k, v) in c.params.iter().chain(&c.attrs) {
                let _ = write!(value, " [{k}: {v}]");
            }
            let pins: Vec<String> = c
                .pins
                .iter()
                .map(|p| format!("{}:{}", p.pin, p.net))
                .collect();
            let _ = writeln!(
                out,
                "  {:name_w$}  {:sym_w$}  {}  ({})",
                c.name,
                c.symbol,
                value.trim(),
                pins.join(" ")
            );
        }
        let _ = writeln!(out, "Nets ({}):", self.nets.len());
        for n in &self.nets {
            let _ = writeln!(
                out,
                "  {}{}: {}",
                n.name,
                if n.labelled { " [label]" } else { "" },
                n.members.join(", ")
            );
        }
        if self.directives.is_empty() {
            let _ = writeln!(out, "Directives: none");
        } else {
            let _ = writeln!(out, "Directives:");
            for d in &self.directives {
                let _ = writeln!(out, "  {d}");
            }
        }
        if !self.comments.is_empty() {
            let _ = writeln!(out, "Comments: {}", self.comments.join(" | "));
        }
        if self.findings.is_empty() {
            let _ = writeln!(out, "Checks: no problems found.");
        } else {
            let _ = writeln!(out, "Checks:");
            for f in &self.findings {
                let _ = writeln!(out, "  {:?} [{}] {}", f.severity, f.rule, f.message);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schematic::parse;

    #[test]
    fn rc_summary_text() {
        let src = "Version 4\nSHEET 1 880 680\nWIRE 96 96 32 96\nWIRE 240 96 176 96\nWIRE 240 128 240 96\nFLAG 32 176 0\nFLAG 240 192 0\nFLAG 240 96 out\nSYMBOL voltage 32 80 R0\nSYMATTR InstName V1\nSYMATTR Value SINE(0 1 1k)\nSYMATTR SpiceLine AC 1\nSYMBOL res 192 80 R90\nSYMATTR InstName R1\nSYMATTR Value 1k\nSYMBOL cap 224 128 R0\nSYMATTR InstName C1\nSYMATTR Value 100n\nTEXT 0 232 Left 2 !.ac dec 20 10 100k\n";
        let (sch, _) = parse(src);
        let s = summarize(&sch, &SymbolLibrary::builtin_only());
        assert_eq!(s.analysis.as_deref(), Some(".ac dec 20 10 100k"));
        let text = s.to_text();
        assert!(
            text.contains("V1  voltage  SINE(0 1 1k) [SpiceLine: AC 1]  (+:N001 -:0)"),
            "{text}"
        );
        assert!(text.contains("R1  res      1k  (A:out B:N001)"), "{text}");
        assert!(text.contains("out [label]: R1.A, C1.A"), "{text}");
        assert!(text.contains("Checks: no problems found."), "{text}");
    }

    /// Was a trap: the op-amp's SpiceLine2 (`GBW=1Meg`) showed in the value
    /// column, so models called set_value with it and replaced the
    /// subcircuit name. The value is the subcircuit name; parameters are
    /// listed apart, as attributes.
    #[test]
    fn opamp_value_is_its_subckt_and_params_are_apart() {
        let src = "Version 4\nSHEET 1 880 680\nFLAG 192 96 fb\nFLAG 192 128 in\nFLAG 256 112 out\nSYMBOL OpAmps/opamp 224 48 R0\nSYMATTR InstName U1\nSYMATTR SpiceLine2 GBW=1Meg\nTEXT 0 232 Left 2 !.op\n";
        let (sch, _) = parse(src);
        let s = summarize(&sch, &SymbolLibrary::builtin_only());
        let u1 = &s.components[0];
        assert_eq!(u1.value.as_deref(), Some("opamp"));
        assert_eq!(u1.params["SpiceLine"], "Aol=100K");
        assert_eq!(u1.params["SpiceLine2"], "GBW=1Meg");
        assert!(u1.attrs.is_empty(), "{:?}", u1.attrs);
        let text = s.to_text();
        assert!(
            text.contains("U1  OpAmps/opamp  opamp [SpiceLine: Aol=100K] [SpiceLine2: GBW=1Meg]  (invin:fb noninvin:in out:out)"),
            "{text}"
        );
    }
}
