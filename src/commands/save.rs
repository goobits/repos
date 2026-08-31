//! Intent-driven save command.
//!
//! `repos save` is the safe daily workflow: stage tracked changes, commit, and
//! push. Untracked files are opt-in to avoid committing local scratch files or
//! secrets across a repository fleet.

use anyhow::Result;
use futures::stream::{FuturesUnordered, StreamExt};

use crate::core::{
    acquire_semaphore_permit, clean_error_message, create_processing_context,
    gitlink_prerequisites_at_head, init_command, set_terminal_title, set_terminal_title_and_flush,
    BatchOperation, GitlinkPrerequisite, RepositoryOrder, RepositoryTopology, GIT_CONCURRENT_CAP,
    NO_REPOS_MESSAGE,
};
use crate::git::worktree::{inspect_refreshed_repository_state, HeadState};
use crate::git::{
    commit_changes, fetch_and_analyze, has_staged_changes, push_if_needed, stage_all_changes,
    stage_tracked_changes, Status,
};

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";

/// Handles `repos save`.
pub async fn handle_save_command(
    message: String,
    include_untracked: bool,
    auto_upstream: bool,
    dry_run: bool,
) -> Result<()> {
    set_terminal_title("💾 repos save");

    let (start_time, repos) = init_command(SCANNING_MESSAGE).await?;

    if repos.is_empty() {
        println!("\r{NO_REPOS_MESSAGE}");
        set_terminal_title_and_flush("✅ repos save");
        return Ok(());
    }

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    let action = if dry_run { "Planning save" } else { "Saving" };
    print!("\r💾 {action} across {total_repos} {repo_word}                    \n\n");

    let context =
        match create_processing_context(std::sync::Arc::new(repos), start_time, GIT_CONCURRENT_CAP)
        {
            Ok(context) => context,
            Err(e) => {
                set_terminal_title_and_flush("✅ repos save");
                return Err(e);
            }
        };

    process_save_repositories(context, message, include_untracked, auto_upstream, dry_run).await?;

    set_terminal_title_and_flush("✅ repos save");
    Ok(())
}

async fn process_save_repositories(
    context: crate::core::ProcessingContext,
    commit_message: String,
    include_untracked: bool,
    auto_upstream: bool,
    dry_run: bool,
) -> Result<()> {
    use crate::core::create_progress_bar;

    let operation = BatchOperation::Save { dry_run };

    let mut progress_bars = Vec::new();
    for (repo_name, _) in context.repositories.iter() {
        let progress_bar =
            create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
        progress_bar.set_message(if dry_run { "planning..." } else { "saving..." });
        progress_bars.push(progress_bar);
    }

    let _separator_pb = crate::core::create_separator_progress_bar(&context.multi_progress);
    let footer_pb = crate::core::create_footer_progress_bar(&context.multi_progress);
    let initial_stats = crate::core::SyncStatistics::new();
    footer_pb
        .set_message(initial_stats.generate_batch_live_summary(operation, context.total_repos));
    let _separator_pb2 = crate::core::create_separator_progress_bar(&context.multi_progress);

    let max_name_length = context.max_name_length;
    let start_time = context.start_time;
    let total_repos = context.total_repos;

    let topology = RepositoryTopology::new(&context.repositories);
    let mut completed = vec![None; context.total_repos];
    for wave in topology.waves(RepositoryOrder::ChildrenFirst) {
        let mut futures = FuturesUnordered::new();
        for index in wave {
            let (repo_name, repo_path) = &context.repositories[index];
            let progress_bar = progress_bars[index].clone();
            let gitlink_prerequisites =
                topology.gitlink_prerequisites(index, &context.repositories);
            let gitlink_inspection_error =
                topology.gitlink_inspection_error(index).map(str::to_string);
            let failed_gitlink_children = topology
                .gitlink_children(index)
                .iter()
                .filter_map(|child| {
                    completed[*child]
                        .filter(|status| is_save_failure(*status))
                        .map(|_| context.repositories[*child].0.clone())
                })
                .collect::<Vec<_>>();
            let planned_gitlink_change = dry_run
                && topology
                    .gitlink_children(index)
                    .iter()
                    .any(|child| completed[*child] == Some(Status::Staged));
            let semaphore = std::sync::Arc::clone(&context.semaphore);
            let stats = std::sync::Arc::clone(&context.statistics);
            let footer = footer_pb.clone();
            let commit_message = commit_message.clone();

            let future = async move {
                let _permit = acquire_semaphore_permit(&semaphore).await;

                let (status, message, has_uncommitted) = save_one_repo(
                    repo_path,
                    &commit_message,
                    include_untracked,
                    auto_upstream,
                    dry_run,
                    SaveDependencies {
                        prerequisites: &gitlink_prerequisites,
                        inspection_error: gitlink_inspection_error.as_deref(),
                        failed_children: &failed_gitlink_children,
                        planned_change: planned_gitlink_change,
                    },
                )
                .await;

                progress_bar.set_prefix(format!(
                    "{} {:width$}",
                    status.symbol(),
                    repo_name,
                    width = max_name_length
                ));
                progress_bar.set_message(format!("{:<12}   {}", status.text(), message));
                progress_bar.finish();

                stats.update(
                    repo_name,
                    &repo_path.to_string_lossy(),
                    &status,
                    &message,
                    has_uncommitted,
                );
                footer.set_message(stats.generate_batch_live_summary(operation, total_repos));
                (index, status)
            };

            futures.push(future);
        }

        while let Some((index, status)) = futures.next().await {
            completed[index] = Some(status);
        }
    }

    footer_pb.finish();

    let final_stats = context.statistics.as_ref();
    println!(
        "\n{}\n",
        final_stats.generate_batch_report(operation, start_time.elapsed())
    );

    let error_count = final_stats
        .error_repos
        .load(std::sync::atomic::Ordering::Relaxed);
    if error_count > 0 {
        anyhow::bail!("{error_count} repositories failed to save");
    }

    Ok(())
}

fn is_save_failure(status: Status) -> bool {
    matches!(
        status,
        Status::Error
            | Status::ConfigError
            | Status::StagingError
            | Status::CommitError
            | Status::PullError
    )
}

struct SaveDependencies<'a> {
    prerequisites: &'a [GitlinkPrerequisite],
    inspection_error: Option<&'a str>,
    failed_children: &'a [String],
    planned_change: bool,
}

async fn save_one_repo(
    repo_path: &std::path::Path,
    commit_message: &str,
    include_untracked: bool,
    auto_upstream: bool,
    dry_run: bool,
    dependencies: SaveDependencies<'_>,
) -> (Status, String, bool) {
    if let Some(error) = dependencies.inspection_error {
        return (
            Status::StagingError,
            format!("submodule relationship inspection failed: {error}"),
            false,
        );
    }
    let worktree = match inspect_refreshed_repository_state(repo_path).await {
        Ok(state) => state,
        Err(error) => {
            return (
                Status::StagingError,
                format!("repository state inspection failed: {error}"),
                false,
            )
        }
    };
    match worktree.head() {
        HeadState::Detached => {
            return (
                Status::Skip,
                "detached HEAD; checkout a branch before save".to_string(),
                false,
            );
        }
        HeadState::Unknown => {
            return (
                Status::StagingError,
                "branch inspection returned no HEAD state".to_string(),
                false,
            )
        }
        HeadState::Branch(_) | HeadState::Unborn => {}
    }

    if worktree.has_conflicts() {
        return (
            Status::StagingError,
            "unresolved conflicts; resolve or abort the merge/rebase before save".to_string(),
            true,
        );
    }

    let has_tracked_changes = worktree.has_tracked_changes();
    let has_untracked_changes = worktree.has_untracked_changes();
    let has_changes_to_commit = has_tracked_changes || (include_untracked && has_untracked_changes);
    let should_push_existing = matches!(worktree.head(), HeadState::Branch(_))
        && (worktree
            .ahead_behind()
            .is_some_and(|counts| counts.ahead > 0)
            || (auto_upstream && worktree.upstream().is_none()));
    let leaves_untracked = has_untracked_changes && !include_untracked;

    if !has_changes_to_commit
        && !should_push_existing
        && !dependencies.planned_change
        && leaves_untracked
    {
        return (
            Status::NoChanges,
            "only untracked changes; pass --all".to_string(),
            true,
        );
    }

    if !has_changes_to_commit && !should_push_existing && !dependencies.planned_change {
        return (Status::Synced, "clean".to_string(), false);
    }

    if dry_run {
        let message = if has_changes_to_commit {
            let stage_mode = if include_untracked {
                "stage all changes"
            } else {
                "stage tracked changes"
            };
            format!("{stage_mode}, commit, push")
        } else if dependencies.planned_change {
            "refresh submodule pointers, commit, push".to_string()
        } else if leaves_untracked {
            "push existing commits; leave untracked changes untouched".to_string()
        } else {
            "push existing commits".to_string()
        };
        return (
            Status::Staged,
            message,
            has_tracked_changes || has_untracked_changes || dependencies.planned_change,
        );
    }

    let committed = if has_changes_to_commit {
        let stage_result = if include_untracked {
            stage_all_changes(repo_path).await
        } else {
            stage_tracked_changes(repo_path).await
        };

        match stage_result {
            Ok((true, _, _)) => {}
            Ok((false, _, stderr)) => {
                return (Status::StagingError, clean_error_message(&stderr), true);
            }
            Err(e) => return (Status::StagingError, format!("stage failed: {e}"), true),
        }

        match has_staged_changes(repo_path).await {
            Ok(true) => {}
            Ok(false) => return (Status::NoChanges, "nothing staged".to_string(), true),
            Err(e) => {
                return (
                    Status::StagingError,
                    format!("stage check failed: {e}"),
                    true,
                )
            }
        }

        match commit_changes(repo_path, commit_message, false).await {
            Ok((true, _, _)) => {}
            Ok((false, _, stderr)) => {
                return (Status::CommitError, clean_error_message(&stderr), true);
            }
            Err(e) => return (Status::CommitError, format!("commit failed: {e}"), true),
        }
        true
    } else {
        false
    };

    let fetch_result = fetch_and_analyze(repo_path, auto_upstream).await;
    if fetch_result.will_push(auto_upstream) && !dependencies.prerequisites.is_empty() {
        let current_prerequisites = match gitlink_prerequisites_at_head(
            repo_path,
            dependencies.prerequisites,
        ) {
            Ok(prerequisites) => prerequisites,
            Err(error) => {
                let prefix = if committed { "committed; " } else { "" };
                return (
                    Status::Error,
                    format!(
                        "{prefix}push blocked because committed submodule inspection failed: {error}"
                    ),
                    fetch_result.has_uncommitted,
                );
            }
        };
        let unpublished =
            crate::git::operations::unpublished_gitlinks(&current_prerequisites).await;
        if !unpublished.is_empty() {
            let prefix = if committed { "committed; " } else { "" };
            let failed_dependency = unpublished
                .iter()
                .any(|name| dependencies.failed_children.contains(name));
            let reason = if failed_dependency {
                "dependent submodule save failed and its commit is not reachable"
            } else {
                "submodule commits are not reachable"
            };
            return (
                Status::Error,
                format!(
                    "{prefix}push blocked because {reason} from fetched remote refs: {}",
                    unpublished.join(", ")
                ),
                fetch_result.has_uncommitted,
            );
        }
    }
    let (push_status, push_message, has_uncommitted) =
        push_if_needed(repo_path, &fetch_result, auto_upstream).await;

    if committed {
        match push_status {
            Status::Pushed | Status::Fetched | Status::Synced => (
                Status::Committed,
                format!("committed; {push_message}"),
                has_uncommitted,
            ),
            other => (
                other,
                format!("committed; push skipped: {push_message}"),
                has_uncommitted,
            ),
        }
    } else if push_status == Status::Synced && leaves_untracked {
        (
            Status::NoChanges,
            "only untracked changes; pass --all".to_string(),
            true,
        )
    } else if push_status == Status::Pushed && leaves_untracked {
        (
            push_status,
            format!("{push_message}; untracked changes left untouched"),
            has_uncommitted,
        )
    } else {
        (push_status, push_message, has_uncommitted)
    }
}
