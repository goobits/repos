//! Common test utilities and helpers
#![allow(dead_code)]

pub mod fixtures;
pub mod git;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tokio::sync::{Mutex, MutexGuard};

static TEST_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

/// Acquires a global lock for tests that modify process-wide state (like CWD)
pub async fn lock_test() -> MutexGuard<'static, ()> {
    TEST_MUTEX.get_or_init(|| Mutex::new(())).lock().await
}

/// Restores the process working directory even when a test panics.
pub struct CurrentDirGuard {
    original: PathBuf,
}

impl CurrentDirGuard {
    pub fn enter(path: &Path) -> std::io::Result<Self> {
        let original = std::env::current_dir()?;
        std::env::set_current_dir(path)?;
        Ok(Self { original })
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}
