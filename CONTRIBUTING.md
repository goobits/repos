# Contributing to repos

Thank you for your interest in contributing! This guide will help you get started with development.

## Getting Started

### Fork and Clone

```bash
# Fork the repository on GitHub, then clone your fork
git clone https://github.com/YOUR_USERNAME/repos.git
cd repos
```

### Development Setup

```bash
# Build the project
cargo build

# Run the tool in development mode
cargo run -- --help
```

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name
```

## Project Structure

The codebase is organized as follows:

- `src/main.rs` - CLI entry point and command-line argument parsing
- `src/lib.rs` - Library exports for public API
- `src/commands/` - Command implementations (status, pull, push, exec, etc.)
- `src/core/` - Core functionality (repository discovery, config, progress bars)
- `src/package/` - Package manager integrations (npm, cargo, pypi)
- `src/git/` - Git operations and repository management
- `src/subrepo/` - Subrepo management and synchronization
- `src/audit/` - Security auditing functionality
- `src/utils/` - Utility functions and helpers
- `tests/integration_tests.rs` - Integration tests

## Development Workflow

### Make Changes

1. Create a feature branch: `git checkout -b feature/my-feature`
2. Make your changes in the appropriate module
3. Add tests for new functionality

### Code Quality

```bash
# Run tests
cargo test
# Or use: make test

# Format code
cargo fmt
# Or use: make fmt

# Lint code (warnings treated as errors)
cargo clippy -- -D warnings
# Or use: make lint

# Build release binary
cargo build --release
# Or use: make build
```

### Using the Makefile

The project includes a Makefile for common tasks:

```bash
make test      # Run all tests
make fmt       # Format code with rustfmt
make lint      # Lint code with clippy
make build     # Build optimized release binary
make dev       # Build and run debug version
make clean     # Remove build artifacts
make install   # Build and install locally
```

## Testing

### Running Tests

```bash
# Run all tests (unit + integration)
cargo test

# Run specific test
cargo test test_name

# Run with verbose output
cargo test -- --nocapture
```

### Integration Tests

Integration tests are located in `tests/integration_tests.rs` and test end-to-end workflows including repository discovery, command execution, and package management.

## Code Style

- Use `rustfmt` for consistent formatting (runs via `cargo fmt`)
- Follow `clippy` suggestions for idiomatic Rust
- Write doc comments (`///`) for public APIs and modules
- Keep functions focused and testable
- Prefer descriptive variable names
- Handle errors appropriately using `Result<T, E>`

## Adding Features

### Adding a New Command

1. Create command module in `src/commands/`
2. Implement command logic and error handling
3. Register command in `src/main.rs` CLI parser
4. Add tests in the command module or `tests/integration_tests.rs`
5. Update documentation in `docs/commands.md` and `README.md`

### Adding Package Manager Support

1. Create new module in `src/package/`
2. Implement package manager trait/interface
3. Add detection logic in core discovery
4. Add tests for new package manager
5. Update documentation

## Pull Requests

1. **Create feature branch**: `git checkout -b feature/descriptive-name`
2. **Write clear commit messages**: Use conventional commit format when possible
3. **Ensure tests pass**: Run `make test` and `make lint`
4. **Update CHANGELOG.md**: Add entry for your changes
5. **Submit PR**: Link to related issue if applicable
6. **Respond to feedback**: Address review comments promptly

### Commit Message Guidelines

- Use present tense ("Add feature" not "Added feature")
- Use imperative mood ("Move cursor to..." not "Moves cursor to...")
- First line should be concise (<72 characters)
- Reference issues: "Fix #123" or "Closes #456"

## Questions?

If you have questions or need help:
- Open an issue for discussion
- Check existing documentation in `docs/`
- Review closed issues for similar questions
