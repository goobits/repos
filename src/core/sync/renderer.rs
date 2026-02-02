//! HUD renderer for sync operations.

use crate::core::sync::state::{Stage, SyncState};
use crate::core::SyncStatistics;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct HudRenderer;

impl HudRenderer {
    pub fn new() -> Self {
        Self
    }

    pub fn render(&self, state: &SyncState, _stats: &SyncStatistics) -> String {
        let now = Instant::now();
        let total = state.total_repos.max(1);
        let done = count_stage(state, Stage::Done);
        let percent = (done.saturating_mul(100)) / total;
        let eta = estimate_eta(state, done, total, now);

        format!(
            "🔄 Syncing {} repos • {}% • ETA {}",
            state.total_repos, percent, eta
        )
    }
}

fn count_stage(state: &SyncState, stage: Stage) -> usize {
    state
        .repos
        .values()
        .filter(|repo| repo.stage == stage)
        .count()
}

fn estimate_eta(state: &SyncState, done: usize, total: usize, now: Instant) -> String {
    if done == 0 {
        return "--".to_string();
    }
    let elapsed = now.duration_since(state.start_time);
    let avg = elapsed.as_secs_f64() / done as f64;
    let remaining = (total.saturating_sub(done)) as f64 * avg;
    format_duration(Duration::from_secs_f64(remaining))
}

fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    let mins = secs / 60;
    let rem = secs % 60;
    if mins > 0 {
        format!("{mins}m {rem}s")
    } else {
        format!("{rem}s")
    }
}
