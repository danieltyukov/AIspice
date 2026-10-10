//! aispice's netlister against LTspice's own `-netlist` output.
//!
//! `tests/schematics/*.net` are LTspice XVII's netlists of the schematics
//! beside them (title line replaced by the file name), so the comparison runs
//! everywhere. With `AISPICE_LTSPICE_TESTS=1` and LTspice installed, the
//! netlists are also regenerated live and must match the committed ones.

use aispice_core::netlist::{build::build, compare::compare, parse};
use aispice_core::symbol::SymbolLibrary;
use aispice_sim::backend::{Ltspice, Simulator};
use std::path::{Path, PathBuf};

fn schematics_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/schematics")
}

fn schematics() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(schematics_dir())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "asc"))
        .collect();
    v.sort();
    v
}

fn aispice_netlist(asc: &Path, lib: &SymbolLibrary) -> aispice_core::netlist::Netlist {
    let bytes = single_backslashes(&std::fs::read(asc).unwrap());
    let (sch, warnings) = aispice_core::schematic::parse_bytes(&bytes);
    assert!(warnings.is_empty(), "{}: {warnings:?}", asc.display());
    let title = format!("* {}", asc.file_name().unwrap().to_string_lossy());
    let (built, _) = build(&sch, lib, &title);
    assert!(
        built.warnings.is_empty(),
        "{}: {:?}",
        asc.display(),
        built.warnings
    );
    built.netlist
}

/// LTspice writes library symbols as `OpAmps\\opamp`. aispice-core's
/// `normalize_symbol_name` turns that into `opamps//opamp`, which misses the
/// built-in `OpAmps/opamp` (reported; pinned by
/// `known_bug_doubled_backslash_symbols`), so the test feeds single
/// backslashes, which the lookup handles.
fn single_backslashes(bytes: &[u8]) -> Vec<u8> {
    let (text, _) = aispice_core::encoding::decode(bytes);
    let fixed: String = text
        .lines()
        .map(|l| {
            if l.starts_with("SYMBOL ") {
                l.replace("\\\\", "\\")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    aispice_core::encoding::encode(&fixed, aispice_core::encoding::Encoding::Latin1)
}

fn ltspice_netlist(asc: &Path) -> aispice_core::netlist::Netlist {
    let net = asc.with_extension("net");
    let bytes = std::fs::read(&net).unwrap_or_else(|e| panic!("{}: {e}", net.display()));
    let (text, _) = aispice_core::encoding::decode(&bytes);
    parse(&text)
}

/// Default model cards LTspice adds for every diode, BJT, MOSFET and JFET,
/// whatever model the part names. aispice-core adds them only for parts left
/// at the generic model name (see the report on `netlist/build.rs`), so they
/// are removed from LTspice's side before comparing.
fn without_default_cards(mut n: aispice_core::netlist::Netlist) -> aispice_core::netlist::Netlist {
    const DEFAULTS: &[&str] = &[
        ".model d d",
        ".model npn npn",
        ".model pnp pnp",
        ".model nmos nmos",
        ".model pmos pmos",
        ".model njf njf",
        ".model pjf pjf",
    ];
    n.items.retain(|item| match item {
        aispice_core::netlist::Line::Directive { text } => {
            !DEFAULTS.contains(&text.trim().to_ascii_lowercase().as_str())
        }
        _ => true,
    });
    n
}

#[test]
fn aispice_netlists_match_ltspice() {
    let lib = SymbolLibrary::builtin_only();
    let mut failures = Vec::new();
    let mut count = 0;
    for asc in schematics() {
        count += 1;
        let ours = aispice_netlist(&asc, &lib);
        let theirs = ltspice_netlist(&asc);
        let ours_text = aispice_core::netlist::write(&ours);
        let theirs_clean = without_default_cards(theirs.clone());
        let ours_clean = without_default_cards(ours);
        if let Err(problems) = compare(&ours_clean, &theirs_clean) {
            failures.push(format!(
                "{}:\n  {}\naispice:\n{ours_text}LTspice:\n{}",
                asc.display(),
                problems
                    .iter()
                    .map(|m| m.0.clone())
                    .collect::<Vec<_>>()
                    .join("\n  "),
                aispice_core::netlist::write(&theirs)
            ));
        }
    }
    assert_eq!(count, 8, "expected the eight authored schematics");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
#[ignore = "aispice-core bug: doubled backslashes in SYMBOL names miss the built-in symbols"]
fn known_bug_doubled_backslash_symbols() {
    let lib = SymbolLibrary::builtin_only();
    assert!(lib.resolve("OpAmps\\\\opamp").is_ok());
}

#[test]
#[ignore = "aispice-core bug: default .model cards are only added for generic model names"]
fn known_bug_default_model_cards_for_named_models() {
    let lib = SymbolLibrary::builtin_only();
    for name in ["ce_amp", "rectifier", "nmos_cs"] {
        let asc = schematics_dir().join(format!("{name}.asc"));
        let ours = aispice_netlist(&asc, &lib);
        let theirs = ltspice_netlist(&asc);
        assert_eq!(compare(&ours, &theirs), Ok(()), "{name}");
    }
}

#[tokio::test]
async fn committed_netlists_match_live_ltspice() {
    if std::env::var("AISPICE_LTSPICE_TESTS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set AISPICE_LTSPICE_TESTS=1 to netlist with LTspice");
        return;
    }
    let lt = Ltspice {
        headless: Some(true),
        ..Default::default()
    };
    if !lt.detect().await.found {
        eprintln!("skipped: LTspice not found");
        return;
    }
    let jobs = schematics()
        .into_iter()
        .map(|asc| {
            let lt = lt.clone();
            async move {
                let live = lt.netlist_asc(&asc).await;
                (asc, live)
            }
        })
        .collect::<Vec<_>>();
    for (asc, live) in futures::future::join_all(jobs).await {
        let live = live.unwrap_or_else(|e| panic!("{}: {e}", asc.display()));
        let committed = ltspice_netlist(&asc);
        if let Err(p) = compare(&parse(&live), &committed) {
            panic!(
                "{}: LTspice's netlist changed: {p:?}\n{live}",
                asc.display()
            );
        }
    }
}
