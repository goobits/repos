# repos Documentation

**repos** is a humane CLI for fleet-scale Git orchestration across multiple
repositories. It turns common intent into one safe command: understand state,
save work, and sync changes.

**Core capabilities:**
- `status`, `save`, and `sync` workflows for daily fleet work
- Safe defaults: tracked changes only for `save`, dirty repos skipped by `sync`
- Automatic nested repository drift detection and synchronization
- Package publishing to npm, Cargo, and PyPI with one line
- Built-in security scanning for secrets and credential leaks

Perfect for multi-project workspaces, monorepos with nested Git repositories,
and developers who want fleet operations without acting as a human `for` loop.

---

## Documentation

### Start Here (Everyone)

- **[Getting Started](getting_started.md)** - Quick 5-minute tutorial with examples
- **[Installation](installation.md)** - Setup and verification

### Core Guides (Regular Users)

- **[Commands Reference](guides/commands.md)** - All commands, flags, and usage patterns
- **[Publishing](guides/publishing.md)** - Package publishing workflows
- **[Credentials Setup](guides/credentials_setup.md)** - Authentication for npm, Cargo, PyPI
- **[Troubleshooting](guides/troubleshooting.md)** - Common issues and solutions

### Advanced Topics (Power Users)

- **[Nested Repository Management](guides/subrepo_management.md)** - Nested repository drift detection and sync
- **[Security Auditing](guides/security_auditing.md)** - Secret scanning and automated fixes
- **[Architecture](architecture.md)** - Technical internals and concurrency model

### Reference (As Needed)

- **[Glossary](glossary.md)** - Terms, flags, and concepts
- **[Examples](examples/README.md)** - CI/CD integration templates and scripts

### Developer

- **[Contributing](../CONTRIBUTING.md)** - Development guide and module boundaries
