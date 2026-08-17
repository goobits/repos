# Installation Guide

## Prerequisites

- **Rust toolchain** (1.78+): `rustc` and `cargo`
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

The build cache stays in a private user cache outside the checkout. Automation
can override locations:

```bash
CARGO_TARGET_DIR=/path/to/cache \
REPOS_INSTALL_DIR=/path/to/bin \
REPOS_SKIP_PATH_SETUP=1 \
./install.sh
```

### Method 3: Make

Familiar workflow for users who prefer make commands. It delegates to the install script.

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
cargo install --path . --locked --force
```

This installs to Cargo's binary directory, normally `~/.cargo/bin`.

### Method 5: From Source (Development)

For contributors working on the codebase. Creates unoptimized debug build for faster compilation during development.

```bash
git clone https://github.com/goobits/repos.git
cd repos
cargo run -- --help
```

Repository builds use an external Cargo target directory, so scripts and docs
must not assume a checkout-local `target/` directory.

## Verify Installation

```bash
repos --version      # Should show the installed repos version
command -v repos     # Shows the active installed binary
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
Updates are staged and executed before atomically replacing the installed
binary. This preserves the previous installation if validation fails and avoids
macOS terminating an in-place executable replacement because of cached code
signature state.

## Uninstalling

For Cargo installations:

```bash
cargo uninstall goobits-repos
```

For install-script installations, remove the path reported by
`command -v repos`. Remove the `repos-env` line from shell RC files if desired.

## Troubleshooting

**"command not found"**
- Check PATH: `echo $PATH | grep -o "$HOME/.local/bin"`
- Locate the active binary: `command -v repos`
- Reload shell: `source ~/.bashrc` or restart terminal

**Build errors**
- Update Rust: `rustup update`
- Check versions: `rustc --version && cargo --version` (need 1.78+)
- Clean rebuild: `cargo clean && cargo build --release`

**Permission denied**
- Inspect the active binary: `ls -l "$(command -v repos)"`
- Use `sudo` only if installing to `/usr/local/bin`

**`Killed: 9` immediately after an update on macOS**
- Pull the latest installer and rerun `./install.sh`; current releases replace
  the executable atomically instead of overwriting a previously run binary
  in place.
- If an older installer already damaged the destination, rerunning the current
  installer repairs it without requiring manual code signing.

**Missing dependencies on Linux**
- Install build tools: `sudo apt install build-essential pkg-config libssl-dev`
- For RHEL/Fedora: `sudo dnf install gcc openssl-devel`
