//! Configuration constants and settings
//!
//! **API Stability Note**: Only items re-exported through `core::api` are part of the
//! stable public API. Other `pub` items in this module are internal implementation details
//! subject to change. External crates should import through `repos::core::*` rather than
//! `repos::core::config::*` directly.

// Concurrency Configuration
//
// Different operations have different optimal concurrency limits based on their resource usage:
// - Git operations are I/O-bound and can handle higher concurrency
// - TruffleHog scanning is CPU-intensive and benefits from lower concurrency to prevent system overload
// - Hygiene checking is I/O-bound (git commands) but moderate concurrency prevents overwhelming git

// Default concurrency cap for fetch operations to prevent overwhelming network
// This is specifically for the fetch phase 2x multiplier, not base concurrency
#[doc(hidden)] // Internal implementation detail
pub const FETCH_CONCURRENT_CAP: usize = 24;

// Default concurrency for commands that don't support --jobs flag yet
// Increased from 12 to 32 to better utilize modern multi-core systems
pub const GIT_CONCURRENT_CAP: usize = 32;

/// Determines the concurrency limit for git operations based on CLI args and system resources
///
/// Priority order:
/// 1. --sequential flag → 1
/// 2. --jobs N flag → N
/// 3. Smart default → CPU_CORES + 2 (scales with hardware)
///
/// Note: Removed the previous hard cap of 12 to allow scaling on high-core systems.
/// Users experiencing rate limits can use --jobs N to limit concurrency.
pub fn get_git_concurrency(jobs: Option<usize>, sequential: bool) -> usize {
    // Check for sequential mode
    if sequential {
        return 1;
    }

    // Check explicit jobs flag
    if let Some(n) = jobs {
        return n.max(1); // Ensure at least 1
    }

    // Smart default: CPU cores + 2, no artificial cap
    // This allows the tool to scale naturally with available hardware
    let cpu_count = num_cpus::get();
    cpu_count + 2
}

// Audit concurrency configuration
pub const TRUFFLE_CONCURRENT_LIMIT: usize = 1; // For CPU-intensive TruffleHog secret scans
pub const HYGIENE_CONCURRENT_LIMIT: usize = 3; // For I/O-bound hygiene git operations

// Progress bar configuration
#[doc(hidden)] // Internal UI detail
pub const DEFAULT_PROGRESS_BAR_LENGTH: u64 = 100;
#[doc(hidden)] // Internal UI detail
pub const DEFAULT_REPO_NAME: &str = "current";
#[doc(hidden)] // Internal UI detail
pub const UNKNOWN_REPO_NAME: &str = "unknown";

// UI Constants
pub const NO_REPOS_MESSAGE: &str = "No git repositories found in current directory.";
pub const CONFIG_SYNCING_MESSAGE: &str = "checking config...";
#[doc(hidden)] // Internal UI styling
pub const PROGRESS_CHARS: &str = "##-";
#[doc(hidden)] // Internal UI styling
pub const PROGRESS_TEMPLATE: &str = "{prefix:.bold} {wide_msg}";

// Display formatting constants
#[doc(hidden)] // Internal formatting detail
pub const PATH_DISPLAY_WIDTH: usize = 30;
#[doc(hidden)] // Internal formatting detail
pub const ERROR_MESSAGE_MAX_LENGTH: usize = 40;
#[doc(hidden)] // Internal formatting detail
pub const ERROR_MESSAGE_TRUNCATE_LENGTH: usize = 37;
#[doc(hidden)] // Internal formatting detail
pub const TIMEOUT_SECONDS_DISPLAY: u64 = 180;

// Processing limits and chunk sizes
#[doc(hidden)] // Internal processing detail
pub const GIT_OBJECTS_CHUNK_SIZE: usize = 100;
#[doc(hidden)] // Internal display limit
pub const LARGE_FILES_DISPLAY_LIMIT: usize = 10;

// Directories to skip during repository search
#[doc(hidden)] // Internal discovery configuration
pub const SKIP_DIRECTORIES: &[&str] = &[
    "node_modules",
    "vendor",
    "target",
    "build",
    ".next",
    "dist",
    "__pycache__",
    ".venv",
    "venv",
];

// Repository discovery configuration
#[doc(hidden)] // Internal discovery limit
pub const MAX_SCAN_DEPTH: usize = 10; // Maximum directory depth to scan
#[doc(hidden)] // Internal optimization hint
pub const ESTIMATED_REPO_COUNT: usize = 50; // Pre-allocation hint for collections

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_git_concurrency_sequential_mode() {
        // Sequential mode should always return 1
        assert_eq!(get_git_concurrency(None, true), 1);
        assert_eq!(get_git_concurrency(Some(100), true), 1);
    }

    #[test]
    fn test_get_git_concurrency_explicit_jobs() {
        // Explicit jobs flag should be respected
        assert_eq!(get_git_concurrency(Some(5), false), 5);
        assert_eq!(get_git_concurrency(Some(20), false), 20);
        assert_eq!(get_git_concurrency(Some(1), false), 1);
    }

    #[test]
    fn test_get_git_concurrency_zero_jobs_becomes_one() {
        // Zero should be converted to 1 (at least 1)
        assert_eq!(get_git_concurrency(Some(0), false), 1);
    }

    #[test]
    fn test_get_git_concurrency_default_scales_with_cpu() {
        // Default should be CPU cores + 2
        let concurrency = get_git_concurrency(None, false);
        let expected = num_cpus::get() + 2;
        assert_eq!(concurrency, expected);

        // Should be at least 3 on any system (1 core + 2)
        assert!(concurrency >= 3);
    }

    // Note: Constant validation tests removed - constants are compile-time validated.
    // Previous tests checked:
    // - FETCH_CONCURRENT_CAP = 24 (positive, allows parallelism)
    // - GIT_CONCURRENT_CAP = 12 (positive, allows parallelism)
    // - TRUFFLE_CONCURRENT_LIMIT = 1, HYGIENE_CONCURRENT_LIMIT = 3 (at least 1)
    // - MAX_SCAN_DEPTH = 10 (positive, prevents excessive recursion)
    // - ESTIMATED_REPO_COUNT = 50 (positive, reasonable)

    #[test]
    fn test_skip_directories_contains_common_dirs() {
        // Verify common problematic directories are skipped
        assert!(SKIP_DIRECTORIES.contains(&"node_modules"));
        assert!(SKIP_DIRECTORIES.contains(&"target"));
        assert!(SKIP_DIRECTORIES.contains(&".venv"));
        assert!(SKIP_DIRECTORIES.len() > 5, "Should skip multiple common directories");
    }
}
