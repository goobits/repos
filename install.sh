#!/usr/bin/env bash
#
# repos installer script
# Installs the repos tool for managing multiple git repositories
#

set -euo pipefail

# Get the directory where this script is located
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# Change to script directory to ensure we're in the right place
cd "$SCRIPT_DIR"

# Function to add cargo to PATH in shell configuration files
add_cargo_to_path() {
    local shell_config=""

    # Detect which shell configuration file to use
    if [ -n "${ZSH_VERSION:-}" ]; then
        shell_config="$HOME/.zshrc"
    elif [ -n "${BASH_VERSION:-}" ]; then
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
            echo "# Added by repos installer" >> "$shell_config"
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

# Keep build artifacts outside the source checkout. An explicit Cargo target
# directory still wins, which makes the installer friendly to CI and wrappers.
if [ -n "${CARGO_TARGET_DIR:-}" ]; then
    BUILD_DIR="$CARGO_TARGET_DIR"
else
    TEMP_ROOT="${TMPDIR:-${TMP:-${TEMP:-/tmp}}}"
    if [ ! -d "$TEMP_ROOT" ] || [ ! -w "$TEMP_ROOT" ]; then
        TEMP_ROOT="${XDG_CACHE_HOME:-$HOME/.cache}"
    fi
    BUILD_DIR="${TEMP_ROOT%/}/goobits-repos-target"
fi

HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
if [ -z "$HOST_TARGET" ]; then
    echo "❌ Could not determine the native Rust target." >&2
    exit 1
fi

case "$HOST_TARGET" in
    *-windows-*) BINARY_NAME="repos.exe" ;;
    *) BINARY_NAME="repos" ;;
esac

mkdir -p "$BUILD_DIR"
export CARGO_TARGET_DIR="$BUILD_DIR"

echo "📦 Building repos in $CARGO_TARGET_DIR..."
cargo build --locked --release --bin repos --target "$HOST_TARGET"

SOURCE_BINARY="$CARGO_TARGET_DIR/$HOST_TARGET/release/$BINARY_NAME"
if [ ! -f "$SOURCE_BINARY" ]; then
    echo "❌ Build completed but the repos binary was not found: $SOURCE_BINARY" >&2
    exit 1
fi

if ! "$SOURCE_BINARY" --version >/dev/null; then
    echo "❌ Built binary could not be executed: $SOURCE_BINARY" >&2
    exit 1
fi

# Determine the best installation directory
# Priority: /usr/local/bin (system-wide), ~/.local/bin, ~/bin (user-specific)
if [ -n "${REPOS_INSTALL_DIR:-}" ]; then
    INSTALL_DIR="$REPOS_INSTALL_DIR"
    mkdir -p "$INSTALL_DIR"
elif [ -w "/usr/local/bin" ]; then
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

# Install through a fresh inode, then atomically replace the destination.
# Overwriting an executable in place can leave macOS with a stale cached code
# signature and cause the next launch to be terminated with SIGKILL.
echo "📁 Installing to $INSTALL_DIR..."
INSTALL_PATH="$INSTALL_DIR/$BINARY_NAME"
STAGED_BINARY="$(mktemp "$INSTALL_DIR/.${BINARY_NAME}.install.XXXXXX")"

cleanup_staged_binary() {
    if [ -n "${STAGED_BINARY:-}" ] && [ -f "$STAGED_BINARY" ]; then
        rm -f "$STAGED_BINARY"
    fi
}
trap cleanup_staged_binary EXIT

cp "$SOURCE_BINARY" "$STAGED_BINARY"
chmod +x "$STAGED_BINARY"

if ! "$STAGED_BINARY" --version >/dev/null; then
    echo "❌ Staged binary could not be executed; existing installation was preserved: $INSTALL_PATH" >&2
    exit 1
fi

mv -f "$STAGED_BINARY" "$INSTALL_PATH"
STAGED_BINARY=""
trap - EXIT

# Function to create environment file for PATH management
create_repos_env() {
    local env_file="$HOME/.repos-env"

    cat > "$env_file" << 'EOF'
#!/bin/sh
# repos shell setup
# Check if repos bin directory is already in PATH to avoid duplicates
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
    local source_line=". \"\$HOME/.repos-env\""

    if [ -f "$config_file" ]; then
        # Check if already present
        if ! grep -q "repos-env" "$config_file"; then
            echo "" >> "$config_file"
            echo "# Added by repos installer" >> "$config_file"
            echo "$source_line" >> "$config_file"
            echo "📝 Added to $config_file"
        fi
    fi
}

# Check if installation directory is in PATH and set up environment if not
if [ "${REPOS_SKIP_PATH_SETUP:-0}" = "1" ]; then
    echo "ℹ️  Skipped PATH configuration (REPOS_SKIP_PATH_SETUP=1)"
elif [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    echo ""
    echo "🔧 Setting up PATH configuration..."

    # Create the environment file
    create_repos_env

    # Add to shell configuration files
    add_to_shell_config "$HOME/.bashrc"
    add_to_shell_config "$HOME/.zshrc"

    echo "✅ PATH configuration complete!"
    echo "   Restart your shell or run: source ~/.repos-env"
else
    echo "✅ $INSTALL_DIR is already in PATH"
fi

echo "✅ Installation complete!"
echo "   Installed: $INSTALL_PATH"
echo "   Run 'repos' in any directory to manage your git repositories"
