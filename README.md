# 🔄 repos
Git repository management tool for batch synchronization, configuration, and security auditing.

## ✨ Key Features
- **🔄 Batch Push** - Push commits across all repositories in directory tree
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

# Show help and available commands
repos

# Push all repos in current directory
repos push

# Configure git identity across all repos
repos user --name "Your Name" --email "user@example.com"

# Security scan for exposed secrets
repos audit
```

## 🔄 Push Command
```bash
# Push all unpushed commits to remotes
repos push

# Force upstream tracking for new branches
repos push --force

# Status indicators:
# 🟢 synced/pushed  🟡 no upstream  🟠 skipped  🔴 failed
```

## ⚙️ Configuration Management
```bash
# Interactive mode - choose from available configs
repos user

# Set specific values
repos user --name "Jane Dev" --email "jane@company.com"

# Copy from global config
repos user --from-global

# Copy from current repository
repos user --from-current

# Preview changes without applying
repos user --from-global --dry-run

# Force overwrite without prompting
repos user --from-global --force
```

## 🔒 Security Auditing
```bash
# Basic secret scan (scan only, no fixes)
repos audit

# Install TruffleHog if missing
repos audit --install-tools

# Verify if secrets are still active
repos audit --verify

# Machine-readable output
repos audit --json

# Fix issues interactively (prompts for each fix)
repos audit --interactive

# Apply all fixes automatically
repos audit --fix-all

# Apply specific fixes only
repos audit --fix-gitignore    # Add to .gitignore
repos audit --fix-large        # Remove large files from history
repos audit --fix-secrets      # Remove secrets from history

# Preview fixes without applying
repos audit --fix-all --dry-run

# Fix specific repositories only
repos audit --fix-all --repos "repo1,repo2"

# Combined options for CI/CD
repos audit --install-tools --verify --json
```

> **Note**: The global `--force` flag appears in audit help but only applies to push operations.

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