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
    /// The net on each pin of every new part, as `PIN:net`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub added_pins: BTreeMap<String, Vec<String>>,
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
            match self.added_pins.get(n) {
                Some(pins) => {
                    let _ = writeln!(out, "+ {n} ({})", pins.join(" "));
                }
                None => {
                    let _ = writeln!(out, "+ {n}");
                }
            }
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

/// The nets of a schematic: each net's name, and the net of every pin. Pins
/// are keyed upper-cased (`R1.A`), with their spelling kept for display.
struct NetMap {
    names: Vec<String>,
    of_pin: BTreeMap<String, usize>,
    spelled: BTreeMap<String, String>,
}

fn net_map(sch: &Schematic, lib: &SymbolLibrary) -> NetMap {
    let conn = connect(sch, lib);
    let mut m = NetMap {
        names: Vec::with_capacity(conn.nets.len()),
        of_pin: BTreeMap::new(),
        spelled: BTreeMap::new(),
    };
    for (i, net) in conn.nets.iter().enumerate() {
        m.names.push(net.name.clone());
        for p in &net.pins {
            let shown = format!("{}.{}", p.inst, p.pin);
            let key = shown.to_ascii_uppercase();
            m.of_pin.insert(key.clone(), i);
            m.spelled.insert(key, shown);
        }
    }
    m
}

/// Which net before the edit each net after it continues, judged by the pins
/// both schematics have. A net keeps its identity when it keeps its label
/// or, for numbered nets, most of its pins. Each net before is continued by
/// at most one net after, so a split shows as the pins that left rather than
/// as two copies of the old net.
fn continued_from(a: &NetMap, b: &NetMap) -> BTreeMap<usize, usize> {
    let mut overlap: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (pin, &nb) in &b.of_pin {
        if let Some(&na) = a.of_pin.get(pin) {
            *overlap.entry((nb, na)).or_default() += 1;
        }
    }
    let mut candidates: Vec<(bool, usize, usize, usize)> = overlap
        .into_iter()
        .map(|((nb, na), count)| {
            let same_name =
                is_named(&b.names[nb]) && b.names[nb].eq_ignore_ascii_case(&a.names[na]);
            (same_name, count, nb, na)
        })
        .collect();
    // Name matches first, then the largest overlap; ties by net order, so the
    // result does not depend on how the maps iterate.
    candidates.sort_by(|x, y| {
        y.0.cmp(&x.0)
            .then(y.1.cmp(&x.1))
            .then(x.2.cmp(&y.2))
            .then(x.3.cmp(&y.3))
    });
    let mut out = BTreeMap::new();
    let mut taken = BTreeSet::new();
    for (_, _, nb, na) in candidates {
        if out.contains_key(&nb) || taken.contains(&na) {
            continue;
        }
        out.insert(nb, na);
        taken.insert(na);
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
    // A pin is reported only when its own net changed: it left the net it
    // was on, or that net was renamed. A pin whose net merely gained or lost
    // other pins is unchanged; the pins that moved are reported instead.
    let (ma, mb) = (net_map(before, lib), net_map(after, lib));
    let origin = continued_from(&ma, &mb);
    for (key, &nb) in &mb.of_pin {
        let Some(&na) = ma.of_pin.get(key) else {
            continue;
        };
        let (from, to) = (&ma.names[na], &mb.names[nb]);
        let renamed = !from.eq_ignore_ascii_case(to) && (is_named(from) || is_named(to));
        if origin.get(&nb) != Some(&na) || renamed {
            d.rewired.push(Rewire {
                pin: mb.spelled[key].clone(),
                from_net: from.clone(),
                to_net: to.clone(),
            });
        }
    }
    for name in &d.added {
        let prefix = format!("{}.", name.to_ascii_uppercase());
        let pins: Vec<String> = mb
            .of_pin
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(k, &n)| format!("{}:{}", &mb.spelled[k][prefix.len()..], mb.names[n]))
            .collect();
        if !pins.is_empty() {
            d.added_pins.insert(name.clone(), pins);
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
        // C1.A stays on out; R2 joining out is reported once, with R2.
        assert!(d.rewired.is_empty(), "{:?}", d.rewired);
        assert_eq!(d.added_pins["R2"], vec!["A:out", "B:0"]);
        assert!(d.to_text().contains("+ R2 (A:out B:0)"), "{}", d.to_text());
        assert_eq!(d.directives_added, vec![".tran 1m"]);
        assert_eq!(d.directives_removed, vec![".op"]);
        assert!(d.text.contains("+SYMBOL res"));
        let h = d.highlights();
        assert!(h.contains(&("R2".to_string(), HighlightKind::Added)));
        assert!(h.contains(&("R1".to_string(), HighlightKind::Changed)));
    }

    /// A new part on an existing net must not make every pin already on that
    /// net show up as "now on in (was in)".
    #[test]
    fn unchanged_connections_are_not_reported() {
        let lib = SymbolLibrary::builtin_only();
        let src = RC.replace("FLAG 32 176 0\n", "FLAG 32 176 0\nFLAG 96 96 in\n");
        let (before, _) = parse(&src);
        let mut after = before.clone();
        let ops: Vec<EditOp> = serde_json::from_str(r#"[{"op":"add_component","symbol":"res","name":"R2","value":"10k","near":"V1"},{"op":"connect_to_net","pin":"R2.A","net":"in"},{"op":"connect_to_net","pin":"R2.B","net":"0"}]"#).unwrap();
        apply(&mut after, &lib, &ops).unwrap();
        let d = diff(&before, &after, &lib);
        assert!(d.rewired.is_empty(), "{:?}", d.rewired);
        let text = d.to_text();
        assert!(!text.contains("now on"), "{text}");
        assert!(text.contains("+ R2 (A:in B:0)"), "{text}");
    }

    /// Joining two nets reports the pins that moved, not the ones that kept
    /// their net; splitting one reports the pins that left.
    #[test]
    fn merges_and_splits_report_the_pins_that_moved() {
        let lib = SymbolLibrary::builtin_only();
        let src = RC.replace("FLAG 32 176 0\n", "FLAG 32 176 0\nFLAG 96 96 in\n");
        let (before, _) = parse(&src);
        // Merge: a label `in` on out's wire joins out into in.
        let mut merged = before.clone();
        apply(
            &mut merged,
            &lib,
            &serde_json::from_str::<Vec<EditOp>>(
                r#"[{"op":"remove_label","label":"out"},{"op":"add_label","at":[240,96],"label":"in"}]"#,
            )
            .unwrap(),
        )
        .unwrap();
        let d = diff(&before, &merged, &lib);
        let mut moved: Vec<&str> = d.rewired.iter().map(|r| r.pin.as_str()).collect();
        moved.sort_unstable();
        assert_eq!(moved, vec!["C1.A", "R1.A"], "{:?}", d.rewired);
        assert!(
            d.rewired
                .iter()
                .all(|r| r.from_net == "out" && r.to_net == "in")
        );
        // Split: C1.A leaves out; R1.A stays.
        let mut split = before.clone();
        apply(
            &mut split,
            &lib,
            &serde_json::from_str::<Vec<EditOp>>(
                r#"[{"op":"remove_wire","from":[240,128],"to":[240,96]}]"#,
            )
            .unwrap(),
        )
        .unwrap();
        let d = diff(&before, &split, &lib);
        let moved: Vec<&str> = d.rewired.iter().map(|r| r.pin.as_str()).collect();
        assert_eq!(moved, vec!["C1.A"], "{:?}", d.rewired);
    }

    #[test]
    fn no_change_is_empty() {
        let lib = SymbolLibrary::builtin_only();
        let (a, _) = parse(RC);
        assert!(diff(&a, &a.clone(), &lib).is_empty());
    }
}
