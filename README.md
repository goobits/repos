# sync-repos

A fast Git repository synchronization tool. Automatically discovers all Git repositories in the current directory tree and pushes any unpushed commits to their upstream remotes.

## Features

- 🔍 **Automatic Discovery**: Recursively finds all Git repositories
- 🚀 **Smart Sync**: Pushes unpushed commits automatically
- 📊 **Live Updates**: Real-time status with colored output
- ⚡ **Parallel Processing**: Handles up to 3 repositories concurrently
- 🔒 **Secure**: Uses your existing Git authentication
- 📦 **Portable**: Single binary with no dependencies

## Installation

### Quick Install (Recommended)
```bash
chmod +x install.sh && ./install.sh
```

This will automatically install Rust if needed and build the tool.

### Manual Build
```bash
cargo build --release
cp target/release/sync-repos ~/.local/bin/
```

## Usage

Run in any directory containing Git repositories:
```bash
sync-repos
```

The tool will:
1. **Scan**: Recursively discover all Git repositories
2. **Analyze**: Check each repository for unpushed commits
3. **Sync**: Push pending changes to their upstream remotes
4. **Report**: Display a summary with color-coded status indicators

## Requirements

- **Runtime**: Git (must be installed and configured)
- **Build**: Rust 1.75+ (only needed for manual compilation)

## License

MIT License - see [LICENSE](LICENSE) for details.
