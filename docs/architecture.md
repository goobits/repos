# Architecture

Technical overview of the `repos` codebase structure and design patterns.

## Overview

`repos` is a Rust-based CLI tool organized as both a binary application and a library. The architecture emphasizes modularity, concurrent operations, and clean separation of concerns.

## Project Structure

```
repos/
├── src/
│   ├── main.rs              # CLI entry point and argument parsing
│   ├── lib.rs               # Public library API exports
│   ├── commands/            # Command implementations
│   ├── core/                # Core functionality
│   ├── package/             # Package manager integrations
│   ├── git/                 # Git operations
│   ├── subrepo/             # Subrepo management
│   ├── audit/               # Security auditing
│   └── utils/               # Utility functions
├── tests/
│   └── integration_tests.rs # End-to-end tests
└── docs/                    # Documentation
```

## Module Design

### Commands (`src/commands/`)

Command implementations for each CLI subcommand. Each command module:
- Accepts parsed arguments from CLI
- Orchestrates operations across repositories
- Uses core, git, and specialized modules
- Handles progress reporting and output formatting

**Key commands:**
- `push.rs` - Push operations with upstream handling
- `stage.rs`, `unstage.rs` - Staging operations
- `commit.rs` - Commit across repositories
- `config.rs` - Git config synchronization
- `publish.rs` - Package publishing orchestration
- `audit.rs` - Security scanning coordination
- `subrepo.rs` - Nested repository management

### Core (`src/core/`)

Foundational functionality used across all commands:
- **Repository Discovery** - Finding git repositories in directory trees
- **Configuration Management** - Reading and writing git configs
- **Progress Reporting** - User feedback during operations
- **Concurrency Control** - Managing parallel operations across repositories

**Design principles:**
- Concurrent operations with bounded parallelism (typically 3 concurrent ops)
- Timeouts for long-running operations (3-minute default)
- Progress bars and status indicators for user feedback

### Package Managers (`src/package/`)

Integrations for publishing packages to registries:
- `npm.rs` - npm (JavaScript/TypeScript)
- `cargo.rs` - Cargo (Rust)
- `pypi.rs` - PyPI (Python)

**Common patterns:**
- Auto-detection via manifest files (`package.json`, `Cargo.toml`, `pyproject.toml`)
- Credential management using existing package manager configs
- Dry-run support for safe preview
- Visibility filtering (public/private repository handling)

### Git Operations (`src/git/`)

Low-level git functionality abstraction:
- Repository status checking
- Staging and committing
- Push operations with upstream handling
- Config reading and writing
- Tag creation and management

**Design principles:**
- Wraps `git2` library and shell commands
- Handles edge cases (missing upstreams, dirty working trees)
- Provides consistent error handling

### Subrepo Management (`src/subrepo/`)

Nested repository detection and synchronization:
- **Discovery** - Finding nested `.git` directories
- **Grouping** - Matching subrepos by remote URL
- **Drift Detection** - Comparing commits across instances
- **Synchronization** - Updating subrepos to target commits

**Key algorithms:**
- Sync score calculation: `(total_instances - unique_commits) / (total_instances - 1) × 100`
- Sync target selection: Latest commit without uncommitted changes
- Stash handling: Safe preservation of uncommitted work

### Security Auditing (`src/audit/`)

Security scanning and hygiene checking:
- **TruffleHog Integration** - Secret detection via external tool
- **Hygiene Checks** - Gitignore violations, bad patterns, large files
- **Automated Fixes** - Safe (.gitignore updates) and destructive (history rewriting)

**Components:**
- Secret scanning orchestration
- File pattern matching
- Git history analysis (large files via `git rev-list`)
- Interactive fix prompts

## Concurrency Model

### Parallel Processing

Operations across repositories run concurrently with bounded parallelism:

| Operation | Concurrency Limit | Reason |
|-----------|-------------------|--------|
| TruffleHog scanning | 1 | CPU-intensive, memory-heavy |
| Hygiene checking | 3 | Balanced I/O and CPU |
| Publishing | 3 | Network I/O, rate limiting concerns |
| Standard git operations | 3 | Balance between speed and system load |

### Timeout Handling

Long-running operations have 3-minute timeouts to prevent hangs. This applies to:
- Repository scans
- Publishing operations
- Git operations (large pushes)

## Error Handling

### Strategy

- Use Rust's `Result<T, E>` for recoverable errors
- Provide actionable error messages
- Continue operations on partial failures (report summary at end)
- Exit codes: 0 (success), 1 (failure), especially for CI/CD integration

### Examples

```rust
// Audit verification mode exits 1 if verified secrets found
repos audit --verify  // Exit code 1 → fail CI build

// Publishing continues despite individual failures
repos publish  // Reports "3 published, 1 failed" → Exit code 0
```

## Data Flow

### Typical Command Flow

1. **CLI Parsing** (`main.rs`) - Parse arguments with `clap`
2. **Repository Discovery** (`core/`) - Find all git repositories
3. **Command Execution** (`commands/`) - Execute operation across repos
4. **Module Coordination** - Use `git/`, `package/`, etc. as needed
5. **Progress Reporting** (`core/`) - Show status indicators
6. **Result Aggregation** - Collect successes/failures
7. **Output** - Display summary and exit

### Example: Publishing Flow

```
main.rs
  └─> commands/publish.rs
       ├─> core/discovery (find repos)
       ├─> package/npm.rs (detect + publish npm packages)
       ├─> package/cargo.rs (detect + publish cargo crates)
       ├─> package/pypi.rs (detect + publish python packages)
       ├─> git/tags.rs (create git tags if --tag)
       └─> core/progress (show status updates)
```

## Testing Strategy

### Integration Tests

Located in `tests/integration_tests.rs`:
- End-to-end command testing
- Repository discovery validation
- Package manager detection
- Git operations verification

### Unit Tests

Embedded in module files:
- Function-level testing
- Edge case handling
- Error condition validation

### Test Execution

```bash
cargo test              # All tests
cargo test test_name    # Specific test
cargo test -- --nocapture  # Show output
```

## Build and Release

### Debug Builds

```bash
cargo build
./target/debug/repos
```

Faster compilation, slower runtime. Use for development.

### Release Builds

```bash
cargo build --release
./target/release/repos
```

Optimized binary with full optimizations. Used for distribution.

### Installation

The `install.sh` script:
1. Runs `cargo build --release`
2. Detects first writable location (`/usr/local/bin`, `~/.local/bin`, `~/bin`)
3. Copies binary to location
4. Updates PATH if needed

## Extension Points

### Adding a New Command

1. Create `src/commands/new_command.rs`
2. Implement command logic using core modules
3. Register in `src/main.rs` CLI parser
4. Add tests
5. Update `docs/guides/commands.md`

### Adding Package Manager Support

1. Create `src/package/new_manager.rs`
2. Implement detection (manifest file check)
3. Implement publish logic (credentials, API calls)
4. Add to `commands/publish.rs` detection list
5. Update `docs/guides/credentials_setup.md`

### Adding Audit Checks

1. Extend `src/audit/` with new check type
2. Add detection logic
3. Implement fix strategies (safe/destructive)
4. Add to `commands/audit.rs` orchestration
5. Update `docs/guides/security_auditing.md`

## Performance Considerations

### Repository Discovery

- Walks directory tree recursively
- Checks for `.git` directories
- Skips common ignore patterns (`.git` subdirectories, `node_modules`)
- Concurrent discovery for large directory structures

### Large Monorepos

- Expected behavior: Processing time scales with repository count
- Subrepo detection can be slow with deeply nested structures
- Audit scans (TruffleHog) are CPU/memory intensive

### Optimization Tips

- Use `--repos` flag to target specific repositories
- Limit scope when operating on large monorepos
- TruffleHog scans: Run sequentially (1 concurrent) to avoid memory issues

## Dependencies

### Core Libraries

- `clap` - CLI argument parsing
- `git2` - Git operations (libgit2 bindings)
- `indicatif` - Progress bars and status indicators
- `tokio` / `rayon` - Async and parallel processing
- `serde` / `serde_json` - JSON serialization (for `--json` output)

### External Tools

- **TruffleHog** - Secret scanning (optional, auto-installed via `--install-tools`)
- **git-filter-repo** - History rewriting (required for `--fix-large`, `--fix-secrets`)
- **gh** - GitHub CLI (for repo visibility detection)

## Security Considerations

### Credential Handling

- Never stores credentials
- Uses existing package manager credential files
- Recommends token-based auth over passwords
- File permissions: `chmod 600` for credential files

### Git History Rewriting

- Creates backup refs: `refs/original/pre-fix-backup-<type>-<timestamp>`
- Requires force-push awareness
- Warns about collaborator impact
- Provides rollback instructions

### Secret Scanning

- Detection only (no automatic secret rotation)
- Verification mode confirms if secrets are active
- Emphasizes rotation over deletion

---

**Related Documentation:**
- [Documentation Index](README.md)
- [Contributing Guide](contributing.md)
- [Commands Reference](guides/commands.md)
