# 🔄 sync-repos
Multi-purpose Git repository management tool for syncing, auditing, and configuration.

## TL;DR

**3 main commands:**

```bash
# 1. SYNC (default) - Push all git repos
sync-repos

# 2. USER - Set git name/email across all repos
sync-repos user --name "Your Name" --email "user@example.com"

# 3. AUDIT - Scan for secrets/credentials
sync-repos audit
```

**What it does:** Finds all git repositories in current directory, then syncs, configures, or scans them in parallel with live progress bars.

**Status indicators:** 🟢 success, 🟡 no upstream, 🟠 skipped, 🔴 failed

## 🚀 Installation
```bash
chmod +x install.sh && ./install.sh

# Alternative: Manual build with Cargo
cargo build --release
# Copy to a directory in your PATH (installer chooses best location)
```

## 🔄 Main Commands

### 1. **SYNC** (Default Command) - Push All Repositories

```bash
# Basic sync - push all repositories
sync-repos

# Force push branches without upstream tracking
sync-repos --force
```

**What it does:**
- Finds all Git repositories in current directory and subdirectories
- Pushes any unpushed commits to their upstream remotes
- Shows real-time progress with status indicators:
  - 🟢 **Synced/Pushed** - Repository is up to date
  - 🟡 **No Upstream** - Branch needs upstream tracking
  - 🟠 **Skipped** - No remote or detached HEAD
  - 🔴 **Failed** - Push error occurred

### 2. **USER** - Manage Git Configuration

```bash
# Interactive mode - shows available configs and lets you choose
sync-repos user

# Set specific name/email across all repos
sync-repos user --name "Your Name" --email "user@example.com"

# Copy from global git config
sync-repos user --from-global

# Copy from current repository
sync-repos user --from-current

# Preview changes without applying
sync-repos user --name "Your Name" --email "user@example.com" --dry-run

# Force overwrite without prompting
sync-repos user --from-global --force
```

**Interactive Mode (NEW):**
When running `sync-repos user` without arguments, it will:
1. Display your global config (~/.gitconfig)
2. Display your current directory's config
3. Let you choose which to use or enter custom values
4. Show exactly what will be synchronized

### 3. **AUDIT** - Security Scanning

```bash
# Basic security scan for secrets
sync-repos audit

# Auto-install TruffleHog if needed
sync-repos audit --auto-install

# Verify if discovered secrets are still active
sync-repos audit --verify

# Output results in JSON format
sync-repos audit --json

# Combine options
sync-repos audit --auto-install --verify --json
```

**What audit does:**
- Scans all repositories for secrets using TruffleHog
- Detects API keys, passwords, private keys, tokens
- Shows results with:
  - 🟢 **Clean** - No secrets found
  - 🔴 **Secrets** - Secrets detected
  - 🟠 **Failed** - Scan error

## 🎯 Common Usage Examples

```bash
# Daily workflow: sync all repos
sync-repos

# Set up consistent git identity across projects
sync-repos user --name "Jane Developer" --email "jane@company.com"

# Security audit before deployment
sync-repos audit --verify

# Check what user config would change
sync-repos user --from-global --dry-run

# Force sync repos that need upstream setup
sync-repos --force
```

## ⚙️ Automatic Features

- **Auto-discovery**: Recursively finds all Git repositories
- **Parallel processing**: Handles multiple repos concurrently
- **Smart skipping**: Automatically skips `node_modules`, `vendor`, `target`, etc.
- **Timeout protection**: 3-minute limit per repository
- **Live progress**: Multi-line progress bars with real-time status

## 📊 Requirements

- **Git** must be installed and configured
- **Rust 1.56+** (only for manual compilation)
- **TruffleHog** (auto-installed by audit command if needed)

The tool is designed to work in any directory containing Git repositories and will recursively discover and manage them all efficiently.

## 📝 License
MIT