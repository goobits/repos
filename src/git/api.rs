//! Public API for git operations.
//!
//! This module provides the stable public API for git-related functionality:
//! - Repository status checking and pushing
//! - User configuration management
//! - Git command execution
//! - Tag creation and publishing
//!
//! Internal staging operations remain internal (pub(crate)) for command modules.

// Core operations
pub use super::operations::{check_repo, fetch_and_analyze, push_if_needed, run_git};
pub use super::operations::{FetchResult, RepoVisibility};

// Status
pub use super::status::Status;

// Configuration
pub use super::config::{
    ConfigArgs, ConfigCommand, ConfigSource, UserConfig,
    validate_user_config, is_valid_email, is_valid_name,
    get_current_user_config, get_global_user_config, check_repo_config
};

// Additional operations for command modules and tests
pub use super::operations::{
    has_uncommitted_changes, create_and_push_tag, get_repo_visibility,
    stage_files, unstage_files, get_staging_status, has_staged_changes, commit_changes
};
