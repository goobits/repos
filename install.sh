#!/bin/bash

# sync-repos installer script

set -e

echo "📦 Building sync-repos..."

# Function to add cargo to PATH
add_cargo_to_path() {
    local shell_config=""
    
    if [ -n "$ZSH_VERSION" ]; then
        shell_config="$HOME/.zshrc"
    elif [ -n "$BASH_VERSION" ]; then
        if [ -f "$HOME/.bashrc" ]; then
            shell_config="$HOME/.bashrc"
        elif [ -f "$HOME/.bash_profile" ]; then
            shell_config="$HOME/.bash_profile"
        fi
    fi
    
    if [ -n "$shell_config" ] && [ -f "$shell_config" ]; then
        if ! grep -q '.cargo/env' "$shell_config"; then
            echo "" >> "$shell_config"
            echo "# Added by sync-repos installer" >> "$shell_config"
            echo "source \"\$HOME/.cargo/env\"" >> "$shell_config"
            echo "📝 Added cargo to PATH in $shell_config"
        fi
    fi
}

# Check if cargo is installed or source it if available
if ! command -v cargo &> /dev/null; then
    # Try sourcing cargo env first
    if [ -f "$HOME/.cargo/env" ]; then
        source "$HOME/.cargo/env"
    fi
    
    # Check again after sourcing
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
            
            # Add to shell config for future sessions
            add_cargo_to_path
            
            echo "✅ Rust installed successfully!"
        else
            echo "Please install Rust manually:"
            echo "   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
            exit 1
        fi
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