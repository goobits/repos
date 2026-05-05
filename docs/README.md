# repos Documentation

**repos** is a CLI tool for fleet-scale Git orchestration across multiple repositories. One command instead of dozens of `cd` + `git` loops.

**Core capabilities:**
- `status`, `save`, and `sync` workflows for daily fleet work
- Automatic nested repository drift detection and synchronization
- Package publishing to npm, Cargo, and PyPI with one line
- Built-in security scanning for secrets and credential leaks

Perfect for monorepo management, multi-project workflows, and keeping nested repositories in sync.

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
