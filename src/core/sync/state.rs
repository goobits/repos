//! Shared synchronization state for HUD rendering.

use crate::git::Status;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Queued,
    Checking,
    Waiting,
    Updating,
    Done,
}

#[derive(Debug, Clone)]
pub struct RepoState {
    pub stage: Stage,
    pub last_update: Instant,
    pub started_at: Instant,
    pub last_op: String,
    pub status: Option<Status>,
    pub message: Option<String>,
}

impl RepoState {
    fn new(now: Instant) -> Self {
        Self {
            stage: Stage::Queued,
            last_update: now,
            started_at: now,
            last_op: "queued".to_string(),
            status: None,
            message: None,
        }
    }
}

pub struct SyncState {
    pub repos: HashMap<String, RepoState>,
    pub total_repos: usize,
    pub start_time: Instant,
    pub fetch_concurrency: usize,
    pub update_concurrency: usize,
}

impl SyncState {
    pub fn new(
        repo_names: &[String],
        total_repos: usize,
        fetch_concurrency: usize,
        update_concurrency: usize,
    ) -> Self {
        let now = Instant::now();
        let repos = repo_names
            .iter()
            .map(|name| (name.clone(), RepoState::new(now)))
            .collect();
        Self {
            repos,
            total_repos,
            start_time: now,
            fetch_concurrency,
            update_concurrency,
        }
    }

    pub fn set_stage(&mut self, repo_name: &str, stage: Stage, op: &str) {
        if let Some(repo_state) = self.repos.get_mut(repo_name) {
            repo_state.stage = stage;
            repo_state.last_update = Instant::now();
            repo_state.last_op = op.to_string();
        }
    }

    pub fn set_status(&mut self, repo_name: &str, status: Status, message: &str) {
        if let Some(repo_state) = self.repos.get_mut(repo_name) {
            repo_state.stage = Stage::Done;
            repo_state.last_update = Instant::now();
            repo_state.status = Some(status);
            repo_state.message = Some(message.to_string());
        }
    }
}
