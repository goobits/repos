//! Git status enumeration and utilities

/// Status enum representing the result of git operations
#[derive(Clone, Debug)]
pub enum Status {
    /// Repository is already up to date with remote
    Synced,
    /// Repository had commits that were successfully pushed
    Pushed,
    /// Repository was skipped (no remote, detached HEAD, etc.)
    Skip,
    /// Repository has no upstream tracking branch
    NoUpstream,
    /// Repository has no remote configured
    NoRemote,
    /// An error occurred during synchronization
    Error,
    /// Config was already synced
    ConfigSynced,
    /// Config was updated
    ConfigUpdated,
    /// Config operation was skipped
    ConfigSkipped,
    /// Config operation failed
    ConfigError,
}

impl Status {
    /// Returns the emoji symbol for this status
    pub fn symbol(&self) -> &str {
        match self {
            Status::Synced | Status::Pushed | Status::ConfigSynced | Status::ConfigUpdated => "🟢",
            Status::Skip | Status::NoRemote | Status::ConfigSkipped => "🟠",
            Status::NoUpstream => "🟡",
            Status::Error | Status::ConfigError => "🔴",
        }
    }

    /// Returns the text representation of this status
    pub fn text(&self) -> &str {
        match self {
            Status::Synced => "synced",
            Status::Pushed => "pushed",
            Status::Skip => "skip",
            Status::NoUpstream => "no-upstream",
            Status::NoRemote => "skip",
            Status::Error => "failed",
            Status::ConfigSynced => "config-ok",
            Status::ConfigUpdated => "config-updated",
            Status::ConfigSkipped => "config-skip",
            Status::ConfigError => "config-failed",
        }
    }
}