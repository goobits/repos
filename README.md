# repos

Git repository management tool for batch operations across multiple repositories.

## Features

- **Batch Operations** - Push, commit, stage across all repos
- **Subrepo Drift Detection** - Track and sync nested repos across parents
- **Publishing** - Publish to npm, cargo, PyPI with visibility filtering
- **Config Management** - Sync git user.name/email across projects
- **Security Audit** - Scan for secrets and vulnerabilities
- **Concurrent Processing** - Parallel operations with progress tracking

## Documentation

### Getting Started
- **[Getting Started](docs/getting_started.md)** - Quick 5-minute tutorial
- **[Installation](docs/installation.md)** - Detailed setup guide

### Guides
- **[Commands Reference](docs/guides/commands.md)** - All commands and flags
- **[Publishing Guide](docs/guides/publishing.md)** - Package publishing workflows
- **[Security Auditing](docs/guides/security_auditing.md)** - Secret scanning and hygiene
- **[Subrepo Management](docs/guides/subrepo_management.md)** - Drift detection and sync
- **[Troubleshooting](docs/guides/troubleshooting.md)** - Common issues and solutions

### Reference
- **[Glossary](docs/glossary.md)** - Terms, flags, and concepts
- **[Architecture](docs/architecture.md)** - Technical design and patterns
- **[Examples](docs/examples/README.md)** - CI/CD integration and scripts

### Developer
- **[Contributing](CONTRIBUTING.md)** - Development guide

## Quick Install

```bash
chmod +x install.sh && ./install.sh
```

Or build from source:
```bash
cargo build --release
```

## Commands

### Push

```bash
repos push                      # Push all unpushed commits + check drift
repos push --force              # Auto-push branches with no upstream
repos push --no-drift-check     # Skip drift check for speed
```

**Integrated Health Check**: `repos push` automatically checks for subrepo drift after pushing, giving you a complete repository health report in one command.

### Staging & Commits

```bash
repos status                    # Show staging status across all repos
repos stage "*.md"              # Stage files matching pattern
repos unstage "*.md"            # Unstage files
repos commit "Message"          # Commit staged changes
repos commit "Message" --include-empty  # Include repos with no changes
```

### Configuration

```bash
repos config --name "Name" --email "you@example.com"
repos config --from-global      # Copy from global git config
repos config --from-current     # Copy from current repo
repos config --dry-run          # Preview changes
repos config --force            # Skip prompts
```

### Publishing

```bash
repos publish                   # Public repos only (default)
repos publish my-app my-lib     # Specific repos
repos publish --dry-run         # Preview
repos publish --all             # All repos (public + private)
repos publish --public-only     # Explicit public only
repos publish --private-only    # Private only
repos publish --tag             # Create git tags (v1.2.3)
repos publish --allow-dirty     # Allow uncommitted changes
```

See [docs/guides/publishing.md](docs/guides/publishing.md) for details.

See [docs/guides/credentials_setup.md](docs/guides/credentials_setup.md) for credential setup.

### Security Auditing

```bash
repos audit                     # Scan for secrets
repos audit --install-tools     # Auto-install TruffleHog
repos audit --verify            # Verify secrets are active
repos audit --json              # JSON output
repos audit --interactive       # Choose fixes interactively
repos audit --fix-gitignore     # Add .gitignore entries
repos audit --fix-large         # Remove large files
repos audit --fix-secrets       # Remove secrets from history
repos audit --fix-all           # Apply all fixes
repos audit --dry-run           # Preview fixes
repos audit --repos repo1,repo2 # Specific repos only
```

### Subrepo Drift Detection

```bash
repos subrepo validate          # Discover all nested repos
repos subrepo status            # Show drift (problem-first)
repos subrepo status --all      # Show all subrepos
repos subrepo sync <name> --to <commit> --stash  # Safe sync
repos subrepo update <name>     # Update to origin/main
```

Detects nested repositories shared across multiple parents and provides
smart suggestions to synchronize them. Uses `🎯 SYNC TARGET` to identify
the latest clean commit for safe syncing.

**Key features:**
- Identifies drift across shared subrepos
- Smart sync target detection (latest clean commit)
- `--stash` flag for safe, reversible syncing
- Visual indicators: ✅ clean, ⚠️ uncommitted, 🎯 SYNC TARGET, ⬆️ LATEST
- Groups by remote URL to avoid false positives

## How It Works

- Recursively scans for `.git` directories in current tree
- Excludes: `node_modules/`, `vendor/`, `target/`, `build/`, `dist/`
- Parallel processing with controlled concurrency
- 3-minute timeout per repository
- Real-time progress bars

## Development

```bash
cargo build
cargo test
cargo clippy
cargo fmt
```

## License

MIT - see [LICENSE](LICENSE)
