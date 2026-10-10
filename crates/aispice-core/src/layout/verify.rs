//! The round trip that makes a generated schematic trustworthy: netlist the
//! drawing the way LTspice would and compare it with the netlist it came
//! from. Layout returns an error rather than a schematic that fails this.

use super::circuit::{norm_value, subckt_text};
use crate::netlist::{Line, Netlist, compare};
use crate::schematic::Schematic;
use crate::symbol::SymbolLibrary;
use std::collections::BTreeMap;

/// Generic model cards the netlister adds for parts left at a default model.
const DEFAULT_CARDS: &[&str] = &[
    ".model d d",
    ".model npn npn",
    ".model pnp pnp",
    ".model nmos nmos",
    ".model pmos pmos",
    ".model njf njf",
    ".model pjf pjf",
];

/// Equivalent spellings brought to one form: `gnd` is ground, and a
/// three-node BJT has its substrate on ground, which is what LTspice writes.
fn normalise(n: &Netlist) -> Netlist {
    let mut out = n.clone();
    for item in out.items.iter_mut() {
        if let Line::Element(e) = item {
            for node in e.nodes.iter_mut() {
                if node.eq_ignore_ascii_case("gnd") {
                    *node = "0".into();
                }
            }
            if e.letter() == 'Q' && e.nodes.len() == 3 {
                e.nodes.push("0".into());
            }
        }
    }
    out
}

fn subckts(n: &Netlist) -> BTreeMap<String, String> {
    n.subckts()
        .map(|s| (s.name.to_ascii_uppercase(), norm_value(&subckt_text(s))))
        .collect()
}

/// Whether `other` describes the same circuit as `input`: the same elements
/// with the same values and connections, the same directives and the same
/// subcircuit definitions. Net names given in `input` must match; nets left
/// to automatic numbering may be numbered differently. Generic model cards
/// that a netlister adds for parts left at a default model (`.model NPN NPN`)
/// are ignored when `input` does not have them.
///
/// `other` is typically a netlist written by LTspice or by
/// [`crate::netlist::build`] for a schematic made from `input`.
pub fn compare_netlists(input: &Netlist, other: &Netlist) -> Result<(), Vec<String>> {
    // Writing and reading back turns subcircuit text and SPICE lines kept in
    // directives into the same structures the input has.
    let reparsed = crate::netlist::parse(&crate::netlist::write(other));
    let a = normalise(input);
    let mut b = normalise(&reparsed);
    let input_cards: Vec<String> = a.directives().map(norm_value).collect();
    b.items.retain(|item| match item {
        Line::Directive { text } => {
            let t = norm_value(text);
            !DEFAULT_CARDS.contains(&t.as_str()) || input_cards.contains(&t)
        }
        _ => true,
    });
    let mut problems = Vec::new();
    if let Err(diffs) = compare(&a, &b) {
        problems.extend(diffs.into_iter().map(|m| m.0));
    }
    let (sa, sb) = (subckts(&a), subckts(&b));
    if sa != sb {
        problems.push(format!(
            "subcircuit definitions differ: {:?} vs {:?}",
            sa.keys().collect::<Vec<_>>(),
            sb.keys().collect::<Vec<_>>()
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

/// Ok when the schematic netlists to the same circuit as `input`; otherwise
/// every difference found.
pub(crate) fn round_trip(
    input: &Netlist,
    sch: &Schematic,
    lib: &SymbolLibrary,
) -> Result<(), Vec<String>> {
    let (built, _) = crate::netlist::build(sch, lib, "* layout check");
    let mut problems: Vec<String> = built
        .warnings
        .iter()
        .map(|w| format!("netlisting the drawing: {w}"))
        .collect();
    if let Err(p) = compare_netlists(input, &built.netlist) {
        problems.extend(p);
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}
