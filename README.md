# sync-repos

Fast Git repository synchronization tool. Automatically finds all Git repositories in the current directory tree and pushes unpushed commits.

## Features

- 🔍 Recursively finds all Git repositories  
- 🚀 Pushes unpushed commits automatically
- 📊 Live status updates with colored output
- ⚡ Fast parallel processing (3 repos concurrently)
- 🔒 Uses your existing Git authentication
- 📦 Single portable binary

## Installation

### Quick Install
```bash
chmod +x install.sh && ./install.sh
```

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
1. Scan for Git repositories recursively
2. Check each repo for unpushed commits  
3. Push pending changes to upstream
4. Display summary with status indicators

## Requirements

- Git (runtime)
- Rust 1.70+ (build only)

## License

MIT
