//! Finding pins by name and working out which way they face.

use crate::geometry::{Point, Rect};
use crate::schematic::{Item, Schematic};
use crate::symbol::{SymbolDef, SymbolLibrary};
use std::sync::Arc;

use super::EditError;

/// A pin on a placed component, resolved to a position and an outward
/// direction (the side of the part the pin sticks out of).
#[derive(Debug, Clone)]
pub struct PinLoc {
    pub inst: String,
    pub pin: String,
    pub at: Point,
    /// Unit step away from the part body: one of (±1, 0) or (0, ±1).
    pub out: (i32, i32),
}

/// Split `R1.A`, `R1.2`, `V1.+` or `U1.In-` into instance and pin.
pub fn split_pin_spec(spec: &str) -> Option<(&str, &str)> {
    let spec = spec.trim();
    let dot = spec.find('.')?;
    let (inst, pin) = (&spec[..dot], &spec[dot + 1..]);
    (!inst.is_empty() && !pin.is_empty()).then_some((inst, pin))
}

pub(crate) fn resolve_symbol(
    sch: &Schematic,
    lib: &SymbolLibrary,
    inst: &str,
) -> Result<(usize, Arc<SymbolDef>), EditError> {
    let idx = sch
        .symbol_index(inst)
        .ok_or_else(|| EditError::NoSuchComponent(inst.to_string(), known_names(sch)))?;
    let Item::Symbol(sym) = &sch.items[idx] else {
        unreachable!("symbol_index returns symbols")
    };
    let (def, _) = lib
        .resolve(&sym.name)
        .map_err(|e| EditError::Library(e.to_string()))?;
    Ok((idx, def))
}

pub(crate) fn known_names(sch: &Schematic) -> String {
    let mut names: Vec<&str> = sch.symbols().filter_map(|s| s.inst_name()).collect();
    names.sort_unstable();
    if names.is_empty() {
        "none".into()
    } else {
        names.join(", ")
    }
}

/// Resolve a pin spec to its location.
pub fn locate(sch: &Schematic, lib: &SymbolLibrary, spec: &str) -> Result<PinLoc, EditError> {
    let (inst, pin) =
        split_pin_spec(spec).ok_or_else(|| EditError::BadPinSpec(spec.to_string()))?;
    let (idx, def) = resolve_symbol(sch, lib, inst)?;
    let Item::Symbol(sym) = &sch.items[idx] else {
        unreachable!()
    };
    let ordered = def.pins_in_spice_order();
    let pdef = def
        .pin(pin)
        .or_else(|| {
            pin.parse::<usize>()
                .ok()
                .and_then(|n| ordered.get(n.checked_sub(1)?).copied())
        })
        .ok_or_else(|| {
            let names: Vec<String> = ordered
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{} ({})", p.name, i + 1))
                .collect();
            let real: Vec<&str> = ordered.iter().map(|p| p.name.as_str()).collect();
            let mut list = names.join(", ");
            if let Some(g) = guess_pin(pin, &real) {
                list.push_str(&format!(
                    "; did you mean {}.{g}?",
                    sym.inst_name().unwrap_or(inst)
                ));
            }
            EditError::NoSuchPin(spec.to_string(), list)
        })?;
    let at = def.pin_position(pdef, sym.at, sym.orient);
    let center = body_center(&def, sym.at, sym.orient);
    let (dx, dy) = (at.x - center.0, at.y - center.1);
    let out = if dx == 0 && dy == 0 {
        (0, -1)
    } else if dx.abs() >= dy.abs() {
        (dx.signum(), 0)
    } else {
        (0, dy.signum())
    };
    Ok(PinLoc {
        inst: sym.inst_name().unwrap_or(inst).to_string(),
        pin: pdef.name.clone(),
        at,
        out,
    })
}

/// The pin a model most likely meant by a name the part does not have:
/// op-amp pins are written In-, In+ and OUT in many libraries, while the
/// built-in op-amp calls them invin, noninvin and out.
fn guess_pin<'a>(asked: &str, real: &[&'a str]) -> Option<&'a str> {
    const GROUPS: &[(&[&str], &[&str])] = &[
        (
            &[
                "in-",
                "-in",
                "inn",
                "in_n",
                "inm",
                "vin-",
                "inv",
                "inverting",
                "minus",
                "-",
            ],
            &["invin", "in-", "inn", "-"],
        ),
        (
            &[
                "in+",
                "+in",
                "inp",
                "in_p",
                "vin+",
                "noninv",
                "noninverting",
                "non-inverting",
                "plus",
                "+",
            ],
            &["noninvin", "in+", "inp", "+"],
        ),
        (&["out", "output", "vout", "o"], &["out", "output", "vout"]),
        (&["v+", "vcc", "vdd", "vs+"], &["v+", "vcc", "vdd"]),
        (&["v-", "vee", "vss", "vs-"], &["v-", "vee", "vss"]),
    ];
    let asked = asked.to_ascii_lowercase();
    GROUPS
        .iter()
        .filter(|(aliases, _)| aliases.contains(&asked.as_str()))
        .flat_map(|(_, targets)| targets.iter())
        .find_map(|t| real.iter().find(|r| r.eq_ignore_ascii_case(t)).copied())
}

/// Centre of the part body. Uses the drawing when there is one, otherwise the
/// centroid of the pins, which for two-pin parts is the midpoint.
fn body_center(def: &SymbolDef, origin: Point, orient: crate::geometry::Orient) -> (i32, i32) {
    let pins: Vec<Point> = def
        .pins
        .iter()
        .map(|p| origin + orient.apply(p.at))
        .collect();
    if def.graphics.is_empty() {
        // Sums in i64 so far-off coordinates cannot overflow.
        let n = pins.len().max(1) as i64;
        let sx: i64 = pins.iter().map(|p| p.x as i64).sum();
        let sy: i64 = pins.iter().map(|p| p.y as i64).sum();
        return ((sx / n) as i32, (sy / n) as i32);
    }
    let b = def
        .placed_bounds(origin, orient)
        .unwrap_or(Rect::from_points(origin, origin));
    (midpoint(b.min.x, b.max.x), midpoint(b.min.y, b.max.y))
}

/// The drawn body of each placed part, shrunk slightly so pins on its edge are
/// not counted as inside. Used to keep wires from crossing parts.
pub(crate) fn bodies(sch: &Schematic, lib: &SymbolLibrary) -> Vec<(String, Rect)> {
    sch.symbols()
        .filter_map(|s| {
            let (def, _) = lib.resolve(&s.name).ok()?;
            if def.graphics.is_empty() {
                return None;
            }
            let r = def.placed_bounds(s.at, s.orient)?;
            (r.width() > 8 && r.height() > 8)
                .then(|| (s.inst_name().unwrap_or("").to_string(), r.inflate(-4)))
        })
        .collect()
}

fn midpoint(a: i32, b: i32) -> i32 {
    ((a as i64 + b as i64) / 2) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_specs() {
        assert_eq!(split_pin_spec("R1.A"), Some(("R1", "A")));
        assert_eq!(split_pin_spec("U1.In-"), Some(("U1", "In-")));
        assert_eq!(split_pin_spec("V1.+"), Some(("V1", "+")));
        assert_eq!(split_pin_spec("R1"), None);
    }
}
