# 🔄 sync-repos
Git repository management tool for batch synchronization, configuration, and security auditing.

## ✨ Key Features
- **🔄 Batch Sync** - Push commits across all repositories in directory tree
- **⚙️ Config Management** - Synchronize git user.name/email across projects
- **🔒 Security Audit** - Scan for exposed secrets and credentials
- **⚡ Concurrent Processing** - Parallel operations with live progress tracking
- **🎯 Auto Discovery** - Recursive repository detection with smart filtering
- **🛠️ Zero Configuration** - Works immediately in any directory structure

## 🚀 Quick Start
```bash
# Installation
chmod +x install.sh && ./install.sh

# Alternative: Build from source
cargo build --release

# Basic usage - sync all repos in current directory
sync-repos

# Configure git identity across all repos
sync-repos user --name "Your Name" --email "user@example.com"

# Security scan for exposed secrets
sync-repos audit
```

## 🔄 Sync Command
```bash
# Default behavior - push all unpushed commits
sync-repos

# Force upstream tracking for new branches
sync-repos --force
sync-repos sync --force

# Status indicators:
# 🟢 synced/pushed  🟡 no upstream  🟠 skipped  🔴 failed
```

## ⚙️ Configuration Management
```bash
# Interactive mode - choose from available configs
sync-repos user

# Set specific values
sync-repos user --name "Jane Dev" --email "jane@company.com"

# Copy from global config
sync-repos user --from-global

# Copy from current repository
sync-repos user --from-current

# Preview changes without applying
sync-repos user --from-global --dry-run

# Force overwrite without prompting
sync-repos user --from-global --force
```

## 🔒 Security Auditing
```bash
# Basic secret scan (scan only, no fixes)
sync-repos audit

# Install TruffleHog if missing
sync-repos audit --install-tools

# Verify if secrets are still active
sync-repos audit --verify

# Machine-readable output
sync-repos audit --json

# Fix issues interactively (prompts for each fix)
sync-repos audit --interactive

# Apply all fixes automatically
sync-repos audit --fix-all

# Apply specific fixes only
sync-repos audit --fix-gitignore    # Add to .gitignore
sync-repos audit --fix-large        # Remove large files from history
sync-repos audit --fix-secrets      # Remove secrets from history

# Preview fixes without applying
sync-repos audit --fix-all --dry-run

# Fix specific repositories only
sync-repos audit --fix-all --repos "repo1,repo2"

# Combined options for CI/CD
sync-repos audit --install-tools --verify --json
```

## 🛠️ Advanced Features
```bash
# Batch operations on discovered repositories
# - Automatic timeout protection (3min per repo)
# - Intelligent directory filtering (skips node_modules, target, vendor)
# - Parallel processing with controlled concurrency
# - Real-time progress bars with repository status

# Repository discovery scope
# ✅ Included: .git directories in current tree
# ❌ Excluded: node_modules/, vendor/, target/, build/, dist/

# Error handling
# - Network timeouts handled gracefully
# - Authentication errors reported clearly
# - Merge conflicts detected and reported
# - Partial failures don't block other repositories
```


## 🧪 Development
```bash
# Build and test
cargo build
cargo test

# Check for dependency updates (built-in)
cargo update --dry-run

# View dependency tree
cargo tree

# Lint and format
cargo clippy
cargo fmt
```

## 📝 License
MIT - see [LICENSE](LICENSE) for details

## 💡 Support
- Report issues via GitHub Issues
- Contributions welcome via Pull Requests