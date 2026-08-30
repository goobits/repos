# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **SSH-only Git transport policy:** `git config --global repos.transportPolicy ssh-only` blocks effective HTTP(S) fetch and push URLs before credential helpers run, including macOS Keychain helpers. Transfer failures now name the repository and sanitized remote, provide an exact SSH conversion command for common hosts, and distinguish SSH key failures from transport fixes.
- **Fetch command:** `repos fetch` refreshes every configured remote without changing local branches or worktrees and uses the same attributable, exclusive report contract as push/pull.

### Changed
- Exact per-checkout nested inspection now runs through a bounded eight-worker pool and restores deterministic fleet order before reporting.
- Automatic Git concurrency now scales as `min(CPU cores + 2, 32)` to bound subprocess/network pressure on large hosts; explicit `--jobs` remains an override and fetch multipliers use saturating arithmetic.
- Oversized command, Git, audit, nested-sync, and reporting modules were decomposed into focused orchestration, mutation, inspection, installation, and rendering components. No production responsibility exceeds 500 lines; larger physical files contain inline test modules only.
- Transfer operations now carry exact commit/ref quantities into statistics instead of deriving totals from display text. The already thread-safe statistics collector is shared directly, removing the fleet-wide outer mutex from top-level commands.
- Publish planning now limits repository inspection fanout to eight, matches GitHub by its normalized remote host, and parses each package manifest once. Malformed/static-uninspectable manifests fail explicitly instead of producing placeholder metadata.
- Doctor reads configured fetch/push URLs in one byte-safe Git config query per repository while retaining separate effective-URL policy checks. Config synchronization reads effective user name/email together and reports malformed or failed inspection.
- Audit history scanning now streams one `rev-list` process through one `cat-file` process while retaining the exact largest-file results. TruffleHog JSON is parsed one finding at a time and raw secret output is no longer buffered for an entire repository.
- Push/pull analysis now reads branch, worktree, upstream, and both ancestry counts from porcelain-v2 repository snapshots around fetch. Status, doctor, and save reuse those typed snapshots, and active pushes reuse the already selected remote.
- Fleet topology now resolves nearest parents with an immutable path index, batches gitlink inspection once per relevant parent, and is reused across both phases of `repos sync` and its final nested report.
- Git state inspection now counts ahead/behind commits from one revision-graph snapshot, reuses that result across status and publish checks, and avoids push-specific remote/LFS probes when a tracked branch has nothing to push.
- Nested status now derives parent/child relationships from the same fleet inventory as transfer commands, assigns each checkout to its nearest parent exactly once, and makes `--all` include synced, unique, and missing-origin repositories. Summaries distinguish shared groups, physical copies, independent repositories, Git submodules, and linked worktrees.
- Transfer and sync reports now place their single compact summary at the bottom, including pull/push transfer counts for `repos sync`, so long actionable reports leave the run result visible without duplicating totals.
- Repository-oriented report sections are now sorted by path, grouping nested packages under their top-level project; nested package drift is sorted alphabetically by package, then by each copy's project path.
- Transfer and sync reports now combine failed, skipped, and local follow-up details into one project-grouped attention section with fixed-width `!`, `·`, and `~` markers. Final reports also begin after a single visual break from progress output.
- `repos status` now refreshes upstream refs before comparing commits, treats ahead and behind branches as work to do, and reports exact next actions without moving local branches or worktrees.
- Concise transfer progress is explicitly finished before final reports render, preventing progress repaint from indenting or erasing report details in interactive terminals.
- The source installer now builds and installs from one portable external Cargo target directory, respects `CARGO_TARGET_DIR`, and no longer assumes a checkout-local `target/` path.
- Repository discovery no longer applies Git/tool ignore rules to fleet inventory, so ignored nested repositories still participate. Curated dependency/build directories and explicit `.reposignore` entries remain excluded.
- Status, publish, and nested mutation reports now name their outcomes, include checked totals, and provide paths and next steps where action is required.
- `repos doctor` now emits a sorted summary with separate warnings/blockers, inspects both fetch and push URLs, skips HTTP access probes that could trigger credential helpers, and provides sanitized per-repo fixes.
- Push and pull summaries now use exclusive outcome counts that add up to `Checked`; skipped repositories are named with path/reason/next-step details, while local and nested work is explicitly non-exclusive follow-up.
- `repos sync` now discovers repositories once and emits one combined pull/push report with exclusive per-repository outcomes.
- `save`, `stage`, `unstage`, `commit`, and `config` now use operation-specific summaries that name every changed or planned repository and give path/reason/next-step details for non-successes.
- **Push/pull report UX:** Both transfer commands now use the same compact ANSI-colored report, name repositories and commit counts, deduplicate follow-up work, and keep nested drift as a short action list.
- **Push progress:** Concise mode names a repository when its operation is still running after 10 seconds instead of leaving only the last completed repository visible.
- Commit, save, push, pull, sync, and audit fixes now use dependency waves: children run before parent commits/pushes, while parents run before child pulls. Independent repositories within each wave remain concurrent.
- Parent commits refresh gitlinks after child commits, and parent pushes verify that each exact gitlink commit is reachable from fetched child remote refs.
- Package publishing now verifies a clean, fully pushed release commit and publishes local package dependencies in topological waves. Duplicate identities and dependency cycles fail before registry mutation.
- Nested sync/update preflight every copy before mutation and use one immutable target commit across the batch. Interactive config prompts are serialized.

### Fixed
- `repos sync` and `repos pull --rebase` now rebase clean diverged branches instead of rejecting them during preflight before Git can reconcile the histories.
- Declared submodule validation now honors `.reposignore`, so intentionally excluded checkouts are not misreported as uninitialized.
- Repository discovery no longer silently stops at depth 10, so deeply nested checkouts participate in fleet and nested-drift reporting.
- Nested status now reports declared but uninitialized submodules separately with exact initialization commands. Gitlink inspection failures make the inventory incomplete instead of silently classifying affected checkouts as independent.
- Worktree and conflict detection now uses NUL-delimited porcelain v2, including every unmerged state and filenames containing newlines or non-UTF-8 bytes. Git config command failures are reported instead of being treated as unset values.
- Repository discovery now canonicalizes physical checkouts before naming them, so symlink aliases cannot make fleet operations or nested drift reports process the same repository twice.
- Nested inventory no longer drops registered submodules or linked worktrees; checkout type is reported as metadata instead of narrowing drift coverage. Successful automatic drift checks remain visible even when no group is drifted.
- Automatic push, pull, and sync reports now surface an incomplete nested drift check instead of silently rendering it as no drift, and concise drift output reports its shared-group/copy coverage.
- Source installation now uses a private build cache, validates a mode-`0755` staged executable, atomically replaces and rechecks the installed binary, and restores the previous executable on failure. This prevents macOS stale code-signature kills without weakening system-wide permissions.
- Dirty worktree classification is retained when remote status refresh fails.
- Existing release tags are never moved to a different commit; `--tag` can push a matching local tag after an already-published registry result.
- Nested drift commands classify independent embedded repositories, Git submodules, and linked worktrees without conflating or excluding them.

### Security
- Installer-generated PATH configuration now quotes custom installation directories safely, and default build artifacts live in a private user cache instead of a predictable shared `/tmp` directory.
- HTTP credentials, URL user information, and query strings are redacted from Git failure reports.
- Secret and hygiene reports retain repository and file attribution, while `repos audit --json` emits one redirect-safe JSON document even with dry-run fixes.
- Audit safety is rechecked immediately before each mutation, and bulk history rewrites across parent/submodule dependency sets are refused.

## [4.0.0] - 2026-05-05

### Added
- **Humane workflow commands:** Added intent-first daily commands for managing a fleet like one project.
  - `repos save` stages tracked changes, commits, and pushes in one step.
  - `repos sync` fetches and safely updates clean repositories with rebase semantics.
  - `repos doctor` diagnoses auth, remotes, nested state, and required tooling.
- **Nested repository UX:** Added the `repos nested` command surface for nested repo drift management.

### Changed
- **CLI command model:** Reframed the primary interface around `status`, `save`, and `sync`, with granular Git commands kept for explicit control.
- **Safer push semantics:** Replaced ambiguous force-style upstream behavior with `repos push --auto-upstream`.
- **Safer sync behavior:** Dirty repositories are skipped by default during sync instead of being mutated implicitly.
- **Reporting parity:** Push and pull now use operation-aware summaries.
  - Pull reports pulled repositories and pulled commits instead of showing push counters.
  - Diverged repositories are grouped separately from generic failures.
  - Email privacy failures are grouped as push policy blocks.
  - Missing upstream guidance now points to `repos push --auto-upstream`.
- **Documentation:** Rewrote README and core docs around humane intent, safety defaults, and day-to-day workflows.

### Fixed
- **Nested drift output:** Automatic post-run drift checks no longer print a stray nested scan line when there is no drift to report.
- **Test isolation:** Fixed cwd restoration in blocking discovery tests so temporary directories cannot leak across tests.

### Breaking Changes
- **Subrepo command naming:** Public-facing nested repository workflows now use `nested` terminology instead of `subrepo`.
- **Push upstream flag:** Use `--auto-upstream`; force-style naming is no longer the intended workflow.

## [3.1.0] - 2025-12-21

### Added
- **Git LFS support:** Full support for Git Large File Storage in push and pull operations
  - Automatic LFS detection via `.gitattributes` and `git lfs ls-files`
  - Pre-push LFS object upload to prevent incomplete operations
  - Pre-pull LFS object fetch to avoid delays during checkout
  - Clear indicators in output ("with LFS" suffix on success messages)
  - Handles LFS-enabled repositories transparently
- **Subrepo sync testing:** Integration test for subrepo synchronization functionality
  - Validates sync across multiple parent repositories
  - Tests commit targeting and verification

### Changed
- **Modular architecture:** Major refactoring for extensibility and maintainability
  - Package manager trait system replacing hardcoded enum (easy to add new package managers)
  - Publish command split into focused modules (planner, executor, orchestrator)
  - Hygiene audit module reorganized (rules, scanner, report separation)
  - Net reduction of 710 lines of code while improving structure
- **Code quality improvements:** Comprehensive clippy pedantic fixes
  - 471 warnings reduced to 185 (61% improvement)
  - Modernized format strings (inline variable syntax)
  - Removed redundant else blocks and improved variable naming
  - Zero behavioral changes, pure refactoring

### Fixed
- **Critical LFS issues:** Addressed race conditions and error handling in LFS operations
- **Security:** Verified SHA256 checksum for TruffleHog installer script
- **Compiler warnings:** Eliminated all clippy warnings in strict mode
- **Documentation:** Removed outdated MODULE_BOUNDARIES.md, TESTING.md, and TEST_AUDIT_REPORT.md

### Performance
- **Package manager detection:** Trait-based system enables compile-time optimization
- **Parallel analysis:** Improved concurrency in publish command planning phase

## [3.0.0] - 2025-11-12

### Breaking Changes
- **Package renamed:** `repos` → `goobits-repos` for better namespace clarity
  - Library name changed from `repos` to `goobits_repos`
  - Binary name remains `repos` (no breaking change for CLI users)
  - **Migration:** Update `Cargo.toml` dependencies and `use` statements to `goobits_repos`

### Added
- **Pull command:** New `repos pull` command with safety checks and concurrency fixes
  - Automatic conflict detection and abort on merge conflicts
  - Progress tracking with visual feedback
  - Respects `--jobs` and `--sequential` flags
- **Enhanced progress display:** Show active repository names during fetch phase
- **Slow repository warnings:** Display elapsed time warnings for repos taking >10 seconds
- **Test audit report:** Comprehensive analysis of test suite quality and coverage gaps

### Changed
- **Zero panic risk:** Eliminated all 59 `unwrap()` calls across production and test code
  - All 30 production unwraps replaced with proper error handling
  - All 29 test unwraps replaced with descriptive `expect()` messages
  - Created `path_to_str()` helper for UTF-8 safe path handling
  - Used `total_cmp()` for NaN-safe float sorting
- **Dependencies updated:** Major version updates for core dependencies
  - `tokio` 1.48 (was 1.0) - Latest async runtime with performance improvements
  - `clap` 4.5 (was 4.0) - Modern CLI argument parsing
  - `reqwest` 0.12.24 - Latest HTTP client with security fixes
  - `serde` 1.0.228 - Serialization with latest optimizations
  - `flate2` 1.1, `indicatif` 0.18, `tempfile` 3.23
- **Test coverage:** Increased from 63 to 107 tests (+70% improvement)
  - Added 32 audit system tests (scanner, hygiene, fixes modules)
  - Added 16 command tests (sync, staging operations)
  - Removed 4 redundant tests testing Rust language features
  - Coverage grade improved from D+ (38%) to B- (65%)

### Performance
- **Pipelined fetch+push:** Optimized per-repo processing architecture
- **Staggered fetch starts:** Prevent connection bursts to git remotes (improved rate limiting)
- **Overhauled progress tracking:** Eliminated stuck/hanging progress bar appearance

### Fixed
- **Zero compiler warnings:** All Rust warnings eliminated
  - Fixed AtomicU64 comparison errors in tests
  - Resolved unused import warnings
  - Added appropriate `#[allow]` attributes for test infrastructure
- **Critical safety fixes:** Phase 2A safety and correctness improvements
- **Progress bar updates:** Fixed non-verbose mode fetch phase progress display
- **Export visibility:** Fixed `find_repos_from_path` export for library usage
- **Test isolation:** Disabled commit signing in tests to prevent GPG interference

### Security
- **Error handling hardening:** Eliminated all panic-prone unwrap calls
- **Audit system testing:** 100% test coverage for critical security scanning code
- **Dependency updates:** Latest versions include security patches

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
