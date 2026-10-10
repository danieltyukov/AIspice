//! The operating point of each semiconductor device, from ngspice, in the
//! terms analog designers size with: region, bias, gm, gds, gm/Id and
//! intrinsic gain for MOSFETs; gm, rpi, ro and beta for BJTs; the
//! small-signal resistance of diodes.
//!
//! ngspice writes a device quantity to the raw file when a `.save` line
//! names it as `@device[param]`, beside the node voltages. Two runs make the
//! report. The first adds `.options savecurrents`, which saves the terminal
//! currents of every device under its full hierarchical name (`@m.x1.m7[id]`
//! for M7 inside X1): that is the only list of devices batch-mode ngspice
//! gives without cutting long names short. The second saves a fixed set of
//! quantities for each MOSFET, BJT and diode found.
//!
//! A quantity a model does not have is still written, as 0, with the warning
//! `unrecognized variable - @m1[vth]`: level 1 to 3 and VDMOS have `von` and
//! no `vth`, BSIM has `vth` and no `von`, VDMOS has no `vdsat`. Those
//! warnings decide which values are real.
//!
//! Signs differ between models. All of them report vgs, vds and vbs in the
//! device's own polarity (a conducting PMOS has a positive vgs, which is its
//! vsg), and so do BSIM's vth and vdsat; level 1 to 3 and VDMOS give von and
//! vdsat the type's sign, negative for PMOS. BJTs report vbe and vbc in their
//! own polarity and ic and ib as terminal currents, negative for a PNP. The
//! report puts everything in the device's own polarity, taking the type from
//! its `.model` card when the netlist has one and otherwise from those signs.

use crate::dataset::{AnalysisKind, Dataset};
use crate::measure::plain;
use aispice_core::netlist::{Line, Netlist};
use aispice_core::units;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Below threshold, a MOSFET carrying at least this much is called
/// subthreshold (weak inversion) rather than cutoff.
pub const SUBTHRESHOLD_CURRENT: f64 = 1e-9;
/// A BJT junction counts as on above this forward voltage (silicon).
pub const JUNCTION_ON: f64 = 0.4;
/// A saturated MOSFET closer than this to the triode edge is pointed out.
pub const NEAR_EDGE: f64 = 0.05;
/// Output conductances below this (gmin) are numerical noise, not a
/// finite output resistance.
pub const NEGLIGIBLE_G: f64 = 1e-12;

/// The devices the report covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Mosfet,
    Bjt,
    Diode,
}

const MOSFET_QUANTITIES: &[&str] = &[
    "id", "vgs", "vds", "vbs", "vth", "von", "vdsat", "gm", "gds", "gmbs", "cgs", "cgd", "w", "l",
];
const BJT_QUANTITIES: &[&str] = &["ic", "ib", "vbe", "vbc", "gm", "gpi", "go", "cpi", "cmu"];
const DIODE_QUANTITIES: &[&str] = &["id", "vd", "gd", "cd"];

impl Kind {
    /// The kind of a device from its ngspice name (`m1`, `m.x1.m7`, `q2`).
    pub fn of(device: &str) -> Option<Kind> {
        match device.chars().next()?.to_ascii_lowercase() {
            'm' => Some(Kind::Mosfet),
            'q' => Some(Kind::Bjt),
            'd' => Some(Kind::Diode),
            _ => None,
        }
    }

    /// The quantities saved for this kind of device: every name any common
    /// model has, since a missing one only costs a warning.
    pub fn quantities(self) -> &'static [&'static str] {
        match self {
            Kind::Mosfet => MOSFET_QUANTITIES,
            Kind::Bjt => BJT_QUANTITIES,
            Kind::Diode => DIODE_QUANTITIES,
        }
    }

    fn order(self) -> u8 {
        match self {
            Kind::Mosfet => 0,
            Kind::Bjt => 1,
            Kind::Diode => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    /// NMOS or NPN.
    N,
    /// PMOS or PNP.
    P,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    /// A MOSFET below threshold carrying almost nothing, or a BJT with
    /// neither junction on.
    Cutoff,
    /// A MOSFET below threshold that still conducts (weak inversion).
    Subthreshold,
    /// A MOSFET above threshold with vds below vdsat.
    Triode,
    /// A MOSFET above threshold with vds at or above vdsat, or a BJT with
    /// both junctions on (the two meanings differ, as in textbooks).
    Saturation,
    /// A BJT in forward-active operation.
    Active,
    /// A BJT with the collector junction on and the emitter junction off.
    ReverseActive,
}

impl Region {
    pub fn label(self) -> &'static str {
        match self {
            Region::Cutoff => "cutoff",
            Region::Subthreshold => "subthreshold",
            Region::Triode => "triode",
            Region::Saturation => "saturation",
            Region::Active => "active",
            Region::ReverseActive => "reverse active",
        }
    }
}

/// One device's operating point.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// ngspice's name, lower case: `m1`, `m.x1.m7`.
    pub id: String,
    /// The name as the netlist spells it, with its instance path: `M1`,
    /// `X1.M7`. Upper case when the netlist does not show the device.
    pub name: String,
    pub kind: Kind,
    pub polarity: Option<Polarity>,
    pub region: Option<Region>,
    /// Values in the device's own polarity. MOSFET: id, vgs, vds, vbs, vth
    /// (or von), vdsat, gm, gds, gmbs, gm_id, gm_gds, cgs, cgd, w, l. BJT:
    /// ic, ib, beta, vbe, vce, gm, rpi, ro, cpi, cmu. Diode: id, vd, rd, cd.
    /// Only what the model reports, and what follows from it.
    pub params: BTreeMap<String, f64>,
    /// Terminal nets when the netlist shows the device (drain, gate, source,
    /// bulk; collector, base, emitter; anode, cathode), lower case. A net
    /// inside a subcircuit that is not one of its ports carries the instance
    /// path (`x1.n1`).
    pub nets: Option<Vec<String>>,
    /// Caveats about this device's numbers.
    pub notes: Vec<String>,
}

impl Device {
    pub fn get(&self, key: &str) -> Option<f64> {
        self.params.get(key).copied()
    }

    /// `nmos`, `pmos`, `npn`, `pnp` or `diode`; `mosfet` or `bjt` when the
    /// polarity is unknown.
    pub fn type_name(&self) -> &'static str {
        match (self.kind, self.polarity) {
            (Kind::Mosfet, Some(Polarity::N)) => "nmos",
            (Kind::Mosfet, Some(Polarity::P)) => "pmos",
            (Kind::Mosfet, None) => "mosfet",
            (Kind::Bjt, Some(Polarity::N)) => "npn",
            (Kind::Bjt, Some(Polarity::P)) => "pnp",
            (Kind::Bjt, None) => "bjt",
            (Kind::Diode, _) => "diode",
        }
    }

    /// Whether `filter` names this device (see [`matches`]).
    pub fn matches(&self, filter: &str) -> bool {
        matches(&self.name, &self.id, filter)
    }
}

/// Whether `filter` names a device: its name (`X1.M7`), its ngspice name
/// (`m.x1.m7`), or a subcircuit instance it sits in (`X1`). Any case.
pub fn matches(name: &str, id: &str, filter: &str) -> bool {
    let f = filter.trim().to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    name == f || id.eq_ignore_ascii_case(&f) || name.starts_with(&format!("{f}."))
}

/// The device and quantity a raw-file vector holds, lower case: `@m1[gm]`,
/// `I(@m1[id])` and `V(@m.x1.m7[vgs])` are device quantities; `V(out)` is
/// not.
pub fn device_quantity(vector: &str) -> Option<(String, String)> {
    let lower = vector.trim().to_ascii_lowercase();
    let inner = ["v(", "i("]
        .iter()
        .find_map(|p| lower.strip_prefix(p).and_then(|r| r.strip_suffix(')')))
        .unwrap_or(&lower);
    let (device, quantity) = inner
        .strip_prefix('@')?
        .strip_suffix(']')?
        .split_once('[')?;
    (!device.is_empty() && !quantity.is_empty()).then(|| (device.to_string(), quantity.to_string()))
}

fn op_dataset(datasets: &[Dataset]) -> Option<&Dataset> {
    datasets.iter().find(|d| d.kind == AnalysisKind::Op)
}

/// Every MOSFET, BJT and diode of a run made with `.options savecurrents`,
/// in the order ngspice lists them.
pub fn discover(datasets: &[Dataset]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in op_dataset(datasets)
        .map(|d| d.vectors.as_slice())
        .unwrap_or(&[])
    {
        if let Some((device, _)) = device_quantity(&v.name)
            && Kind::of(&device).is_some()
            && !out.contains(&device)
        {
            out.push(device);
        }
    }
    out
}

/// The `.save` lines for the second run: every node voltage and source
/// current, and each device's quantities.
pub fn save_directives(devices: &[String]) -> Vec<String> {
    let mut out = vec![".save all".to_string()];
    for d in devices {
        if let Some(kind) = Kind::of(d) {
            let items: Vec<String> = kind
                .quantities()
                .iter()
                .map(|q| format!("@{d}[{q}]"))
                .collect();
            out.push(format!(".save {}", items.join(" ")));
        }
    }
    out
}

/// The quantities ngspice did not recognise, from its warnings (`Warning:
/// unrecognized variable - @m1[vth]`), lower case.
pub fn unrecognized(log: &str) -> HashSet<String> {
    const PHRASE: &str = "unrecognized variable";
    log.lines()
        .filter_map(|l| {
            let lower = l.to_ascii_lowercase();
            let at = lower.find(PHRASE)? + PHRASE.len();
            let rest = lower[at..].trim_start_matches([' ', '-', ':', '\t']);
            rest.split_whitespace().next().map(str::to_string)
        })
        .collect()
}

/// Whether a simulator warning is one of the `unrecognized variable` notes
/// the second run causes on purpose.
pub fn is_unrecognized_warning(warning: &str) -> bool {
    warning
        .to_ascii_lowercase()
        .contains("unrecognized variable")
}

/// The device quantities of a run, by device, leaving out those ngspice did
/// not recognise.
pub fn quantities(
    datasets: &[Dataset],
    unrecognized: &HashSet<String>,
) -> HashMap<String, BTreeMap<String, f64>> {
    let mut out: HashMap<String, BTreeMap<String, f64>> = HashMap::new();
    for v in op_dataset(datasets)
        .map(|d| d.vectors.as_slice())
        .unwrap_or(&[])
    {
        let Some((device, quantity)) = device_quantity(&v.name) else {
            continue;
        };
        if unrecognized.contains(&format!("@{device}[{quantity}]")) {
            continue;
        }
        if let Some(x) = v.data.real().first().copied().filter(|x| x.is_finite()) {
            out.entry(device).or_default().insert(quantity, x);
        }
    }
    out
}

/// Node voltages and source currents of an operating point, as the run
/// names them (`V(out)`, `I(vdd)`), without device quantities.
pub fn node_values(datasets: &[Dataset]) -> Vec<(String, f64)> {
    op_dataset(datasets)
        .map(|op| {
            op.vectors
                .iter()
                .filter(|v| device_quantity(&v.name).is_none())
                .filter_map(|v| v.data.real().first().map(|x| (v.name.clone(), *x)))
                .collect()
        })
        .unwrap_or_default()
}

/// The instance path of an ngspice name: `m.x1.m7`, M7 inside X1, is
/// `["x1", "m7"]`; `m1` is `["m1"]`.
fn hierarchy(id: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = id.split('.').collect();
    if parts.len() > 1 && parts[0].len() == 1 {
        parts.remove(0);
    }
    parts
}

/// A device name for people when the netlist does not show the device:
/// `m.x1.m7` is `X1.M7`.
pub fn display_name(id: &str) -> String {
    hierarchy(id)
        .iter()
        .map(|s| s.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(".")
}

/// Where a device sits in the netlist.
#[derive(Debug, Clone, PartialEq)]
pub struct Located {
    /// The instance path as the netlist spells it: `X1.M7`.
    pub name: String,
    /// Terminal nets, as in [`Device::nets`].
    pub nets: Vec<String>,
    /// From the device's `.model` card, when the netlist has it.
    pub polarity: Option<Polarity>,
}

/// Find a device by its ngspice name, following subcircuit instances
/// through their port connections. `None` when a definition is missing
/// (a subcircuit from a library file the netlist only includes).
pub fn locate(netlist: &Netlist, id: &str) -> Option<Located> {
    let path = hierarchy(id);
    let mut scope: &[Line] = &netlist.items;
    let mut scopes: Vec<&[Line]> = vec![&netlist.items];
    let mut ports: HashMap<String, String> = HashMap::new();
    let mut prefix = String::new();
    let mut names = Vec::new();
    for (i, seg) in path.iter().enumerate() {
        let el = scope.iter().find_map(|l| match l {
            Line::Element(e) if e.name.eq_ignore_ascii_case(seg) => Some(e),
            _ => None,
        })?;
        names.push(el.name.clone());
        let nets: Vec<String> = el.nodes.iter().map(|n| net(n, &ports, &prefix)).collect();
        if i + 1 == path.len() {
            let polarity = el
                .first_word()
                .and_then(|m| scopes.iter().rev().find_map(|s| polarity_in(s, m)));
            return Some(Located {
                name: names.join("."),
                nets,
                polarity,
            });
        }
        if el.letter() != 'X' {
            return None;
        }
        let sub_name = el.first_word()?;
        let sub = scopes.iter().rev().find_map(|s| {
            s.iter().find_map(|l| match l {
                Line::Subckt(sc) if sc.name.eq_ignore_ascii_case(sub_name) => Some(sc),
                _ => None,
            })
        })?;
        ports = sub
            .ports
            .iter()
            .zip(&nets)
            .map(|(p, n)| (p.to_ascii_lowercase(), n.clone()))
            .collect();
        prefix = format!("{prefix}{}.", seg.to_ascii_lowercase());
        scope = &sub.body;
        scopes.push(&sub.body);
    }
    None
}

fn net(local: &str, ports: &HashMap<String, String>, prefix: &str) -> String {
    let l = local.to_ascii_lowercase();
    if l == "0" || l == "gnd" {
        return "0".into();
    }
    ports
        .get(&l)
        .cloned()
        .unwrap_or_else(|| format!("{prefix}{l}"))
}

fn polarity_in(lines: &[Line], model: &str) -> Option<Polarity> {
    lines.iter().find_map(|l| match l {
        Line::Directive { text } => model_polarity(text, model),
        _ => None,
    })
}

/// The polarity a `.model` card gives devices that use it.
fn model_polarity(card: &str, model: &str) -> Option<Polarity> {
    let mut words = card.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case(".model") {
        return None;
    }
    let name = words.next()?;
    if !name.eq_ignore_ascii_case(model) {
        return None;
    }
    let rest = words.collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    let kind: String = rest.chars().take_while(char::is_ascii_alphabetic).collect();
    match kind.as_str() {
        "nmos" | "npn" => Some(Polarity::N),
        "pmos" | "pnp" | "lpnp" => Some(Polarity::P),
        "vdmos" if rest.contains("pchan") => Some(Polarity::P),
        "vdmos" => Some(Polarity::N),
        _ => None,
    }
}

type Raw = BTreeMap<String, f64>;

/// One device's report from its raw quantities and, when the netlist shows
/// it, where it sits.
pub fn device(id: &str, raw: &Raw, at: Option<&Located>) -> Option<Device> {
    let kind = Kind::of(id)?;
    let mut d = Device {
        id: id.to_ascii_lowercase(),
        name: at.map_or_else(|| display_name(id), |l| l.name.clone()),
        kind,
        polarity: at.and_then(|l| l.polarity),
        region: None,
        params: BTreeMap::new(),
        nets: at.map(|l| l.nets.clone()),
        notes: Vec::new(),
    };
    match kind {
        Kind::Mosfet => mosfet(&mut d, raw),
        Kind::Bjt => bjt(&mut d, raw),
        Kind::Diode => diode(&mut d, raw),
    }
    Some(d)
}

fn copy(d: &mut Device, raw: &Raw, keys: &[&str]) {
    for k in keys {
        if let Some(v) = raw.get(*k) {
            d.params.insert((*k).to_string(), *v);
        }
    }
}

fn mosfet(d: &mut Device, raw: &Raw) {
    let get = |k: &str| raw.get(k).copied();
    let vdsat_raw = get("vdsat");
    // BSIM's vth is in the device's polarity already; von from level 1 to 3
    // and VDMOS has the type's sign, and so has their vdsat.
    let threshold = if let Some(v) = get("vth") {
        Some(("vth", v))
    } else if let Some(v) = get("von") {
        if d.polarity.is_none()
            && let Some(s) = vdsat_raw.filter(|s| *s != 0.0)
        {
            d.polarity = Some(if s < 0.0 { Polarity::P } else { Polarity::N });
        }
        let sign = match d.polarity {
            Some(Polarity::P) => -1.0,
            Some(Polarity::N) => 1.0,
            // Unknown type and no vdsat to tell it: assume an enhancement
            // device, whose threshold is positive in its own polarity.
            None => v.signum(),
        };
        Some(("von", v * sign))
    } else {
        None
    };
    let mut vdsat = vdsat_raw.map(f64::abs);
    copy(
        d,
        raw,
        &["id", "vgs", "vds", "vbs", "gm", "gds", "gmbs", "w", "l"],
    );
    if let Some((key, v)) = threshold {
        d.params.insert(key.into(), v);
    }
    for k in ["cgs", "cgd"] {
        // BSIM reports the transcapacitance dQg/dV, negative for a
        // capacitance the gate sees as positive.
        if let Some(c) = get(k) {
            d.params.insert(k.into(), c.abs());
        }
    }
    let (id, vgs, vds, gm, gds) = (get("id"), get("vgs"), get("vds"), get("gm"), get("gds"));
    if let (Some(gm), Some(id)) = (gm, id)
        && id != 0.0
    {
        d.params.insert("gm_id".into(), gm / id.abs());
    }
    match (gm, gds) {
        (Some(gm), Some(gds)) if gds > NEGLIGIBLE_G => {
            d.params.insert("gm_gds".into(), gm / gds);
        }
        (_, Some(_)) => d.notes.push(
            "gds is below 1 pS: the model has no channel-length modulation, so gm/gds is unbounded"
                .into(),
        ),
        _ => {}
    }
    let (Some((source, vth)), Some(vgs), Some(vds)) = (threshold, vgs, vds) else {
        if let Some(v) = vdsat {
            d.params.insert("vdsat".into(), v);
        }
        return;
    };
    // With vds < 0 the drain and source swap roles: the gate drive is then
    // vgd and the device's "vds" is -vds.
    let (vgs, vds) = if vds < 0.0 {
        d.notes
            .push("drain and source are swapped (vds < 0); region judged with vgd".into());
        (vgs - vds, -vds)
    } else {
        (vgs, vds)
    };
    let vov = vgs - vth;
    if vdsat.is_none() && source == "von" && vov >= 0.0 {
        d.notes
            .push("the model reports no vdsat; taken as vgs - von".into());
        vdsat = Some(vov);
    }
    if let Some(v) = vdsat {
        d.params.insert("vdsat".into(), v);
    }
    d.region = if vov < 0.0 {
        Some(if id.unwrap_or(0.0).abs() >= SUBTHRESHOLD_CURRENT {
            Region::Subthreshold
        } else {
            Region::Cutoff
        })
    } else {
        vdsat.map(|sat| {
            if vds >= sat {
                Region::Saturation
            } else {
                Region::Triode
            }
        })
    };
}

fn bjt(d: &mut Device, raw: &Raw) {
    let get = |k: &str| raw.get(k).copied();
    let (ic, ib, vbe, vbc) = (get("ic"), get("ib"), get("vbe"), get("vbc"));
    if d.polarity.is_none()
        && let (Some(vbe), Some(ib)) = (vbe, ib)
        && vbe > 0.3
        && ib != 0.0
    {
        // A forward-biased base takes current in an NPN and gives it out
        // of a PNP.
        d.polarity = Some(if ib > 0.0 { Polarity::N } else { Polarity::P });
    }
    let sign = if d.polarity == Some(Polarity::P) {
        -1.0
    } else {
        1.0
    };
    if let Some(ic) = ic {
        d.params.insert("ic".into(), ic * sign);
    }
    if let Some(ib) = ib {
        d.params.insert("ib".into(), ib * sign);
    }
    if let (Some(ic), Some(ib)) = (ic, ib)
        && ib != 0.0
    {
        d.params.insert("beta".into(), ic / ib);
    }
    copy(d, raw, &["vbe", "gm", "cpi", "cmu"]);
    if let (Some(vbe), Some(vbc)) = (vbe, vbc) {
        d.params.insert("vce".into(), vbe - vbc);
        d.region = Some(match (vbe > JUNCTION_ON, vbc > JUNCTION_ON) {
            (true, false) => Region::Active,
            (true, true) => Region::Saturation,
            (false, true) => Region::ReverseActive,
            (false, false) => Region::Cutoff,
        });
    }
    if let Some(g) = get("gpi").filter(|g| *g > 0.0) {
        d.params.insert("rpi".into(), 1.0 / g);
    }
    match get("go") {
        Some(g) if g > NEGLIGIBLE_G => {
            d.params.insert("ro".into(), 1.0 / g);
        }
        Some(_) => d
            .notes
            .push("ro is above 1 TΩ: the model has no Early effect".into()),
        None => {}
    }
}

fn diode(d: &mut Device, raw: &Raw) {
    copy(d, raw, &["id", "vd", "cd"]);
    if let Some(g) = raw.get("gd").copied().filter(|g| *g > 0.0) {
        d.params.insert("rd".into(), 1.0 / g);
    }
}

/// Order for the report: devices at the top level first, then those inside
/// subcircuits; MOSFETs, BJTs, diodes; names in natural order (M2 before
/// M10).
pub fn sort(devices: &mut [Device]) {
    devices.sort_by(|a, b| {
        (a.id.contains('.'), a.kind.order())
            .cmp(&(b.id.contains('.'), b.kind.order()))
            .then_with(|| natural(&a.name).cmp(&natural(&b.name)))
    });
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Chunk {
    Text(String),
    Number(u64),
}

fn natural(s: &str) -> Vec<Chunk> {
    let mut out = Vec::new();
    let mut digits = String::new();
    let mut text = String::new();
    for c in s.to_ascii_lowercase().chars() {
        if c.is_ascii_digit() {
            if !text.is_empty() {
                out.push(Chunk::Text(std::mem::take(&mut text)));
            }
            digits.push(c);
        } else {
            if !digits.is_empty() {
                out.push(Chunk::Number(digits.parse().unwrap_or(u64::MAX)));
                digits.clear();
            }
            text.push(c);
        }
    }
    if !digits.is_empty() {
        out.push(Chunk::Number(digits.parse().unwrap_or(u64::MAX)));
    }
    if !text.is_empty() {
        out.push(Chunk::Text(text));
    }
    out
}

fn fmt(v: f64, unit: &str) -> String {
    units::format_with_unit(v, unit)
}

/// One line for a device: its name, type and region, then its key numbers
/// to four significant digits.
pub fn line(d: &Device) -> String {
    fn put(parts: &mut Vec<String>, d: &Device, key: &str, unit: &str) {
        if let Some(v) = d.get(key) {
            parts.push(format!("{key}={}", fmt(v, unit)));
        }
    }
    let mut parts: Vec<String> = Vec::new();
    let p = &mut parts;
    match d.kind {
        Kind::Mosfet => {
            put(p, d, "id", "A");
            put(p, d, "vgs", "V");
            put(p, d, "vds", "V");
            if d.get("vbs").is_some_and(|v| v.abs() >= 1e-3) {
                put(p, d, "vbs", "V");
            }
            put(p, d, "vth", "V");
            put(p, d, "von", "V");
            put(p, d, "vdsat", "V");
            put(p, d, "gm", "S");
            put(p, d, "gds", "S");
            if let Some(v) = d.get("gm_id") {
                p.push(format!("gm/id={}/V", plain(v)));
            }
            if let Some(v) = d.get("gm_gds") {
                p.push(format!("gm/gds={}", plain(v)));
            }
            for c in ["cgs", "cgd"] {
                if d.get(c).is_some_and(|v| v != 0.0) {
                    put(p, d, c, "F");
                }
            }
            if let (Some(w), Some(l)) = (d.get("w"), d.get("l")) {
                p.push(format!("W/L={}/{}", units::format(w), units::format(l)));
            }
        }
        Kind::Bjt => {
            put(p, d, "ic", "A");
            put(p, d, "ib", "A");
            if let Some(v) = d.get("beta") {
                p.push(format!("beta={}", plain(v)));
            }
            put(p, d, "vbe", "V");
            put(p, d, "vce", "V");
            put(p, d, "gm", "S");
            put(p, d, "rpi", "Ω");
            put(p, d, "ro", "Ω");
        }
        Kind::Diode => {
            put(p, d, "id", "A");
            put(p, d, "vd", "V");
            put(p, d, "rd", "Ω");
        }
    }
    let region = match (d.kind, d.region) {
        (_, Some(r)) => format!(" {}", r.label()),
        (Kind::Diode, None) => String::new(),
        (_, None) => " (region unknown)".into(),
    };
    let mut s = format!("{} {}{region}: {}", d.name, d.type_name(), parts.join(" "));
    for n in &d.notes {
        s.push_str(&format!("; {n}"));
    }
    s
}

/// The diode-connected device `d` mirrors: one of the same kind and
/// polarity whose gate (base) is also its drain (collector) and is `d`'s
/// gate (base), while `d` itself is not diode-connected.
fn mirror_reference<'a>(d: &Device, all: &'a [Device]) -> Option<&'a Device> {
    if d.kind == Kind::Diode {
        return None;
    }
    let nets = d.nets.as_ref()?;
    let (drain, gate) = (nets.first()?, nets.get(1)?);
    if drain == gate {
        return None;
    }
    all.iter().find(|o| {
        o.id != d.id
            && o.kind == d.kind
            && (o.polarity.is_none() || d.polarity.is_none() || o.polarity == d.polarity)
            && o.nets
                .as_ref()
                .is_some_and(|n| n.len() >= 2 && n[0] == n[1] && &n[1] == gate)
    })
}

/// Devices whose bias looks wrong for what they seem to do. The netlist
/// rarely says what a device is for, so these say what would be wrong
/// rather than that something is: only a mirror output, recognised by its
/// gate shared with a diode-connected device, is called out outright.
pub fn checks(devices: &[Device]) -> Vec<String> {
    let mut out = Vec::new();
    for d in devices {
        let Some(region) = d.region else {
            continue;
        };
        let mirror = mirror_reference(d, devices);
        let v = |k: &str| d.get(k).map(|x| fmt(x, "V")).unwrap_or_else(|| "?".into());
        let threshold = if d.get("vth").is_some() { "vth" } else { "von" };
        match (d.kind, region) {
            (Kind::Mosfet, Region::Triode) => {
                let short = match (d.get("vdsat"), d.get("vds")) {
                    (Some(sat), Some(vds)) => fmt(sat - vds.abs(), "V"),
                    _ => "?".into(),
                };
                out.push(match mirror {
                    Some(r) => format!(
                        "{} shares its gate with diode-connected {}, so it looks like a mirror output, but it is in triode (vds {} < vdsat {}): its current will not track {}'s until it has about {short} more vds.",
                        d.name, r.name, v("vds"), v("vdsat"), r.name
                    ),
                    None => format!(
                        "{} is in triode (vds {} < vdsat {}): fine for a switch or a resistor; a gain, cascode or current-source device needs about {short} more vds.",
                        d.name, v("vds"), v("vdsat")
                    ),
                });
            }
            (Kind::Mosfet, Region::Cutoff) => out.push(match mirror {
                Some(r) => format!(
                    "{} shares its gate with diode-connected {}, so it looks like a mirror output, but it is off (vgs {} < {threshold} {}).",
                    d.name, r.name, v("vgs"), v(threshold)
                ),
                None => format!(
                    "{} is off (vgs {} < {threshold} {}): expected for an open switch; a bias or signal-path device there carries no current.",
                    d.name, v("vgs"), v(threshold)
                ),
            }),
            (Kind::Mosfet, Region::Saturation) => {
                if let (Some(vds), Some(sat)) = (d.get("vds"), d.get("vdsat")) {
                    let margin = vds.abs() - sat;
                    if (0.0..NEAR_EDGE).contains(&margin) {
                        out.push(format!(
                            "{} is saturated with only {} between vds and vdsat: little room for signal swing or process spread.",
                            d.name,
                            fmt(margin, "V")
                        ));
                    }
                }
            }
            (Kind::Bjt, Region::Saturation) => out.push(match mirror {
                Some(r) => format!(
                    "{} shares its base with diode-connected {}, so it looks like a mirror output, but it is saturated (vce {}): its current will not track {}'s.",
                    d.name, r.name, v("vce"), r.name
                ),
                None => format!(
                    "{} is saturated (vce {}): fine for a switch; an amplifier or current source needs vce above about 0.3 V.",
                    d.name,
                    v("vce")
                ),
            }),
            (Kind::Bjt, Region::Cutoff) => out.push(match mirror {
                Some(r) => format!(
                    "{} shares its base with diode-connected {}, so it looks like a mirror output, but it is off (vbe {}).",
                    d.name, r.name, v("vbe")
                ),
                None => format!(
                    "{} is off (vbe {}): expected for an open switch.",
                    d.name,
                    v("vbe")
                ),
            }),
            (Kind::Bjt, Region::ReverseActive) => out.push(format!(
                "{} is reverse active (vbe {}, vce {}): check whether its collector and emitter are swapped.",
                d.name,
                v("vbe"),
                v("vce")
            )),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{Quantity, Vector, VectorData};

    fn raw(pairs: &[(&str, f64)]) -> Raw {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1e-30)
    }

    fn op(vectors: &[(&str, f64)]) -> Dataset {
        Dataset {
            title: "t".into(),
            plotname: "Operating Point".into(),
            kind: AnalysisKind::Op,
            axis: None,
            vectors: vectors
                .iter()
                .map(|(n, x)| Vector {
                    name: n.to_string(),
                    quantity: Quantity::Other,
                    data: VectorData::Real(vec![*x]),
                })
                .collect(),
            steps: Vec::new(),
        }
    }

    #[test]
    fn device_vectors_are_recognised() {
        assert_eq!(
            device_quantity("I(@m1[id])"),
            Some(("m1".into(), "id".into()))
        );
        assert_eq!(
            device_quantity("V(@m.x1.m7[vgs])"),
            Some(("m.x1.m7".into(), "vgs".into()))
        );
        assert_eq!(device_quantity("@Q1[gm]"), Some(("q1".into(), "gm".into())));
        assert_eq!(device_quantity("V(out)"), None);
        assert_eq!(device_quantity("I(vdd)"), None);
        assert_eq!(device_quantity("@m1[]"), None);
    }

    #[test]
    fn discovery_and_save_lines() {
        let ds = op(&[
            ("V(d)", 1.0),
            ("I(vdd)", -1e-3),
            ("I(@m1[id])", 1e-3),
            ("I(@m1[is])", -1e-3),
            ("I(@r1[i])", 1e-3),
            ("I(@q1[ic])", 1e-3),
            ("I(@d1[id])", 1e-3),
            ("I(@m.x1.m7[id])", 1e-4),
        ]);
        let found = discover(std::slice::from_ref(&ds));
        assert_eq!(found, ["m1", "q1", "d1", "m.x1.m7"]);
        let saves = save_directives(&found);
        assert_eq!(saves[0], ".save all");
        assert!(saves[1].starts_with(".save @m1[id] @m1[vgs] "), "{saves:?}");
        assert!(saves[1].contains("@m1[vth] @m1[von] @m1[vdsat]"));
        assert!(saves[2].contains("@q1[gpi] @q1[go]"));
        assert_eq!(saves[3], ".save @d1[id] @d1[vd] @d1[gd] @d1[cd]");
        assert!(saves[4].contains("@m.x1.m7[gm]"));
        assert_eq!(
            node_values(&[ds]),
            [("V(d)".to_string(), 1.0), ("I(vdd)".to_string(), -1e-3)]
        );
    }

    #[test]
    fn unrecognised_quantities_are_dropped() {
        let log = "Using SPARSE 1.3\nWarning: unrecognized variable - @m1[vth]\nWarning: unrecognized variable - @M.X1.M7[VTH]\nWarning: Pd = 0 is less than W.\n";
        let missing = unrecognized(log);
        assert_eq!(missing.len(), 2);
        assert!(missing.contains("@m1[vth]") && missing.contains("@m.x1.m7[vth]"));
        assert!(is_unrecognized_warning(
            "Warning: unrecognized variable - @m1[vth]"
        ));
        assert!(!is_unrecognized_warning("Warning: Pd = 0 is less than W."));
        let ds = op(&[
            ("V(@m1[vth])", 0.0),
            ("V(@m1[von])", 0.7),
            ("@m1[gm]", 8e-4),
        ]);
        let q = quantities(&[ds], &missing);
        assert_eq!(q["m1"], raw(&[("von", 0.7), ("gm", 8e-4)]));
    }

    // ngspice 42 values for a level-1 NMOS common-source stage (kp 100u,
    // W/L 10, vto 0.7, lambda 0.02, vgs 1.5 V, 10k from 5 V).
    fn level1_nmos() -> Raw {
        raw(&[
            ("id", 3.308270692685424e-4),
            ("vgs", 1.5),
            ("vds", 1.691729307314576),
            ("vbs", 0.0),
            ("von", 0.7),
            ("vdsat", 0.8),
            ("gm", 8.270676689170329e-4),
            ("gds", 6.4e-6),
            ("gmbs", 0.0),
            ("cgs", 1e-14),
            ("cgd", 1e-14),
            ("w", 10e-6),
            ("l", 1e-6),
        ])
    }

    #[test]
    fn level1_nmos_in_saturation() {
        let d = device("m1", &level1_nmos(), None).unwrap();
        assert_eq!(d.name, "M1");
        assert_eq!(d.type_name(), "nmos");
        assert_eq!(d.region, Some(Region::Saturation));
        assert!(close(d.get("von").unwrap(), 0.7));
        assert!(d.get("vth").is_none());
        // Square law: gm/id = 2 / (vgs - vth) = 2.5 per volt.
        assert!((d.get("gm_id").unwrap() - 2.5).abs() < 1e-7);
        assert!(close(d.get("gm_gds").unwrap(), 129.2293232682864));
        let l = line(&d);
        assert!(
            l.starts_with("M1 nmos saturation: id=330.8uA vgs=1.5V vds=1.692V von=700mV vdsat=800mV gm=827.1uS gds=6.4uS gm/id=2.5/V gm/gds=129.2 cgs=10fF cgd=10fF W/L=10u/1u"),
            "{l}"
        );
    }

    #[test]
    fn level1_pmos_signs_are_normalised() {
        // ngspice 42: vgs and vds come positive, von and vdsat negative.
        let d = device(
            "mp1",
            &raw(&[
                ("id", 1.3229e-4),
                ("vgs", 1.5),
                ("vds", 1.677),
                ("von", -0.7),
                ("vdsat", -0.8),
                ("gm", 3.307e-4),
                ("gds", 2.56e-6),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(d.type_name(), "pmos");
        assert!(close(d.get("von").unwrap(), 0.7));
        assert!(close(d.get("vdsat").unwrap(), 0.8));
        assert_eq!(d.region, Some(Region::Saturation));
        // A depletion NMOS also has a negative von, but a positive vdsat.
        let dep = device(
            "md",
            &raw(&[
                ("id", 2.68e-4),
                ("vgs", 0.0),
                ("vds", 0.319),
                ("von", -1.0),
                ("vdsat", 1.0),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(dep.type_name(), "nmos");
        assert!(close(dep.get("von").unwrap(), -1.0));
        assert_eq!(dep.region, Some(Region::Triode));
    }

    #[test]
    fn bsim_values_and_regions() {
        // BSIM4 PMOS: everything positive, cgs and cgd negative.
        let p = Located {
            name: "MP4".into(),
            nets: vec!["dp4".into(), "gp".into(), "vdd".into(), "vdd".into()],
            polarity: Some(Polarity::P),
        };
        let d = device(
            "mp4",
            &raw(&[
                ("id", 2.6087e-4),
                ("vgs", 1.5),
                ("vds", 0.3913),
                ("vth", 0.3493),
                ("vdsat", 0.7276),
                ("gm", 2.216e-4),
                ("gds", 4.302e-4),
                ("cgs", -1.2e-15),
                ("cgd", -4.3e-16),
            ]),
            Some(&p),
        )
        .unwrap();
        assert_eq!(d.type_name(), "pmos");
        assert_eq!(d.region, Some(Region::Triode));
        assert!(close(d.get("cgs").unwrap(), 1.2e-15));
        assert!(close(d.get("cgd").unwrap(), 4.3e-16));
        // Without a model card a BSIM device's polarity cannot be told.
        let n = device(
            "mb",
            &raw(&[
                ("vth", 0.35),
                ("vgs", 0.3),
                ("vds", 1.0),
                ("vdsat", 0.05),
                ("id", 2e-8),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(n.type_name(), "mosfet");
        assert_eq!(n.region, Some(Region::Subthreshold));
        let off = device(
            "mb",
            &raw(&[
                ("vth", 2.16),
                ("vgs", 0.9),
                ("vds", 1.8),
                ("vdsat", 0.03),
                ("id", 3.9e-19),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(off.region, Some(Region::Cutoff));
        // Level 1 below threshold carries nothing: cutoff.
        let l1 = device(
            "m1",
            &raw(&[
                ("von", 0.7),
                ("vgs", 0.5),
                ("vds", 5.0),
                ("vdsat", 0.0),
                ("id", 1.7e-12),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(l1.region, Some(Region::Cutoff));
        assert_eq!(l1.type_name(), "mosfet");
    }

    #[test]
    fn vdmos_without_vdsat_and_reversed_devices() {
        let d = device(
            "mv",
            &raw(&[
                ("id", 0.2846),
                ("vgs", 3.997),
                ("vds", 0.148),
                ("von", 2.0),
                ("gm", 0.148),
                ("gds", 1.849),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(d.region, Some(Region::Triode));
        assert!(close(d.get("vdsat").unwrap(), 1.997));
        assert!(d.notes[0].contains("no vdsat"), "{:?}", d.notes);
        let r = device(
            "m2",
            &raw(&[
                ("id", -1e-4),
                ("vgs", 1.0),
                ("vds", -0.5),
                ("vth", 0.4),
                ("vdsat", 0.3),
            ]),
            None,
        )
        .unwrap();
        // vgd = 1.5 V, so vov = 1.1 V; |vds| 0.5 V >= vdsat 0.3 V.
        assert_eq!(r.region, Some(Region::Saturation));
        assert!(r.notes[0].contains("swapped"));
    }

    #[test]
    fn bjt_values_and_regions() {
        // ngspice 42, NPN with is 1e-15, bf 100, vaf 50 at vbe 0.7 V.
        let q = device(
            "q1",
            &raw(&[
                ("ic", 5.730110973032762e-4),
                ("ib", 5.670346943433543e-6),
                ("vbe", 0.7),
                ("vbc", -0.5269889014697363),
                ("gm", 2.214264868971018e-2),
                ("gpi", 2.192292670310164e-4),
                ("go", 1.134069354286548e-5),
                ("cpi", 0.0),
                ("cmu", 0.0),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(q.type_name(), "npn");
        assert_eq!(q.region, Some(Region::Active));
        assert!(close(q.get("beta").unwrap(), 101.05397483073638));
        assert!(close(q.get("vce").unwrap(), 1.2269889014697364));
        assert!((q.get("rpi").unwrap() - 4561.4).abs() < 0.1);
        assert!((q.get("ro").unwrap() - 88178.0).abs() < 1.0);
        let l = line(&q);
        assert!(l.starts_with("Q1 npn active: ic=573uA ib=5.67uA beta=101.1 vbe=700mV vce=1.227V gm=22.14mS rpi=4.561kΩ ro=88.18kΩ"), "{l}");
        // PNP: currents come negative and are turned round.
        let p = device(
            "qp1",
            &raw(&[
                ("ic", -5.9126e-4),
                ("ib", -7.0879e-6),
                ("vbe", 0.7),
                ("vbc", -1.7087),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(p.type_name(), "pnp");
        assert!(close(p.get("ic").unwrap(), 5.9126e-4));
        assert!(p.get("beta").unwrap() > 0.0);
        let regions = [
            (0.75, 0.6, Region::Saturation),
            (0.2, -3.0, Region::Cutoff),
            (0.1, 0.7, Region::ReverseActive),
        ];
        for (vbe, vbc, want) in regions {
            let d = device("q9", &raw(&[("vbe", vbe), ("vbc", vbc)]), None).unwrap();
            assert_eq!(d.region, Some(want), "vbe {vbe} vbc {vbc}");
        }
    }

    #[test]
    fn diode_resistance() {
        let d = device(
            "d1",
            &raw(&[
                ("id", 1.000005e-3),
                ("vd", 0.6551180333590192),
                ("gd", 3.866261483690631e-2),
                ("cd", 0.0),
            ]),
            None,
        )
        .unwrap();
        assert!((d.get("rd").unwrap() - 25.8648).abs() < 1e-3);
        assert_eq!(d.region, None);
        assert_eq!(line(&d), "D1 diode: id=1mA vd=655.1mV rd=25.86Ω");
    }

    #[test]
    fn negligible_output_conductance_is_not_a_resistance() {
        // ngspice 42: an NPN without vaf has go of order 1e-21 S.
        let q = device("q2", &raw(&[("go", 1.16e-21), ("gpi", 3.6e-4)]), None).unwrap();
        assert!(q.get("ro").is_none());
        assert!(q.get("rpi").is_some());
        assert!(q.notes[0].contains("no Early effect"), "{:?}", q.notes);
        assert!(line(&q).ends_with("; ro is above 1 TΩ: the model has no Early effect"));
        let m = device("m7", &raw(&[("gm", 2.5e-4), ("gds", 0.0)]), None).unwrap();
        assert!(m.get("gm_gds").is_none());
        assert!(
            m.notes[0].contains("no channel-length modulation"),
            "{:?}",
            m.notes
        );
    }

    const HIER: &str = "* hierarchy
VDD vdd 0 5
M1 vdd g 0 0 nch W=1u L=1u
X1 vdd g out stage
X2 vdd g out2 libpart
.model nch nmos level=1
.subckt stage top in out
RL top out 20k
M7 out in mid 0 lp
M8 mid mid 0 0 nch
.model lp pmos level=1
.ends
";

    #[test]
    fn devices_are_found_through_subcircuits() {
        let n = aispice_core::netlist::parse(HIER);
        let top = locate(&n, "m1").unwrap();
        assert_eq!(top.name, "M1");
        assert_eq!(top.nets, ["vdd", "g", "0", "0"]);
        assert_eq!(top.polarity, Some(Polarity::N));
        let inner = locate(&n, "m.x1.m7").unwrap();
        assert_eq!(inner.name, "X1.M7");
        assert_eq!(inner.nets, ["out", "g", "x1.mid", "0"]);
        assert_eq!(inner.polarity, Some(Polarity::P), "local model");
        let m8 = locate(&n, "m.x1.m8").unwrap();
        assert_eq!(m8.polarity, Some(Polarity::N), "model from the top level");
        assert!(
            locate(&n, "m.x2.m1").is_none(),
            "subcircuit not in the netlist"
        );
        assert!(locate(&n, "m9").is_none());
        assert_eq!(display_name("m.x2.m1"), "X2.M1");
        assert_eq!(
            model_polarity(".model sw VDMOS(pchan Vto=-2)", "SW"),
            Some(Polarity::P)
        );
        assert_eq!(
            model_polarity(".MODEL q2 PNP(Is=1e-15)", "q2"),
            Some(Polarity::P)
        );
        assert_eq!(model_polarity(".model dd D(Is=1e-14)", "dd"), None);
    }

    fn mos(name: &str, nets: [&str; 4], region: Region, vds: f64, vdsat: f64) -> Device {
        Device {
            id: name.to_ascii_lowercase(),
            name: name.into(),
            kind: Kind::Mosfet,
            polarity: Some(Polarity::N),
            region: Some(region),
            params: raw(&[("vds", vds), ("vdsat", vdsat), ("vgs", 0.9), ("vth", 0.5)]),
            nets: Some(nets.iter().map(|s| s.to_string()).collect()),
            notes: Vec::new(),
        }
    }

    #[test]
    fn checks_name_mirror_outputs_and_hedge_the_rest() {
        let devices = vec![
            mos(
                "M3",
                ["bias", "bias", "0", "0"],
                Region::Saturation,
                0.9,
                0.4,
            ),
            mos("M4", ["out", "bias", "0", "0"], Region::Triode, 0.12, 0.4),
            mos("M5", ["sw", "en", "0", "0"], Region::Triode, 0.01, 0.4),
            mos("M6", ["x", "y", "0", "0"], Region::Cutoff, 1.0, 0.4),
            mos("M7", ["z", "w", "0", "0"], Region::Saturation, 0.43, 0.4),
        ];
        let c = checks(&devices);
        assert_eq!(c.len(), 4, "{c:#?}");
        assert!(c[0].starts_with("M4 shares its gate with diode-connected M3, so it looks like a mirror output, but it is in triode (vds 120mV < vdsat 400mV)"), "{}", c[0]);
        assert!(c[0].contains("about 280mV more vds"), "{}", c[0]);
        assert!(
            c[1].starts_with("M5 is in triode") && c[1].contains("fine for a switch"),
            "{}",
            c[1]
        );
        assert!(
            c[2].starts_with("M6 is off (vgs 900mV < vth 500mV)"),
            "{}",
            c[2]
        );
        assert!(
            c[3].starts_with("M7 is saturated with only 30mV"),
            "{}",
            c[3]
        );
        // A PMOS does not mirror an NMOS.
        let mut p = mos(
            "M9",
            ["out", "bias", "vdd", "vdd"],
            Region::Triode,
            0.1,
            0.4,
        );
        p.polarity = Some(Polarity::P);
        let c = checks(&[devices[0].clone(), p]);
        assert!(c[0].starts_with("M9 is in triode"), "{}", c[0]);
    }

    #[test]
    fn filters_and_order() {
        let mut ds: Vec<Device> = ["m10", "m.x1.m1", "q1", "m2", "d1"]
            .iter()
            .map(|id| device(id, &Raw::new(), None).unwrap())
            .collect();
        sort(&mut ds);
        let names: Vec<&str> = ds.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["M2", "M10", "Q1", "D1", "X1.M1"]);
        let inner = &ds[4];
        assert!(inner.matches("x1.m1") && inner.matches("X1") && inner.matches("m.x1.m1"));
        assert!(!inner.matches("M1") && !inner.matches("X"));
    }
}
