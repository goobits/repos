//! Configuration constants and settings

// Concurrency Configuration
//
// Different operations have different optimal concurrency limits based on their resource usage:
// - Git operations are I/O-bound and can handle higher concurrency
// - TruffleHog scanning is CPU-intensive and benefits from lower concurrency to prevent system overload
// - Hygiene checking is I/O-bound (git commands) but moderate concurrency prevents overwhelming git

// Default concurrency cap to prevent overwhelming GitHub's concurrent request limits
pub const GIT_CONCURRENT_CAP: usize = 12;

/// Determines the concurrency limit for git operations based on CLI args and system resources
///
/// Priority order:
/// 1. --sequential flag → 1
/// 2. --jobs N flag → N
/// 3. REPOS_CONCURRENCY env var → N (deprecated, shows warning)
/// 4. Smart default → min(CPU_CORES + 2, 12)
pub fn get_git_concurrency(jobs: Option<usize>, sequential: bool) -> (usize, bool) {
    // Check for sequential mode
    if sequential {
        return (1, false);
    }

    // Check explicit jobs flag
    if let Some(n) = jobs {
        return (n.max(1), false); // Ensure at least 1
    }

    // Check environment variable (deprecated)
    if let Ok(env_concurrency) = std::env::var("REPOS_CONCURRENCY") {
        if let Ok(n) = env_concurrency.parse::<usize>() {
            if n > 0 {
                eprintln!("⚠️  REPOS_CONCURRENCY environment variable is deprecated. Use --jobs N instead.");
                return (n, true); // Return with deprecation flag
            }
        }
    }

    // Smart default: CPU cores + 2, capped at 12
    let cpu_count = num_cpus::get();
    let default = (cpu_count + 2).min(GIT_CONCURRENT_CAP);
    (default, false)
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
