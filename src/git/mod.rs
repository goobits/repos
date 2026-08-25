// Internal modules - not part of public API
pub(crate) mod ancestry;
pub(crate) mod config;
pub(crate) mod failure;
pub(crate) mod operations;
pub(crate) mod remote;
pub(crate) mod runner;
pub(crate) mod status;
pub(crate) mod worktree;

// Public API - curated exports only
pub mod api;

// Re-export key items at module level for convenience
pub use api::*;
