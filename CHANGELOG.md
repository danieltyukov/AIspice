# Changelog

All notable changes to aispice are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.2.0]

A rewrite. aispice is now one Rust engine with three front ends: a desktop app, a command line tool and an MCP server.

### Added

- `aispice` command line tool: `chat`, `sim`, `check`, `netlist`, `lint`, `show`, `render`, `draw`, `mcp`, `doctor`, `keys`, `models` and `docs`.
- MCP server (`aispice mcp`) exposing every tool to Claude Code, Cursor, Codex, Claude Desktop and other clients.
- Lossless `.asc` and `.asy` reading and writing in every encoding LTspice uses.
- A netlister that follows LTspice's own, checked against LTspice's `-netlist` output.
- Typed schematic edits with automatic placement and wire routing, a semantic diff after each edit, and undo through snapshots.
- Netlist to schematic: `create_schematic` and `aispice draw` lay a SPICE netlist out as a readable LTspice schematic (signal flow left to right, ground down, rails up, feedback above its stage) and refuse any drawing that does not netlist back to the input.
- `poles_zeros` tool: pole-zero analysis of a transfer function on ngspice's `.pz`, with f0 and Q for each complex pair, the corner of each real root, and a stability verdict.
- Simulation on ngspice, Xyce, LTspice (native or under Wine, headless with `xvfb-run`) and Cadence Spectre over SSH.
- Measurements (gain, bandwidth, phase and gain margin, overshoot, settling, rise and fall time, RMS and more), spec tables with margins, parameter sweeps, Monte Carlo with yield, and an optimizer.
- `aispice check` for testing circuits against their specs in CI.
- Providers: Anthropic, OpenAI, Google Gemini, OpenRouter, Ollama and any OpenAI-compatible endpoint.
- Lint rules, SVG and PNG rendering, and plots.
- Desktop app rebuilt: schematic view, waveforms with cursors, netlist, log, specs and history tabs, and a chat that shows each tool call as a card with its diff and an Undo button. Optional approval before each edit.
- New name style (`aispice`), logo, website and documentation.

### Changed

- The desktop app's backend now calls the shared engine instead of its own copy of the format, AI and simulation code.
- API keys move from the plain config file to the OS keychain. An existing `config.json` with keys is migrated on first use and the keys are removed from it.
- Licensed under MIT.

### Security

- Every tool is confined to the project folder, including through symbolic links.
- Netlists and everything they include are checked against an allowlist before any simulator runs them.
- Every process is started with an argument vector; there are no shell strings.
- In ask mode, every write (edits, spec files, optimizer results, undo) waits for approval.
- Strict content security policy and minimal capabilities in the desktop app; SVG output is sanitised before display.
- Chat sessions are kept in the user's data folder, never in a project, so a cloned repository cannot plant a conversation.

## [0.1.0]

The first version: a Tauri desktop chat beside LTspice that edited `.asc` files and triggered LTspice to reload.

[Unreleased]: https://github.com/danieltyukov/aispice/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/danieltyukov/aispice/releases/tag/v0.2.0
[0.1.0]: https://github.com/danieltyukov/aispice/tree/808c0de
