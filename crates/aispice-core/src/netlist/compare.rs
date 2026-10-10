//! Compare two netlists for electrical equivalence.
//!
//! Used to check aispice's netlister against LTspice's: element names and
//! values must match, labelled nets must match by name, and automatically
//! numbered nets (`N001`, `P001`, `NC_01`) may differ in number as long as the
//! two netlists connect the same pins together.

use super::spice::Netlist;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch(pub String);

fn auto_named(n: &str) -> bool {
    let up = n.to_ascii_uppercase();
    (up.starts_with('N') || up.starts_with('P'))
        && up.len() == 4
        && up[1..].bytes().all(|b| b.is_ascii_digit())
        || up.starts_with("NC_")
}

fn norm_value(s: &str) -> String {
    s.replace(['\u{b5}', '\u{3bc}'], "u")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn norm_name(s: &str) -> String {
    s.replace('\u{a7}', "").to_ascii_uppercase()
}

/// Ok when `a` and `b` describe the same circuit. Directive lines are compared
/// as sets, ignoring `.lib` paths, `.backanno` and comments.
pub fn compare(a: &Netlist, b: &Netlist) -> Result<(), Vec<Mismatch>> {
    let mut problems = Vec::new();
    let eb: HashMap<String, &super::Element> =
        b.elements().map(|e| (norm_name(&e.name), e)).collect();
    let mut map_ab: HashMap<String, String> = HashMap::new();
    let mut map_ba: HashMap<String, String> = HashMap::new();
    let mut seen = 0;
    for ea in a.elements() {
        let key = norm_name(&ea.name);
        let Some(ebx) = eb.get(&key) else {
            problems.push(Mismatch(format!(
                "{} is only in the first netlist",
                ea.name
            )));
            continue;
        };
        seen += 1;
        if norm_value(&ea.rest) != norm_value(&ebx.rest) {
            problems.push(Mismatch(format!(
                "{}: `{}` vs `{}`",
                ea.name, ea.rest, ebx.rest
            )));
        }
        if ea.nodes.len() != ebx.nodes.len() {
            problems.push(Mismatch(format!(
                "{}: {} nodes vs {}",
                ea.name,
                ea.nodes.len(),
                ebx.nodes.len()
            )));
            continue;
        }
        for (na, nb) in ea.nodes.iter().zip(&ebx.nodes) {
            let (ua, ub) = (na.to_ascii_uppercase(), nb.to_ascii_uppercase());
            let named_a = !auto_named(&ua);
            let named_b = !auto_named(&ub);
            if named_a || named_b {
                if ua != ub {
                    problems.push(Mismatch(format!("{}: node {na} vs {nb}", ea.name)));
                }
                continue;
            }
            match (map_ab.get(&ua), map_ba.get(&ub)) {
                (None, None) => {
                    map_ab.insert(ua.clone(), ub.clone());
                    map_ba.insert(ub, ua);
                }
                (Some(x), Some(y)) if *x == ub && *y == ua => {}
                _ => problems.push(Mismatch(format!(
                    "{}: node {na} does not correspond to {nb} consistently",
                    ea.name
                ))),
            }
        }
    }
    if seen != eb.len() {
        for eb_el in b.elements() {
            if !a
                .elements()
                .any(|e| norm_name(&e.name) == norm_name(&eb_el.name))
            {
                problems.push(Mismatch(format!(
                    "{} is only in the second netlist",
                    eb_el.name
                )));
            }
        }
    }
    let directives = |n: &Netlist| -> Vec<String> {
        let mut v: Vec<String> = n
            .directives()
            .filter(|d| {
                let kw = d
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                !matches!(kw.as_str(), ".lib" | ".backanno" | ".inc" | ".include")
            })
            .map(norm_value)
            .collect();
        v.sort();
        v
    };
    let (da, db) = (directives(a), directives(b));
    if da != db {
        problems.push(Mismatch(format!("directives differ: {da:?} vs {db:?}")));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netlist::parse;

    #[test]
    fn renumbered_nets_are_equivalent() {
        let a = parse("t\nR1 N001 out 1k\nC1 out 0 1\u{b5}\nV1 N001 0 1\n.op\n");
        let b = parse("t\nR1 N007 out 1k\nC1 out 0 1u\nV1 N007 0 1\n.op\n.backanno\n");
        assert_eq!(compare(&a, &b), Ok(()));
    }

    #[test]
    fn different_topology_is_caught() {
        let a = parse("t\nR1 N001 N002 1k\nR2 N002 0 1k\n");
        let b = parse("t\nR1 N001 N002 1k\nR2 N001 0 1k\n");
        assert!(compare(&a, &b).is_err());
    }

    #[test]
    fn section_separator_is_ignored_in_names() {
        let a = parse("t\nR\u{a7}load a 0 1k\n");
        let b = parse("t\nRload a 0 1k\n");
        assert_eq!(compare(&a, &b), Ok(()));
    }
}
