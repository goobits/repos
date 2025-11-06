//! Configuration constants and settings

// Concurrency Configuration
//
// Different operations have different optimal concurrency limits based on their resource usage:
// - Git operations are I/O-bound and can handle higher concurrency
// - TruffleHog scanning is CPU-intensive and benefits from lower concurrency to prevent system overload
// - Hygiene checking is I/O-bound (git commands) but moderate concurrency prevents overwhelming git

// Default concurrency cap for fetch operations to prevent overwhelming network
// This is specifically for the fetch phase 2x multiplier, not base concurrency
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
pub const DEFAULT_PROGRESS_BAR_LENGTH: u64 = 100;
pub const DEFAULT_REPO_NAME: &str = "current";
pub const UNKNOWN_REPO_NAME: &str = "unknown";

// UI Constants
pub const NO_REPOS_MESSAGE: &str = "No git repositories found in current directory.";
pub const CONFIG_SYNCING_MESSAGE: &str = "checking config...";
pub const PROGRESS_CHARS: &str = "##-";
pub const PROGRESS_TEMPLATE: &str = "{prefix:.bold} {wide_msg}";

// Display formatting constants
pub const PATH_DISPLAY_WIDTH: usize = 30;
pub const ERROR_MESSAGE_MAX_LENGTH: usize = 40;
pub const ERROR_MESSAGE_TRUNCATE_LENGTH: usize = 37;
pub const TIMEOUT_SECONDS_DISPLAY: u64 = 180;

// Processing limits and chunk sizes
pub const GIT_OBJECTS_CHUNK_SIZE: usize = 100;
pub const LARGE_FILES_DISPLAY_LIMIT: usize = 10;

// Directories to skip during repository search
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
pub const MAX_SCAN_DEPTH: usize = 10; // Maximum directory depth to scan
pub const ESTIMATED_REPO_COUNT: usize = 50; // Pre-allocation hint for collections
