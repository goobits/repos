# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.4.0] - 2025-10-31

### Added
- Subrepo drift detection and synchronization
- `repos subrepo validate` - Discover all nested repositories
- `repos subrepo status` - Show drift with smart sync suggestions
- `repos subrepo sync` - Sync to specific commit with `--stash` flag
- `repos subrepo update` - Update to latest from origin/main
- Smart SYNC TARGET detection (latest clean commit)
- Problem-first output (shows only drifted subrepos by default)
- Visual indicators: ✅ clean, ⚠️ uncommitted, 🎯 SYNC TARGET, ⬆️ LATEST
- Commit timestamp sorting to identify newest commits
- Groups by remote URL (not name) to avoid false positives
- Sync score calculation (0-100% synchronized)
- `--verbose` flag for push command (detailed per-repo progress)

### Changed
- Push command now shows live tally by default (cleaner output)
- Use `--verbose` flag to see detailed per-repo progress bars

## [1.3.0] - 2025-10-30

### Added
- Git commit command across repositories with `repos commit <message>`
- `--include-empty` flag to force empty commits
- Repository visibility filtering to `repos publish` command
- `--all`, `--public-only`, `--private-only` flags for publish command
- `--tag` flag to create and push git tags after successful publish
- `--allow-dirty` flag to publish with uncommitted changes

### Changed
- `repos publish` now filters to public repositories only by default
- Auto-skip repositories with no staged changes when committing

### Fixed
- Only show uncommitted changes suffix for synced repos, not pushed
- Refresh git index before checking for uncommitted changes

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
