# Installation Guide

## Prerequisites

- **Rust toolchain** (1.70+): `rustc` and `cargo`
- **Git** (2.0+)
- **Platform**: Linux or macOS (Windows via WSL)

## Installation Methods

### Method 1: Install Script (Recommended)

```bash
git clone https://github.com/goobits/repos.git
cd repos
./install.sh
```

The install script:
- Builds optimized release binary with `cargo build --release`
- Installs to `~/.local/bin/repos` (or `/usr/local/bin` if writable)
- Adds installation directory to PATH automatically
- Optionally installs Rust toolchain if missing

### Method 2: Make

```bash
git clone https://github.com/goobits/repos.git
cd repos
make install
```

Builds release binary and runs the install script.

### Method 3: Cargo Direct Install

```bash
git clone https://github.com/goobits/repos.git
cd repos
cargo build --release
mkdir -p ~/.local/bin
cp target/release/repos ~/.local/bin/
```

Manual installation - requires PATH configuration (see below).

### Method 4: From Source (Development)

```bash
git clone https://github.com/goobits/repos.git
cd repos
cargo build
./target/debug/repos --help
```

Debug builds compile faster but run slower. Use for development only.

## Verify Installation

```bash
repos --version      # Should show: repos 1.4.0
which repos          # Should show: /home/user/.local/bin/repos
repos --help         # Display command help
```

## PATH Setup

Ensure `~/.local/bin` is in your PATH:

**Bash** (`~/.bashrc`):
```bash
export PATH="$HOME/.local/bin:$PATH"
```

**Zsh** (`~/.zshrc`):
```bash
export PATH="$HOME/.local/bin:$PATH"
```

**Fish** (`~/.config/fish/config.fish`):
```fish
set -gx PATH $HOME/.local/bin $PATH
```

Apply changes:
```bash
source ~/.bashrc    # or ~/.zshrc, or restart shell
```

## Updating

```bash
cd repos
git pull
./install.sh
```

The install script rebuilds and reinstalls automatically.

## Uninstalling

```bash
rm ~/.local/bin/repos
```

Remove PATH configuration from shell RC files if desired.

## Troubleshooting

**"command not found"**
- Check PATH: `echo $PATH | grep -o "$HOME/.local/bin"`
- Verify binary exists: `ls -l ~/.local/bin/repos`
- Reload shell: `source ~/.bashrc` or restart terminal

**Build errors**
- Update Rust: `rustup update`
- Check version: `rustc --version` (need 1.70+)
- Clean rebuild: `cargo clean && cargo build --release`

**Permission denied**
- Make binary executable: `chmod +x ~/.local/bin/repos`
- Check ownership: `ls -l ~/.local/bin/repos`
- Use `sudo` only if installing to `/usr/local/bin`

**Missing dependencies on Linux**
- Install build tools: `sudo apt install build-essential pkg-config libssl-dev`
- For RHEL/Fedora: `sudo dnf install gcc openssl-devel`
