//! Commands that work on a schematic file alone.

use aispice_core::lint::Severity;
use aispice_core::schematic;
use aispice_core::symbol::SymbolLibrary;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub struct Ctx {
    pub symbols: Vec<PathBuf>,
    pub json: bool,
}

pub fn load(ctx: &Ctx, file: &Path) -> Result<(schematic::Schematic, SymbolLibrary)> {
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let (sch, warnings) = schematic::parse_bytes(&bytes);
    for w in warnings {
        eprintln!("{}:{}: {}", file.display(), w.line, w.message);
    }
    let dir = file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let lib = SymbolLibrary::for_project(Some(dir), &ctx.symbols, ltspice_lib().as_deref());
    Ok((sch, lib))
}

/// LTspice's symbol library, if an installation is found.
pub fn ltspice_lib() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("AISPICE_LTSPICE_LIB") {
        return Some(PathBuf::from(p));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    let candidates = [
        home.join("Documents/LTspiceXVII/lib/sym"),
        home.join(".wine/drive_c/Program Files/LTC/LTspiceXVII/lib/sym"),
        home.join("AppData/Local/LTspice/lib/sym"),
        home.join("Library/Application Support/LTspice/lib/sym"),
        PathBuf::from("C:/Program Files/LTC/LTspiceXVII/lib/sym"),
    ];
    candidates.into_iter().find(|p| p.is_dir())
}

pub fn netlist(ctx: &Ctx, file: &Path) -> Result<ExitCode> {
    let (sch, lib) = load(ctx, file)?;
    let title = format!("* {}", file.display());
    let (built, _) = aispice_core::netlist::build(&sch, &lib, &title);
    for w in &built.warnings {
        eprintln!("warning: {w}");
    }
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&built)?);
    } else {
        print!("{}", aispice_core::netlist::write(&built.netlist));
    }
    Ok(ExitCode::SUCCESS)
}

pub fn lint(ctx: &Ctx, file: &Path, strict: bool) -> Result<ExitCode> {
    let (sch, lib) = load(ctx, file)?;
    let report = aispice_core::lint::lint(&sch, &lib);
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&report.findings)?);
    } else if report.findings.is_empty() {
        println!("{}: no problems found", file.display());
    } else {
        for f in &report.findings {
            let at = f.at.map(|p| format!(" at {p}")).unwrap_or_default();
            println!(
                "{}: {:?} [{}] {}{at}",
                file.display(),
                f.severity,
                f.rule,
                f.message
            );
        }
    }
    let failing = report
        .findings
        .iter()
        .any(|f| f.severity == Severity::Error || (strict && f.severity == Severity::Warning));
    Ok(if failing {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

pub fn show(ctx: &Ctx, file: &Path) -> Result<ExitCode> {
    let (sch, lib) = load(ctx, file)?;
    let summary = aispice_core::summary::summarize(&sch, &lib);
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        print!("{}", summary.to_text());
    }
    Ok(ExitCode::SUCCESS)
}

pub fn render(ctx: &Ctx, file: &Path, output: &Path, scale: f32) -> Result<ExitCode> {
    let (sch, lib) = load(ctx, file)?;
    let out = aispice_core::render::render_svg(
        &sch,
        &lib,
        &aispice_core::render::RenderOptions::default(),
    );
    for w in &out.warnings {
        eprintln!("warning: {w}");
    }
    let is_png = output
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    if is_png {
        let png = aispice_core::render::render_png(&out.svg, scale)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        std::fs::write(output, png).with_context(|| format!("writing {}", output.display()))?;
    } else {
        std::fs::write(output, out.svg).with_context(|| format!("writing {}", output.display()))?;
    }
    if !ctx.json {
        println!("wrote {}", output.display());
    }
    Ok(ExitCode::SUCCESS)
}
