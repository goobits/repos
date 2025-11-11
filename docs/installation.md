# Installation Guide

## Prerequisites

- **Rust toolchain** (1.70+): `rustc` and `cargo`
- **Git** (2.0+)
- **Platform**: Linux or macOS (Windows via WSL)

## Installation Methods

### Method 1: Cargo Install (Recommended)

Install directly from crates.io. Fastest and simplest method.

```bash
cargo install goobits-repos
```

This will install the `repos` binary to `~/.cargo/bin/` (make sure it's in your PATH).

### Method 2: Install Script

Recommended for most users. Handles Rust installation, binary compilation, and PATH setup automatically. **Choose this unless you have specific requirements.**

```bash
git clone https://github.com/goobits/repos.git
cd repos
./install.sh
```

After running the script, you'll have:
- Optimized release binary built and installed
- Binary in `/usr/local/bin`, `~/.local/bin`, or `~/bin` (first writable location)
- Installation directory added to your PATH
- Rust toolchain installed if it was missing

### Method 3: Make

Familiar workflow for users who prefer make commands. Equivalent to Method 1 but uses make.

```bash
git clone https://github.com/goobits/repos.git
cd repos
make install
```

### Method 4: Cargo Direct Install

For users who want full control over installation paths and prefer manual setup.

```bash
git clone https://github.com/goobits/repos.git
cd repos
cargo build --release
mkdir -p ~/.local/bin
cp target/release/repos ~/.local/bin/
```

Requires manual PATH configuration (see below).

### Method 5: From Source (Development)

For contributors working on the codebase. Creates unoptimized debug build for faster compilation during development.

```bash
git clone https://github.com/goobits/repos.git
cd repos
cargo build
./target/debug/repos --help
```

## Verify Installation

```bash
repos --version      # Should show: repos 2.1.0
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
