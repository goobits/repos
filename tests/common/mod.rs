//! Common test utilities and helpers
//!
//! This module provides shared functionality for integration tests,
//! reducing duplication and improving test maintainability.

pub mod fixtures;
pub mod git;

pub use fixtures::TestRepoBuilder;
#[allow(unused_imports)]
pub use git::{create_multiple_repos, is_git_available, setup_git_repo};
