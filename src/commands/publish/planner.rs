use crate::git::operations::run_git;
use crate::git::remote::{inspect_remote, policy_violation, RemoteDirection};
use crate::git::{
    fetch_and_analyze, get_repo_visibility, has_uncommitted_changes, RepoVisibility, Status,
};
use crate::package::{detect_manager, PackageManager};
use futures::stream::{self, StreamExt};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
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

const PUBLISH_INSPECTION_CONCURRENCY: usize = 8;

fn is_targeted(name: &str, targets: &[String]) -> bool {
    targets.iter().any(|target| name == target)
}

pub(super) fn missing_requested_targets(
    repos: &[(String, PathBuf)],
    targets: &[String],
) -> Vec<String> {
    let available = repos
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<HashSet<_>>();
    targets
        .iter()
        .filter(|target| !available.contains(target.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
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
    let detection_results = stream::iter(filtered_repos)
        .map(|(name, path)| async move {
            let (visibility, manager) =
                tokio::join!(get_repo_visibility(&path), detect_manager(&path));
            (name, path, visibility, manager)
        })
        .buffer_unordered(PUBLISH_INSPECTION_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    let mut plan = PublishPlan {
        packages: Vec::new(),
        dirty_repos: Vec::new(),
        skipped_count: 0,
        unknown_count: 0,
        inspection_errors: Vec::new(),
    };
    let mut candidates = Vec::new();

    for (name, path, visibility, manager) in detection_results {
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

    let analysis_results = stream::iter(candidates)
        .map(|(name, path, manager)| {
            let allow_dirty = options.allow_dirty;
            let dry_run = options.dry_run;
            async move {
                let manifest_result = manager.inspect_manifest(&path).await;
                let expected_tag = match &manifest_result {
                    Ok(Some(manifest)) => Some(format!("v{}", manifest.info.version)),
                    _ => None,
                };
                let manifest_is_valid = matches!(&manifest_result, Ok(Some(_)));
                let (dirty_result, release_result) = tokio::join!(
                    async {
                        if !allow_dirty && !dry_run {
                            has_uncommitted_changes(&path).await
                        } else {
                            Ok(false)
                        }
                    },
                    async {
                        if dry_run || !manifest_is_valid {
                            Ok(())
                        } else {
                            verify_release_commit(
                                &path,
                                expected_tag
                                    .as_deref()
                                    .expect("valid manifests always derive a version tag"),
                            )
                            .await
                        }
                    }
                );
                (
                    name,
                    path,
                    manager,
                    manifest_result,
                    dirty_result,
                    release_result,
                )
            }
        })
        .buffer_unordered(PUBLISH_INSPECTION_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    for (name, path, manager, manifest_result, dirty_result, release_result) in analysis_results {
        let manifest = match manifest_result {
            Ok(Some(manifest)) => manifest,
            Ok(None) => {
                plan.inspection_errors.push((
                    name,
                    "package manifest is missing a usable name or version".to_string(),
                ));
                continue;
            }
            Err(error) => {
                plan.inspection_errors.push((name, error.to_string()));
                continue;
            }
        };
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
        plan.packages.push(PackageToPublish {
            name,
            package_name: manifest.info.name,
            version: manifest.info.version,
            dependencies: manifest.dependencies,
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

async fn verify_release_commit(path: &Path, expected_tag: &str) -> anyhow::Result<()> {
    if crate::git::is_detached_head(path).await? {
        return verify_detached_tag_release(path, expected_tag).await;
    }

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

async fn verify_detached_tag_release(path: &Path, tag: &str) -> anyhow::Result<()> {
    let head = required_git_value(path, &["rev-parse", "HEAD"], "release commit").await?;
    let tag_ref = format!("refs/tags/{tag}");
    let commit_ref = format!("{tag_ref}^{{commit}}");
    let local_target =
        required_git_value(path, &["rev-parse", "--verify", &commit_ref], "release tag").await?;
    if local_target != head {
        anyhow::bail!(
            "release tag {tag} points to {}, not detached HEAD {}",
            short_oid(&local_target),
            short_oid(&head)
        );
    }

    let remote = detached_release_remote(path).await?;
    let contexts = inspect_remote(path, &remote, RemoteDirection::Fetch).await?;
    if let Some(violation) = policy_violation(&contexts)? {
        anyhow::bail!("{}", violation.message());
    }
    let peeled_ref = format!("{tag_ref}^{{}}");
    let (success, refs, stderr) = run_git(
        path,
        &["ls-remote", "--tags", &remote, &tag_ref, &peeled_ref],
    )
    .await?;
    if !success {
        anyhow::bail!(
            "could not verify remote release tag {tag}: {}",
            if stderr.trim().is_empty() {
                "remote query failed"
            } else {
                stderr.trim()
            }
        );
    }
    let remote_target = remote_tag_commit(&refs, &tag_ref)
        .ok_or_else(|| anyhow::anyhow!("release tag {tag} is not published on remote {remote}"))?;
    if remote_target != head {
        anyhow::bail!(
            "remote release tag {tag} points to {}, not detached HEAD {}",
            short_oid(&remote_target),
            short_oid(&head)
        );
    }
    Ok(())
}

async fn detached_release_remote(path: &Path) -> anyhow::Result<String> {
    let (success, remotes, stderr) = run_git(path, &["remote"]).await?;
    if !success {
        anyhow::bail!(
            "release remote inspection failed: {}",
            if stderr.trim().is_empty() {
                "git remote failed"
            } else {
                stderr.trim()
            }
        );
    }
    let names = remotes
        .lines()
        .filter(|name| !name.trim().is_empty())
        .collect::<Vec<_>>();
    if names.contains(&"origin") {
        return Ok("origin".to_string());
    }
    if let [remote] = names.as_slice() {
        return Ok((*remote).to_string());
    }
    if names.is_empty() {
        anyhow::bail!("detached release commit has no remote for tag verification");
    }
    anyhow::bail!("detached release commit has ambiguous remotes; configure an origin remote")
}

async fn required_git_value(
    path: &Path,
    args: &[&str],
    description: &str,
) -> anyhow::Result<String> {
    let (success, value, stderr) = run_git(path, args).await?;
    if success && !value.trim().is_empty() {
        Ok(value)
    } else {
        anyhow::bail!(
            "could not resolve {description}: {}",
            if stderr.trim().is_empty() {
                "git returned no value"
            } else {
                stderr.trim()
            }
        )
    }
}

fn remote_tag_commit(output: &str, tag_ref: &str) -> Option<String> {
    let peeled_ref = format!("{tag_ref}^{{}}");
    let mut direct = None;
    let mut peeled = None;
    for line in output.lines() {
        let Some((oid, reference)) = line.split_once('\t') else {
            continue;
        };
        if reference == peeled_ref {
            peeled = Some(oid.to_string());
        } else if reference == tag_ref {
            direct = Some(oid.to_string());
        }
    }
    peeled.or(direct)
}

fn short_oid(oid: &str) -> String {
    oid.chars().take(7).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        is_targeted, missing_requested_targets, verify_release_commit, visibility_selected,
    };
    use crate::git::RepoVisibility;
    use std::path::PathBuf;
    use std::process::Command;

    fn git(path: &std::path::Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[test]
    fn publish_targets_match_repository_names_exactly() {
        let targets = vec!["api".to_string()];

        assert!(is_targeted("api", &targets));
        assert!(!is_targeted("api-client", &targets));
        assert!(!is_targeted("my-api", &targets));
    }

    #[test]
    fn missing_publish_targets_are_deduplicated_and_sorted() {
        let repos = vec![
            ("web".to_string(), PathBuf::from("web")),
            ("api".to_string(), PathBuf::from("api")),
        ];
        let targets = vec![
            "worker".to_string(),
            "api".to_string(),
            "missing".to_string(),
            "worker".to_string(),
        ];

        assert_eq!(
            missing_requested_targets(&repos, &targets),
            vec!["missing".to_string(), "worker".to_string()]
        );
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

    #[tokio::test]
    async fn detached_release_requires_exact_local_and_remote_version_tag() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }

        let root = tempfile::TempDir::new().expect("temporary directory");
        let repository = root.path().join("repository");
        let remote = root.path().join("remote.git");
        git(root.path(), &["init", repository.to_str().unwrap()]);
        git(root.path(), &["init", "--bare", remote.to_str().unwrap()]);
        git(&repository, &["config", "user.name", "repos test"]);
        git(
            &repository,
            &["config", "user.email", "repos@example.invalid"],
        );
        git(
            &repository,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        std::fs::write(repository.join("release.txt"), "release\n")
            .expect("release file should be written");
        git(&repository, &["add", "release.txt"]);
        git(&repository, &["commit", "-m", "release"]);
        let branch = git(&repository, &["symbolic-ref", "--short", "HEAD"]);
        git(&repository, &["push", "-u", "origin", &branch]);
        git(&repository, &["tag", "v1.2.3"]);
        git(
            &repository,
            &["push", "origin", "refs/tags/v1.2.3:refs/tags/v1.2.3"],
        );
        git(&repository, &["checkout", "--detach", "v1.2.3"]);

        verify_release_commit(&repository, "v1.2.3")
            .await
            .expect("published matching tag should authorize detached release");

        git(&repository, &["tag", "v2.0.0"]);
        assert!(verify_release_commit(&repository, "v2.0.0")
            .await
            .expect_err("local-only tag must fail")
            .to_string()
            .contains("is not published on remote"));
    }
}
