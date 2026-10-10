//! `aispice`: the command-line face of aispice, and its MCP server.

mod cmd_files;
mod mcp;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "aispice", version, about = "AI agent for SPICE circuit design", long_about = None)]
struct Cli {
    /// Extra folders to search for symbols (.asy), in addition to the
    /// circuit's folder and an LTspice installation.
    #[arg(long, global = true, value_name = "DIR")]
    symbols: Vec<PathBuf>,
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the SPICE netlist of a schematic.
    Netlist {
        /// The .asc file.
        file: PathBuf,
    },
    /// Check a schematic for electrical problems (exit code 1 on errors).
    Lint {
        file: PathBuf,
        /// Also fail on warnings.
        #[arg(long)]
        strict: bool,
    },
    /// Describe a schematic: parts, the net on every pin, directives.
    Show { file: PathBuf },
    /// Draw a schematic to SVG or PNG (chosen by the output extension).
    Render {
        file: PathBuf,
        /// Output file, `.svg` or `.png`.
        #[arg(short, long)]
        output: PathBuf,
        /// Pixels per schematic unit for PNG.
        #[arg(long, default_value_t = 2.0)]
        scale: f32,
    },
    /// Serve aispice's tools over the Model Context Protocol on stdio.
    Mcp {
        /// Project folder the tools may read and write (default: the
        /// current folder).
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("aispice: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let ctx = cmd_files::Ctx {
        symbols: cli.symbols,
        json: cli.json,
    };
    match cli.command {
        Command::Netlist { file } => cmd_files::netlist(&ctx, &file),
        Command::Lint { file, strict } => cmd_files::lint(&ctx, &file, strict),
        Command::Show { file } => cmd_files::show(&ctx, &file),
        Command::Render {
            file,
            output,
            scale,
        } => cmd_files::render(&ctx, &file, &output, scale),
        Command::Mcp { project } => {
            let dir = match project {
                Some(p) => p,
                None => std::env::current_dir()?,
            };
            let options = aispice_tools::ProjectOptions {
                extra_symbol_dirs: ctx.symbols.clone(),
                ltspice_lib: cmd_files::ltspice_lib(),
            };
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(mcp::serve(&dir, options))?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
