# 🔄 Goobits Sync Repos
Automatically discovers and synchronizes all Git repositories in the current directory tree.

## ✨ Key Features
- **🔍 Auto-Discovery** - Recursively finds all Git repositories
- **⚡ Parallel Sync** - Processes up to 5 repositories concurrently
- **⏱️ Timeout Protection** - 3-minute limit per repository prevents hanging
- **📊 Live Progress** - Multi-line progress bars with real-time status

## 🚀 Quick Start
```bash
# Installation
chmod +x install.sh && ./install.sh

# Alternative: Manual build with Cargo
cargo build --release
# Copy to a directory in your PATH (installer chooses best location)

# Basic usage - run in any directory
sync-repos

# Force push branches without upstream tracking
sync-repos --force

# The tool will:
# 1. Scan for all Git repositories recursively
# 2. Check each for unpushed commits
# 3. Push pending changes to upstream remotes
# 4. Display summary with color-coded status
```

## 📊 Status Indicators
- **🟢** Synced/Pushed - Repository is up to date
- **🟡** No Upstream - Branch needs upstream tracking
- **🟠** Skipped - No remote or detached HEAD
- **🔴** Failed - Push error occurred

## ⚙️ Configuration
```bash
# Skipped directories (hardcoded):
# node_modules, vendor, target, build, .next, dist
# __pycache__, .venv, venv

# Command-line options
sync-repos --force  # Auto-push branches without upstream
```

## 🛠️ Requirements
```bash
# Runtime dependency
git --version  # Git must be installed and configured

# Build dependency (manual compilation only)
rustc --version  # Rust 1.56+ (2021 edition)
```

## 📝 License
MIT