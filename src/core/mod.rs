// Internal modules - not part of public API
pub(crate) mod attention;
pub(crate) mod config;
pub(crate) mod discovery;
pub(crate) mod progress;
pub(crate) mod report;
pub(crate) mod stats;
pub(crate) mod topology;

// Test modules
#[cfg(test)]
mod stats_tests;

// Public API - curated exports only
pub mod api;

// Re-export key items at module level for convenience
pub use api::*;
