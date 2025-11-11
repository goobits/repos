# Architecture Overview

This document describes the architecture of the `repos` CLI tool and provides guidelines for maintaining clean module boundaries.

## Module Structure

The codebase follows a **3-layer architecture** to ensure clear dependencies and maintainability:

```
┌─────────────────────────────────────────────────┐
│  Layer 3: Commands (commands/*)                 │
│  - CLI command handlers                         │
│  - UI/Progress bars orchestration               │
│  - User interaction                             │
└─────────────────┬───────────────────────────────┘
                  │ depends on ↓
┌─────────────────▼───────────────────────────────┐
│  Layer 2: Core Business Logic (core/*)          │
│  - Repository discovery                         │
│  - Processing context management                │
│  - Statistics tracking                          │
│  - Configuration                                │
└─────────────────┬───────────────────────────────┘
                  │ depends on ↓
┌─────────────────▼───────────────────────────────┐
│  Layer 1: Infrastructure (git/*, utils/*, etc.) │
│  - Git operations                               │
│  - File system utilities                        │
│  - Terminal I/O                                 │
│  - Package management (package/*)               │
│  - Security audit (audit/*)                     │
│  - Subrepo management (subrepo/*)               │
└─────────────────────────────────────────────────┘
```

## Dependency Rules

**Allowed:**
- ✅ Layer 3 (commands) can import from Layer 2 (core) and Layer 1 (infrastructure)
- ✅ Layer 2 (core) can import from Layer 1 (infrastructure)
- ✅ Layer 1 modules can use each other (they're parallel - no hierarchy within Layer 1)

**Prohibited:**
- ❌ Layer 1 cannot import from Layer 2 or 3
- ❌ Layer 2 cannot import from Layer 3
- ❌ Circular dependencies between any modules

## Module Public APIs

Each major module exposes a curated public API through either:
1. **Explicit re-exports** in `mod.rs` (for small modules)
2. **API facade** in `api.rs` (for large modules like core/, git/)

### Core Module (`src/core/`)

**Public API** (via `core/api.rs`):
- `ProcessingContext`, `GenericProcessingContext` - Context for concurrent operations
- `create_processing_context()` - Constructor
- `SyncStatistics` - Statistics tracking
- `find_repos()`, `init_command()` - Repository discovery
- `get_git_concurrency()` - Concurrency configuration
- Constants: `NO_REPOS_MESSAGE`, `CONFIG_SYNCING_MESSAGE`, `GIT_CONCURRENT_CAP`

**Internal** (pub(crate), for command modules only):
- `create_progress_bar()`, `create_progress_style()` - UI helpers
- `acquire_stats_lock()` - Synchronization helper
- `create_separator_progress_bar()`, `create_footer_progress_bar()` - UI helpers
- `clean_error_message()` - Error formatting
- Display constants (formatting details)

### Git Module (`src/git/`)

**Public API** (via `git/api.rs`):
- `Status` - Repository status enum
- `check_repo()`, `fetch_and_analyze()`, `push_if_needed()` - Main operations
- `run_git()` - Low-level git command execution
- `FetchResult`, `RepoVisibility` - Result types
- Configuration types: `UserConfig`, `ConfigSource`, `ConfigCommand`, `ConfigArgs`
- Config validation: `validate_user_config()`, `is_valid_email()`, `is_valid_name()`

**Internal** (pub(crate), for command modules only):
- Staging operations: `stage_files()`, `unstage_files()`, `get_staging_status()`
- Commit operations: `has_staged_changes()`, `commit_changes()`, `has_uncommitted_changes()`
- Publishing: `create_and_push_tag()`, `get_repo_visibility()`
- Config accessors: `get_git_config()`, `set_git_config()`

### Utils Module (`src/utils/`)

**Public API**:
- `shorten_path()` - Path display formatting
- `set_terminal_title()`, `set_terminal_title_and_flush()` - Terminal control

**Internal**: All implementation details

### Other Modules

`audit/`, `package/`, `subrepo/` - These are domain-specific modules at Layer 1. They expose their APIs directly through their mod.rs files.

## Visibility Guidelines

When adding new code, follow these rules:

### Default to Private
```rust
// Default: private to the module
fn helper_function() { }

// If needed by other modules in same file
pub(super) fn parent_helper() { }

// If needed by other files in same crate (e.g., command modules)
pub(crate) fn internal_api() { }

// Only if truly part of public library API
pub fn public_api() { }
```

### Adding Public APIs

To add a new public API item:

1. **For core module**: Add to `src/core/api.rs`
2. **For git module**: Add to `src/git/api.rs`
3. **For utils**: Add explicit re-export in `src/utils/mod.rs`
4. **Document why** it needs to be public

### Checking Module Boundaries

Run these commands to validate architecture:

```bash
# Should find NO wildcard re-exports (except api.rs files)
rg "pub use.*\*" src/ --type rust | grep -v "api::*"

# Should find many pub(crate) items (50+)
rg "pub\(crate\)" src/ | wc -l

# Should find api.rs files for major modules
ls src/*/api.rs
```

## Testing Philosophy

- **Layer 1 (Infrastructure)**: Unit test pure functions in isolation
- **Layer 2 (Core)**: Unit test business logic with mocked infrastructure
- **Layer 3 (Commands)**: Integration tests with real repositories

The visibility model enables this:
- `pub(crate)` allows commands to import helpers for integration tests
- Private internals can be tested via public APIs
- Clear boundaries make mocking easier

## Migration from Old Structure

**Before** (wildcard re-exports everywhere):
```rust
// src/core/mod.rs
pub use config::*;  // Exports everything!
pub use stats::*;
```

**After** (explicit API):
```rust
// src/core/mod.rs
pub(crate) mod config;  // Module is internal
pub mod api;            // Public API facade
pub use api::*;         // Re-export curated items
```

This provides:
- ✅ Clear API surface (only what's in api.rs is public)
- ✅ Safe refactoring (change internals without breaking consumers)
- ✅ Better IDE support (autocomplete shows curated API)
- ✅ AI navigability (clear entry points)

## AI/Tool Navigation

For AI code assistants and analysis tools:

- **Start here**: `src/lib.rs` shows all top-level modules
- **Public APIs**: Check `*/api.rs` files for public exports
- **Command entry points**: `src/commands/*.rs` files
- **Core business logic**: `src/core/` (discovery, processing, stats)
- **Git operations**: `src/git/` (wraps git commands)

## Maintenance

### When refactoring:
1. Keep the 3-layer architecture
2. Don't introduce upward dependencies
3. Update this document if adding new layers/modules
4. Run validation commands after changes

### When adding features:
1. Identify which layer it belongs to
2. Add implementation in that layer
3. Expose through API if needed by upper layers
4. Add tests at appropriate layer

### Red flags:
- ⚠️ Import from Layer 1 to Layer 2 (business logic depending on UI)
- ⚠️ Circular dependencies between modules
- ⚠️ Everything marked `pub` (no encapsulation)
- ⚠️ Wildcard re-exports (flattens boundaries)

---

Last updated: 2025-01-XX (Phase 1 refactoring complete)
