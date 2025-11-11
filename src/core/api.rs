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
pub use super::progress::{GenericProcessingContext, ProcessingContext, create_processing_context, create_generic_processing_context};
pub use super::stats::SyncStatistics;

// Discovery
pub use super::discovery::init_command;
#[allow(unused_imports)] // Used by integration tests
pub use super::discovery::find_repos_from_path;

// Configuration
pub use super::config::GIT_CONCURRENT_CAP;
pub use super::config::{TRUFFLE_CONCURRENT_LIMIT, HYGIENE_CONCURRENT_LIMIT};

// User-facing messages
pub use super::config::{NO_REPOS_MESSAGE, CONFIG_SYNCING_MESSAGE};

// Terminal utilities (re-exported from utils)
pub use crate::utils::{set_terminal_title, set_terminal_title_and_flush};

// Internal helpers for command modules
pub(crate) use super::progress::{
    create_progress_bar, acquire_stats_lock,
    create_separator_progress_bar, create_footer_progress_bar,
    acquire_semaphore_permit
};
pub(crate) use super::stats::clean_error_message;
