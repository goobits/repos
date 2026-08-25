use crate::git::{
    fetch_and_analyze, get_repo_visibility, has_uncommitted_changes, RepoVisibility, Status,
};
use crate::package::{detect_manager, PackageManager};
use futures::stream::{FuturesUnordered, StreamExt};
use std::path::PathBuf;
use std::sync::Arc;

pub struct PublishPlan {
    pub packages: Vec<PackageToPublish>,
    pub dirty_repos: Vec<String>,
    pub skipped_count: usize,
    pub unknown_count: usize,
    pub inspection_errors: Vec<(String, String)>,
}

#[derive(Clone)]
pub struct PackageToPublish {
    pub name: String,
    pub package_name: String,
    pub version: String,
    pub dependencies: Vec<String>,
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

fn is_targeted(name: &str, targets: &[String]) -> bool {
    targets.iter().any(|target| name == target)
}

fn visibility_selected(visibility: RepoVisibility, desired: Option<RepoVisibility>) -> bool {
    desired.map_or(true, |desired| {
        visibility == desired
            || visibility == RepoVisibility::Unknown && desired == RepoVisibility::Private
    })
}

pub async fn plan_publish(repos: Vec<(String, PathBuf)>, options: PlannerOptions) -> PublishPlan {
    // Filter repositories if specific targets were requested
    let mut filtered_repos = repos;
    if !options.target_repos.is_empty() {
        filtered_repos.retain(|(name, _)| is_targeted(name, &options.target_repos));
    }

    // Determine visibility filter
    let filter_visibility = if options.all {
        None
    } else if options.private_only {
        Some(RepoVisibility::Private)
    } else {
        Some(RepoVisibility::Public)
    };

    // Detect visibility and package type first so repositories excluded by the
    // command's filter never perform release-network preflights.
    let detection_futures: FuturesUnordered<_> = filtered_repos
        .into_iter()
        .map(|(name, path)| async move {
            let (visibility, manager) =
                tokio::join!(get_repo_visibility(&path), detect_manager(&path));
            (name, path, visibility, manager)
        })
        .collect();

    let mut plan = PublishPlan {
        packages: Vec::new(),
        dirty_repos: Vec::new(),
        skipped_count: 0,
        unknown_count: 0,
        inspection_errors: Vec::new(),
    };
    let mut candidates = Vec::new();

    for (name, path, visibility, manager) in detection_futures.collect::<Vec<_>>().await {
        if visibility == RepoVisibility::Unknown {
            plan.unknown_count += 1;
        }
        if !visibility_selected(visibility, filter_visibility) {
            plan.skipped_count += 1;
            continue;
        }
        if let Some(manager) = manager {
            candidates.push((name, path, manager));
        }
    }

    let analysis_futures: FuturesUnordered<_> = candidates
        .into_iter()
        .map(|(name, path, manager)| {
            let allow_dirty = options.allow_dirty;
            let dry_run = options.dry_run;
            async move {
                let (info, dependencies, dirty_result, release_result) = tokio::join!(
                    manager.get_info(&path),
                    manager.dependencies(&path),
                    async {
                        if !allow_dirty && !dry_run {
                            has_uncommitted_changes(&path).await
                        } else {
                            Ok(false)
                        }
                    },
                    async {
                        if dry_run {
                            Ok(())
                        } else {
                            verify_release_commit(&path).await
                        }
                    }
                );
                (
                    name,
                    path,
                    manager,
                    info,
                    dependencies,
                    dirty_result,
                    release_result,
                )
            }
        })
        .collect();

    for (name, path, manager, info, dependencies, dirty_result, release_result) in
        analysis_futures.collect::<Vec<_>>().await
    {
        let is_dirty = match dirty_result {
            Ok(is_dirty) => is_dirty,
            Err(error) => {
                plan.inspection_errors.push((name, error.to_string()));
                continue;
            }
        };
        if is_dirty {
            plan.dirty_repos.push(name.clone());
        }
        if let Err(error) = release_result {
            plan.inspection_errors.push((name, error.to_string()));
            continue;
        }
        let Some(info) = info else {
            plan.inspection_errors.push((
                name,
                "package manifest is missing a usable name or version".to_string(),
            ));
            continue;
        };
        plan.packages.push(PackageToPublish {
            name,
            package_name: info.name,
            version: info.version,
            dependencies,
            path,
            manager,
        });
    }

    plan.packages.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.name.cmp(&right.name))
    });

    plan
}

async fn verify_release_commit(path: &std::path::Path) -> anyhow::Result<()> {
    let state = fetch_and_analyze(path, false).await;
    if state.status != Status::Synced {
        anyhow::bail!("release commit preflight failed: {}", state.message);
    }
    if !state.upstream_exists {
        anyhow::bail!("release commit has no configured upstream");
    }
    if state.ahead_count > 0 {
        anyhow::bail!(
            "release commit is {} commits ahead; run `repos push` first",
            state.ahead_count
        );
    }

    if state.behind_count > 0 {
        anyhow::bail!(
            "release commit is {} commits behind; run `repos pull` first",
            state.behind_count
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{is_targeted, visibility_selected};
    use crate::git::RepoVisibility;

    #[test]
    fn publish_targets_match_repository_names_exactly() {
        let targets = vec!["api".to_string()];

        assert!(is_targeted("api", &targets));
        assert!(!is_targeted("api-client", &targets));
        assert!(!is_targeted("my-api", &targets));
    }

    #[test]
    fn unknown_visibility_is_private_by_default() {
        assert!(visibility_selected(
            RepoVisibility::Unknown,
            Some(RepoVisibility::Private)
        ));
        assert!(!visibility_selected(
            RepoVisibility::Unknown,
            Some(RepoVisibility::Public)
        ));
        assert!(visibility_selected(RepoVisibility::Unknown, None));
    }
}
