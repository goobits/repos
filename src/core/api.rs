//! Public API for the core module.
//!
//! This module provides the stable public API for core functionality including:
//! - Repository discovery
//! - Processing context management
//! - Statistics tracking
//! - Configuration utilities
//!
//! Internal implementation details are not exposed through this API.

// Core types
pub use super::progress::{
    create_generic_processing_context, create_processing_context, GenericProcessingContext,
    ProcessingContext,
};
pub use super::stats::SyncStatistics;

// Discovery
#[allow(unused_imports)] // Used by integration tests
pub use super::discovery::find_repos_from_path;
pub use super::discovery::init_command;

// Configuration
pub use super::config::GIT_CONCURRENT_CAP;
pub use super::config::{HYGIENE_CONCURRENT_LIMIT, TRUFFLE_CONCURRENT_LIMIT};

// User-facing messages
pub use super::config::{CONFIG_SYNCING_MESSAGE, NO_REPOS_MESSAGE};

// Terminal utilities (re-exported from utils)
pub use crate::utils::{set_terminal_title, set_terminal_title_and_flush};

// Internal helpers for command modules
pub(crate) use super::progress::{
    acquire_semaphore_permit, acquire_stats_lock, create_footer_progress_bar, create_progress_bar,
    create_separator_progress_bar,
};
pub(crate) use super::stats::clean_error_message;
