//! Git status enumeration and utilities

/// Status enum representing the result of git operations
#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// Files successfully staged
    Staged,
    /// Files successfully unstaged
    Unstaged,
    /// Staging operation failed
    StagingError,
    /// No files matched pattern
    NoChanges,
    /// Commit was successful
    Committed,
    /// Commit operation failed
    CommitError,
    /// Successfully pulled commits from remote
    Pulled,
    /// Pull failed due to conflicts or other errors
    PullError,
    /// Repository has local worktree changes
    Dirty,
}

impl Status {
    /// Returns the emoji symbol for this status
    #[must_use]
    pub fn symbol(&self) -> &str {
        match self {
            Status::Synced
            | Status::Pushed
            | Status::ConfigSynced
            | Status::ConfigUpdated
            | Status::Staged
            | Status::Unstaged
            | Status::Committed
            | Status::Pulled => "🟢",
            Status::Skip | Status::NoRemote | Status::ConfigSkipped | Status::NoChanges => "🟠",
            Status::NoUpstream | Status::Dirty => "🟡",
            Status::Error
            | Status::ConfigError
            | Status::StagingError
            | Status::CommitError
            | Status::PullError => "🔴",
        }
    }

    /// Returns the text representation of this status
    #[must_use]
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
            Status::Staged => "staged",
            Status::Unstaged => "unstaged",
            Status::StagingError => "failed",
            Status::NoChanges => "no-changes",
            Status::Committed => "committed",
            Status::CommitError => "failed",
            Status::Pulled => "pulled",
            Status::PullError => "pull-failed",
            Status::Dirty => "dirty",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_symbol_green_success_states() {
        // All success states should have green circle
        assert_eq!(Status::Synced.symbol(), "🟢");
        assert_eq!(Status::Pushed.symbol(), "🟢");
        assert_eq!(Status::ConfigSynced.symbol(), "🟢");
        assert_eq!(Status::ConfigUpdated.symbol(), "🟢");
        assert_eq!(Status::Staged.symbol(), "🟢");
        assert_eq!(Status::Unstaged.symbol(), "🟢");
        assert_eq!(Status::Committed.symbol(), "🟢");
    }

    #[test]
    fn test_status_symbol_orange_skip_states() {
        // Skip/no-op states should have orange circle
        assert_eq!(Status::Skip.symbol(), "🟠");
        assert_eq!(Status::NoRemote.symbol(), "🟠");
        assert_eq!(Status::ConfigSkipped.symbol(), "🟠");
        assert_eq!(Status::NoChanges.symbol(), "🟠");
    }

    #[test]
    fn test_status_symbol_yellow_warning_states() {
        // Warning states should have yellow circle
        assert_eq!(Status::NoUpstream.symbol(), "🟡");
    }

    #[test]
    fn test_status_symbol_red_error_states() {
        // All error states should have red circle
        assert_eq!(Status::Error.symbol(), "🔴");
        assert_eq!(Status::ConfigError.symbol(), "🔴");
        assert_eq!(Status::StagingError.symbol(), "🔴");
        assert_eq!(Status::CommitError.symbol(), "🔴");
    }

    #[test]
    fn test_status_text_git_operations() {
        // Test git operation status text representations
        assert_eq!(Status::Synced.text(), "synced");
        assert_eq!(Status::Pushed.text(), "pushed");
        assert_eq!(Status::Skip.text(), "skip");
        assert_eq!(Status::NoUpstream.text(), "no-upstream");
        assert_eq!(Status::NoRemote.text(), "skip");
        assert_eq!(Status::Error.text(), "failed");
    }

    #[test]
    fn test_status_text_config_operations() {
        // Test config operation status text representations
        assert_eq!(Status::ConfigSynced.text(), "config-ok");
        assert_eq!(Status::ConfigUpdated.text(), "config-updated");
        assert_eq!(Status::ConfigSkipped.text(), "config-skip");
        assert_eq!(Status::ConfigError.text(), "config-failed");
    }

    #[test]
    fn test_status_text_staging_operations() {
        // Test staging operation status text representations
        assert_eq!(Status::Staged.text(), "staged");
        assert_eq!(Status::Unstaged.text(), "unstaged");
        assert_eq!(Status::StagingError.text(), "failed");
        assert_eq!(Status::NoChanges.text(), "no-changes");
    }

    #[test]
    fn test_status_text_commit_operations() {
        // Test commit operation status text representations
        assert_eq!(Status::Committed.text(), "committed");
        assert_eq!(Status::CommitError.text(), "failed");
    }
}
