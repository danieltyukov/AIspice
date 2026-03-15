<p align="center">
  <img src="src/assets/icon.svg" width="120" alt="AIspice logo" />
</p>

<h1 align="center">AIspice</h1>

<p align="center">
  AI-powered LTspice circuit assistant
</p>

<p align="center">
  <img src="src/assets/logo.svg" width="280" alt="AIspice wordmark" />
</p>

---

An agentic AI chat that sits alongside LTspice and controls it. Edit schematics, run simulations, and analyze circuits — all through natural language.

## Features

- **Agentic LTspice Control** — Edit `.asc` schematics, trigger reloads, and run simulations through chat
- **Multi-Provider AI** — OpenAI, Anthropic (Claude), Google (Gemini), OpenRouter, and local Ollama
- **LTspice Integration** — Auto-detects LTspice (native, Wine, or macOS), reloads files after edits, runs batch simulations
- **Multi-File Support** — Browse and switch between `.asc` files in your project
- **Simulation Logs** — Read and analyze LTspice simulation output directly in chat
- **Persistent Settings** — API keys saved securely to `~/.config/aispice/config.json`
- **Cross-Platform** — Windows, macOS, Linux (including Wine-based LTspice)
- **Dark Theme** — Toggle light/dark mode

## How It Works

1. Open a folder containing LTspice `.asc` files
2. Have LTspice open alongside AIspice
3. Ask the AI to modify circuits, explain behavior, or run simulations
4. AIspice edits the `.asc` file and triggers LTspice to reload

## Setup

```bash
# Install dependencies
npm install

# Run in development
npm run tauri dev

# Build for production
npm run tauri build
```

### System Requirements

**Linux:**
```bash
sudo apt install libsoup-3.0-dev libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev
```

**LTspice** should be installed separately:
- **Linux:** Via Wine (`wine LTspice64.exe`)
- **macOS:** Download from [Analog Devices](https://www.analog.com/en/resources/design-tools-and-calculators/ltspice-simulator.html)
- **Windows:** Same download link above

### API Keys

Open Settings (gear icon) inside the app to configure API keys for:

| Provider | Models |
|----------|--------|
| OpenAI | GPT-4o, GPT-4o-mini |
| Anthropic | Claude Sonnet 4, Claude Haiku 4 |
| Google | Gemini 2.5 Flash, Gemini 2.5 Pro |
| OpenRouter | All models via single key |
| Ollama | Local models (no key needed) |

## Tech Stack

- **Frontend:** React 19, TypeScript, Vite
- **Backend:** Rust, Tauri 2
- **AI:** OpenAI / Anthropic / Google / OpenRouter APIs + Ollama
- **LTspice:** Native, Wine, or macOS integration
