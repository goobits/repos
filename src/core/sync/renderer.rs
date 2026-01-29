//! HUD renderer for sync operations.

use crate::core::config::SLOW_REPO_THRESHOLD_SECS;
use crate::core::sync::state::{Stage, SyncMode, SyncState};
use crate::core::SyncStatistics;
use crate::git::Status;
use std::time::{Duration, Instant};

const STALL_THRESHOLD_SECS: u64 = 120;
const BAR_WIDTH: usize = 20;

#[derive(Clone)]
pub struct HudRenderer {
    mode: SyncMode,
}

impl HudRenderer {
    pub fn new(mode: SyncMode) -> Self {
        Self { mode }
    }

    pub fn render(&self, state: &SyncState, _stats: &SyncStatistics) -> String {
        let now = Instant::now();
        let total = state.total_repos.max(1);
        let done = count_stage(state, Stage::Done);
        let percent = (done.saturating_mul(100)) / total;
        let activity_bar = render_bar(percent);

        let checking = count_stage(state, Stage::Checking);
        let updating = count_stage(state, Stage::Updating);
        let waiting = count_stage(state, Stage::Waiting);
        let queued = count_stage(state, Stage::Queued);
        let pending = waiting + queued;

        let (identical, pushed, pulled, errored) = count_statuses(state);
        let latent = count_latent(state, now);

        let stalled = collect_stalled(state, now);
        let held = collect_waiting(state, now);

        let eta = estimate_eta(state, done, total, now);
        let rate = rate_per_sec(state, done, now);

        let mode_label = match self.mode {
            SyncMode::Pull => "[PULL]",
            SyncMode::Push => "[PUSH]",
        };

        let mut lines = Vec::new();
        lines.push(format!(
            "  🔄  Syncing {} Repositories — {}",
            state.total_repos, mode_label
        ));
        lines.push("  ───────────────────────────────────────────────────────────────────────────".to_string());
        lines.push(format!(
            "  ACTIVITY    [{}]  {}%",
            activity_bar, percent
        ));
        lines.push(format!(
            "  CAPACITY    📡 {} Scanning  •  📤 {} Uploading  •  📥 {} Writing  •  ⏳ {} Pending",
            checking,
            if self.mode == SyncMode::Push { updating } else { 0 },
            if self.mode == SyncMode::Pull { updating } else { 0 },
            pending
        ));

        lines.push(String::new());
        lines.push("  ──  CRITICAL SIGNALS  ─────────────────────────────────────────────────────".to_string());
        if let Some(signal) = stalled.first() {
            lines.push(format!(
                "  🟠  STALLED   {} ({})",
                signal.repo, signal.phase
            ));
            lines.push(format!(
                "                └─ {} idle  •  {}",
                format_duration(signal.idle),
                signal.last_op
            ));
        } else {
            lines.push("  🟢  STALLED   none".to_string());
        }

        if let Some(held_signal) = held.first() {
            lines.push(format!(
                "  🔵  HELD      {} ({})",
                held_signal.repo, held_signal.phase
            ));
            lines.push(format!(
                "                └─ {} waiting  •  {}",
                format_duration(held_signal.idle),
                held_signal.last_op
            ));
        } else {
            lines.push("  🔵  HELD      none".to_string());
        }

        lines.push(String::new());
        lines.push("  ──  INVENTORY  ────────────────────────────────────────────────────────────".to_string());
        lines.push(format!(
            "  🟢 {} Identical   📤 {} Pushed   📥 {} Pulled   🐢 {} Latent   🔴 {} Errored",
            identical, pushed, pulled, latent, errored
        ));

        lines.push("  ───────────────────────────────────────────────────────────────────────────".to_string());
        lines.push(format!(
            "  ETA {}  •  {:.2} r/s",
            eta, rate
        ));

        lines.join("\n")
    }
}

struct Signal {
    repo: String,
    phase: String,
    idle: Duration,
    last_op: String,
}

fn count_stage(state: &SyncState, stage: Stage) -> usize {
    state
        .repos
        .values()
        .filter(|repo| repo.stage == stage)
        .count()
}

fn count_statuses(state: &SyncState) -> (usize, usize, usize, usize) {
    let mut identical = 0;
    let mut pushed = 0;
    let mut pulled = 0;
    let mut errored = 0;

    for repo in state.repos.values() {
        let status = match repo.status {
            Some(status) => status,
            None => continue,
        };
        match status {
            Status::Synced | Status::ConfigSynced | Status::ConfigUpdated => identical += 1,
            Status::Pushed => pushed += 1,
            Status::Pulled => pulled += 1,
            Status::Error
            | Status::ConfigError
            | Status::StagingError
            | Status::CommitError
            | Status::PullError => errored += 1,
            _ => {}
        }
    }

    (identical, pushed, pulled, errored)
}

fn count_latent(state: &SyncState, now: Instant) -> usize {
    state
        .repos
        .values()
        .filter(|repo| now.duration_since(repo.started_at).as_secs() >= SLOW_REPO_THRESHOLD_SECS)
        .count()
}

fn collect_stalled(state: &SyncState, now: Instant) -> Vec<Signal> {
    let mut stalled = Vec::new();
    for (name, repo) in &state.repos {
        let idle = now.duration_since(repo.last_update);
        let is_active = matches!(repo.stage, Stage::Checking | Stage::Updating);
        if is_active && idle.as_secs() >= STALL_THRESHOLD_SECS {
            stalled.push(Signal {
                repo: name.clone(),
                phase: stage_label(repo.stage),
                idle,
                last_op: repo.last_op.clone(),
            });
        }
    }
    stalled.sort_by(|a, b| b.idle.cmp(&a.idle));
    stalled
}

fn collect_waiting(state: &SyncState, now: Instant) -> Vec<Signal> {
    let mut waiting = Vec::new();
    for (name, repo) in &state.repos {
        if repo.stage == Stage::Waiting {
            let idle = now.duration_since(repo.last_update);
            waiting.push(Signal {
                repo: name.clone(),
                phase: stage_label(repo.stage),
                idle,
                last_op: repo.last_op.clone(),
            });
        }
    }
    waiting.sort_by(|a, b| b.idle.cmp(&a.idle));
    waiting
}

fn stage_label(stage: Stage) -> String {
    match stage {
        Stage::Queued => "queued",
        Stage::Checking => "scanning",
        Stage::Waiting => "waiting",
        Stage::Updating => "updating",
        Stage::Done => "done",
    }
    .to_string()
}

fn render_bar(percent: usize) -> String {
    let filled = (percent * BAR_WIDTH) / 100;
    let empty = BAR_WIDTH.saturating_sub(filled);
    format!(
        "{}{}",
        "|".repeat(filled),
        " ".repeat(empty)
    )
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

fn rate_per_sec(state: &SyncState, done: usize, now: Instant) -> f64 {
    let elapsed = now.duration_since(state.start_time).as_secs_f64();
    if elapsed <= 0.0 {
        return 0.0;
    }
    done as f64 / elapsed
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
