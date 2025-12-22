//! Common test utilities and helpers
#![allow(dead_code, unused_imports)]

pub mod fixtures;
pub mod git;

pub use self::fixtures::TestRepoBuilder;
pub use self::git::{create_multiple_repos, is_git_available, setup_git_repo};

use std::sync::{Mutex, MutexGuard};
use std::sync::OnceLock;

static TEST_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

/// Acquires a global lock for tests that modify process-wide state (like CWD)
pub fn lock_test() -> MutexGuard<'static, ()> {
    TEST_MUTEX.get_or_init(|| Mutex::new(())).lock().unwrap()
}
