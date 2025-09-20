# 🔄 sync-repos
Multi-purpose Git repository management tool for syncing, auditing, and configuration.

## ✨ Key Features
- **🔍 Auto-Discovery** - Recursively finds all Git repositories
- **⚡ Parallel Processing** - Handles up to 5 repositories concurrently
- **🔐 Security Audit** - Scans for secrets with TruffleHog integration
- **👤 User Config Sync** - Manages git user settings across repositories
- **⏱️ Timeout Protection** - 3-minute limit per repository prevents hanging
- **📊 Live Progress** - Multi-line progress bars with real-time status

## 🚀 Quick Start
```bash
# Installation
chmod +x install.sh && ./install.sh

# Alternative: Manual build with Cargo
cargo build --release
# Copy to a directory in your PATH (installer chooses best location)

# SYNC: Push all repositories (default command)
sync-repos
sync-repos --force  # Push branches without upstream

# AUDIT: Scan for security vulnerabilities
sync-repos audit
sync-repos audit --verify  # Verify if secrets are active
sync-repos audit --json    # Output in JSON format

# USER: Synchronize git configuration
sync-repos user --name "Your Name" --email "you@example.com"
sync-repos user --from-global   # Copy from global git config
sync-repos user --from-current  # Copy from current repo
sync-repos user --dry-run       # Preview changes
```

## 📊 Status Indicators
- **🟢** Synced/Pushed - Repository is up to date
- **🟡** No Upstream - Branch needs upstream tracking
- **🟠** Skipped - No remote or detached HEAD
- **🔴** Failed - Push error occurred

## ⚙️ Configuration
```bash
# Automatically skipped directories:
# node_modules, vendor, target, build, .next, dist
# __pycache__, .venv, venv
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