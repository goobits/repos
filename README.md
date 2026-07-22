# repos

Fleet-scale Git orchestration for humans.

`repos` lets you manage a directory full of Git repositories like one project.
The daily path is intent-first: understand state, save work, sync changes. The
Git-shaped commands are still there when you need exact control.

## Quick Start

```bash
# Install from source
git clone https://github.com/goobits/repos.git
cd repos
./install.sh

# Or install from crates.io
cargo install goobits-repos

# Daily usage
repos status                    # Understand fleet state
repos save "Update docs"        # Stage tracked changes, commit, push
repos sync                      # Pull safe changes, push commits, report drift

# Optional: require SSH for every effective Git remote
git config --global repos.transportPolicy ssh-only
```

[Full installation guide →](docs/installation.md)

## Key Features

- **Humane Daily Workflow** - `status`, `save`, and `sync` map to developer intent
- **Safe Defaults** - `save` stages tracked changes only; untracked files require opt-in
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
repos sync                      # Pull safe changes, push commits, report drift

# Git Control
repos push                      # Push only + drift check
repos push --auto-upstream      # Set upstream for new branches
repos pull --rebase             # Pull only with rebase

# Staging & Commits
repos stage "pattern"           # Stage by pattern
repos commit "message"          # Commit staged changes

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

## Safety Model

`repos` is intent-first, not magic-first.

- `repos save "message"` uses the equivalent of `git add -u`, so new files are not committed accidentally.
- Use `repos save "message" --include-untracked` when you intentionally want new files.
- Use `repos save "message" --dry-run` to preview the save plan.
- `repos sync` pulls safe remote changes, pushes local commits, and skips dirty repositories instead of stashing or overwriting local work.
- `repos push --auto-upstream` replaces the old “force” wording for publishing new branches.
- SSH-only policy blocks HTTP(S) before Git can consult a credential helper and reports the repository, effective remote, and exact SSH fix for common hosts.

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

### Agent Workspace

This repo expects a local [Agent Workspace](https://github.com/goobits/aw)
checkout at `infra/aw`. The checkout is ignored by Git so each developer can
update it locally.

```bash
git clone https://github.com/goobits/aw.git infra/aw
make aw-install                 # Install aw, Zellij setup, and repo adapters
make aw-doctor                  # Validate repo adapters and config/aw
make aw-update                  # Fast-forward the local checkout and reinstall
```

The default local workspace is `main` with `dev`, `git`, and `scratch` tabs.

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup.

## License

MIT - see [LICENSE](LICENSE)
