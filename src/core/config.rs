//! Configuration constants and settings

// Concurrency Configuration
//
// Different operations have different optimal concurrency limits based on their resource usage:
// - Git operations are I/O-bound and can handle higher concurrency
// - TruffleHog scanning is CPU-intensive and benefits from lower concurrency to prevent system overload
// - Hygiene checking is I/O-bound (git commands) but moderate concurrency prevents overwhelming git
pub const GIT_CONCURRENT_LIMIT: usize = 5; // For I/O-bound git operations (push, pull, fetch)

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
