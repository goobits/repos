//! Common test utilities and helpers
//!
//! This module provides shared functionality for integration tests,
//! reducing duplication and improving test maintainability.

pub mod git;
pub mod fixtures;

pub use git::{setup_git_repo, create_test_commit, create_multiple_repos};
pub use fixtures::{TestRepo, TestRepoBuilder};
