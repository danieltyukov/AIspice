//! Which points of a schematic are electrically the same node.
//!
//! The rules follow LTspice: a wire joins its two ends; a wire end, pin or
//! flag that lands anywhere on another wire joins that wire (so a T-junction
//! needs no extra segment); pins and flags on the same point join; two wires
//! that merely cross do not. Flags with the same label join wherever they are.
//! The labels `0` and `GND` are ground. Every rule here was checked against
//! LTspice's own netlister.

use crate::geometry::{Point, SegmentIndex};
use crate::schematic::{Schematic, Wire};
use crate::symbol::{SymbolDef, SymbolLibrary};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Labels LTspice netlists as ground.
pub fn is_ground_label(label: &str) -> bool {
    label == "0" || label.eq_ignore_ascii_case("gnd")
}
use std::sync::Arc;

/// One pin of one placed component.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinRef {
    /// Instance name as written in the schematic.
    pub inst: String,
    pub pin: String,
    pub spice_order: u32,
    pub at: Point,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Net {
    pub name: String,
    /// True when the name comes from a flag rather than numbering.
    pub labelled: bool,
    pub pins: Vec<PinRef>,
    /// Every flag label on the net, as written.
    pub labels: Vec<String>,
    pub wire_count: usize,
}

impl Net {
    pub fn is_ground(&self) -> bool {
        self.name == "0"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnWarning {
    pub message: String,
    pub at: Option<Point>,
}

/// A placed component with its resolved symbol.
#[derive(Debug, Clone)]
pub struct Placed {
    /// Index into `Schematic::items`.
    pub item: usize,
    pub inst: String,
    pub def: Option<Arc<SymbolDef>>,
}

#[derive(Debug, Clone, Default)]
pub struct Connectivity {
    pub nets: Vec<Net>,
    /// Upper-cased instance name to its pins' net indices, in SPICE order.
    pub pin_nets: HashMap<String, Vec<(String, usize)>>,
    pub warnings: Vec<ConnWarning>,
    pub placed: Vec<Placed>,
}

impl Connectivity {
    pub fn net(&self, name: &str) -> Option<&Net> {
        self.nets.iter().find(|n| n.name.eq_ignore_ascii_case(name))
    }

    /// The net a pin is on, by instance and pin name (or 1-based SPICE order
    /// given as a number).
    pub fn net_of(&self, inst: &str, pin: &str) -> Option<&Net> {
        let pins = self.pin_nets.get(&inst.to_ascii_uppercase())?;
        let idx = pins
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(pin))
            .or_else(|| {
                pin.parse::<usize>()
                    .ok()
                    .and_then(|n| pins.get(n.checked_sub(1)?))
            })
            .map(|(_, i)| *i)?;
        self.nets.get(idx)
    }
}

struct Dsu {
    parent: Vec<usize>,
}

impl Dsu {
    fn new() -> Self {
        Self { parent: Vec::new() }
    }
    fn add(&mut self) -> usize {
        self.parent.push(self.parent.len());
        self.parent.len() - 1
    }
    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra.max(rb)] = ra.min(rb);
        }
    }
}

/// Work out the nets of a schematic.
pub fn connect(sch: &Schematic, lib: &SymbolLibrary) -> Connectivity {
    let mut dsu = Dsu::new();
    let mut ids: HashMap<Point, usize> = HashMap::new();
    let mut id_of = |p: Point, dsu: &mut Dsu| *ids.entry(p).or_insert_with(|| dsu.add());
    let mut warnings = Vec::new();

    let wires: Vec<Wire> = sch.wires().copied().collect();
    let mut wire_ids = Vec::with_capacity(wires.len());
    for w in &wires {
        let a = id_of(w.a, &mut dsu);
        let b = id_of(w.b, &mut dsu);
        dsu.union(a, b);
        wire_ids.push(a);
    }

    // Placed pins.
    let mut placed = Vec::new();
    let mut pins: Vec<(PinRef, usize)> = Vec::new();
    for (item, it) in sch.items.iter().enumerate() {
        let crate::schematic::Item::Symbol(sym) = it else {
            continue;
        };
        let inst = sym.inst_name().unwrap_or("").to_string();
        if inst.is_empty() {
            warnings.push(ConnWarning {
                message: format!("a `{}` symbol has no InstName", sym.name),
                at: Some(sym.at),
            });
        }
        let def = match lib.resolve(&sym.name) {
            Ok((def, _)) => Some(def),
            Err(e) => {
                warnings.push(ConnWarning {
                    message: format!("{inst}: {e}"),
                    at: Some(sym.at),
                });
                None
            }
        };
        if let Some(def) = &def {
            for pin in def.pins_in_spice_order() {
                let at = def.pin_position(pin, sym.at, sym.orient);
                let id = id_of(at, &mut dsu);
                pins.push((
                    PinRef {
                        inst: inst.clone(),
                        pin: pin.name.clone(),
                        spice_order: pin.spice_order,
                        at,
                    },
                    id,
                ));
            }
        }
        placed.push(Placed { item, inst, def });
    }

    let flags: Vec<(Point, String)> = sch.flags().map(|f| (f.at, f.label.clone())).collect();
    let mut flag_ids = Vec::new();
    for (at, _) in &flags {
        flag_ids.push(id_of(*at, &mut dsu));
    }

    // Anything that lands on a wire joins it: T-junctions, pins on wires,
    // flags on wires. Overlapping collinear wires are one conductor.
    let segs: Vec<(Point, Point)> = wires.iter().map(|w| (w.a, w.b)).collect();
    let (index, merged) = SegmentIndex::new(&segs);
    for (a, b) in merged {
        dsu.union(wire_ids[a], wire_ids[b]);
    }
    let points: Vec<(Point, usize)> = ids.iter().map(|(p, id)| (*p, *id)).collect();
    for (p, id) in points {
        let cover = index.cover(p);
        for wi in cover
            .horizontal
            .map(|s| s.wire)
            .into_iter()
            .chain(cover.vertical.map(|s| s.wire))
            .chain(cover.diagonal)
        {
            dsu.union(id, wire_ids[wi]);
        }
    }

    // Same label, same net, wherever the flags are. SPICE node names are
    // case-insensitive.
    let mut by_label: HashMap<String, usize> = HashMap::new();
    for ((_, label), id) in flags.iter().zip(&flag_ids) {
        let key = label.to_ascii_uppercase();
        match by_label.get(&key) {
            Some(&other) => dsu.union(*id, other),
            None => {
                by_label.insert(key, *id);
            }
        }
    }

    // Group into nets.
    let mut root_to_net: BTreeMap<usize, usize> = BTreeMap::new();
    let mut nets: Vec<Net> = Vec::new();
    let mut net_for = |root: usize, nets: &mut Vec<Net>| {
        *root_to_net.entry(root).or_insert_with(|| {
            nets.push(Net {
                name: String::new(),
                labelled: false,
                pins: Vec::new(),
                labels: Vec::new(),
                wire_count: 0,
            });
            nets.len() - 1
        })
    };
    // Visit in schematic order so unnamed nets are numbered predictably.
    for (pin, id) in &pins {
        let r = dsu.find(*id);
        let n = net_for(r, &mut nets);
        nets[n].pins.push(pin.clone());
    }
    for ((_, label), id) in flags.iter().zip(&flag_ids) {
        let r = dsu.find(*id);
        let n = net_for(r, &mut nets);
        if !nets[n].labels.iter().any(|l| l.eq_ignore_ascii_case(label)) {
            nets[n].labels.push(label.clone());
        }
    }
    for id in &wire_ids {
        let r = dsu.find(*id);
        let n = net_for(r, &mut nets);
        nets[n].wire_count += 1;
    }

    // Names. Ground first, then flag labels; unlabelled nets are numbered the
    // way LTspice does it: N001.. for nets with a wire, ordered top to bottom,
    // P001.. for pins touching pins directly, NC_01.. for lone pins.
    let mut top_left: HashMap<usize, Point> = HashMap::new();
    for (i, net) in nets.iter().enumerate() {
        if let Some(p) = net.pins.iter().map(|p| p.at).min_by_key(|p| (p.y, p.x)) {
            top_left.insert(i, p);
        }
    }
    for (w, id) in wires.iter().zip(&wire_ids) {
        let n = root_to_net[&dsu.find(*id)];
        let best = [w.a, w.b]
            .into_iter()
            .min_by_key(|p| (p.y, p.x))
            .expect("two points");
        top_left
            .entry(n)
            .and_modify(|p| {
                if (best.y, best.x) < (p.y, p.x) {
                    *p = best;
                }
            })
            .or_insert(best);
    }
    for net in nets.iter_mut() {
        let ground: Vec<&String> = net.labels.iter().filter(|l| is_ground_label(l)).collect();
        if !ground.is_empty() {
            net.name = "0".into();
            net.labelled = true;
            let others: Vec<String> = net
                .labels
                .iter()
                .filter(|l| !is_ground_label(l))
                .cloned()
                .collect();
            if !others.is_empty() {
                warnings.push(ConnWarning {
                    message: format!(
                        "ground is also labelled {}; those labels are shorted to ground",
                        others.join(", ")
                    ),
                    at: net.pins.first().map(|p| p.at),
                });
            }
        } else if let Some(first) = net.labels.first() {
            net.name = first.clone();
            net.labelled = true;
            if net.labels.len() > 1 {
                warnings.push(ConnWarning {
                    message: format!(
                        "one net carries several labels ({}); they are shorted together",
                        net.labels.join(", ")
                    ),
                    at: net.pins.first().map(|p| p.at),
                });
            }
        }
    }
    let mut order: Vec<usize> = (0..nets.len())
        .filter(|&i| nets[i].name.is_empty())
        .collect();
    order.sort_by_key(|i| {
        top_left
            .get(i)
            .map(|p| (p.y, p.x))
            .unwrap_or((i32::MAX, i32::MAX))
    });
    let (mut n_count, mut p_count, mut nc_count) = (0, 0, 0);
    for i in order {
        let net = &mut nets[i];
        net.name = if net.wire_count > 0 {
            n_count += 1;
            format!("N{n_count:03}")
        } else if net.pins.len() > 1 {
            p_count += 1;
            format!("P{p_count:03}")
        } else {
            nc_count += 1;
            format!("NC_{nc_count:02}")
        };
    }

    let mut pin_nets: HashMap<String, Vec<(String, usize)>> = HashMap::new();
    for (pin, id) in &pins {
        let r = dsu.find(*id);
        let n = root_to_net[&r];
        pin_nets
            .entry(pin.inst.to_ascii_uppercase())
            .or_default()
            .push((pin.pin.clone(), n));
    }

    Connectivity {
        nets,
        pin_nets,
        warnings,
        placed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schematic::parse;

    fn conn(src: &str) -> Connectivity {
        let (sch, _) = parse(src);
        connect(&sch, &SymbolLibrary::builtin_only())
    }

    const RC: &str = "Version 4\nSHEET 1 880 680\nWIRE 96 96 32 96\nWIRE 240 96 176 96\nWIRE 240 128 240 96\nFLAG 32 176 0\nFLAG 240 192 0\nFLAG 240 96 out\nSYMBOL voltage 32 80 R0\nSYMATTR InstName V1\nSYMATTR Value SINE(0 1 1k)\nSYMBOL res 192 80 R90\nSYMATTR InstName R1\nSYMATTR Value 1k\nSYMBOL cap 224 128 R0\nSYMATTR InstName C1\nSYMATTR Value 100n\n";

    #[test]
    fn rc_low_pass_nets() {
        let c = conn(RC);
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
        assert_eq!(c.net_of("V1", "+").unwrap().name, "N001");
        assert_eq!(c.net_of("R1", "B").unwrap().name, "N001");
        assert_eq!(c.net_of("R1", "A").unwrap().name, "out");
        assert_eq!(c.net_of("C1", "A").unwrap().name, "out");
        assert_eq!(c.net_of("C1", "B").unwrap().name, "0");
        assert_eq!(c.net_of("V1", "2").unwrap().name, "0");
    }

    #[test]
    fn crossing_wires_do_not_join_but_t_junctions_do() {
        let src = "Version 4\nSHEET 1 880 680\nWIRE 0 64 128 64\nWIRE 64 0 64 128\nWIRE 128 64 128 0\nWIRE 0 192 128 192\nWIRE 64 192 64 256\nFLAG 0 64 a\nFLAG 64 0 b\nFLAG 128 0 c\nFLAG 0 192 d\nFLAG 64 256 e\n";
        let c = conn(src);
        // a and c share a wire chain; b crosses without a junction.
        let a = c.net("a").unwrap();
        assert!(a.labels.iter().any(|l| l == "c"));
        assert!(!a.labels.iter().any(|l| l == "b"));
        // e's wire ends on the middle of d's wire: a T-junction.
        assert!(c.net("d").unwrap().labels.iter().any(|l| l == "e"));
    }

    #[test]
    fn flags_with_the_same_label_join_anywhere() {
        let src = "Version 4\nSHEET 1 880 680\nFLAG 16 16 vcc\nFLAG 400 400 VCC\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMBOL res 384 384 R0\nSYMATTR InstName R2\n";
        let c = conn(src);
        assert_eq!(c.net_of("R1", "A").unwrap().name, "vcc");
        assert_eq!(c.net_of("R2", "A").unwrap().name, "vcc");
        assert_eq!(c.net_of("R1", "B").unwrap().name, "NC_01");
    }

    #[test]
    fn pin_on_wire_interior_connects() {
        // R1's pin A at (16,16) sits on the middle of a horizontal wire.
        let src = "Version 4\nSHEET 1 880 680\nWIRE -64 16 96 16\nFLAG -64 16 x\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\n";
        let c = conn(src);
        assert_eq!(c.net_of("R1", "A").unwrap().name, "x");
    }

    #[test]
    fn many_collinear_wires_stay_fast() {
        // 40k overlapping wires on one row plus 40k crossing stubs: quadratic
        // matching would take minutes; the span index takes milliseconds.
        let mut src = String::from("Version 4\nSHEET 1 880 680\n");
        for i in 0..40_000 {
            let x = i * 16;
            src.push_str(&format!("WIRE {x} 0 {} 0\nWIRE {x} -16 {x} 16\n", x + 48));
        }
        src.push_str("FLAG 0 0 rail\n");
        let started = std::time::Instant::now();
        let c = conn(&src);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
        assert_eq!(c.net("rail").unwrap().wire_count, 80_000);
    }

    #[test]
    fn gnd_label_is_ground() {
        let src =
            "Version 4\nSHEET 1 880 680\nFLAG 16 16 GND\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\n";
        assert_eq!(conn(src).net_of("R1", "A").unwrap().name, "0");
    }

    #[test]
    fn pin_to_pin_nets_are_p_numbered() {
        // R1's pin B (16,96) touches R2's pin A (16,96) directly, no wire.
        let src = "Version 4\nSHEET 1 880 680\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMBOL res 0 80 R0\nSYMATTR InstName R2\n";
        assert_eq!(conn(src).net_of("R1", "B").unwrap().name, "P001");
    }

    #[test]
    fn missing_symbol_and_inst_name_warn() {
        let src = "Version 4\nSHEET 1 880 680\nSYMBOL mystery 0 0 R0\nSYMATTR InstName U1\nSYMBOL res 64 0 R0\n";
        let c = conn(src);
        assert_eq!(c.warnings.len(), 2);
    }
}
