# Setup

This page covers installing aispice, connecting simulators and model providers, the config file, and using aispice from an MCP client. Run `aispice doctor` at any point to see what aispice found.

## Install

**Desktop app.** Download the installer for your platform from the [releases page](https://github.com/danieltyukov/aispice/releases): `aispice_amd64.deb` or `aispice_x86_64.AppImage` on Linux, `aispice_x64-setup.exe` or `aispice_x64.msi` on Windows, `aispice_universal.dmg` on macOS. The desktop app includes the same agent and tools as the command line.

**Command line.** Prebuilt archives are on the same page (`aispice-cli-x86_64-linux.tar.gz`, `aispice-cli-x86_64-windows.zip`, `aispice-cli-universal-macos.tar.gz`). To build from source instead, with Rust 1.88 or newer:

```sh
cargo install --locked --git https://github.com/danieltyukov/aispice aispice
```

**From a clone**, for development:

```sh
git clone https://github.com/danieltyukov/aispice
cd aispice
cargo build -p aispice          # the CLI, at target/debug/aispice
npm ci                          # the desktop interface and the site
npm run desktop                 # the desktop app, with hot reload
```

The desktop app on Linux needs the WebKitGTK development packages. On Debian and Ubuntu:

```sh
sudo apt-get install libwebkit2gtk-4.1-dev libxdo-dev libssl-dev \
  libayatana-appindicator3-dev librsvg2-dev
```

## Simulators

aispice needs at least one simulator. With `simulator = "auto"` (the default) it prefers ngspice, then LTspice, then Xyce. Every tool and command also takes an explicit simulator.

### ngspice

The recommended default: free, fast, and on every platform.

| Platform | Install |
|---|---|
| Debian, Ubuntu | `sudo apt-get install ngspice` |
| Fedora | `sudo dnf install ngspice` |
| macOS | `brew install ngspice` |
| Windows | Download from [ngspice.sourceforge.io](https://ngspice.sourceforge.io/download.html), unpack, and put the folder with `ngspice_con.exe` on `PATH` |

aispice runs ngspice with `ngbehavior=ltpsa`, so LTspice-style expressions, behavioural sources and model cards work. Each run happens in a private temporary folder.

### LTspice

aispice looks in the standard install locations:

| Platform | Locations checked |
|---|---|
| Windows | `%LOCALAPPDATA%\Programs\ADI\LTspice\LTspice.exe`, `C:\Program Files\ADI\LTspice\LTspice.exe`, `C:\Program Files\LTC\LTspiceXVII\XVIIx64.exe` |
| macOS | `/Applications/LTspice.app` |
| Linux | LTspice installed under Wine, in `$WINEPREFIX` or `~/.wine` |

For anything else, set `ltspice_path` in the config file. On Linux, aispice runs LTspice through `wine`, and through `xvfb-run` when there is no display, so batch simulation works on a headless server.

When LTspice is installed, aispice also reads its symbol library, so schematics that use LTspice's own parts (op-amps, regulators, comparators) netlist correctly. Point `AISPICE_LTSPICE_LIB` at a `lib/sym` folder to use a different library.

After an edit, the desktop app opens the schematic in LTspice again, which brings LTspice forward with the saved version, so you can keep LTspice open next to aispice. Turn this off in Settings.

### Xyce

Install Xyce from [xyce.sandia.gov](https://xyce.sandia.gov/) and make sure `Xyce` is on `PATH`. aispice translates LTspice-specific syntax where Xyce needs it.

### Spectre

Cadence Spectre usually lives on a licensed server, so aispice runs it over SSH: it copies the netlist to the server with `scp`, runs Spectre with `ssh`, and copies the results back. Add this to the config file:

```toml
[spectre]
ssh_host = "user@cadence.example.edu"
remote_dir = "/home/user/aispice-runs"
# Optional: a command that sets up the Cadence environment first.
setup_command = "source /opt/cadence/setup.sh"
```

SSH runs with `BatchMode=yes`, so the host must accept your SSH key without a password prompt. Check with `ssh user@cadence.example.edu true`. aispice creates a private folder under `remote_dir` for every run.

These settings come only from your config file. A model or a netlist can never change the host or the command. Keep process design kits and anything under an NDA out of your project folders if you share them.

## Model providers

aispice works with Anthropic, OpenAI, Google Gemini, OpenRouter, Ollama, and any OpenAI-compatible endpoint (LM Studio, vLLM, a company gateway).

Store a key once:

```sh
aispice keys set anthropic      # reads the key from stdin
aispice keys list               # which providers have a key, and from where
aispice keys rm anthropic
```

Keys go into the OS keychain (Secret Service on Linux, Keychain on macOS, Credential Manager on Windows). Where no keychain is available, as on a headless Linux server, they go into `keys.json` in the config folder, readable only by you. Environment variables win over both, which suits CI and one-off runs:

| Provider | Variable |
|---|---|
| `anthropic` | `ANTHROPIC_API_KEY` |
| `openai` | `OPENAI_API_KEY` |
| `google` | `GOOGLE_API_KEY` or `GEMINI_API_KEY` |
| `openrouter` | `OPENROUTER_API_KEY` |

Ollama needs no key and is reached at `http://localhost:11434/v1`. To list the models a provider offers: `aispice models google`.

The desktop app manages the same keys in Settings.

## Config file

| Platform | Path |
|---|---|
| Linux | `~/.config/aispice/config.toml` |
| macOS | `~/Library/Application Support/aispice/config.toml` |
| Windows | `%APPDATA%\aispice\config.toml` |

Every field is optional:

```toml
provider = "anthropic"          # default provider
model = "claude-opus-5-5"       # default model for that provider
simulator = "auto"              # auto, ngspice, ltspice, xyce or spectre
edit_mode = "apply"             # apply (with undo) or ask (approve each edit)
# ltspice_path = "/home/me/.wine/drive_c/Program Files/ADI/LTspice/LTspice.exe"

[agent]
max_steps = 40                  # tool calls per request
thinking = true

# A local OpenAI-compatible server under its own name.
[providers.lmstudio]
base_url = "http://localhost:1234/v1"
```

With `provider = "lmstudio"` (or `--provider lmstudio`) aispice talks to that endpoint.

## Using aispice from an MCP client

`aispice mcp` serves every tool over the Model Context Protocol on stdio. It works in the folder you give it and refuses paths outside it.

Claude Code:

```sh
claude mcp add aispice -- aispice mcp --project /path/to/circuits
```

Cursor, Claude Desktop, and other clients that take a JSON server list:

```json
{
  "mcpServers": {
    "aispice": {
      "command": "aispice",
      "args": ["mcp", "--project", "/path/to/circuits"]
    }
  }
}
```

## What aispice writes in a project

| Path | Contents |
|---|---|
| `<circuit>.specs` | The spec table for a circuit, one spec per line. Commit it with the circuit. |
| `.aispice/history/` | A snapshot before every edit, for undo |
| `.aispice/runs/` | Recent simulation output |

Add `.aispice/` to `.gitignore` unless you want history in version control. Chat sessions from the desktop app are kept in your user data folder, not in the project.

A specs file looks like this:

```text
# rc.specs
fc   = bandwidth_3db(V(out)) in 950..1050 Hz
gain = gain_db_at(V(out)/V(in), 10) >= -0.5
```

## Troubleshooting

Start with `aispice doctor`. It lists every simulator it found with its version, the LTspice library, which providers have a key, and the config file in use.

**"No simulator found."** Install ngspice (see above) and check that `ngspice` runs in a terminal. On Windows, make sure the folder with `ngspice_con.exe` is on `PATH`, then open a new terminal.

**LTspice under Wine is not found.** aispice looks in `$WINEPREFIX`, then `~/.wine`. If LTspice is in another prefix, set `WINEPREFIX` or `ltspice_path`. Run LTspice once by hand so Wine finishes creating its prefix.

**LTspice hangs or fails on a headless machine.** Install `xvfb` (`sudo apt-get install xvfb`). aispice uses `xvfb-run` when neither `DISPLAY` nor `WAYLAND_DISPLAY` is set.

**A part is reported as an unknown symbol.** The schematic uses a symbol from LTspice's library or a custom `.asy` file. Install LTspice, set `AISPICE_LTSPICE_LIB`, or pass the folder with `--symbols <dir>`. Symbols next to the schematic are always found.

**Keys are not saved on a Linux server.** Without a Secret Service (GNOME Keyring or KWallet), keys go to `keys.json` in the config folder. You can also use the environment variables above.

**HTTP 429 from the provider.** Free tiers allow only a few requests per minute. aispice retries with backoff; for long agent runs, use a paid tier or a local model through Ollama.

**The agent cannot reach a local model.** Check the server is running and that `base_url` ends in `/v1` for OpenAI-compatible servers. Pick a model that supports tool calling.

**Spectre runs fail.** Run `ssh <host> true` in a terminal; it must succeed without a prompt. Check that `spectre` is on the remote `PATH` after your `setup_command`.

**The desktop app shows a blank window on Linux.** Some GPU drivers break WebKitGTK's DMA-BUF renderer. Start the app with `WEBKIT_DISABLE_DMABUF_RENDERER=1`.

If none of this helps, [open an issue](https://github.com/danieltyukov/aispice/issues/new/choose) with the output of `aispice doctor`.
