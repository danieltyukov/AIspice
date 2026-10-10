//! What an edit changed, in circuit terms.
//!
//! A text diff of an `.asc` file says "line 14 changed"; this says "R1 went
//! from 1k to 2.2k and C1.A moved from net out to N003". Connection changes
//! are found by comparing which pins share a net, so automatic renumbering of
//! unnamed nets (N001 becoming N002) is not reported as a change.

use crate::netlist::connect;
use crate::schematic::{Schematic, TextKind, write};
use crate::symbol::SymbolLibrary;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttrChange {
    pub name: String,
    pub key: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rewire {
    /// `R1.A`
    pub pin: String,
    pub from_net: String,
    pub to_net: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HighlightKind {
    Added,
    Removed,
    Changed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchematicDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<AttrChange>,
    pub moved: Vec<String>,
    pub rewired: Vec<Rewire>,
    pub directives_added: Vec<String>,
    pub directives_removed: Vec<String>,
    pub wires_added: usize,
    pub wires_removed: usize,
    /// Unified diff of the `.asc` text.
    pub text: String,
}

impl SchematicDiff {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Parts to highlight in a drawing of the new schematic.
    pub fn highlights(&self) -> Vec<(String, HighlightKind)> {
        let mut out: Vec<(String, HighlightKind)> = self
            .added
            .iter()
            .map(|n| (n.clone(), HighlightKind::Added))
            .collect();
        let mut changed: BTreeSet<String> = self.changed.iter().map(|c| c.name.clone()).collect();
        changed.extend(self.moved.iter().cloned());
        changed.extend(
            self.rewired
                .iter()
                .filter_map(|r| r.pin.split('.').next().map(str::to_string)),
        );
        for n in changed {
            if !self.added.contains(&n) {
                out.push((n, HighlightKind::Changed));
            }
        }
        out
    }

    /// A short human summary, one change per line.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for n in &self.added {
            let _ = writeln!(out, "+ {n}");
        }
        for n in &self.removed {
            let _ = writeln!(out, "- {n}");
        }
        for c in &self.changed {
            let _ = writeln!(
                out,
                "~ {}.{}: {} -> {}",
                c.name,
                c.key,
                c.from.as_deref().unwrap_or("(unset)"),
                c.to.as_deref().unwrap_or("(unset)")
            );
        }
        for n in &self.moved {
            let _ = writeln!(out, "~ {n} moved");
        }
        for r in &self.rewired {
            let _ = writeln!(out, "~ {} now on {} (was {})", r.pin, r.to_net, r.from_net);
        }
        for d in &self.directives_added {
            let _ = writeln!(out, "+ {d}");
        }
        for d in &self.directives_removed {
            let _ = writeln!(out, "- {d}");
        }
        if self.wires_added + self.wires_removed > 0 {
            let _ = writeln!(
                out,
                "  wires: +{} -{}",
                self.wires_added, self.wires_removed
            );
        }
        out
    }
}

type Parts = BTreeMap<
    String,
    (
        String,
        crate::geometry::Point,
        crate::geometry::Orient,
        BTreeMap<String, String>,
    ),
>;

fn parts(sch: &Schematic) -> Parts {
    sch.symbols()
        .filter_map(|s| {
            let name = s.inst_name()?.to_string();
            let attrs = s
                .attrs
                .iter()
                .filter(|a| !a.key.eq_ignore_ascii_case("InstName"))
                .map(|a| (a.key.clone(), a.value.clone()))
                .collect();
            Some((name.to_ascii_uppercase(), (name, s.at, s.orient, attrs)))
        })
        .collect()
}

/// Pin to (net name, set of other pins on that net).
fn membership(
    sch: &Schematic,
    lib: &SymbolLibrary,
) -> BTreeMap<String, (String, BTreeSet<String>)> {
    let conn = connect(sch, lib);
    let mut out = BTreeMap::new();
    for net in &conn.nets {
        let members: BTreeSet<String> = net
            .pins
            .iter()
            .map(|p| format!("{}.{}", p.inst, p.pin).to_ascii_uppercase())
            .collect();
        for p in &net.pins {
            let key = format!("{}.{}", p.inst, p.pin);
            let mut others = members.clone();
            others.remove(&key.to_ascii_uppercase());
            out.insert(key, (net.name.clone(), others));
        }
    }
    out
}

pub fn diff(before: &Schematic, after: &Schematic, lib: &SymbolLibrary) -> SchematicDiff {
    let mut d = SchematicDiff::default();
    let (pa, pb) = (parts(before), parts(after));
    for (k, (name, ..)) in &pb {
        if !pa.contains_key(k) {
            d.added.push(name.clone());
        }
    }
    for (k, (name, ..)) in &pa {
        if !pb.contains_key(k) {
            d.removed.push(name.clone());
        }
    }
    for (k, (name, at_a, or_a, attrs_a)) in &pa {
        let Some((_, at_b, or_b, attrs_b)) = pb.get(k) else {
            continue;
        };
        if at_a != at_b || or_a != or_b {
            d.moved.push(name.clone());
        }
        let keys: BTreeSet<&String> = attrs_a.keys().chain(attrs_b.keys()).collect();
        for key in keys {
            let (x, y) = (attrs_a.get(key), attrs_b.get(key));
            if x != y {
                d.changed.push(AttrChange {
                    name: name.clone(),
                    key: key.clone(),
                    from: x.cloned(),
                    to: y.cloned(),
                });
            }
        }
    }
    let (ma, mb) = (membership(before, lib), membership(after, lib));
    for (pin, (net_b, others_b)) in &mb {
        if let Some((net_a, others_a)) = ma.get(pin)
            && (others_a != others_b || (net_a != net_b && (is_named(net_a) || is_named(net_b))))
        {
            d.rewired.push(Rewire {
                pin: pin.clone(),
                from_net: net_a.clone(),
                to_net: net_b.clone(),
            });
        }
    }
    let dirs = |s: &Schematic| -> Vec<String> {
        s.texts()
            .filter(|t| t.kind == TextKind::Directive)
            .flat_map(|t| t.lines())
            .collect()
    };
    let (da, db) = (dirs(before), dirs(after));
    d.directives_added = db.iter().filter(|x| !da.contains(x)).cloned().collect();
    d.directives_removed = da.iter().filter(|x| !db.contains(x)).cloned().collect();
    let wa: Vec<_> = before.wires().collect();
    let wb: Vec<_> = after.wires().collect();
    d.wires_added = wb
        .iter()
        .filter(|w| !wa.iter().any(|x| x.same_as(w)))
        .count();
    d.wires_removed = wa
        .iter()
        .filter(|w| !wb.iter().any(|x| x.same_as(w)))
        .count();
    let (ta, tb) = (write(before), write(after));
    if ta != tb {
        d.text = similar::TextDiff::from_lines(&ta, &tb)
            .unified_diff()
            .context_radius(2)
            .header("before", "after")
            .to_string();
    }
    d
}

fn is_named(net: &str) -> bool {
    let up = net.to_ascii_uppercase();
    !(up.starts_with("NC_")
        || ((up.starts_with('N') || up.starts_with('P'))
            && up.len() == 4
            && up[1..].bytes().all(|b| b.is_ascii_digit())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{EditOp, apply};
    use crate::schematic::parse;

    const RC: &str = "Version 4\nSHEET 1 880 680\nWIRE 96 96 32 96\nWIRE 240 96 176 96\nWIRE 240 128 240 96\nFLAG 32 176 0\nFLAG 240 192 0\nFLAG 240 96 out\nSYMBOL voltage 32 80 R0\nSYMATTR InstName V1\nSYMATTR Value 1\nSYMBOL res 192 80 R90\nSYMATTR InstName R1\nSYMATTR Value 1k\nSYMBOL cap 224 128 R0\nSYMATTR InstName C1\nSYMATTR Value 100n\nTEXT 0 232 Left 2 !.op\n";

    #[test]
    fn value_change_and_new_part() {
        let lib = SymbolLibrary::builtin_only();
        let (before, _) = parse(RC);
        let mut after = before.clone();
        let ops: Vec<EditOp> = serde_json::from_str(r#"[{"op":"set_value","name":"R1","value":"2.2k"},{"op":"add_component","symbol":"res","name":"R2","value":"10k","near":"C1"},{"op":"connect","from":"R2.A","to":"C1.A"},{"op":"connect_to_net","pin":"R2.B","net":"0"},{"op":"replace_directive","matching":".op","text":".tran 1m"}]"#).unwrap();
        apply(&mut after, &lib, &ops).unwrap();
        let d = diff(&before, &after, &lib);
        assert_eq!(d.added, vec!["R2"]);
        assert!(
            d.changed
                .iter()
                .any(|c| c.name == "R1" && c.to.as_deref() == Some("2.2k"))
        );
        assert!(d.rewired.iter().any(|r| r.pin == "C1.A"), "{:?}", d.rewired);
        assert_eq!(d.directives_added, vec![".tran 1m"]);
        assert_eq!(d.directives_removed, vec![".op"]);
        assert!(d.text.contains("+SYMBOL res"));
        let h = d.highlights();
        assert!(h.contains(&("R2".to_string(), HighlightKind::Added)));
        assert!(h.contains(&("R1".to_string(), HighlightKind::Changed)));
    }

    #[test]
    fn no_change_is_empty() {
        let lib = SymbolLibrary::builtin_only();
        let (a, _) = parse(RC);
        assert!(diff(&a, &a.clone(), &lib).is_empty());
    }
}
