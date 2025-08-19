.PHONY: build install clean release-all

# Default target
build:
	cargo build --release

# Install locally
install: build
	./install.sh

# Clean build artifacts
clean:
	cargo clean

# Build for multiple platforms (requires cross)
release-all:
	# Linux x86_64
	cargo build --release --target x86_64-unknown-linux-gnu
	# macOS x86_64
	cargo build --release --target x86_64-apple-darwin
	# macOS ARM64
	cargo build --release --target aarch64-apple-darwin
	# Windows
	cargo build --release --target x86_64-pc-windows-gnu

# Development build
dev:
	cargo build
	./target/debug/sync-repos

# Run tests
test:
	cargo test

# Format code
fmt:
	cargo fmt

# Lint code
lint:
	cargo clippy -- -D warnings