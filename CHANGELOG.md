# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.1.0] - 2025-11-06

### Added
- **--show-changes flag:** Display file changes in repos with uncommitted changes using `repos push --show-changes` (or `-c`)
  - Tree-style display with git status for each repo
  - Limits to first 10 files per repo for clarity
  - Combines with `--verbose` for detailed progress tracking

### Performance
- **50-100x faster publish command:** Reduced from 15 minutes to 10-20 seconds for 500+ repos
  - Combined parallel analysis for visibility, package detection, and dirty status checks
  - In-memory visibility caching to avoid repeated process spawning
  - Async package detection using tokio::fs for non-blocking filesystem checks
- **2.5x faster overall runtime:** Repository processing improved from ~3 minutes to ~1.2 minutes
  - Parallelized repository discovery using up to 8 threads (5-10x faster)
  - Removed artificial 12-operation concurrency cap to scale with CPU cores
  - Increased publish concurrency from 3 to 8 operations
  - DashMap for lock-free concurrent access (20-40% reduction in mutex contention)

### Security
- **TruffleHog installer hardening:** Fixed two medium-priority security issues
  - Proper cleanup of test files in /usr/local/bin
  - Eliminated pipe-to-shell pattern (curl | sh) in download script

### Fixed
- Verbose mode progress bars now update correctly after fetch phase (no more hanging)
- Async variable capture in push phase futures (verbose, timing, and stats now display properly)

## [2.0.0] - 2025-11-05

### Breaking Changes
- **Removed CLI flags:** `--fast`, `--safe`, and `--concurrency` replaced with simplified `--jobs N` and `--sequential`
- **Removed env var:** `REPOS_CONCURRENCY` no longer supported (use `--jobs` instead)
- **Internal API changes:** Module structure refactored with API facade pattern
- **Constant renamed:** `GIT_CONCURRENT_LIMIT` → `GIT_CONCURRENT_CAP`

### Added
- **Smart concurrency detection:** Automatically uses `min(CPU_CORES + 2, 12)` for optimal performance
- **Two-phase pipeline:** Separate fetch (2x concurrency) and push (1x concurrency) phases for 2x performance improvement
- **Rate limit protection:** Automatic GitHub rate limit detection with retry logic (2-second backoff)
- **ARCHITECTURE.md:** Comprehensive documentation of 3-layer architecture and module boundaries
- **Subrepo drift integration:** `repos push` now automatically checks for subrepo drift with `--no-drift-check` flag to disable
- **API facades:** `core/api.rs` and `git/api.rs` for clean public API (reduced from ~150 to ~30 exports)

### Changed
- **Concurrency configuration:** Simplified from 6 options to 2 (`--jobs N` or `--sequential`)
- **Architecture:** Refactored to 3-layer design (Commands → Core → Infrastructure)
- **Module visibility:** 29 functions marked `pub(crate)` for internal use only
- **Documentation:** Comprehensive overhaul for consistency and AI/tool navigation

### Fixed
- Export subrepo module in `lib.rs` for library usage
- Visual clarity improvements in subrepo drift output

### Performance
- **2x faster push operations** through two-phase pipeline architecture
- Optimized repository discovery and processing

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
