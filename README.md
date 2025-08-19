# sync-repos

A fast, portable Git repository synchronization tool written in Rust. Automatically finds all Git repositories in the current directory tree and pushes any unpushed commits.

## Features

- 🔍 Recursively finds all Git repositories
- 🚀 Pushes unpushed commits automatically  
- 📊 Live status updates with colored output
- ⚡ Fast parallel processing
- 🔒 Uses your existing Git authentication
- 📦 Single portable binary

## Installation

### Quick Install
```bash
chmod +x install.sh
./install.sh
```

### Manual Build
```bash
# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build and install
cargo build --release
cp target/release/sync-repos ~/.local/bin/
```

### Install from crates.io (future)
```bash
cargo install sync-repos
```

## Usage

Simply run in any directory:
```bash
sync-repos
```

The tool will:
1. Find all Git repositories recursively
2. Check each repo for unpushed commits
3. Push any pending changes
4. Display a summary of actions taken

## Comparison with Python Version

| Feature | Python (200 lines) | Rust (180 lines) |
|---------|-------------------|------------------|
| Dependencies | python3, rich | Single binary |
| Performance | ~1s per repo | ~0.3s per repo |
| Installation | pip install | Copy binary |
| Portability | Needs Python env | Works anywhere |

## Build Requirements

- Rust 1.70+ (for building only)
- Git (runtime requirement)

## License

MIT
# sync-repos
