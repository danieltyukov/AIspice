//! Compare aispice's netlister with LTspice's on a folder of schematics.
//!
//! For every `x.asc` that has an `x.net` next to it (made by
//! `LTspice -netlist x.asc`), build aispice's netlist and compare. Used to
//! check the netlister against real-world files locally; LTspice output is
//! never committed.
//!
//!     cargo run -p aispice-core --example compare_ltspice -- <folder> [ltspice lib/sym]

use aispice_core::netlist::{build, compare, parse};
use aispice_core::schematic::parse_bytes;
use aispice_core::symbol::SymbolLibrary;
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("folder"));
    let ltlib = args.next().map(PathBuf::from);
    let (mut ok, mut bad, mut skipped) = (0, 0, 0);
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for asc in entries
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "asc"))
    {
        let net = asc.with_extension("net");
        let Ok(net_bytes) = std::fs::read(&net) else {
            skipped += 1;
            continue;
        };
        let (ltspice_text, _) = aispice_core::encoding::decode(&net_bytes);
        let ours_sch = parse_bytes(&std::fs::read(asc).unwrap()).0;
        let lib = SymbolLibrary::for_project(Some(&dir), &[], ltlib.as_deref());
        let (built, _) = build(&ours_sch, &lib, "* compare");
        let theirs = parse(&ltspice_text);
        let ours = parse(&aispice_core::netlist::write(&built.netlist));
        match compare(&ours, &theirs) {
            Ok(()) => ok += 1,
            Err(problems) => {
                bad += 1;
                println!("== {}", asc.file_name().unwrap().to_string_lossy());
                for p in problems.iter().take(6) {
                    println!("   {}", p.0);
                }
            }
        }
    }
    println!("\nmatched {ok}, differed {bad}, no LTspice netlist {skipped}");
}
