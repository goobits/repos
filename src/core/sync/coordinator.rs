//! Shared coordinator for sync events and HUD rendering.

use crate::core::sync::renderer::HudRenderer;
use crate::core::sync::state::{Stage, SyncState};
use crate::core::SyncStatistics;
use crate::git::Status;
use std::io::Write;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub struct SyncCoordinator {
    state: Arc<Mutex<SyncState>>,
    stats: Arc<Mutex<SyncStatistics>>,
    renderer: HudRenderer,
}

impl SyncCoordinator {
    pub fn new(
        repo_names: &[String],
        total_repos: usize,
        fetch_concurrency: usize,
        update_concurrency: usize,
        stats: Arc<Mutex<SyncStatistics>>,
    ) -> Self {
        let state = SyncState::new(
            repo_names,
            total_repos,
            fetch_concurrency,
            update_concurrency,
        );
        let renderer = HudRenderer::new();
        Self {
            state: Arc::new(Mutex::new(state)),
            stats,
            renderer,
        }
    }

    pub fn start(&self) -> (watch::Sender<bool>, JoinHandle<()>) {
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let state = Arc::clone(&self.state);
        let stats = Arc::clone(&self.stats);
        let renderer = self.renderer.clone();

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let state_guard = state.lock().unwrap();
                        let stats_guard = stats.lock().unwrap();
                        let hud = renderer.render(&state_guard, &stats_guard);
                        drop(stats_guard);
                        drop(state_guard);
                        print!("\x1b[2J\x1b[H{hud}");
                        let _ = std::io::stdout().flush();
                    }
                    _ = stop_rx.changed() => {
                        if *stop_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        });

        (stop_tx, handle)
    }

    pub fn set_stage(&self, repo_name: &str, stage: Stage, op: &str) {
        if let Ok(mut guard) = self.state.lock() {
            guard.set_stage(repo_name, stage, op);
        }
    }

    pub fn set_status(&self, repo_name: &str, status: Status, message: &str) {
        if let Ok(mut guard) = self.state.lock() {
            guard.set_status(repo_name, status, message);
        }
    }
}
