use crate::git::{get_repo_visibility, has_uncommitted_changes, RepoVisibility};
use crate::package::{detect_manager, PackageManager};
use futures::stream::{FuturesUnordered, StreamExt};
use std::path::PathBuf;
use std::sync::Arc;

pub struct PublishPlan {
    pub packages: Vec<PackageToPublish>,
    pub dirty_repos: Vec<String>,
    pub skipped_count: usize,
    pub unknown_count: usize,
}

#[derive(Clone)]
pub struct PackageToPublish {
    pub name: String,
    pub path: PathBuf,
    pub manager: Arc<dyn PackageManager>,
}

pub struct PlannerOptions {
    pub target_repos: Vec<String>,
    pub all: bool,
    pub private_only: bool,
    pub allow_dirty: bool,
    pub dry_run: bool,
}

pub async fn plan_publish(repos: Vec<(String, PathBuf)>, options: PlannerOptions) -> PublishPlan {
    // Filter repositories if specific targets were requested
    let mut filtered_repos = repos;
    if !options.target_repos.is_empty() {
        filtered_repos.retain(|(name, _)| {
            options
                .target_repos
                .iter()
                .any(|target| name.contains(target))
        });
    }

    // Determine visibility filter
    let filter_visibility = if options.all {
        None
    } else if options.private_only {
        Some(RepoVisibility::Private)
    } else {
        Some(RepoVisibility::Public)
    };

    // Parallel analysis
    let analysis_futures: FuturesUnordered<_> = filtered_repos
        .into_iter()
        .map(|(name, path)| {
            let allow_dirty = options.allow_dirty;
            let dry_run = options.dry_run;
            async move {
                let (visibility, manager, is_dirty) =
                    tokio::join!(get_repo_visibility(&path), detect_manager(&path), async {
                        if !allow_dirty && !dry_run {
                            has_uncommitted_changes(&path).await
                        } else {
                            false
                        }
                    });
                (name, path, visibility, manager, is_dirty)
            }
        })
        .collect();

    let analysis_results: Vec<_> = analysis_futures.collect().await;

    let mut plan = PublishPlan {
        packages: Vec::new(),
        dirty_repos: Vec::new(),
        skipped_count: 0,
        unknown_count: 0,
    };

    for (name, path, visibility, manager, is_dirty) in analysis_results {
        // Apply visibility filter
        if let Some(desired) = filter_visibility {
            if visibility != desired {
                if visibility == RepoVisibility::Unknown && desired == RepoVisibility::Private {
                    // Treat unknown as private
                } else {
                    plan.skipped_count += 1;
                    if visibility == RepoVisibility::Unknown {
                        plan.unknown_count += 1;
                    }
                    continue;
                }
            }
        }

        if let Some(mgr) = manager {
            if is_dirty {
                plan.dirty_repos.push(name.clone());
            }
            plan.packages.push(PackageToPublish {
                name,
                path,
                manager: mgr,
            });
        }
    }

    plan
}
