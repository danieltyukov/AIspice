//! Laying out the netlist corpus in `tests/fixtures/layout/`: every circuit
//! must produce a schematic that netlists back to the input, passes lint
//! without errors, and reads cleanly by the quality metrics.
//!
//! Quality thresholds: a score of 90 or more means no wire through a part
//! body, no overlapping parts, and at most a couple of minor blemishes (a
//! crossing costs 5 points, text touching a wire 6, sprawl a few). Every
//! circuit is held to that except the two-stage CMOS op-amp, the hardest
//! topology in the set (eight MOSFETs, two mirrors, bulk pins drawn to
//! ground), which is held to 60 and to the same zero hard defects.

use aispice_core::layout::{LayoutOptions, LayoutResult, compare_netlists, from_netlist};
use aispice_core::lint::{Severity, lint};
use aispice_core::netlist::{build, parse};
use aispice_core::schematic::write;
use aispice_core::symbol::SymbolLibrary;
use std::path::Path;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/layout")
        .join(format!("{name}.cir"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn lay(name: &str) -> LayoutResult {
    let netlist = parse(&fixture(name));
    from_netlist(
        &netlist,
        &SymbolLibrary::builtin_only(),
        &LayoutOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Everything the corpus promises for one circuit.
fn check(name: &str, min_score: f64) -> LayoutResult {
    let lib = SymbolLibrary::builtin_only();
    let input = parse(&fixture(name));
    let r = lay(name);
    // Round trip, checked again here independently of layout's own check.
    let (built, _) = build(&r.schematic, &lib, "* check");
    if let Err(p) = compare_netlists(&input, &built.netlist) {
        // Nets LTspice would number itself may be renamed when they are
        // joined by labels; layout reports that as a warning.
        assert!(
            r.warnings.iter().any(|w| w.contains("joined by labels")),
            "{name}: {p:?}"
        );
    }
    let errors: Vec<_> = lint(&r.schematic, &lib)
        .findings
        .into_iter()
        .filter(|f| f.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{name}: {errors:#?}");
    let q = &r.quality;
    assert_eq!(q.wires_through_bodies, 0, "{name}: {q:?}");
    assert_eq!(q.overlapping_parts, 0, "{name}: {q:?}");
    assert!(q.score >= min_score, "{name}: {q:?}");
    // The file must read back as the same schematic.
    let text = write(&r.schematic);
    let (again, warnings) = aispice_core::schematic::parse(&text);
    assert!(warnings.is_empty(), "{name}: {warnings:?}");
    assert_eq!(write(&again), text, "{name}");
    r
}

macro_rules! corpus {
    ($($name:ident: $min:expr),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                check(stringify!($name), $min);
            }
        )*
    };
}

corpus! {
    rc_lowpass: 90.0,
    rlc_lowpass: 90.0,
    voltage_divider: 90.0,
    half_wave_rectifier: 90.0,
    full_wave_bridge: 90.0,
    common_emitter: 90.0,
    common_source: 90.0,
    emitter_follower: 90.0,
    diff_pair_mirror: 90.0,
    two_stage_ideal_opamp: 90.0,
    two_stage_cmos_opamp: 60.0,
    inverting_amp: 90.0,
    noninverting_amp: 90.0,
    sallen_key_lowpass: 90.0,
    integrator: 90.0,
    buck_converter: 90.0,
    colpitts_oscillator: 90.0,
    unknown_subckt: 90.0,
    transformer_coupled: 90.0,
}

#[test]
fn unmappable_elements_become_directives_with_warnings() {
    let r = lay("unknown_subckt");
    assert!(
        r.warnings.iter().any(|w| w.contains("XF1")),
        "{:?}",
        r.warnings
    );
    let directives: Vec<String> = r.schematic.directives().flat_map(|t| t.lines()).collect();
    assert!(
        directives.iter().any(|d| d == "XF1 a b myfilter"),
        "{directives:?}"
    );
    assert!(directives.iter().any(|d| d.starts_with(".subckt myfilter")));
    let r = lay("transformer_coupled");
    assert!(
        r.warnings.iter().any(|w| w.contains("K1")),
        "{:?}",
        r.warnings
    );
}

#[test]
fn layout_is_deterministic() {
    let a = write(&lay("common_emitter").schematic);
    let b = write(&lay("common_emitter").schematic);
    assert_eq!(a, b);
}

#[test]
fn snapshots() {
    for name in [
        "rc_lowpass",
        "inverting_amp",
        "common_emitter",
        "buck_converter",
    ] {
        let text = write(&lay(name).schematic).replace("\r\n", "\n");
        insta::assert_snapshot!(format!("layout_{name}"), text);
    }
}
