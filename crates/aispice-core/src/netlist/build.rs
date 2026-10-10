//! Derive a SPICE netlist from a schematic, the way LTspice does.
//!
//! Each convention here was checked against LTspice XVII's `-netlist` output:
//! instance names, the extra substrate or bulk node of three-pin transistors,
//! the order attributes are joined in, `.lib` lines for SpiceModel, and the
//! default `.model` cards for parts left at their generic model.

use super::connect::{Connectivity, connect};
use super::spice::{Element, Line, Netlist};
use crate::schematic::{Item, Schematic, TextKind};
use crate::symbol::{SymbolLibrary, SymbolType};
use serde::{Deserialize, Serialize};

/// Generic model names LTspice defines on the fly, and the standard library
/// file it adds after them.
const DEFAULT_MODELS: &[(&str, &[&str], &str)] = &[
    ("D", &["D"], "standard.dio"),
    ("NPN", &["NPN", "PNP"], "standard.bjt"),
    ("PNP", &["NPN", "PNP"], "standard.bjt"),
    ("NMOS", &["NMOS", "PMOS"], "standard.mos"),
    ("PMOS", &["NMOS", "PMOS"], "standard.mos"),
    ("NJF", &["NJF", "PJF"], "standard.jft"),
    ("PJF", &["NJF", "PJF"], "standard.jft"),
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Built {
    pub netlist: Netlist,
    /// LTspice standard library files the netlist relies on (`standard.bjt`,
    /// ...). LTspice finds these itself; other simulators need the models
    /// resolved, which is the simulation layer's job.
    pub standard_libs: Vec<String>,
    /// `.lib` files named by SpiceModel attributes.
    pub model_libs: Vec<String>,
    pub warnings: Vec<String>,
}

/// The netlist instance name for a symbol: the prefix's device letter plus the
/// instance name, with LTspice's `§` separator when the name does not already
/// start with that letter. Subcircuits always get `X` prepended.
pub fn instance_name(prefix: &str, inst: &str) -> String {
    let letter = prefix.chars().next().unwrap_or('X').to_ascii_uppercase();
    if letter == 'X' {
        return format!("X{inst}");
    }
    match inst.chars().next() {
        Some(c) if c.eq_ignore_ascii_case(&letter) => inst.to_string(),
        _ => format!("{letter}\u{a7}{inst}"),
    }
}

/// Build the netlist. `title` becomes the first line (LTspice writes the
/// schematic's path there).
pub fn build(sch: &Schematic, lib: &SymbolLibrary, title: &str) -> (Built, Connectivity) {
    let conn = connect(sch, lib);
    let mut built = Built {
        warnings: conn.warnings.iter().map(|w| w.message.clone()).collect(),
        ..Default::default()
    };
    let mut items = Vec::new();
    let mut defaults_needed: Vec<&str> = Vec::new();

    for p in &conn.placed {
        let Item::Symbol(sym) = &sch.items[p.item] else {
            continue;
        };
        let Some(def) = &p.def else { continue };
        if def.kind == SymbolType::Block {
            built.warnings.push(format!("{}: hierarchical blocks are not netlisted by aispice yet; use LTspice's netlister for this schematic", p.inst));
            continue;
        }
        let prefix = sym
            .attr("Prefix")
            .unwrap_or_else(|| def.prefix())
            .to_string();
        let name = instance_name(&prefix, &p.inst);
        let pins = conn
            .pin_nets
            .get(&p.inst.to_ascii_uppercase())
            .cloned()
            .unwrap_or_default();
        let mut nodes: Vec<String> = pins
            .iter()
            .map(|(_, n)| conn.nets[*n].name.clone())
            .collect();
        let up = prefix.to_ascii_uppercase();
        if (up == "QN" || up == "QP") && nodes.len() == 3 {
            nodes.push("0".into());
        }
        if (up == "MN" || up == "MP") && nodes.len() == 3 {
            nodes.push(nodes[2].clone());
        }
        let field = |key: &str| {
            sym.attr(key)
                .or_else(|| def.attr(key))
                .map(str::trim)
                .filter(|v| !v.is_empty())
        };
        let rest: Vec<&str> = ["Value", "Value2", "SpiceLine", "SpiceLine2"]
            .into_iter()
            .filter_map(field)
            .collect();
        let rest = rest.join(" ");
        if let Some(model) = rest.split_whitespace().next() {
            let model_up = model.to_ascii_uppercase();
            if let Some((_, cards, _)) = DEFAULT_MODELS.iter().find(|(m, _, _)| *m == model_up) {
                let letter = prefix.chars().next().unwrap_or(' ').to_ascii_uppercase();
                if matches!(letter, 'D' | 'Q' | 'M' | 'J') && !defaults_needed.contains(&cards[0]) {
                    defaults_needed.push(cards[0]);
                }
            }
        }
        if let Some(lib_file) = field("SpiceModel")
            && !built
                .model_libs
                .iter()
                .any(|l| l.eq_ignore_ascii_case(lib_file))
        {
            built.model_libs.push(lib_file.to_string());
        }
        items.push(Line::Element(Element { name, nodes, rest }));
    }

    for t in sch.texts() {
        match t.kind {
            TextKind::Directive => {
                for line in t.lines() {
                    items.push(Line::Directive {
                        text: line.to_string(),
                    });
                }
            }
            TextKind::Comment => {
                for line in t.content.split("\\n") {
                    if !line.trim().is_empty() {
                        items.push(Line::Comment {
                            text: line.trim().to_string(),
                        });
                    }
                }
            }
        }
    }

    for first in defaults_needed {
        let (_, cards, file) = DEFAULT_MODELS
            .iter()
            .find(|(m, _, _)| *m == first)
            .expect("listed");
        for c in *cards {
            items.push(Line::Directive {
                text: format!(".model {c} {c}"),
            });
        }
        if !built.standard_libs.iter().any(|l| l == file) {
            built.standard_libs.push(file.to_string());
        }
    }
    for l in &built.model_libs {
        items.push(Line::Directive {
            text: format!(".lib {l}"),
        });
    }

    built.netlist = Netlist {
        title: title.to_string(),
        items,
    };
    (built, conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schematic::parse;

    #[test]
    fn instance_names_follow_ltspice() {
        assert_eq!(instance_name("R", "R1"), "R1");
        assert_eq!(instance_name("R", "load"), "R\u{a7}load");
        assert_eq!(instance_name("QN", "T1"), "Q\u{a7}T1");
        assert_eq!(instance_name("QN", "q2"), "q2");
        assert_eq!(instance_name("X", "U1"), "XU1");
        assert_eq!(instance_name("X", "X1"), "XX1");
    }

    #[test]
    fn rc_low_pass_netlist() {
        let src = "Version 4\nSHEET 1 880 680\nWIRE 96 96 32 96\nWIRE 240 96 176 96\nWIRE 240 128 240 96\nFLAG 32 176 0\nFLAG 240 192 0\nFLAG 240 96 out\nSYMBOL voltage 32 80 R0\nSYMATTR InstName V1\nSYMATTR Value SINE(0 1 1k)\nSYMATTR SpiceLine AC 1\nSYMBOL res 192 80 R90\nSYMATTR InstName R1\nSYMATTR Value 1k\nSYMBOL cap 224 128 R0\nSYMATTR InstName C1\nSYMATTR Value 100n\nTEXT 0 232 Left 2 !.ac dec 20 10 100k\nTEXT 0 264 Left 2 ;RC low-pass\n";
        let (sch, _) = parse(src);
        let (built, _) = build(&sch, &SymbolLibrary::builtin_only(), "* rc.asc");
        let text = crate::netlist::write(&built.netlist);
        assert_eq!(
            text,
            "* rc.asc\nV1 N001 0 SINE(0 1 1k) AC 1\nR1 out N001 1k\nC1 out 0 100n\n.ac dec 20 10 100k\n* RC low-pass\n.end\n"
        );
    }

    #[test]
    fn transistors_get_extra_nodes_and_default_models() {
        let src = "Version 4\nSHEET 1 880 680\nFLAG 64 0 c\nFLAG 0 48 b\nFLAG 64 96 0\nSYMBOL npn 0 0 R0\nSYMATTR InstName Q1\nFLAG 304 0 d\nFLAG 256 80 g\nFLAG 304 96 s\nSYMBOL nmos 256 0 R0\nSYMATTR InstName M1\n";
        let (sch, _) = parse(src);
        let (built, _) = build(&sch, &SymbolLibrary::builtin_only(), "* t");
        let text = crate::netlist::write(&built.netlist);
        assert!(text.contains("Q1 c b 0 0 NPN\n"), "{text}");
        assert!(text.contains("M1 d g s s NMOS\n"), "{text}");
        assert!(text.contains(".model NPN NPN\n.model PNP PNP\n"), "{text}");
        assert!(
            text.contains(".model NMOS NMOS\n.model PMOS PMOS\n"),
            "{text}"
        );
        assert_eq!(built.standard_libs, vec!["standard.bjt", "standard.mos"]);
    }

    #[test]
    fn opamp_uses_symbol_default_spicelines() {
        let src = "Version 4\nSHEET 1 880 680\nFLAG -32 48 inn\nFLAG -32 80 inp\nFLAG 32 64 out\nSYMBOL OpAmps\\opamp 0 0 R0\nSYMATTR InstName U1\n";
        let (sch, _) = parse(src);
        let (built, _) = build(&sch, &SymbolLibrary::builtin_only(), "* op");
        let text = crate::netlist::write(&built.netlist);
        assert!(
            text.contains("XU1 inn inp out opamp Aol=100K GBW=10Meg\n"),
            "{text}"
        );
    }
}
