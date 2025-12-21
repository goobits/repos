//! Hygiene rules and constants

// Universal patterns that should never be committed to git
pub const UNIVERSAL_BAD_PATTERNS: &[&str] = &[
    "node_modules/",
    "vendor/",
    "dist/",
    "build/",
    "target/debug/",
    "target/release/",
    ".env",
    "*.log",
    ".DS_Store",
    "Thumbs.db",
    "*.tmp",
    "*.cache",
    "__pycache__/",
    ".venv/",
    ".idea/",
    ".vscode/settings.json",
    "*.key",
    "*.pem",
    "*.p12",
    "*.jks",
];

// Large file threshold in bytes (1MB)
pub const LARGE_FILE_THRESHOLD: u64 = 1_048_576;
