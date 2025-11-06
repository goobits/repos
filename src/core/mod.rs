// Internal modules - not part of public API
pub(crate) mod config;
pub(crate) mod discovery;
pub(crate) mod progress;
pub(crate) mod stats;

// Test modules
#[cfg(test)]
mod stats_tests;

// Public API - curated exports only
pub mod api;

// Re-export key items at module level for convenience
pub use api::*;
