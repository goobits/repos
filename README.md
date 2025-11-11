# 🔄 repos

Batch git operations across multiple repositories. One command instead of dozens of `cd` + `git` loops.

## ✨ Key Features

- **🚀 Batch Operations** - Push, commit, stage across all repositories simultaneously
- **🔍 Subrepo Drift Detection** - Track and sync nested repos with automatic conflict detection
- **📦 Package Publishing** - Publish to npm, Cargo, PyPI with visibility filtering
- **⚙️ Config Sync** - Synchronize git user.name/email across all projects
- **🔒 Security Auditing** - Scan for exposed secrets and credential leaks
- **⚡ Concurrent Processing** - Parallel operations with configurable concurrency (CPU cores + 2)

## 🚀 Quick Start

```bash
# Install
./install.sh

# Or build from source
cargo build --release

# Basic workflow
repos stage "*.md"           # Stage files by pattern
repos commit "Update docs"   # Commit across all repos
repos push                   # Push all + drift check

# Publishing
repos publish --dry-run      # Preview
repos publish --tag          # Publish + git tags
```

## 📖 Commands

### Push Operations

```bash
repos push                      # Push all + drift check
repos push --force              # Auto-create upstream branches
repos push --show-changes       # Display file changes
repos push --jobs 4             # Limit concurrency
repos push --sequential         # Serial execution (debug)
repos push --no-drift-check     # Skip drift check
```

### Staging & Commits

```bash
repos status                    # Staging status across repos
repos stage "*.md"              # Stage by pattern
repos unstage "*"               # Unstage all
repos commit "Message"          # Commit staged changes
repos commit "Fix" --include-empty  # Force empty commits
```

### Configuration

```bash
repos config --name "Alice" --email "alice@example.com"
repos config --from-global      # Copy from global config
repos config --from-current     # Copy from current repo
repos config --dry-run          # Preview changes
```

### Publishing

```bash
repos publish                   # Public repos only (default)
repos publish my-app my-lib     # Specific repos
repos publish --dry-run         # Preview
repos publish --tag             # Create git tags (v1.2.3)
repos publish --all             # Include private repos
repos publish --allow-dirty     # Skip clean check
```

**Setup**: See [credentials_setup.md](docs/guides/credentials_setup.md) for npm/Cargo/PyPI authentication.

### Security Auditing

```bash
repos audit                     # Scan for secrets
repos audit --install-tools     # Auto-install TruffleHog
repos audit --verify            # Verify secrets are active (CI mode)
repos audit --fix-gitignore     # Safe: add .gitignore entries
repos audit --fix-secrets       # Destructive: rewrite history
repos audit --interactive       # Choose fixes manually
repos audit --json              # JSON output
```

### Subrepo Drift Detection

```bash
repos subrepo validate          # Discover nested repos
repos subrepo status            # Show drift (problem-first)
repos subrepo status --all      # Include synced repos
repos subrepo sync lib --to abc1234 --stash  # Safe sync with stash
repos subrepo update lib        # Update to origin/main
```

**Features**: Smart sync target detection, safe stashing, visual indicators (✅ clean, ⚠️ uncommitted, → sync target).

## ⚙️ Configuration

```bash
# View current settings
git config --list

# Concurrency control
repos push --jobs 8             # Explicit limit
repos push --sequential         # Serial (1 at a time)
# Default: CPU cores + 2, capped at 32

# Verbose output
repos push --verbose            # Detailed progress
repos push -v                   # Short form
```

## 📚 Documentation

**Getting Started**
- **[Installation](docs/installation.md)** - Setup and prerequisites
- **[Getting Started](docs/getting_started.md)** - 5-minute tutorial with examples

**Guides**
- **[Commands Reference](docs/guides/commands.md)** - All commands, flags, and workflows
- **[Publishing](docs/guides/publishing.md)** - Package publishing workflows
- **[Security Auditing](docs/guides/security_auditing.md)** - Secret scanning and fixes
- **[Subrepo Management](docs/guides/subrepo_management.md)** - Drift detection and sync
- **[Troubleshooting](docs/guides/troubleshooting.md)** - Common issues and solutions

**Reference**
- **[Glossary](docs/glossary.md)** - Terms, flags, and concepts
- **[Architecture](docs/architecture.md)** - Concurrency model and design patterns
- **[Examples](docs/examples/README.md)** - CI/CD templates and automation scripts

**Developer**
- **[Contributing](CONTRIBUTING.md)** - Development setup and guidelines

## 🛠️ How It Works

- Discovers `.git` directories recursively (excludes `node_modules/`, `vendor/`, `target/`, `build/`, `dist/`)
- Processes repositories concurrently (default: CPU cores + 2, max 32)
- Operations timeout after 3-5 minutes depending on type
- Real-time progress bars with operation summaries

## 🧪 Development

```bash
cargo build                     # Debug build
cargo build --release           # Release build (optimized)
cargo test                      # Run tests
cargo clippy                    # Linting
cargo fmt                       # Format code
```

## 📝 License

MIT - see [LICENSE](LICENSE)
