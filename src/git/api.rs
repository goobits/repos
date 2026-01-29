//! Public API for git operations.
//!
//! This module provides the stable public API for git-related functionality:
//! - Repository status checking and pushing
//! - User configuration management
//! - Tag creation
//!
//! ## Example: Checking for changes
//!
//! ```rust,no_run
//! use goobits_repos::git::has_uncommitted_changes;
//! use std::path::Path;
//!
//! async fn check(path: &Path) {
//!     if has_uncommitted_changes(path).await {
//!         println!("Repository has changes");
//!     }
//! }
//! ```

// Core operations
pub use super::operations::RepoVisibility;
pub use super::operations::{fetch_and_analyze, push_if_needed};
pub use super::operations::{fetch_and_analyze_for_pull, pull_if_needed};

// Status
pub use super::status::Status;

// Configuration
pub use super::config::{
    check_repo_config, get_current_user_config, get_global_user_config, validate_user_config,
    ConfigArgs, ConfigCommand, ConfigSource, PromptFn, UserConfig,
};

// Additional operations for command modules and tests
pub use super::operations::{
    commit_changes, create_and_push_tag, get_repo_visibility, get_staging_status,
    has_staged_changes, has_uncommitted_changes, stage_files, unstage_files,
};

// LFS functions - used internally by push_if_needed, exported for integration tests
#[allow(unused_imports)]
pub use super::operations::{check_uses_git_lfs, has_pending_lfs_objects, push_lfs_objects};
