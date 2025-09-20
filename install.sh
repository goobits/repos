#!/bin/bash
#
# sync-repos installer script
# Installs the sync-repos tool for managing multiple git repositories
#

set -e  # Exit on any error

# Get the directory where this script is located
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# Change to script directory to ensure we're in the right place
cd "$SCRIPT_DIR"

echo "📦 Building sync-repos..."

# Function to add cargo to PATH in shell configuration files
add_cargo_to_path() {
    local shell_config=""

    # Detect which shell configuration file to use
    if [ -n "$ZSH_VERSION" ]; then
        shell_config="$HOME/.zshrc"
    elif [ -n "$BASH_VERSION" ]; then
        if [ -f "$HOME/.bashrc" ]; then
            shell_config="$HOME/.bashrc"
        elif [ -f "$HOME/.bash_profile" ]; then
            shell_config="$HOME/.bash_profile"
        fi
    fi

    # Add cargo environment to shell config if not already present
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
    # Try sourcing cargo environment first (in case Rust is installed but not in PATH)
    if [ -f "$HOME/.cargo/env" ]; then
        source "$HOME/.cargo/env"
    fi

    # Check again after attempting to source cargo environment
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

# Determine the best installation directory
# Priority: /usr/local/bin (system-wide), ~/.local/bin, ~/bin (user-specific)
if [ -w "/usr/local/bin" ]; then
    INSTALL_DIR="/usr/local/bin"
elif [ -d "$HOME/.local/bin" ]; then
    INSTALL_DIR="$HOME/.local/bin"
elif [ -d "$HOME/bin" ]; then
    INSTALL_DIR="$HOME/bin"
else
    # Create ~/.local/bin if it doesn't exist
    INSTALL_DIR="$HOME/.local/bin"
    mkdir -p "$INSTALL_DIR"
fi

# Install the binary
echo "📁 Installing to $INSTALL_DIR..."
cp "$SCRIPT_DIR/target/release/sync-repos" "$INSTALL_DIR/"
chmod +x "$INSTALL_DIR/sync-repos"

# Function to create environment file for PATH management
create_sync_repos_env() {
    local env_file="$HOME/.sync-repos-env"

    cat > "$env_file" << 'EOF'
#!/bin/sh
# sync-repos shell setup
# Check if sync-repos bin directory is already in PATH to avoid duplicates
case ":${PATH}:" in
    *:"INSTALL_DIR_PLACEHOLDER":*)
        ;;
    *)
        export PATH="INSTALL_DIR_PLACEHOLDER:$PATH"
        ;;
esac
EOF

    # Replace placeholder with actual install directory
    sed -i.bak "s|INSTALL_DIR_PLACEHOLDER|$INSTALL_DIR|g" "$env_file"
    rm -f "$env_file.bak"

    echo "📝 Created environment file: $env_file"
}

# Function to safely add sourcing line to shell config
add_to_shell_config() {
    local config_file="$1"
    local source_line=". \"\$HOME/.sync-repos-env\""

    if [ -f "$config_file" ]; then
        # Check if already present
        if ! grep -q "sync-repos-env" "$config_file"; then
            echo "" >> "$config_file"
            echo "# Added by sync-repos installer" >> "$config_file"
            echo "$source_line" >> "$config_file"
            echo "📝 Added to $config_file"
        fi
    fi
}

# Check if installation directory is in PATH and set up environment if not
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    echo ""
    echo "🔧 Setting up PATH configuration..."

    # Create the environment file
    create_sync_repos_env

    # Add to shell configuration files
    add_to_shell_config "$HOME/.bashrc"
    add_to_shell_config "$HOME/.zshrc"

    echo "✅ PATH configuration complete!"
    echo "   Restart your shell or run: source ~/.sync-repos-env"
else
    echo "✅ $INSTALL_DIR is already in PATH"
fi

echo "✅ Installation complete!"
echo "   Run 'sync-repos' in any directory to sync all git repositories"