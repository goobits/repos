//! Common test utilities and helpers
//!
//! This module provides shared functionality for integration tests,
//! reducing duplication and improving test maintainability.

pub mod git;
pub mod fixtures;

#[allow(unused_imports)]
pub use git::{is_git_available, setup_git_repo, create_multiple_repos};
pub use fixtures::TestRepoBuilder;
