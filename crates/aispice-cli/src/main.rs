//! `aispice`: the command-line face of aispice, and its MCP server.

mod cmd_agent;
mod cmd_files;
mod cmd_sim;
mod mcp;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "aispice", version, about = "AI agent for SPICE circuit design: edit, simulate, measure and size LTspice circuits", long_about = None)]
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
    /// Chat with the agent about the circuits in a folder (one prompt, or
    /// interactive when none is given).
    Chat {
        prompt: Option<String>,
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
        /// anthropic, openai, google, openrouter, ollama or a configured one.
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },
    /// Simulate a circuit and print the results.
    Sim {
        file: PathBuf,
        /// ngspice, ltspice, xyce or spectre (default: automatic).
        #[arg(long)]
        simulator: Option<String>,
        /// Run this analysis instead of the circuit's, e.g. ".ac dec 50 1 1Meg".
        #[arg(long)]
        analysis: Option<String>,
        /// A measurement, e.g. "f3db = bandwidth_3db(V(out))". Repeatable.
        #[arg(short, long = "measure")]
        measure: Vec<String>,
    },
    /// Check a circuit against its specs (the .specs file beside it, or
    /// --specs). Exit code 1 when a spec fails, for CI.
    Check {
        file: PathBuf,
        #[arg(long)]
        specs: Option<PathBuf>,
        #[arg(long)]
        simulator: Option<String>,
    },
    /// Print the SPICE netlist of a schematic.
    Netlist { file: PathBuf },
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
    /// Check simulators, LTspice's library and model provider keys.
    Doctor,
    /// Manage model provider API keys (kept in the OS keychain).
    Keys {
        #[command(subcommand)]
        action: KeysAction,
    },
    /// List the models a provider offers.
    Models { provider: Option<String> },
    /// Generate documentation.
    Docs {
        #[command(subcommand)]
        what: DocsWhat,
    },
}

#[derive(Subcommand)]
enum KeysAction {
    /// Store a key (read from stdin).
    Set { provider: String },
    /// Delete a stored key.
    Rm { provider: String },
    /// Show which providers have a key and where it comes from.
    List,
}

#[derive(Subcommand)]
enum DocsWhat {
    /// Markdown reference of every tool.
    Tools,
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
        symbols: cli.symbols.clone(),
        json: cli.json,
    };
    match cli.command {
        Command::Chat {
            prompt,
            project,
            provider,
            model,
        } => cmd_agent::chat(cmd_agent::ChatArgs {
            prompt,
            project,
            provider,
            model,
            symbols: cli.symbols,
        }),
        Command::Sim {
            file,
            simulator,
            analysis,
            measure,
        } => cmd_sim::sim(&file, &cli.symbols, simulator, analysis, measure, cli.json),
        Command::Check {
            file,
            specs,
            simulator,
        } => cmd_sim::check(&file, &cli.symbols, specs, simulator, cli.json),
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
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(mcp::serve(&dir, &cli.symbols))?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Doctor => cmd_agent::doctor(cli.json),
        Command::Keys { action } => match action {
            KeysAction::Set { provider } => cmd_agent::keys_set(&provider),
            KeysAction::Rm { provider } => cmd_agent::keys_remove(&provider),
            KeysAction::List => cmd_agent::keys_list(),
        },
        Command::Models { provider } => cmd_agent::models(provider),
        Command::Docs {
            what: DocsWhat::Tools,
        } => cmd_agent::docs_tools(),
    }
}
