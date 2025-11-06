# Testing Infrastructure

## Overview

This document describes the comprehensive testing infrastructure for the `repos` tool. The test suite has been significantly improved to provide excellent coverage of critical functionality and recent performance optimizations.

## Test Structure

```
tests/
├── common/                     # Shared test utilities
│   ├── mod.rs                 # Module declarations
│   ├── git.rs                 # Git test helpers
│   └── fixtures.rs            # Test data builders
├── test_discovery.rs          # Repository discovery tests (10 tests)
└── integration_tests.rs       # General integration tests (13 tests)

src/
├── core/discovery.rs          # + 3 unit tests for DashMap
├── package/mod.rs             # + 10 unit tests for package detection
└── git/operations.rs          # + 2 unit tests for optimizations
```

## Test Coverage

### Integration Tests

#### Repository Discovery (`test_discovery.rs`) - 10 tests
- ✅ Single repository discovery
- ✅ Multiple repositories discovery
- ✅ Duplicate name handling (suffix generation)
- ✅ Skipping node_modules directories
- ✅ Max depth limit enforcement
- ✅ Symlink handling
- ✅ Current directory as repository
- ✅ Alphabetical sorting
- ✅ Parallel processing correctness
- ✅ Deduplication logic

#### Git Operations (`integration_tests.rs`) - 13 tests
- ✅ Staging files (single file, patterns, wildcards)
- ✅ Unstaging files
- ✅ Checking staged changes
- ✅ Committing changes
- ✅ Error scenarios (non-existent files, empty commits)
- ✅ Repository visibility detection
- ✅ Uncommitted changes detection
- ✅ Tag creation and pushing
- ✅ Statistics tracking

### Unit Tests

#### DashMap Concurrent Access (`src/core/discovery.rs`) - 3 tests
- ✅ Concurrent inserts from multiple threads
- ✅ No race conditions in atomic operations
- ✅ Path deduplication correctness

#### Package Manager Detection (`src/package/mod.rs`) - 10 tests
- ✅ NPM package detection (package.json)
- ✅ Cargo package detection (Cargo.toml)
- ✅ PyPI package detection (pyproject.toml)
- ✅ PyPI package detection (setup.py)
- ✅ No package detection
- ✅ Priority handling (npm over cargo)
- ✅ Async detection (npm, cargo, none)
- ✅ Sync vs async consistency

#### String Allocation Optimization (`src/git/operations.rs`) - 2 tests
- ✅ Empty string handling (no unnecessary allocation)
- ✅ Whitespace trimming
- ✅ Content preservation

## Test Utilities

### Common Helpers

#### `setup_git_repo(path: &Path) -> Result<()>`
Initializes a git repository with test user configuration.

#### `create_test_commit(path, file_name, content, message) -> Result<()>`
Creates a commit with specified file and message.

#### `create_multiple_repos(parent_dir, count) -> Result<Vec<String>>`
Creates multiple test repositories in a directory.

#### `is_git_available() -> bool`
Checks if git is available in the system.

### Test Builders

#### `TestRepoBuilder`
Fluent API for creating test repositories:

```rust
let repo = TestRepoBuilder::new("my-repo")
    .with_github_remote("https://github.com/user/repo.git")
    .with_npm_package("my-package", "1.0.0")
    .with_commits(3)
    .build()?;
```

Supported configurations:
- GitHub remotes
- npm packages (package.json)
- Cargo packages (Cargo.toml)
- Python packages (pyproject.toml)
- Multiple commits

#### `TestRepo`
Test repository with automatic cleanup:

```rust
let repo = TestRepoBuilder::new("test").build()?;
repo.create_file("README.md", "# Test")?;
repo.create_package_json("my-pkg", "1.0.0")?;
repo.commit_all("Add package")?;
// Automatically cleaned up when dropped
```

## Running Tests

### Run All Tests
```bash
cargo test
```

### Run Integration Tests Only
```bash
cargo test --test '*'
```

### Run Unit Tests Only
```bash
cargo test --lib
```

### Run Specific Test File
```bash
cargo test --test test_discovery
```

### Run With Output
```bash
cargo test -- --nocapture
```

## Test Quality Standards

### ✅ Good Practices Implemented

1. **Test Independence**: Each test uses `TempDir` for isolation
2. **Clear Naming**: Descriptive test function names
3. **Proper Cleanup**: Automatic cleanup via RAII (TempDir)
4. **No Shared State**: Each test is completely independent
5. **Helper Functions**: Common setup code extracted to helpers
6. **Builder Pattern**: Fluent API for test data creation

### 🎯 Testing Philosophy

1. **Test Behavior, Not Implementation**: Focus on what the code does, not how
2. **One Concept Per Test**: Each test validates a single behavior
3. **Clear Assertions**: Assertions include descriptive messages
4. **Edge Cases**: Comprehensive coverage of edge cases and error paths
5. **No Framework Testing**: Don't test Rust's derive macros or standard library

## Coverage Summary

| Module | Integration Tests | Unit Tests | Coverage |
|--------|------------------|-----------|----------|
| **Repository Discovery** | 10 | 3 | ✅ Excellent |
| **Git Operations** | 13 | 2 | ✅ Good |
| **Package Detection** | 0 | 10 | ✅ Excellent |
| **Staging/Commit** | 6 | 0 | ✅ Good |
| **Performance Optimizations** | 0 | 5 | ✅ Good |

**Total Tests**: 36 tests

## Performance Optimizations Testing

All critical performance optimizations from the recent commits are now tested:

1. **✅ DashMap Concurrent Access** (discovery.rs)
   - Concurrent inserts
   - Atomic operations
   - Path deduplication

2. **✅ Async Package Detection** (package/mod.rs)
   - All package managers
   - Sync/async consistency
   - Error handling

3. **✅ String Allocation Optimization** (git/operations.rs)
   - Empty string handling
   - Trimming behavior

4. **✅ Parallel Discovery** (test_discovery.rs)
   - Correctness with multiple threads
   - Deduplication
   - Sorting

## Removed Tests

- ❌ `test_repo_visibility_enum` - Was testing Rust's derive macros, not application logic

## Future Test Coverage

### High Priority (Not Yet Implemented)

1. **Push Command Integration Tests**
   - End-to-end push flow
   - Rate limit handling
   - Error recovery

2. **Publish Command Integration Tests**
   - Combined parallel analysis
   - Visibility filtering
   - Dry-run mode

3. **Config Sync Tests**
   - Config propagation
   - Force mode
   - Dry-run

### Medium Priority

4. **Performance Regression Tests**
   - 100+ repo scenarios
   - Timing assertions
   - Concurrency scaling

5. **Error Recovery Tests**
   - Partial failures
   - Network timeouts
   - Rollback scenarios

## Contributing Tests

When adding tests:

1. Use `TestRepoBuilder` for test repositories
2. Use common helpers from `tests/common/`
3. Add integration tests to appropriate file or create new test file
4. Add unit tests in `#[cfg(test)]` modules in source files
5. Include descriptive assertion messages
6. Test both happy path and error cases

## Test Performance

Expected test run time (with git available):
- **Fast unit tests** (<1s): 15 tests
- **Medium git operations** (1-3s): 8 tests
- **Slow integration tests** (3-5s): 13 tests

**Total estimated time**: 30-40 seconds

Tests run in parallel by default via cargo's test runner.
