# repos

Fleet-scale Git orchestration for humans. One command instead of dozens of `cd` + `git` loops.

## Quick Start

```bash
# Install from source
git clone https://github.com/goobits/repos.git
cd repos
./install.sh

# Or install from crates.io
cargo install goobits-repos

# Usage
repos status                    # Understand fleet state
repos save "Update docs"        # Stage tracked changes, commit, push
repos sync                      # Fetch, rebase, and report drift
```

[Full installation guide →](docs/installation.md)

## Key Features

- **Humane Daily Workflow** - `status`, `save`, and `sync` cover common fleet work
- **Batch Operations** - Push, pull, commit, stage across all repositories concurrently
- **Git LFS Support** - Automatic detection and handling of Large File Storage in push/pull operations
- **Nested Drift Detection** - Track and sync nested repos at different commits
- **Package Publishing** - Publish to npm/Cargo/PyPI with visibility filtering
- **Config Sync** - Synchronize git user.name/email across projects
- **Security Auditing** - Scan for secrets with TruffleHog; automated fixes

## Quick Reference

```bash
# Everyday
repos status                    # Understand fleet state
repos save "Update docs"        # Stage tracked changes, commit, push
repos sync                      # Fetch, rebase, and report drift

# Git Control
repos push                      # Push all + drift check
repos push --auto-upstream      # Set upstream for new branches
repos pull --rebase             # Granular pull with rebase

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
repos doctor                    # Diagnose common blockers

# Nested repos
repos nested status             # Show drift
repos nested sync lib --to abc1234 --stash  # Safe sync

# Config
repos config --from-global      # Copy from global config
```

## Commands

`repos status` • `repos save` • `repos sync` • `repos push` • `repos pull` • `repos stage` • `repos unstage` • `repos commit` • `repos publish` • `repos audit` • `repos doctor` • `repos nested` • `repos config`

See [Commands Reference](docs/guides/commands.md) for complete flag documentation.

## Documentation

**Getting Started**
- **[Installation](docs/installation.md)** - Setup and verification
- **[Getting Started](docs/getting_started.md)** - 5-minute tutorial with examples

**Guides**
- **[Commands Reference](docs/guides/commands.md)** - All commands, flags, and workflows
- **[Publishing](docs/guides/publishing.md)** - Package publishing workflows
- **[Security Auditing](docs/guides/security_auditing.md)** - Secret scanning and fixes
- **[Nested Repository Management](docs/guides/subrepo_management.md)** - Drift detection and sync
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
