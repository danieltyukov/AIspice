# AIspice

AI-powered circuit schematic editor. Open `.asc`, `.cir`, `.spice`, or `.net` files, view and edit circuits visually on an SVG canvas, and use natural language AI to modify your designs.

## Features

- **SVG Schematic Editor** — Visual drag-and-drop circuit editing with pan/zoom
- **AI Chat Assistant** — Describe circuit changes in plain English
- **Multi-Format Support** — `.asc` (LTspice), `.cir`, `.spice`, `.net`
- **Component Library** — Searchable sidebar (R, C, L, V, I, D, BJT, OpAmp)
- **Undo/Redo** — Full command history for all edits
- **Waveform Viewer** — Plot simulation output from ngspice
- **Cross-Platform** — Windows, macOS, Linux via Tauri
- **Multiple AI Providers** — OpenRouter (cloud) + Ollama (local)
- **Dark Theme** — Toggle light/dark mode
- **Export** — SVG export of schematics

## Setup

```bash
# Install dependencies
npm install

# Set API key (optional, for AI features)
echo "OPENROUTER_API_KEY=your_key_here" > .env

# Run in development
npm run tauri dev

# Build for production
npm run tauri build
```

### System Requirements (Linux)

```bash
sudo apt install libsoup-3.0-dev libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev
```

### Optional: ngspice for simulation

```bash
# Ubuntu/Debian
sudo apt install ngspice

# macOS
brew install ngspice

# Windows
# Download from https://ngspice.sourceforge.io/
```

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| Ctrl/Cmd + Z | Undo |
| Ctrl/Cmd + Shift + Z | Redo |
| R | Rotate selected component |
| Delete | Delete selected component |
| Escape | Clear selection |
| Ctrl/Cmd + S | Save |
| Shift + Drag | Pan canvas |
| Scroll | Zoom in/out |

## Tech Stack

- **Frontend:** React 19, TypeScript, Vite, SVG
- **Backend:** Rust, Tauri 2
- **AI:** OpenRouter API, Ollama (local)
- **Simulation:** ngspice (optional)
