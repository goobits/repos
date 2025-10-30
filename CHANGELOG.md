# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.3.0] - 2025-10-30

### Added
- Git commit command across repositories with `repos commit <message>`
- Support for `--include-empty` flag to force empty commits
- Auto-skip repositories with no staged changes by default
- Display commit hash in success messages
- Comprehensive test coverage for staging functionality
- Repository visibility filtering to `repos publish` command
- `--all` flag to publish all repositories (public + private)
- `--public-only` flag to explicitly publish only public repositories
- `--private-only` flag to publish only private repositories
- `--tag` flag to create and push git tags after successful publish
- `--allow-dirty` flag to publish with uncommitted changes
- `get_repo_visibility()` function using gh CLI to detect repository visibility

### Changed
- Default behavior of `repos publish` now filters to public repositories only (safe default)
- Unknown visibility is treated as private (fail-safe approach)
- Added clear filtering feedback showing skip counts for publish command

### Fixed
- Only show uncommitted changes suffix for synced repos, not pushed
- Refresh git index before checking for uncommitted changes

### Removed
- PUBLISH_DEMO.md (outdated documentation)

## [1.2.0] - 2025-09-24

### Added
- Git staging commands across repositories:
  - `repos stage <pattern>` - Stage files matching pattern in all repos
  - `repos unstage <pattern>` - Unstage files matching pattern in all repos
  - `repos status` - Show staging status across all repos
- Pattern support: `*.md`, `README.md`, `*` (all files)
- Concurrent execution with progress bars for staging operations
- Modern git commands (git restore --staged)

### Changed
- Improved error handling and status reporting for git operations

## [1.1.1] - 2025-09-23

### Changed
- Comprehensive code quality improvements and refactoring
- Renamed `user` command to `config` for better semantics
- Renamed `sync` command to `push` for clarity

### Fixed
- Remove needless borrowing in TruffleHog installer
- Make validation functions available for testing

## [1.0.1] - 2025-09-20

### Changed
- Make CLI version dynamic
- Code formatting and style improvements
- Rename project from sync-repos to repos
