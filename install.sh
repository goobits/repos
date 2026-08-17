#!/usr/bin/env bash
#
# repos installer script
# Installs the repos tool for managing multiple git repositories
#

set -euo pipefail

MINIMUM_RUST_VERSION="1.78"

# Get the directory where this script is located
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# Change to script directory to ensure we're in the right place
cd "$SCRIPT_DIR"

version_is_at_least() {
    local actual_version="$1"
    local required_version="$2"
    local actual_major actual_minor required_major required_minor

    IFS=. read -r actual_major actual_minor _ <<< "$actual_version"
    IFS=. read -r required_major required_minor _ <<< "$required_version"
    if ! [[ "$actual_major" =~ ^[0-9]+$ && "$actual_minor" =~ ^[0-9]+$ ]]; then
        return 1
    fi

    (( actual_major > required_major ||
        (actual_major == required_major && actual_minor >= required_minor) ))
}

user_shell_config() {
    case "${SHELL:-}" in
        zsh | */zsh)
            printf '%s\n' "$HOME/.zshrc"
            ;;
        bash | */bash)
            if [ -f "$HOME/.bashrc" ]; then
                printf '%s\n' "$HOME/.bashrc"
            elif [ -f "$HOME/.bash_profile" ]; then
                printf '%s\n' "$HOME/.bash_profile"
            elif [ "$(uname -s)" = "Darwin" ]; then
                printf '%s\n' "$HOME/.bash_profile"
            else
                printf '%s\n' "$HOME/.bashrc"
            fi
            ;;
        *)
            return 1
            ;;
    esac
}

# Function to add cargo to PATH in shell configuration files
add_cargo_to_path() {
    local shell_config

    # Add cargo environment to shell config if not already present
    if ! shell_config="$(user_shell_config)"; then
        echo "ℹ️  Add $HOME/.cargo/bin to PATH in your shell configuration."
        return
    fi
    touch "$shell_config"
    if ! grep -Fq '.cargo/env' "$shell_config"; then
        printf '\n# Added by repos installer\n. "$HOME/.cargo/env"\n' >> "$shell_config"
        echo "📝 Added cargo to PATH in $shell_config"
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

CARGO_VERSION="$(cargo --version | awk '{print $2}')"
RUSTC_VERSION="$(rustc --version | awk '{print $2}')"
if ! version_is_at_least "$CARGO_VERSION" "$MINIMUM_RUST_VERSION" ||
    ! version_is_at_least "$RUSTC_VERSION" "$MINIMUM_RUST_VERSION"; then
    echo "❌ repos requires Cargo and Rust $MINIMUM_RUST_VERSION or newer." >&2
    echo "   Found: cargo $CARGO_VERSION, rustc $RUSTC_VERSION" >&2
    if command -v rustup &> /dev/null; then
        echo "   Update with: rustup update stable" >&2
    fi
    exit 1
fi

# Keep build artifacts outside the source checkout. An explicit Cargo target
# directory still wins, which makes the installer friendly to CI and wrappers.
if [ -n "${CARGO_TARGET_DIR:-}" ]; then
    BUILD_DIR="$CARGO_TARGET_DIR"
else
    INSTALLER_CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/goobits-repos"
    if [ -L "$INSTALLER_CACHE_DIR" ]; then
        echo "❌ Refusing symlinked installer cache: $INSTALLER_CACHE_DIR" >&2
        exit 1
    fi
    mkdir -p "$INSTALLER_CACHE_DIR"
    chmod 0700 "$INSTALLER_CACHE_DIR"
    BUILD_DIR="$INSTALLER_CACHE_DIR/target"
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
BUILD_DIR="$(cd "$BUILD_DIR" && pwd -P)"
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
elif [ -d "$HOME/.local/bin" ] && [ -w "$HOME/.local/bin" ]; then
    INSTALL_DIR="$HOME/.local/bin"
elif [ -d "$HOME/bin" ] && [ -w "$HOME/bin" ]; then
    INSTALL_DIR="$HOME/bin"
else
    # Create ~/.local/bin if it doesn't exist
    INSTALL_DIR="$HOME/.local/bin"
    mkdir -p "$INSTALL_DIR"
fi

if [ ! -d "$INSTALL_DIR" ] || [ ! -w "$INSTALL_DIR" ]; then
    echo "❌ Installation directory is not writable: $INSTALL_DIR" >&2
    exit 1
fi
INSTALL_DIR="$(cd "$INSTALL_DIR" && pwd -P)"
case "$INSTALL_DIR" in
    *$'\n'* | *$'\r'*)
        echo "❌ Installation directory cannot contain newlines." >&2
        exit 1
        ;;
esac

# Install through a fresh inode, then atomically replace the destination.
# Overwriting an executable in place can leave macOS with a stale cached code
# signature and cause the next launch to be terminated with SIGKILL.
echo "📁 Installing to $INSTALL_DIR..."
INSTALL_PATH="$INSTALL_DIR/$BINARY_NAME"
STAGED_BINARY="$(mktemp "$INSTALL_DIR/.${BINARY_NAME}.install.XXXXXX")"
BACKUP_BINARY=""

cleanup_install_artifacts() {
    if [ -n "${STAGED_BINARY:-}" ] && [ -f "$STAGED_BINARY" ]; then
        rm -f "$STAGED_BINARY"
    fi
    if [ -n "${BACKUP_BINARY:-}" ] && [ -f "$BACKUP_BINARY" ]; then
        rm -f "$BACKUP_BINARY"
    fi
}
trap cleanup_install_artifacts EXIT

cp "$SOURCE_BINARY" "$STAGED_BINARY"
chmod 0755 "$STAGED_BINARY"

if ! "$STAGED_BINARY" --version >/dev/null; then
    echo "❌ Staged binary could not be executed; existing installation was preserved: $INSTALL_PATH" >&2
    exit 1
fi

if [ -e "$INSTALL_PATH" ]; then
    BACKUP_BINARY="$(mktemp "$INSTALL_DIR/.${BINARY_NAME}.backup.XXXXXX")"
    cp -p "$INSTALL_PATH" "$BACKUP_BINARY"
fi

mv -f "$STAGED_BINARY" "$INSTALL_PATH"
STAGED_BINARY=""

if ! "$INSTALL_PATH" --version >/dev/null; then
    if [ -n "$BACKUP_BINARY" ]; then
        mv -f "$BACKUP_BINARY" "$INSTALL_PATH"
        BACKUP_BINARY=""
        echo "❌ Installed binary could not be executed; previous installation was restored: $INSTALL_PATH" >&2
    else
        rm -f "$INSTALL_PATH"
        echo "❌ Installed binary could not be executed: $INSTALL_PATH" >&2
    fi
    exit 1
fi

if [ -n "$BACKUP_BINARY" ]; then
    rm -f "$BACKUP_BINARY"
    BACKUP_BINARY=""
fi
trap - EXIT

quote_for_posix_shell() {
    local value="$1"

    printf "'"
    printf '%s' "$value" | sed "s/'/'\\\\''/g"
    printf "'"
}

# Function to create environment file for PATH management
create_repos_env() {
    local env_file="$HOME/.repos-env"
    local quoted_install_dir

    quoted_install_dir="$(quote_for_posix_shell "$INSTALL_DIR")"

    cat > "$env_file" << EOF
#!/bin/sh
# repos shell setup
# Check if repos bin directory is already in PATH to avoid duplicates
_repos_install_dir=$quoted_install_dir
case ":\${PATH}:" in
    *:"\${_repos_install_dir}":*)
        ;;
    *)
        export PATH="\${_repos_install_dir}:\${PATH}"
        ;;
esac
unset _repos_install_dir
EOF
    chmod 0644 "$env_file"

    echo "📝 Created environment file: $env_file"
}

# Function to safely add sourcing line to shell config
add_to_shell_config() {
    local config_file="$1"
    local source_line=". \"\$HOME/.repos-env\""

    if [ -e "$config_file" ] && [ ! -f "$config_file" ]; then
        echo "❌ Shell configuration is not a regular file: $config_file" >&2
        return 1
    fi
    touch "$config_file"
    if ! grep -Fq "$source_line" "$config_file"; then
        printf '\n# Added by repos installer\n%s\n' "$source_line" >> "$config_file"
        echo "📝 Added to $config_file"
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

    if shell_config="$(user_shell_config)"; then
        add_to_shell_config "$shell_config"
        echo "✅ PATH configuration complete!"
        echo "   Restart your shell or run: source ~/.repos-env"
    else
        echo "ℹ️  Automatic PATH setup is unavailable for ${SHELL:-this shell}."
        echo "   Add this directory to PATH using your shell configuration: $INSTALL_DIR"
    fi
else
    echo "✅ $INSTALL_DIR is already in PATH"
fi

echo "✅ Installation complete!"
echo "   Installed: $INSTALL_PATH"
echo "   Run 'repos' in any directory to manage your git repositories"
