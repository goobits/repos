# repos Makefile
# Build and installation targets for the Git repository management tool

.PHONY: build install clean release-all dev test fmt lint help

# Default target - build optimized release binary
build:
	cargo build --release

# Install the tool locally using the install script
install: build
	./install.sh

# Remove all build artifacts and target directory
clean:
	cargo clean

# Build release binaries for multiple platforms (requires cross-compilation setup)
release-all:
	# Linux x86_64 (most common Linux desktop/server)
	cargo build --release --target x86_64-unknown-linux-gnu
	# macOS Intel (x86_64)
	cargo build --release --target x86_64-apple-darwin
	# macOS Apple Silicon (ARM64)
	cargo build --release --target aarch64-apple-darwin
	# Windows x86_64
	cargo build --release --target x86_64-pc-windows-gnu

# Build and run development version (debug build with faster compilation)
dev:
	cargo build
	./target/debug/repos

# Run all unit and integration tests
test:
	cargo test

# Format code using rustfmt
fmt:
	cargo fmt

# Lint code using clippy with warnings treated as errors
lint:
	cargo clippy -- -D warnings

# Display available make targets
help:
	@echo "Available targets:"
	@echo "  build       - Build optimized release binary"
	@echo "  install     - Install the tool locally"
	@echo "  clean       - Remove build artifacts"
	@echo "  dev         - Build and run debug version"
	@echo "  test        - Run all tests"
	@echo "  fmt         - Format code with rustfmt"
	@echo "  lint        - Lint code with clippy"
	@echo "  release-all - Build for multiple platforms"
	@echo "  help        - Show this help message"