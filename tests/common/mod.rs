//! Common test utilities and helpers
#![allow(dead_code)]

pub mod fixtures;
pub mod git;

use std::sync::OnceLock;
use tokio::sync::{Mutex, MutexGuard};

static TEST_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

/// Serializes tests that exercise shared in-process configuration and caches.
pub async fn lock_test() -> MutexGuard<'static, ()> {
    TEST_MUTEX.get_or_init(|| Mutex::new(())).lock().await
}
