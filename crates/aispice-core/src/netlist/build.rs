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

/// Whether a SpiceModel value names a library file rather than a model.
fn is_library_file(v: &str) -> bool {
    let ext = v
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "lib"
            | "sub"
            | "mod"
            | "cir"
            | "inc"
            | "txt"
            | "net"
            | "sp"
            | "spi"
            | "spice"
            | "ckt"
            | "bjt"
            | "dio"
            | "mos"
            | "jft"
            | "cmp"
    )
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
    build_at_depth(sch, lib, title, 0)
}

/// Hierarchical sheets nest at most this deep; LTspice designs rarely go past
/// three, and the limit stops a sheet that includes itself.
const MAX_HIERARCHY: usize = 8;

/// The schematic behind a hierarchical block: `<symbol>.asc` in the project
/// folder. A BLOCK symbol without one is just a box drawn around an ordinary
/// subcircuit and netlists like any other part.
fn hierarchical_child(lib: &SymbolLibrary, symbol_name: &str) -> Option<(String, Schematic)> {
    let base = crate::schematic::normalize_symbol_name(symbol_name);
    let base = base.rsplit('/').next().unwrap_or(&base).to_string();
    for (dir, source) in lib.dirs() {
        if source != crate::symbol::SymbolSource::Project {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_ascii_lowercase());
            let is_asc = path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("asc"));
            if is_asc && stem.as_deref() == Some(base.as_str()) {
                let bytes = std::fs::read(&path).ok()?;
                let name = path.file_stem()?.to_string_lossy().into_owned();
                return Some((name, crate::schematic::parse_bytes(&bytes).0));
            }
        }
    }
    None
}

fn build_at_depth(
    sch: &Schematic,
    lib: &SymbolLibrary,
    title: &str,
    depth: usize,
) -> (Built, Connectivity) {
    let conn = connect(sch, lib);
    let mut built = Built {
        warnings: conn.warnings.iter().map(|w| w.message.clone()).collect(),
        ..Default::default()
    };
    let mut items = Vec::new();
    let mut blocks: Vec<super::spice::Subckt> = Vec::new();
    let mut defaults_needed: Vec<&str> = Vec::new();
    let mut missing_pins = 0usize;

    for p in &conn.placed {
        let Item::Symbol(sym) = &sch.items[p.item] else {
            continue;
        };
        let Some(def) = &p.def else { continue };
        if super::connect::is_jumper(&sym.name) {
            continue;
        }
        let plain_block = def.kind == SymbolType::Block
            && sym.attr("Value").or_else(|| def.attr("Value")).is_none()
            && sym
                .attr("SpiceModel")
                .or_else(|| def.attr("SpiceModel"))
                .is_none();
        if plain_block {
            match hierarchical_child(lib, &sym.name) {
                Some((child_name, child)) if depth < MAX_HIERARCHY => {
                    let pins = conn
                        .pin_nets
                        .get(&p.inst.to_ascii_uppercase())
                        .cloned()
                        .unwrap_or_default();
                    let nodes: Vec<String> = pins
                        .iter()
                        .map(|(_, n)| conn.nets[*n].name.clone())
                        .collect();
                    let mut rest = child_name.clone();
                    if let Some(params) = sym.attr("SpiceLine").filter(|v| !v.trim().is_empty()) {
                        rest.push_str(&format!(" params: {}", params.trim()));
                    }
                    let prefix = sym.attr("Prefix").unwrap_or("X");
                    items.push(Line::Element(Element {
                        name: instance_name(prefix, &p.inst),
                        nodes,
                        rest,
                    }));
                    if !blocks
                        .iter()
                        .any(|b: &super::spice::Subckt| b.name.eq_ignore_ascii_case(&child_name))
                    {
                        let (child_built, _) = build_at_depth(&child, lib, "", depth + 1);
                        built.warnings.extend(
                            child_built
                                .warnings
                                .iter()
                                .map(|w| format!("{child_name}: {w}")),
                        );
                        let ports = def
                            .pins_in_spice_order()
                            .iter()
                            .map(|pd| pd.name.clone())
                            .collect();
                        blocks.push(super::spice::Subckt {
                            name: child_name,
                            ports,
                            params: String::new(),
                            body: child_built.netlist.items,
                        });
                    }
                }
                Some(_) => built.warnings.push(format!(
                    "{}: hierarchy deeper than {MAX_HIERARCHY} levels was not netlisted",
                    p.inst
                )),
                None => built.warnings.push(format!(
                    "{}: block symbol `{}` has no schematic `{}.asc` beside it and no model",
                    p.inst, sym.name, sym.name
                )),
            }
            continue;
        }
        let prefix = sym
            .attr("Prefix")
            .map(str::trim)
            .filter(|p| !p.is_empty() && *p != "\"\"")
            .unwrap_or_else(|| def.prefix())
            .to_string();
        let letter = prefix.chars().next().unwrap_or('X').to_ascii_uppercase();
        let name = instance_name(&prefix, &p.inst);
        let pins = conn
            .pin_nets
            .get(&p.inst.to_ascii_uppercase())
            .cloned()
            .unwrap_or_default();
        // Nodes go in SpiceOrder positions. Special-function `A` devices always
        // have eight, with unused positions tied to 0; for other parts a gap in
        // the pin numbering becomes a placeholder net MP_nn, as LTspice does.
        let orders: Vec<u32> = def
            .pins_in_spice_order()
            .iter()
            .map(|pd| pd.spice_order)
            .collect();
        let max_order = orders.iter().copied().max().unwrap_or(0) as usize;
        let width = if letter == 'A' {
            max_order.max(8)
        } else {
            max_order.max(pins.len())
        };
        let mut nodes: Vec<Option<String>> = vec![None; width];
        for ((_, net), order) in pins.iter().zip(&orders) {
            if let Some(slot) = (*order as usize)
                .checked_sub(1)
                .and_then(|i| nodes.get_mut(i))
            {
                let name = &conn.nets[*net].name;
                // An unconnected pin of a special-function device is tied to
                // ground rather than left floating.
                *slot = Some(if letter == 'A' && name.starts_with("NC_") {
                    "0".to_string()
                } else {
                    name.clone()
                });
            }
        }
        let mut nodes: Vec<String> = nodes
            .into_iter()
            .map(|n| {
                n.unwrap_or_else(|| {
                    if letter == 'A' {
                        "0".into()
                    } else {
                        missing_pins += 1;
                        format!("MP_{missing_pins:02}")
                    }
                })
            })
            .collect();
        let up = prefix.to_ascii_uppercase();
        if (up == "QN" || up == "QP") && nodes.len() == 3 {
            nodes.push("0".into());
        }
        if (up == "MN" || up == "MP") && nodes.len() == 3 {
            nodes.push(nodes[2].clone());
        }
        // `""` in a file means deliberately empty.
        // `""` in a file means deliberately empty, and `;` starts a comment.
        let field = |key: &str| {
            sym.attr(key)
                .or_else(|| def.attr(key))
                .map(|v| v.split(';').next().unwrap_or("").trim())
                .filter(|v| !v.is_empty() && *v != "\"\"")
        };
        let mut parts: Vec<&str> = Vec::new();
        // SpiceModel is either a library file (it has a library extension and
        // becomes a `.lib` line) or a model name, which LTspice writes first
        // on the instance line: `SpiceModel AC` with `Value 1` gives `AC 1`.
        let spice_model = field("SpiceModel");
        let model_is_file = spice_model.is_some_and(is_library_file);
        if let Some(m) = spice_model.filter(|_| !model_is_file) {
            parts.push(m);
        }
        let value = field("Value");
        let value2 = field("Value2");
        // Old-style library parts name a library file in SpiceModel; their
        // Value is the part number shown on the sheet and Value2 the model it
        // runs (an LT1885 simulates as LT1884), so only Value2 is written.
        // Everything else writes Value then Value2 as they are, even when the
        // two are the same (`AD4011 AD4011`).
        if model_is_file && value2.is_some() {
            parts.extend(value2);
        } else {
            parts.extend(value);
            parts.extend(value2);
        }
        parts.extend(field("SpiceLine"));
        parts.extend(field("SpiceLine2"));
        let rest = parts.join(" ");
        // LTspice adds the generic model cards whenever a part of that kind is
        // present, whatever model the part itself names.
        let category = match letter {
            'D' => Some("D"),
            'Q' => Some("NPN"),
            'J' => Some("NJF"),
            'M' => Some("NMOS"),
            _ => None,
        };
        if let Some(c) = category
            && !defaults_needed.contains(&c)
        {
            defaults_needed.push(c);
        }
        let files = [spice_model.filter(|_| model_is_file), field("ModelFile")];
        for lib_file in files.into_iter().flatten() {
            if !built
                .model_libs
                .iter()
                .any(|l| l.eq_ignore_ascii_case(lib_file))
            {
                built.model_libs.push(lib_file.to_string());
            }
        }
        items.push(Line::Element(Element { name, nodes, rest }));
    }

    // Directive text is SPICE: it can hold .subckt blocks, K couplings,
    // element lines, `+` continuations and `;` comments, so it is parsed like
    // a deck rather than copied line by line.
    let mut deck = String::from("* directives\n");
    for t in sch.texts() {
        match t.kind {
            TextKind::Directive => {
                for line in t.lines() {
                    deck.push_str(&line);
                    deck.push('\n');
                }
            }
            TextKind::Comment => {
                for line in t.content.split("\\n") {
                    if !line.trim().is_empty() {
                        deck.push_str("* ");
                        deck.push_str(line.trim());
                        deck.push('\n');
                    }
                }
            }
        }
    }
    items.extend(super::spice::parse(&deck).items);

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
    if !blocks.is_empty() {
        items.push(Line::Comment {
            text: "block symbol definitions".into(),
        });
        items.extend(blocks.into_iter().map(Line::Subckt));
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

    /// A symbol written to a temp project folder, so rules that depend on
    /// symbol attributes can be tested without LTspice's library.
    fn lib_with(symbols: &[(&str, &str)]) -> (std::path::PathBuf, SymbolLibrary) {
        let dir = std::env::temp_dir().join(format!(
            "aispice-build-{}-{}",
            std::process::id(),
            symbols.len() + symbols.iter().map(|s| s.0.len()).sum::<usize>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in symbols {
            std::fs::write(dir.join(format!("{name}.asy")), text).unwrap();
        }
        let lib =
            SymbolLibrary::builtin_only().with_dir(&dir, crate::symbol::SymbolSource::Project);
        (dir, lib)
    }

    fn netlist_of(src: &str, lib: &SymbolLibrary) -> String {
        let (sch, _) = parse(src);
        crate::netlist::write(&build(&sch, lib, "* t").0.netlist)
    }

    #[test]
    fn special_function_devices_get_eight_nodes_and_model_from_spicemodel() {
        let asy = "SymbolType CELL\nSYMATTR Prefix A\nSYMATTR SpiceModel VARISTOR\nPIN 0 0 NONE 0\nPINATTR PinName A\nPINATTR SpiceOrder 1\nPIN 0 64 NONE 0\nPINATTR PinName B\nPINATTR SpiceOrder 7\nPIN 32 0 NONE 0\nPINATTR PinName C\nPINATTR SpiceOrder 3\n";
        let (_d, lib) = lib_with(&[("vari", asy)]);
        let src = "Version 4\nSHEET 1 880 680\nFLAG 0 0 in\nFLAG 0 64 out\nSYMBOL vari 0 0 R0\nSYMATTR InstName A1\nSYMATTR SpiceLine Rclamp=1\n";
        let text = netlist_of(src, &lib);
        assert!(
            text.contains("A1 in 0 0 0 0 0 out 0 VARISTOR Rclamp=1\n"),
            "{text}"
        );
    }

    #[test]
    fn gaps_in_pin_order_become_placeholder_nets() {
        let asy = "SymbolType CELL\nSYMATTR Prefix X\nSYMATTR Value part\nPIN 0 0 NONE 0\nPINATTR PinName A\nPINATTR SpiceOrder 1\nPIN 0 64 NONE 0\nPINATTR PinName B\nPINATTR SpiceOrder 3\n";
        let (_d, lib) = lib_with(&[("gappy", asy)]);
        let src = "Version 4\nSHEET 1 880 680\nFLAG 0 0 a\nFLAG 0 64 b\nSYMBOL gappy 0 0 R0\nSYMATTR InstName U1\n";
        assert!(netlist_of(src, &lib).contains("XU1 a MP_01 b part\n"));
    }

    #[test]
    fn value2_and_spicemodel_follow_ltspice() {
        // Old style: SpiceModel is a library file, Value2 is the model run.
        let old = "SymbolType CELL\nSYMATTR Prefix X\nSYMATTR Value LT1885\nSYMATTR Value2 LT1884\nSYMATTR SpiceModel LTC.lib\nPIN 0 0 NONE 0\nPINATTR SpiceOrder 1\n";
        // New style: ModelFile is the library, Value and Value2 both written.
        let new = "SymbolType CELL\nSYMATTR Prefix X\nSYMATTR Value AD4011\nSYMATTR Value2 AD4011\nSYMATTR SpiceLine EN=1\nSYMATTR ModelFile AD4011.sub\nPIN 0 0 NONE 0\nPINATTR SpiceOrder 1\n";
        // A model-name SpiceModel goes first on the line.
        let src_model = "SymbolType CELL\nSYMATTR Prefix I\nPIN 0 0 NONE 0\nPINATTR SpiceOrder 1\nPIN 0 80 NONE 0\nPINATTR SpiceOrder 2\n";
        let (_d, lib) = lib_with(&[("oldp", old), ("newp", new), ("isrc", src_model)]);
        let src = "Version 4\nSHEET 1 880 680\nFLAG 0 0 a\nSYMBOL oldp 0 0 R0\nSYMATTR InstName U1\nSYMBOL newp 0 0 R0\nSYMATTR InstName U2\nFLAG 0 80 0\nSYMBOL isrc 0 0 R0\nSYMATTR InstName I1\nSYMATTR Value 1\nSYMATTR SpiceModel AC\n";
        let text = netlist_of(src, &lib);
        assert!(text.contains("XU1 a LT1884\n"), "{text}");
        assert!(text.contains("XU2 a AD4011 AD4011 EN=1\n"), "{text}");
        assert!(text.contains("I1 a 0 AC 1\n"), "{text}");
        assert!(
            text.contains(".lib LTC.lib\n") && text.contains(".lib AD4011.sub\n"),
            "{text}"
        );
    }

    #[test]
    fn directive_text_is_parsed_as_spice() {
        let src = "Version 4\nSHEET 1 880 680\nTEXT 0 0 Left 2 !K1 L1 L2 1\\n.subckt s a b\\nR1 a b 1\\n.ends s\\n;.ac dec 10 1 1k\\n.tran 1m ; note\n";
        let (sch, _) = parse(src);
        let (built, _) = build(&sch, &SymbolLibrary::builtin_only(), "* t");
        let n = &built.netlist;
        assert!(n.element("K1").is_some());
        assert_eq!(n.subckts().count(), 1);
        assert_eq!(n.directives().collect::<Vec<_>>(), vec![".tran 1m"]);
    }

    #[test]
    fn default_model_cards_follow_device_kind() {
        let src = "Version 4\nSHEET 1 880 680\nSYMBOL diode 0 0 R0\nSYMATTR InstName D1\nSYMATTR Value 1N4148\n";
        let (sch, _) = parse(src);
        let text =
            crate::netlist::write(&build(&sch, &SymbolLibrary::builtin_only(), "* t").0.netlist);
        assert!(text.contains(".model D D\n"), "{text}");
    }

    #[test]
    fn jumpers_join_nets_and_emit_nothing() {
        let src = "Version 4\nSHEET 1 880 680\nFLAG -32 64 left\nFLAG 32 64 right\nSYMBOL Misc\\jumper 0 0 R0\nSYMATTR InstName J1\nSYMBOL res -48 48 R0\nSYMATTR InstName R1\nSYMATTR Value 1\n";
        let asy = "SymbolType CELL\nSYMATTR Prefix J\nPIN -32 64 NONE 0\nPINATTR PinName +\nPINATTR SpiceOrder 1\nPIN 32 64 NONE 0\nPINATTR PinName -\nPINATTR SpiceOrder 2\n";
        let (_d, lib) = lib_with(&[("jumper", asy)]);
        let text = netlist_of(src, &lib);
        assert!(!text.contains("J1"), "{text}");
        assert!(
            text.contains("R1 left "),
            "first label in the file names a jumper-joined net: {text}"
        );
    }

    #[test]
    fn lowest_label_names_a_net() {
        let src = "Version 4\nSHEET 1 880 680\nWIRE 16 -64 16 16\nFLAG 16 -32 zzz\nFLAG 16 -64 aaa\nFLAG 16 96 0\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMATTR Value 1k\n";
        let text = netlist_of(src, &SymbolLibrary::builtin_only());
        assert!(text.contains("R1 zzz 0 1k"), "{text}");
    }

    #[test]
    fn hierarchical_block_becomes_inline_subckt() {
        let dir = std::env::temp_dir().join(format!("aispice-hier-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("half.asy"), "SymbolType BLOCK\nPIN -32 0 LEFT 8\nPINATTR PinName in\nPINATTR SpiceOrder 1\nPIN 32 0 RIGHT 8\nPINATTR PinName out\nPINATTR SpiceOrder 2\n").unwrap();
        std::fs::write(dir.join("half.asc"), "Version 4\nSHEET 1 880 680\nFLAG 16 16 in\nFLAG 16 96 out\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMATTR Value 1k\n").unwrap();
        let lib =
            SymbolLibrary::builtin_only().with_dir(&dir, crate::symbol::SymbolSource::Project);
        let src = "Version 4\nSHEET 1 880 680\nFLAG -32 0 a\nFLAG 32 0 b\nSYMBOL half 0 0 R0\nSYMATTR InstName X1\n";
        let text = netlist_of(src, &lib);
        assert!(text.contains("XX1 a b half\n"), "{text}");
        assert!(
            text.contains(".subckt half in out\nR1 in out 1k\n.ends half\n"),
            "{text}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
