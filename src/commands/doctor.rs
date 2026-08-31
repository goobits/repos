//! Repository health diagnostics.

mod config;
mod report;

#[cfg(test)]
use config::parse_configured_remote_urls;
use config::{inspect_configured_remote_urls, ConfiguredRemoteUrls};
use report::{DoctorFinding, DoctorReport, RepositoryDiagnosis};

use anyhow::Result;
use futures::stream::{FuturesUnordered, StreamExt};

use crate::core::{
    acquire_semaphore_permit, clean_error_message, create_processing_context, init_command,
    set_terminal_title, set_terminal_title_and_flush, GIT_CONCURRENT_CAP, NO_REPOS_MESSAGE,
};
use crate::git::failure::GitFailure;
use crate::git::operations::run_git;
use crate::git::remote::{
    context_from_url, inspect_remote, policy_violation, RemoteContext, RemoteDirection,
    RemotePolicyViolation, RemoteTransport,
};
use crate::git::worktree::{inspect_repository_state, HeadState};
use crate::utils::compare_repository_locations;

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";

struct DirectionInspection {
    contexts: Vec<RemoteContext>,
    blocked: bool,
}

/// Diagnose common blockers without mutating repositories.
pub async fn handle_doctor_command() -> Result<()> {
    set_terminal_title("🩺 repos doctor");

    let (start_time, repos) = init_command(SCANNING_MESSAGE).await?;
    if repos.is_empty() {
        println!("\r{NO_REPOS_MESSAGE}");
        set_terminal_title_and_flush("✅ repos doctor");
        return Ok(());
    }

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    print!("\r🩺 Diagnosing {total_repos} {repo_word}                    \n\n");

    let context =
        match create_processing_context(std::sync::Arc::new(repos), start_time, GIT_CONCURRENT_CAP)
        {
            Ok(context) => context,
            Err(error) => {
                set_terminal_title_and_flush("✅ repos doctor");
                return Err(error);
            }
        };

    let report = run_diagnostics(context).await;
    println!("\n{}\n", report.render(start_time.elapsed()));
    set_terminal_title_and_flush("✅ repos doctor");

    let blocker_repos = report.blocker_repos();
    if blocker_repos > 0 || report.nested_drift_count > 0 {
        anyhow::bail!(
            "doctor found {blocker_repos} blocker repositories and {} drifted nested package groups",
            report.nested_drift_count
        );
    }
    Ok(())
}

async fn run_diagnostics(context: crate::core::ProcessingContext) -> DoctorReport {
    use indicatif::{ProgressBar, ProgressStyle};

    let progress = context
        .multi_progress
        .add(ProgressBar::new(context.total_repos as u64));
    if let Ok(style) = ProgressStyle::default_bar().template("[{pos}/{len}] {msg}") {
        progress.set_style(style);
    }
    progress.set_message("diagnosing...");

    let mut futures = FuturesUnordered::new();
    for (repository, path) in context.repositories.iter() {
        let semaphore = std::sync::Arc::clone(&context.semaphore);
        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore).await;
            diagnose_repo(repository, path).await
        };
        futures.push(future);
    }

    let mut report = DoctorReport::default();
    while let Some(diagnosis) = futures.next().await {
        progress.set_message(format!(
            "{} · {}",
            diagnosis.repository,
            diagnosis.progress_label()
        ));
        progress.inc(1);
        report.repositories.push(diagnosis);
    }
    progress.finish_and_clear();
    report.repositories.sort_by(|left, right| {
        compare_repository_locations(&left.path, &left.repository, &right.path, &right.repository)
    });

    match crate::subrepo::status::analyze_nested_status_for_repositories(&context.repositories) {
        Ok(nested) => {
            report.nested_drift_count = nested.drifted_count();
            report.nested_drift_lines =
                crate::subrepo::status::format_drift_work_items_with_inventory(&nested).1;
        }
        Err(error) => report.global_advisories.push(DoctorFinding::new(
            format!("nested package inspection failed: {error}"),
            "run repos nested validate",
        )),
    }

    report
}

async fn diagnose_repo(repository: &str, path: &std::path::Path) -> RepositoryDiagnosis {
    let mut diagnosis = RepositoryDiagnosis::new(repository, path);
    let display_path = diagnosis.path.clone();

    let worktree = inspect_repository_state(path).await;
    match &worktree {
        Ok(state) if state.head() == &HeadState::Detached => {
            diagnosis.blockers.push(DoctorFinding::new(
                "detached HEAD",
                format!("git -C {} switch <branch>", shell_quote(&display_path)),
            ))
        }
        Ok(_) => {}
        Err(error) => diagnosis.blockers.push(DoctorFinding::new(
            format!(
                "branch check failed: {}",
                clean_error_message(&error.to_string())
            ),
            "verify Git is installed and the repository is readable",
        )),
    }

    let remotes = match run_git(path, &["remote"]).await {
        Ok((true, remotes, _)) if remotes.trim().is_empty() => {
            diagnosis.blockers.push(DoctorFinding::new(
                "no remote",
                format!(
                    "git -C {} remote add origin '<SSH clone URL>'",
                    shell_quote(&display_path)
                ),
            ));
            Vec::new()
        }
        Ok((true, remotes, _)) => remotes.lines().map(str::to_string).collect::<Vec<_>>(),
        Ok((false, _, stderr)) => {
            diagnosis.blockers.push(DoctorFinding::new(
                format!("remote check failed: {}", clean_error_message(&stderr)),
                format!("git -C {} remote -v", shell_quote(&display_path)),
            ));
            Vec::new()
        }
        Err(error) => {
            diagnosis.blockers.push(DoctorFinding::new(
                format!(
                    "remote check failed: {}",
                    clean_error_message(&error.to_string())
                ),
                "verify Git is installed and the repository is readable",
            ));
            Vec::new()
        }
    };

    let configured_urls = if remotes.is_empty() {
        None
    } else {
        match inspect_configured_remote_urls(path).await {
            Ok(urls) => Some(urls),
            Err(error) => {
                diagnosis.blockers.push(DoctorFinding::new(
                    format!(
                        "configured remote URL inspection failed: {}",
                        clean_error_message(&error.to_string())
                    ),
                    format!("git -C {} remote -v", shell_quote(&display_path)),
                ));
                None
            }
        }
    };

    for remote in &remotes {
        diagnose_remote(
            path,
            &display_path,
            remote,
            configured_urls.as_ref(),
            &mut diagnosis,
        )
        .await;
    }

    if !remotes.is_empty() {
        match run_git(path, &["rev-parse", "--abbrev-ref", "@{upstream}"]).await {
            Ok((true, _, _)) => {}
            Ok((false, _, _)) | Err(_) => diagnosis.blockers.push(DoctorFinding::new(
                "no upstream",
                "run repos push --auto-upstream",
            )),
        }
    }

    match worktree {
        Ok(worktree) => {
            if worktree.has_conflicts() {
                diagnosis.blockers.push(DoctorFinding::new(
                    "conflicts",
                    format!("git -C {} status", shell_quote(&display_path)),
                ));
            } else if worktree.is_dirty() {
                diagnosis.blockers.push(DoctorFinding::new(
                    "dirty worktree",
                    format!("git -C {} status --short", shell_quote(&display_path)),
                ));
            }
        }
        Err(error) => diagnosis.blockers.push(DoctorFinding::new(
            format!("status failed: {}", clean_error_message(&error.to_string())),
            format!("git -C {} status", shell_quote(&display_path)),
        )),
    }

    diagnosis.finish()
}

async fn diagnose_remote(
    path: &std::path::Path,
    display_path: &str,
    remote: &str,
    configured_urls: Option<&ConfiguredRemoteUrls>,
    diagnosis: &mut RepositoryDiagnosis,
) {
    let fetch_urls = configured_urls
        .map(|urls| urls.urls(remote, RemoteDirection::Fetch))
        .unwrap_or_default();
    let push_urls = configured_urls
        .map(|urls| urls.urls(remote, RemoteDirection::Push))
        .unwrap_or_default();
    let explicit_push = !push_urls.is_empty();

    let fetch = inspect_direction(
        path,
        display_path,
        remote,
        RemoteDirection::Fetch,
        diagnosis,
    )
    .await;
    let push =
        inspect_direction(path, display_path, remote, RemoteDirection::Push, diagnosis).await;

    let mut fetch_advisory = false;
    if !fetch.blocked {
        if let Some(url) = fetch_urls
            .iter()
            .find(|url| RemoteTransport::from_url(url).is_http())
        {
            let context = context_from_url(remote, RemoteDirection::Fetch, url);
            let scope = if explicit_push {
                "fetch"
            } else {
                "fetch and inherited push"
            };
            diagnosis.advisories.push(http_advisory(
                display_path,
                context,
                scope,
                fetch
                    .contexts
                    .iter()
                    .any(|context| context.transport.is_http()),
            ));
            fetch_advisory = true;
        }
    }

    let mut push_advisory = fetch_advisory && !explicit_push;
    if !push.blocked {
        if let Some(url) = push_urls
            .iter()
            .find(|url| RemoteTransport::from_url(url).is_http())
        {
            diagnosis.advisories.push(http_advisory(
                display_path,
                context_from_url(remote, RemoteDirection::Push, url),
                "push",
                false,
            ));
            push_advisory = true;
        }
    }

    let effective_fetch_http = fetch
        .contexts
        .iter()
        .find(|context| context.transport.is_http());
    if !fetch.blocked && !fetch_advisory {
        if let Some(context) = effective_fetch_http {
            diagnosis.advisories.push(http_advisory(
                display_path,
                context.clone(),
                "effective fetch",
                true,
            ));
        }
    }
    if !push.blocked && !push_advisory {
        if let Some(context) = push
            .contexts
            .iter()
            .find(|context| context.transport.is_http())
        {
            diagnosis.advisories.push(http_advisory(
                display_path,
                context.clone(),
                "effective push",
                false,
            ));
        }
    }

    if fetch.blocked || effective_fetch_http.is_some() || fetch.contexts.is_empty() {
        return;
    }

    match run_git(path, &["ls-remote", "--heads", "--", remote]).await {
        Ok((true, _, _)) => {}
        Ok((false, _, stderr)) => diagnosis.blockers.push(DoctorFinding::new(
            format!("{remote} access failed: {}", clean_error_message(&stderr)),
            format!(
                "git -C {} ls-remote --heads -- {}",
                shell_quote(display_path),
                shell_quote(remote)
            ),
        )),
        Err(error) => diagnosis.blockers.push(DoctorFinding::new(
            format!(
                "{remote} access failed: {}",
                clean_error_message(&error.to_string())
            ),
            format!(
                "git -C {} ls-remote --heads -- {}",
                shell_quote(display_path),
                shell_quote(remote)
            ),
        )),
    }
}

async fn inspect_direction(
    path: &std::path::Path,
    display_path: &str,
    remote: &str,
    direction: RemoteDirection,
    diagnosis: &mut RepositoryDiagnosis,
) -> DirectionInspection {
    let contexts = match inspect_remote(path, remote, direction).await {
        Ok(contexts) => contexts,
        Err(error) => {
            diagnosis.blockers.push(DoctorFinding::new(
                format!(
                    "{remote} {} URL inspection failed: {}",
                    direction.label(),
                    clean_error_message(&error.to_string())
                ),
                remote_get_url_action(display_path, remote, direction),
            ));
            return DirectionInspection {
                contexts: Vec::new(),
                blocked: true,
            };
        }
    };

    match policy_violation(&contexts) {
        Ok(Some(violation)) => {
            let message = violation.message();
            let next = GitFailure::from_policy(violation).next_action(display_path);
            diagnosis.blockers.push(DoctorFinding::new(message, next));
            DirectionInspection {
                contexts,
                blocked: true,
            }
        }
        Ok(None) => DirectionInspection {
            contexts,
            blocked: false,
        },
        Err(error) => {
            diagnosis.blockers.push(DoctorFinding::new(
                format!("transport policy inspection failed: {error}"),
                "git config --global repos.transportPolicy ssh-only",
            ));
            DirectionInspection {
                contexts,
                blocked: true,
            }
        }
    }
}

fn remote_get_url_action(display_path: &str, remote: &str, direction: RemoteDirection) -> String {
    let push_flag = if direction == RemoteDirection::Push {
        " --push"
    } else {
        ""
    };
    format!(
        "git -C {} remote get-url{push_flag} --all -- {}",
        shell_quote(display_path),
        shell_quote(remote)
    )
}

fn http_advisory(
    display_path: &str,
    context: RemoteContext,
    scope: &str,
    access_probe_skipped: bool,
) -> DoctorFinding {
    let skipped = if access_probe_skipped {
        "; access probe skipped"
    } else {
        ""
    };
    let message = format!(
        "{} uses HTTP(S) for {scope}; convert to SSH to avoid credential prompts{skipped}",
        context.remote
    );
    let next = GitFailure::from_policy(RemotePolicyViolation { context }).next_action(display_path);
    DoctorFinding::new(message, next)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_advisories_are_sanitized_and_actionable_for_both_directions() {
        let fetch = http_advisory(
            "./repo",
            context_from_url(
                "origin",
                RemoteDirection::Fetch,
                "https://token@github.com/team/repo.git?secret=hidden",
            ),
            "fetch and inherited push",
            false,
        );
        let push = http_advisory(
            "./repo",
            context_from_url(
                "origin",
                RemoteDirection::Push,
                "https://token@github.com/team/repo.git?secret=hidden",
            ),
            "push",
            false,
        );

        assert!(fetch.message.contains("origin uses HTTP(S)"));
        assert!(fetch.next.contains("remote set-url 'origin'"));
        assert!(!fetch.next.contains("--push"));
        assert!(push.next.contains("remote set-url --push 'origin'"));
        for output in [&fetch.message, &fetch.next, &push.message, &push.next] {
            assert!(!output.contains("token"));
            assert!(!output.contains("hidden"));
        }
    }

    #[test]
    fn doctor_report_separates_blockers_from_advisories() {
        let report = DoctorReport {
            repositories: vec![
                RepositoryDiagnosis {
                    repository: "zeta".to_string(),
                    path: "./zeta".to_string(),
                    blockers: vec![DoctorFinding::new("no upstream", "fix upstream")],
                    advisories: Vec::new(),
                },
                RepositoryDiagnosis {
                    repository: "alpha".to_string(),
                    path: "./alpha".to_string(),
                    blockers: Vec::new(),
                    advisories: vec![DoctorFinding::new("uses HTTP(S)", "use SSH")],
                },
                RepositoryDiagnosis {
                    repository: "healthy".to_string(),
                    path: "./healthy".to_string(),
                    blockers: Vec::new(),
                    advisories: Vec::new(),
                },
            ],
            ..DoctorReport::default()
        };

        let output = report.render(std::time::Duration::from_secs(2));

        assert!(output.contains("Healthy         1"));
        assert!(output.contains("Warnings        1"));
        assert!(output.contains("Blockers        1"));
        assert!(output.contains("Checked         3"));
        assert!(output.contains("▌ Blockers"));
        assert!(output.contains("path: ./zeta"));
        assert!(output.contains("next: fix upstream"));
        assert!(output.contains("▌ Advisories"));
        assert!(output.contains("path: ./alpha"));
        assert!(!output.contains("path: ./healthy"));
    }

    #[test]
    fn configured_remote_urls_are_parsed_in_one_batch() {
        let urls = parse_configured_remote_urls(
            b"remote.origin.url\ngit@github.com:team/repo.git\0\
              remote.origin.pushurl\ngit@github.com:team/write.git\0\
              remote.backup.url\nhttps://github.com/team/repo.git\0",
        )
        .expect("valid config records");

        assert_eq!(
            urls.urls("origin", RemoteDirection::Fetch),
            ["git@github.com:team/repo.git"]
        );
        assert_eq!(
            urls.urls("origin", RemoteDirection::Push),
            ["git@github.com:team/write.git"]
        );
        assert_eq!(
            urls.urls("backup", RemoteDirection::Fetch),
            ["https://github.com/team/repo.git"]
        );
    }

    #[test]
    fn malformed_configured_remote_urls_fail_closed() {
        assert!(parse_configured_remote_urls(b"remote.origin.url-without-value\0").is_err());
    }
}
