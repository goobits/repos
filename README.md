# repos

Batch git operations across multiple repositories. One command instead of dozens of `cd` + `git` loops.

## Quick Start

```bash
# Install from source
git clone https://github.com/goobits/repos.git
cd repos
./install.sh

# Or install from crates.io
cargo install goobits-repos

# Usage
repos stage "*.md"              # Stage files by pattern
repos commit "Update docs"      # Commit across all repos
repos push                      # Push all + drift check
```

[Full installation guide →](docs/installation.md)

## Key Features

- **Batch Operations** - Push, commit, stage across all repositories concurrently (CPU cores + 2)
- **Subrepo Drift Detection** - Track and sync nested repos at different commits
- **Package Publishing** - Publish to npm/Cargo/PyPI with visibility filtering
- **Config Sync** - Synchronize git user.name/email across projects
- **Security Auditing** - Scan for secrets with TruffleHog; automated fixes

## Quick Reference

```bash
# Git Operations
repos push                      # Push all + drift check
repos push --force              # Auto-create upstream branches
repos push --jobs 4             # Limit concurrency

# Staging & Commits
repos stage "pattern"           # Stage by pattern
repos commit "message"          # Commit staged changes
repos status                    # Show staging status

# Publishing
repos publish --dry-run         # Preview
repos publish --tag             # Publish + create git tags
repos publish --all             # Include private repos

# Security
repos audit --verify            # Scan for active secrets
repos audit --fix-gitignore     # Safe fixes only

# Subrepos
repos subrepo status            # Show drift
repos subrepo sync lib --to abc1234 --stash  # Safe sync

# Config
repos config --from-global      # Copy from global config
```

## Commands

`repos push` • `repos stage` • `repos unstage` • `repos status` • `repos commit` • `repos publish` • `repos audit` • `repos subrepo` • `repos config`

See [Commands Reference](docs/guides/commands.md) for complete flag documentation.

## Documentation

**Getting Started**
- **[Installation](docs/installation.md)** - Setup and verification
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

[Full documentation index →](docs/README.md)

## Development

```bash
cargo build --release           # Optimized build
cargo test                      # Run tests
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup.

## License

MIT - see [LICENSE](LICENSE)
