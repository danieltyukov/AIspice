//! `aispice`: the command-line face of aispice, and its MCP server.

mod cmd_files;

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
    }
}
