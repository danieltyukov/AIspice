# aispice revamp: design

Date: 2026-10-10. Status: accepted for implementation (the owner asked for the work to run to completion; changes of direction are welcome at any point).

## Why

aispice started as a Tauri chat window that asked an LLM for JSON edit operations and wrote them into an LTspice `.asc` file. By late 2026 that is a baseline: LTspice 26.1 ships its own MCP server, and any coding agent with a small open-source MCP wrapper can edit a schematic and run a simulation. Research (market survey, 2026-10-10) shows what actually moves results:

1. Simulator-in-the-loop verification against a written electrical spec, not just "the netlist parses".
2. Native tool calling with typed tools.
3. Hybrid sizing: the model chooses variables and ranges, a numerical optimizer does the search.
4. Robustness checks (Monte Carlo, corners) where language models alone collapse.
5. Verified topology templates, which lift weaker and local models the most.

The open-source field is small (the closest project has ~60 stars) and nobody covers legible schematic generation, open spec-driven sizing with robustness, or a board-level analog benchmark. That is the gap aispice fills.

## Goals

- One engine, three faces: a desktop app, a CLI, and an MCP server, all calling the same tools.
- Works with no LTspice installed (ngspice), and better with it (exact LTspice netlisting and simulation, live reload).
- Every edit is reviewable: semantic diff, undo/redo, snapshots per agent turn.
- Every claim is checkable: specs are measured from simulation and shown as pass/fail.
- Any model: Anthropic, OpenAI, Google, OpenRouter, Ollama and any OpenAI-compatible endpoint, with model lists fetched live rather than hard-coded.
- Open-source hygiene matching the owner's other projects: MIT, contributor docs, CI on three OSes, CodeQL, protected default branch, a project site on GitHub Pages.

## Non-goals (for this revamp)

- PCB layout or KiCad round-tripping beyond netlist export.
- Shipping LTspice's symbol library or any vendor model files.
- Hosting anything. aispice runs on the user's machine; the only network calls are to the model provider the user configured.

## Architecture

```
crates/aispice-core   pure computation over files
  schematic/          lossless .asc model, parse, write (encoding and line endings preserved)
  symbol/             .asy parser, built-in open symbol set, LTspice library discovery
  netlist/            SPICE netlist model, parser, writer; .asc -> netlist connectivity
  edit/               typed, pin-aware edit operations with validation
  layout/             netlist -> readable .asc placement for generated circuits
  lint/               electrical rule checks
  render/             .asc -> SVG
  summary/            model-friendly description of a schematic
  units, geometry, encoding

crates/aispice-sim    processes and numbers
  raw/                .raw reader (LTspice, ngspice, Xyce, Spectre nutascii), stepped and complex data
  backend/            Simulator trait; ngspice, Xyce, LTspice (native, Wine, macOS), Spectre over SSH
  dialect/            netlist translation per simulator (ngspice LTspice-compat mode, Xyce, Spectre)
  measure/            measurement functions and spec evaluation
  plot/               waveform -> SVG
  sweep, montecarlo, optimize (Nelder-Mead, CMA-ES; log-scaled parameters)

crates/aispice-agent  the model side
  provider/           Anthropic Messages API; OpenAI-compatible Chat Completions (OpenAI, Google, OpenRouter, Ollama, LM Studio, vLLM, ...)
  tools/              one registry of typed tools (JSON Schema via schemars), shared by the agent loop and MCP
  agent               streaming tool-calling loop with step limit and cancellation
  project             root confinement, history/undo store, run cache (.aispice/)
  config, keys        TOML config; API keys in the OS keychain with a 0600 file fallback
  sessions, prompt, templates (verified topology library)

crates/aispice-cli    binary `aispice`
  sim, netlist, render, lint, measure, plot, check (CI), chat, mcp, eval, doctor, keys, models

app/                  Tauri 2 desktop shell (thin commands over the crates) + React UI
site/                 project website (GitHub Pages), shares app/src/tokens.css
examples/             example circuits used by docs and tests
evals/                design task suite for `aispice eval`
```

The Tauri app no longer contains logic; it calls the same `aispice-agent` tools the CLI and MCP server use, so all three faces behave identically and are tested once.

## Tools

The same registry backs the in-app agent and the MCP server.

| Tool | Purpose |
|---|---|
| `list_circuits` | Circuits and results in the project |
| `read_schematic` | Components with values and the net on every pin, nets with members, directives, lint findings |
| `render_schematic` | SVG (and PNG for multimodal clients) |
| `edit_schematic` | Typed ops: add, remove, move, rotate, set value/attribute, connect pins (auto-routed), label net, add/remove directive. Returns a semantic diff and lint results; recorded for undo |
| `create_schematic` | New `.asc` from a SPICE netlist with automatic readable placement |
| `undo` / `history` | Revert to any snapshot |
| `netlist` | `.asc` -> SPICE netlist (LTspice's own netlister when available, else aispice's) |
| `lint` | Electrical rule checks |
| `simulate` | Run on ngspice, Xyce, LTspice or Spectre; returns vectors, operating point, `.meas` results, warnings |
| `measure` | Named measurements on a run: value_at, min, max, pp, avg, rms, rise/fall time, overshoot, settling, crossing, gain_at, bandwidth, unity-gain frequency, phase/gain margin |
| `check_specs` | Evaluate a spec table (measure + limits) and report pass/fail with margins |
| `read_waveform` / `plot` | Downsampled data and stats; SVG/PNG plot |
| `poles_zeros` | Pole-zero analysis of a transfer function |
| `sweep` | Parameter sweep across any simulator |
| `monte_carlo` | Tolerance analysis with yield against specs |
| `optimize` | Size parameters within bounds to meet specs |
| `templates` | Verified reference topologies to start from |
| `symbols` | Search available symbols and models |

Edits are applied to the file immediately and LTspice is reloaded, with one-click undo; a setting switches to "ask before applying".

## Security

- API keys in the OS keychain (Secret Service, Keychain, Credential Manager); the old plaintext config is migrated and removed.
- All tool paths are resolved against the project root and canonicalized; anything escaping it is refused.
- No shell strings: every process is spawned with an argument vector. The Spectre SSH host and command are user configuration, never model output.
- Simulator runs have timeouts, are killed on cancel, and output size is capped.
- Strict Tauri CSP and minimal capabilities.
- File contents are data: a schematic comment cannot widen what the agent may do, because every tool is confined to the project.

## UX (desktop)

Three panes: circuit list; centre tabs for Schematic (live SVG, pan/zoom), Waveforms (uPlot, cursors), Netlist, Log; chat on the right showing each tool call as a compact card (what ran, how long, key numbers, diff with undo). Status bar: simulator, LTspice link, model. Light and dark themes from one token file shared with the site.

## Testing

- Unit and golden tests per module; netlists compared against LTspice's own `-netlist` output for every example.
- Simulation integration tests on ngspice (CI) and Xyce/LTspice (local, opt-in), checked against analytic values.
- Agent loop tested with a scripted provider and with an HTTP mock of the Anthropic and OpenAI streaming APIs, including tool calls.
- MCP server tested end to end over stdio with a real client.
- Desktop app tested end to end with tauri-driver/WebKitWebDriver against the real binary under Xvfb, plus Playwright on the UI with mocked IPC.
- Live model smoke test with a real provider key when present.
- `aispice eval` runs a design task suite and reports pass rate per model.
- Site checked with Playwright screenshots and Lighthouse.

## Repository and project hygiene

MIT licence (replacing the previous all-rights-reserved notice). README, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG, issue and PR templates, CODEOWNERS, Dependabot config. Workflows: CI (fmt, clippy, tests on Linux/macOS/Windows; frontend typecheck, lint, tests; e2e), CodeQL, Pages, Release (desktop bundles and CLI binaries on tag). GitHub: repository renamed to `aispice`, default-branch ruleset (no deletion or force push, linear history, PRs required, status checks), secret scanning and push protection, private vulnerability reporting, topics and homepage.

## Brand

New mark: a step response (rise, overshoot, settle) traced from a probe dot, white on a teal tile, replacing the old monogram that borrowed from LTspice's logo and carried a registered-trademark sign. Wordmark "aispice" in lower case. Instrument Sans and JetBrains Mono, subset and self-hosted under the OFL.

## Phases

1. Foundations: workspace, core model, OSS files, CI skeleton, security fixes (done or in progress).
2. Core engine: symbols, netlister, lint, render, edit, layout, summary.
3. Simulation: raw reader, backends, measure, specs, sweep, Monte Carlo, optimizer, plots.
4. Agent: providers, tools, loop, sessions, CLI, MCP server.
5. Desktop app rebuilt on the crates.
6. Brand, site, docs.
7. End-to-end testing across every layer, evals, live runs; Spectre once the department server is back (after 2026-10-12 12:00, needs eduVPN).
8. GitHub: rename, protection, security settings, Pages, release.
