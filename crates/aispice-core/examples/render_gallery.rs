//! Render the built-in symbol gallery and the renderer's test schematics to
//! SVG and PNG under `target/render-check/`, for checking by eye.
//!
//! ```text
//! cargo run -p aispice-core --example render_gallery
//! cargo run -p aispice-core --example render_gallery -- --ltspice-lib <sym dir> <file.asc>...
//! ```
//!
//! Extra `.asc` files are rendered with the built-in symbols and, when
//! `--ltspice-lib` is given, again with that library, so the two drawings of
//! the same file can be compared side by side.

use aispice_core::render::{
    Highlight, HighlightKind, RenderOptions, render_png, render_svg, symbol_gallery_svg,
};
use aispice_core::schematic::{Schematic, parse_bytes};
use aispice_core::symbol::SymbolLibrary;
use std::error::Error;
use std::path::{Path, PathBuf};

const FIXTURES: &[(&str, &str)] = &[
    (
        "rc_lowpass",
        include_str!("../tests/fixtures/render/rc_lowpass.asc"),
    ),
    (
        "ce_amplifier",
        include_str!("../tests/fixtures/render/ce_amplifier.asc"),
    ),
    (
        "inverting_opamp",
        include_str!("../tests/fixtures/render/inverting_opamp.asc"),
    ),
    (
        "res_orientations",
        include_str!("../tests/fixtures/render/res_orientations.asc"),
    ),
];

fn write(dir: &Path, stem: &str, svg: &str, scale: f32) -> Result<(), Box<dyn Error>> {
    std::fs::write(dir.join(format!("{stem}.svg")), svg)?;
    let png = render_png(svg, scale)?;
    let path = dir.join(format!("{stem}.png"));
    std::fs::write(&path, png)?;
    println!("{}", path.display());
    Ok(())
}

fn render_file(
    dir: &Path,
    stem: &str,
    sch: &Schematic,
    lib: &SymbolLibrary,
) -> Result<(), Box<dyn Error>> {
    let out = render_svg(sch, lib, &RenderOptions::default());
    for w in &out.warnings {
        eprintln!("{stem}: {w}");
    }
    write(dir, stem, &out.svg, 1.5)
}

fn main() -> Result<(), Box<dyn Error>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-check");
    std::fs::create_dir_all(&dir)?;
    let builtin = SymbolLibrary::builtin_only();

    let names: Vec<String> = builtin
        .search("", usize::MAX)
        .into_iter()
        .map(|h| h.name)
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    // A handful of rows per sheet keeps each PNG legible.
    for (i, chunk) in names.chunks(6).enumerate() {
        write(
            &dir,
            &format!("gallery-{i}"),
            &symbol_gallery_svg(&builtin, chunk),
            1.0,
        )?;
    }

    for (stem, src) in FIXTURES {
        let (sch, _) = parse_bytes(src.as_bytes());
        render_file(&dir, stem, &sch, &builtin)?;
    }

    // A themed render with diff highlights, to check custom property
    // fallbacks survive rasterising.
    let (sch, _) = parse_bytes(FIXTURES[1].1.as_bytes());
    let opts = RenderOptions {
        standalone: false,
        show_unconnected_pins: true,
        highlight: vec![
            Highlight {
                inst: "C3".into(),
                kind: HighlightKind::Added,
            },
            Highlight {
                inst: "R3".into(),
                kind: HighlightKind::Changed,
            },
            Highlight {
                inst: "R5".into(),
                kind: HighlightKind::Removed,
            },
        ],
        ..RenderOptions::default()
    };
    write(
        &dir,
        "ce_amplifier-diff",
        &render_svg(&sch, &builtin, &opts).svg,
        1.5,
    )?;

    let mut args = std::env::args().skip(1);
    let mut ltspice: Option<PathBuf> = None;
    let mut files = Vec::new();
    while let Some(a) = args.next() {
        if a == "--ltspice-lib" {
            ltspice = args.next().map(PathBuf::from);
        } else {
            files.push(PathBuf::from(a));
        }
    }
    if !files.is_empty() {
        let extern_dir = dir.join("external");
        std::fs::create_dir_all(&extern_dir)?;
        let ltlib = ltspice
            .as_deref()
            .map(|l| SymbolLibrary::for_project(None, &[], Some(l)));
        for file in files {
            let stem = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "schematic".into());
            let (sch, _) = parse_bytes(&std::fs::read(&file)?);
            render_file(&extern_dir, &format!("{stem}-builtin"), &sch, &builtin)?;
            if let Some(lib) = &ltlib {
                render_file(&extern_dir, &format!("{stem}-ltspice"), &sch, lib)?;
            }
        }
    }
    Ok(())
}
