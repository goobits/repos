#!/bin/bash

# sync-repos installer script

set -e

echo "📦 Building sync-repos..."

# Check if cargo is installed
if ! command -v cargo &> /dev/null; then
    echo "❌ Cargo not found."
    echo ""
    read -p "Would you like to install Rust now? (y/n) " -n 1 -r
    echo ""
    
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        echo "📥 Installing Rust..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        
        # Source cargo environment for current session
        source "$HOME/.cargo/env"
        
        echo "✅ Rust installed successfully!"
    else
        echo "Please install Rust manually:"
        echo "   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        exit 1
    fi
fi

# Build the release binary
cargo build --release

# Determine installation directory
if [ -w "/usr/local/bin" ]; then
    INSTALL_DIR="/usr/local/bin"
elif [ -d "$HOME/.local/bin" ]; then
    INSTALL_DIR="$HOME/.local/bin"
elif [ -d "$HOME/bin" ]; then
    INSTALL_DIR="$HOME/bin"
else
    INSTALL_DIR="$HOME/.local/bin"
    mkdir -p "$INSTALL_DIR"
fi

# Install the binary
echo "📁 Installing to $INSTALL_DIR..."
cp target/release/sync-repos "$INSTALL_DIR/"
chmod +x "$INSTALL_DIR/sync-repos"

# Check if directory is in PATH
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    echo ""
    echo "⚠️  $INSTALL_DIR is not in your PATH"
    echo "   Add this to your shell config file:"
    echo "   export PATH=\"\$PATH:$INSTALL_DIR\""
fi

echo "✅ Installation complete!"
echo "   Run 'sync-repos' in any directory to sync all git repos"