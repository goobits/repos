# repos

Git repository management tool for batch operations across multiple repositories.

## Features

- **Batch Operations** - Push, commit, stage across all repos
- **Publishing** - Publish to npm, cargo, PyPI with visibility filtering
- **Config Management** - Sync git user.name/email across projects
- **Security Audit** - Scan for secrets and vulnerabilities
- **Concurrent Processing** - Parallel operations with progress tracking

## Installation

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
repos push                      # Push all unpushed commits
repos push --force              # Auto-push branches with no upstream
```

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

See [docs/PUBLISH_FEATURES.md](docs/PUBLISH_FEATURES.md) for details.

See [docs/CREDENTIALS_SETUP.md](docs/CREDENTIALS_SETUP.md) for credential setup.

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
