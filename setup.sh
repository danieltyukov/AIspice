#!/bin/bash
set -e

echo "=== AIspice Setup ==="

# Detect OS
if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    echo "Detected Linux"

    # Check for apt
    if command -v apt-get &>/dev/null; then
        echo "Installing system dependencies..."
        sudo apt-get update -qq
        sudo apt-get install -y \
            libsoup-3.0-dev \
            libwebkit2gtk-4.1-dev \
            libjavascriptcoregtk-4.1-dev \
            libayatana-appindicator3-dev \
            librsvg2-dev \
            xdotool \
            wmctrl \
            wine \
            curl \
            build-essential \
            pkg-config \
            libssl-dev
    elif command -v dnf &>/dev/null; then
        echo "Installing system dependencies (Fedora)..."
        sudo dnf install -y \
            webkit2gtk4.1-devel \
            libsoup3-devel \
            libappindicator-gtk3-devel \
            librsvg2-devel \
            xdotool \
            wmctrl \
            wine \
            openssl-devel \
            gcc \
            pkg-config
    elif command -v pacman &>/dev/null; then
        echo "Installing system dependencies (Arch)..."
        sudo pacman -S --noconfirm \
            webkit2gtk-4.1 \
            libsoup3 \
            libappindicator-gtk3 \
            librsvg \
            xdotool \
            wmctrl \
            wine \
            openssl \
            base-devel \
            pkg-config
    else
        echo "Warning: Unknown Linux package manager. Install these manually:"
        echo "  libwebkit2gtk-4.1-dev libsoup-3.0-dev xdotool wmctrl wine"
    fi

elif [[ "$OSTYPE" == "darwin"* ]]; then
    echo "Detected macOS"
    if command -v brew &>/dev/null; then
        echo "Installing system dependencies..."
        brew install --cask wine-stable 2>/dev/null || true
    else
        echo "Install Homebrew first: https://brew.sh"
    fi

elif [[ "$OSTYPE" == "msys"* || "$OSTYPE" == "cygwin"* ]]; then
    echo "Detected Windows — no system packages needed for Tauri"
fi

# Install Rust if not present
if ! command -v cargo &>/dev/null; then
    echo "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi

# Install Node.js deps
echo "Installing Node.js dependencies..."
cd "$(dirname "$0")"
npm install

# Verify Rust compiles
echo "Checking Rust build..."
cd src-tauri && cargo check && cd ..

echo ""
echo "=== Setup complete ==="
echo "Run ./start to launch AIspice"
