//! # goobits-repos
//!
//! `goobits-repos` is a high-performance library for managing and synchronizing 
//! multiple Git repositories concurrently. It powers the `repos` CLI tool.
//!
//! ## Core Features
//!
//! - **Fast Discovery**: Parallel repository scanning using `ignore` and `rayon`.
//! - **Concurrent Operations**: Push, pull, and fetch operations across hundreds of repos.
//! - **Package Management**: Automated publishing for npm, Cargo, and PyPI.
//! - **Security Auditing**: Secret scanning and repository hygiene checks.
//! - **Subrepo Management**: Drift detection and synchronization for nested repositories.
//!
//! ## Example
//!
//! ```rust,no_run
//! use goobits_repos::core::find_repos_from_path;
//!
//! #[tokio::main]
//! async fn main() {
//!     let repos = find_repos_from_path(".");
//!     for (name, path) in repos {
//!         println!("{}: {}", name, path.display());
//!     }
//! }
//! ```

pub mod audit;
pub mod commands;
pub mod core;
pub mod git;
pub mod package;
pub mod subrepo;
pub mod utils;

