//! Git LFS detection and publication helpers.

use super::*;
use crate::git::remote::{transport_policy, RemoteTransport, TransportPolicy};
use sha2::{Digest, Sha256};

pub(crate) const LFS_REMOTE_SELECTION_ARGS: &[&str] = &[
    "-c",
    "lfs.remote.autodetect=false",
    "-c",
    "lfs.remote.searchall=false",
];

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum LfsEndpointOperation {
    Download,
    Upload,
}

impl LfsEndpointOperation {
    const fn label(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::Upload => "upload",
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct LfsEndpointSnapshot {
    pub(crate) fingerprint: [u8; 32],
    transport: RemoteTransport,
}

#[derive(Clone, Copy)]
enum LfsEndpointSource {
    Explicit,
    Derived,
    Local,
}

pub async fn check_uses_git_lfs(path: &Path) -> bool {
    match run_git(path, GIT_LFS_ENV_ARGS).await {
        Ok((true, _, _)) => {
            let gitattributes = path.join(".gitattributes");
            if let Ok(content) = tokio::fs::read_to_string(&gitattributes).await {
                if content.contains("filter=lfs") {
                    return true;
                }
            }
            if let Ok((true, files, _)) = run_git(path, &["lfs", "ls-files"]).await {
                return !files.trim().is_empty();
            }
            false
        }
        _ => false,
    }
}

pub(crate) async fn check_may_push_git_lfs(path: &Path) -> Result<bool> {
    let (available, _, _) = run_git(path, GIT_LFS_ENV_ARGS).await?;
    if !available {
        return Ok(false);
    }

    let gitattributes = path.join(".gitattributes");
    if tokio::fs::read_to_string(&gitattributes)
        .await
        .is_ok_and(|content| content.contains("filter=lfs"))
    {
        return Ok(true);
    }

    for args in [
        &["lfs", "ls-files"][..],
        &["lfs", "ls-files", "--all", "--include=", "--exclude="][..],
    ] {
        let (success, files, _) = run_git(path, args).await?;
        if !success {
            anyhow::bail!("Git LFS push inspection failed");
        }
        if !files.trim().is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) async fn fetch_lfs_for_commit(
    path: &Path,
    remote: &str,
    commit: &str,
    expected_endpoint: Option<[u8; 32]>,
) -> Result<bool> {
    let lfs_available = matches!(run_git(path, GIT_LFS_ENV_ARGS).await, Ok((true, _, _)));
    if !lfs_available {
        let (has_attributes, _, stderr) = run_git(
            path,
            &[
                "grep",
                "-I",
                "-q",
                "-E",
                "filter[[:space:]]*=[[:space:]]*lfs",
                commit,
                "--",
                ":(glob)**/.gitattributes",
            ],
        )
        .await?;
        if has_attributes {
            anyhow::bail!("target commit uses Git LFS, but git-lfs is unavailable");
        }
        if !stderr.is_empty() {
            anyhow::bail!(
                "Git LFS target inspection failed: {}",
                command_error(&stderr, "attributes could not be inspected")
            );
        }
        return Ok(false);
    }

    let (uses_lfs, files, stderr) = run_git(
        path,
        &["lfs", "ls-files", "--include=", "--exclude=", commit],
    )
    .await?;
    if !uses_lfs {
        anyhow::bail!(
            "Git LFS target inspection failed: {}",
            command_error(&stderr, "pointers could not be inspected")
        );
    }
    if files.is_empty() {
        return Ok(false);
    }

    let endpoint = inspect_lfs_endpoint(path, remote, LfsEndpointOperation::Download).await?;
    if expected_endpoint.is_some_and(|expected| expected != endpoint.fingerprint) {
        anyhow::bail!("Git LFS endpoint changed after pull analysis; rerun the command");
    }

    let mut fetch_args = Vec::from(LFS_REMOTE_SELECTION_ARGS);
    fetch_args.extend([
        "-c",
        "lfs.fetchrecentalways=false",
        "lfs",
        "fetch",
        "--include=",
        "--exclude=",
        "--",
        remote,
        commit,
    ]);
    let (fetched, _, stderr) = run_git(path, &fetch_args).await?;
    if !fetched {
        anyhow::bail!(
            "Git LFS target fetch failed: {}",
            command_error(&stderr, "objects could not be fetched")
        );
    }
    let current_endpoint =
        inspect_lfs_endpoint(path, remote, LfsEndpointOperation::Download).await?;
    if current_endpoint != endpoint {
        anyhow::bail!("Git LFS endpoint changed during target fetch; rerun the command");
    }
    let (valid, _, stderr) = run_git(
        path,
        &[
            "-c",
            "lfs.fetchinclude=",
            "-c",
            "lfs.fetchexclude=",
            "lfs",
            "fsck",
            "--objects",
            commit,
        ],
    )
    .await?;
    if !valid {
        anyhow::bail!(
            "Git LFS target verification failed: {}",
            command_error(&stderr, "fetched objects are missing or corrupt")
        );
    }
    Ok(true)
}

pub(crate) async fn inspect_lfs_endpoint(
    path: &Path,
    remote: &str,
    operation: LfsEndpointOperation,
) -> Result<LfsEndpointSnapshot> {
    let snapshot = snapshot_lfs_endpoint(path, remote, operation).await?;
    if transport_policy()? == TransportPolicy::SshOnly && snapshot.transport.is_http() {
        anyhow::bail!(
            "ssh-only policy blocked Git LFS {}: endpoint for remote '{}' uses {}",
            operation.label(),
            remote,
            snapshot.transport.label()
        );
    }
    Ok(snapshot)
}

pub(crate) async fn snapshot_lfs_endpoint(
    path: &Path,
    remote: &str,
    operation: LfsEndpointOperation,
) -> Result<LfsEndpointSnapshot> {
    let explicit = effective_lfs_override(path, remote, operation).await?;
    let (source, raw_url) = if let Some(url) = explicit {
        (LfsEndpointSource::Explicit, url)
    } else if let Some(url) = derived_remote_url(path, remote, operation).await? {
        (LfsEndpointSource::Derived, url)
    } else if remote == "." {
        (LfsEndpointSource::Local, String::new())
    } else {
        anyhow::bail!("Git LFS endpoint for remote '{remote}' could not be resolved");
    };
    let effective_url = if matches!(source, LfsEndpointSource::Local) {
        "local-dot-remote".to_string()
    } else {
        expand_url_alias(path, &raw_url, operation).await?
    };
    let transport = if matches!(source, LfsEndpointSource::Local) {
        RemoteTransport::Local
    } else {
        RemoteTransport::from_url(&effective_url)
    };

    let mut digest = Sha256::new();
    digest.update(b"repos-lfs-endpoint-v1\0");
    digest.update(operation.label().as_bytes());
    digest.update(b"\0");
    digest.update(match source {
        LfsEndpointSource::Explicit => b"explicit".as_slice(),
        LfsEndpointSource::Derived => b"derived".as_slice(),
        LfsEndpointSource::Local => b"local".as_slice(),
    });
    digest.update(b"\0");
    digest.update(effective_url.as_bytes());

    Ok(LfsEndpointSnapshot {
        fingerprint: digest.finalize().into(),
        transport,
    })
}

async fn effective_lfs_override(
    path: &Path,
    remote: &str,
    operation: LfsEndpointOperation,
) -> Result<Option<String>> {
    if operation == LfsEndpointOperation::Upload {
        if let Some(url) = merged_lfs_config_value(path, "lfs.pushurl", true).await? {
            return Ok(Some(url));
        }
    }
    if let Some(url) = merged_lfs_config_value(path, "lfs.url", true).await? {
        return Ok(Some(url));
    }
    if operation == LfsEndpointOperation::Upload {
        let key = format!("remote.{remote}.lfspushurl");
        if let Some(url) = regular_config_value(path, &key).await? {
            return Ok(Some(url));
        }
    }
    let key = format!("remote.{remote}.lfsurl");
    merged_lfs_config_value(path, &key, true).await
}

async fn derived_remote_url(
    path: &Path,
    remote: &str,
    operation: LfsEndpointOperation,
) -> Result<Option<String>> {
    if operation == LfsEndpointOperation::Upload {
        let push_key = format!("remote.{remote}.pushurl");
        if let Some(url) = regular_config_value(path, &push_key).await? {
            return Ok(Some(url));
        }
    }
    regular_config_value(path, &format!("remote.{remote}.url")).await
}

async fn merged_lfs_config_value(
    path: &Path,
    key: &str,
    allow_repository_config: bool,
) -> Result<Option<String>> {
    if let Some(value) = regular_config_value(path, key).await? {
        return Ok(Some(value));
    }
    if allow_repository_config {
        repository_lfs_config_value(path, key).await
    } else {
        Ok(None)
    }
}

async fn regular_config_value(path: &Path, key: &str) -> Result<Option<String>> {
    config_value(path, &["config", "--includes", "--get", key], false).await
}

async fn repository_lfs_config_value(path: &Path, key: &str) -> Result<Option<String>> {
    match std::fs::metadata(path.join(".lfsconfig")) {
        Ok(_) => {
            return config_value(
                path,
                &["config", "--includes", "--file=.lfsconfig", "--get", key],
                false,
            )
            .await;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    for revision in [":.lfsconfig", "HEAD:.lfsconfig"] {
        let source = run_git(
            path,
            &["config", "--includes", "--blob", revision, "--list"],
        )
        .await?;
        if source.0 {
            return config_value(
                path,
                &["config", "--includes", "--blob", revision, "--get", key],
                false,
            )
            .await;
        }
    }
    Ok(None)
}

async fn config_value(
    path: &Path,
    args: &[&str],
    missing_is_error: bool,
) -> Result<Option<String>> {
    let output = run_git_raw(path, args).await?;
    if output.success() {
        return Ok(Some(output.stdout_text()));
    }
    if !missing_is_error && output.exit_code == Some(1) && output.stderr.is_empty() {
        return Ok(None);
    }
    anyhow::bail!("Git LFS configuration could not be inspected")
}

async fn expand_url_alias(
    path: &Path,
    url: &str,
    operation: LfsEndpointOperation,
) -> Result<String> {
    if url.is_empty() {
        return Ok(String::new());
    }

    // Resolve `insteadOf` in memory. Passing the configured endpoint back to
    // Git as an argument would expose credential-bearing URLs in process
    // listings even though diagnostics never render them.
    if operation == LfsEndpointOperation::Upload {
        if let Some(rewritten) =
            rewrite_url_alias(path, url, r"^url\..*\.pushinsteadof$", b".pushinsteadof").await?
        {
            return Ok(rewritten);
        }
    }
    Ok(
        rewrite_url_alias(path, url, r"^url\..*\.insteadof$", b".insteadof")
            .await?
            .unwrap_or_else(|| url.to_string()),
    )
}

async fn rewrite_url_alias(
    path: &Path,
    url: &str,
    pattern: &str,
    key_suffix: &[u8],
) -> Result<Option<String>> {
    let output = run_git_raw(
        path,
        &["config", "--includes", "--null", "--get-regexp", pattern],
    )
    .await?;
    if !output.success() {
        if output.exit_code == Some(1) && output.stderr.is_empty() {
            return Ok(None);
        }
        anyhow::bail!("Git LFS endpoint URL rewrite could not be inspected");
    }

    let mut best: Option<(&[u8], &[u8])> = None;
    for entry in output.stdout.split(|byte| *byte == 0) {
        if entry.is_empty() {
            continue;
        }
        let Some(separator) = entry.iter().position(|byte| *byte == b'\n') else {
            anyhow::bail!("Git LFS endpoint URL rewrite returned invalid configuration");
        };
        let key = &entry[..separator];
        let prefix = &entry[separator + 1..];
        const KEY_PREFIX: &[u8] = b"url.";
        if key.len() <= KEY_PREFIX.len() + key_suffix.len()
            || !key[..KEY_PREFIX.len()].eq_ignore_ascii_case(KEY_PREFIX)
            || !key[key.len() - key_suffix.len()..].eq_ignore_ascii_case(key_suffix)
        {
            anyhow::bail!("Git LFS endpoint URL rewrite returned an invalid key");
        }
        if url.as_bytes().starts_with(prefix)
            && best.map_or(true, |(_, best_prefix)| prefix.len() > best_prefix.len())
        {
            let replacement = &key[KEY_PREFIX.len()..key.len() - key_suffix.len()];
            best = Some((replacement, prefix));
        }
    }

    let Some((replacement, prefix)) = best else {
        return Ok(None);
    };
    let mut rewritten = Vec::with_capacity(replacement.len() + url.len() - prefix.len());
    rewritten.extend_from_slice(replacement);
    rewritten.extend_from_slice(&url.as_bytes()[prefix.len()..]);
    String::from_utf8(rewritten)
        .map(Some)
        .map_err(|_| anyhow::anyhow!("Git LFS endpoint URL rewrite returned invalid UTF-8"))
}

pub(crate) fn option_like_lfs_remote_error(remote: &str) -> Option<String> {
    remote.starts_with('-').then(|| {
        format!("Git LFS cannot safely use remote name '{remote}'; rename it without a leading '-'")
    })
}

pub async fn push_lfs_objects(path: &Path, remote: &str, branch: &str) -> (bool, String) {
    let mut args = Vec::from(LFS_REMOTE_SELECTION_ARGS);
    args.extend(["lfs", "push", "--all", "--", remote, branch]);
    match run_git(path, &args).await {
        Ok((true, _, _)) => (true, String::new()),
        Ok((false, _, stderr)) => {
            let message = if stderr.is_empty() {
                "LFS push failed".to_string()
            } else {
                format!("LFS: {}", stderr.lines().next().unwrap_or("push failed"))
            };
            (false, message)
        }
        Err(error) => (false, format!("LFS error: {error}")),
    }
}

pub async fn has_pending_lfs_objects(path: &Path) -> bool {
    if let Ok((true, stdout, _)) = run_git(path, &["lfs", "status", "--porcelain"]).await {
        !stdout.trim().is_empty()
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{
        check_may_push_git_lfs, fetch_lfs_for_commit, snapshot_lfs_endpoint, LfsEndpointOperation,
        RemoteTransport,
    };
    use std::path::Path;
    use std::process::Command;

    fn git(path: &Path, args: &[&str]) -> String {
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

    fn initialize(path: &Path) {
        git(path, &["init"]);
        git(path, &["config", "user.name", "repos test"]);
        git(path, &["config", "user.email", "repos@example.invalid"]);
    }

    #[tokio::test]
    async fn exact_non_lfs_commit_needs_no_fetch() {
        let directory = tempfile::tempdir().expect("temporary repository");
        initialize(directory.path());
        std::fs::write(directory.path().join("tracked.txt"), "content")
            .expect("write tracked file");
        git(directory.path(), &["add", "tracked.txt"]);
        git(directory.path(), &["commit", "-m", "Initial"]);
        let commit = git(directory.path(), &["rev-parse", "HEAD"]);

        assert!(
            !fetch_lfs_for_commit(directory.path(), "origin", &commit, None)
                .await
                .expect("inspect exact non-LFS commit")
        );
    }

    #[tokio::test]
    async fn exact_lfs_commit_fetches_from_the_selected_remote() {
        if !Command::new("git")
            .args(["lfs", "version"])
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return;
        }

        let root = tempfile::tempdir().expect("temporary root");
        let repository = root.path().join("repository");
        let remote = root.path().join("remote.git");
        std::fs::create_dir(&repository).expect("create repository");
        initialize(&repository);
        git(root.path(), &["init", "--bare", remote.to_str().unwrap()]);
        git(&repository, &["lfs", "install", "--local"]);
        git(
            &repository,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&repository, &["lfs", "track", "*.bin"]);
        std::fs::write(repository.join("asset.bin"), b"LFS payload").expect("write LFS fixture");
        git(&repository, &["add", ".gitattributes", "asset.bin"]);
        git(&repository, &["commit", "-m", "Add LFS asset"]);
        let commit = git(&repository, &["rev-parse", "HEAD"]);
        let branch = git(&repository, &["branch", "--show-current"]);
        git(&repository, &["push", "-u", "origin", &branch]);

        assert!(fetch_lfs_for_commit(&repository, "origin", &commit, None)
            .await
            .expect("fetch exact LFS commit"));
    }

    #[tokio::test]
    async fn push_detection_fails_closed_when_history_cannot_be_scanned() {
        if !Command::new("git")
            .args(["lfs", "version"])
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return;
        }

        let directory = tempfile::tempdir().expect("temporary repository");
        initialize(directory.path());
        git(directory.path(), &["lfs", "install", "--local"]);
        git(directory.path(), &["lfs", "track", "*.bin"]);
        std::fs::write(
            directory.path().join("asset.bin"),
            b"historical LFS payload",
        )
        .expect("write LFS fixture");
        git(directory.path(), &["add", ".gitattributes", "asset.bin"]);
        git(directory.path(), &["commit", "-m", "Add LFS asset"]);
        std::fs::remove_file(directory.path().join("asset.bin")).expect("remove LFS fixture");
        std::fs::remove_file(directory.path().join(".gitattributes")).expect("remove attributes");
        git(directory.path(), &["add", "-u"]);
        git(directory.path(), &["commit", "-m", "Remove LFS asset"]);
        std::fs::write(
            directory.path().join(".git/refs/heads/broken"),
            "0000000000000000000000000000000000000001\n",
        )
        .expect("write broken ref");

        assert!(check_may_push_git_lfs(directory.path()).await.is_err());
    }

    #[tokio::test]
    async fn download_endpoint_uses_global_lfs_url_before_remote_override() {
        let directory = tempfile::tempdir().expect("temporary repository");
        initialize(directory.path());
        git(
            directory.path(),
            &[
                "remote",
                "add",
                "origin",
                "ssh://git@example.invalid/repo.git",
            ],
        );
        git(
            directory.path(),
            &[
                "config",
                "remote.origin.lfsurl",
                "ssh://git@lfs.example.invalid/repo.git",
            ],
        );
        git(
            directory.path(),
            &["config", "lfs.url", "https://lfs.example.invalid/repo"],
        );

        let first =
            snapshot_lfs_endpoint(directory.path(), "origin", LfsEndpointOperation::Download)
                .await
                .expect("inspect download endpoint");
        assert_eq!(first.transport, RemoteTransport::Https);

        git(
            directory.path(),
            &["config", "lfs.url", "ssh://git@other.invalid/repo.git"],
        );
        let changed =
            snapshot_lfs_endpoint(directory.path(), "origin", LfsEndpointOperation::Download)
                .await
                .expect("inspect changed endpoint");
        assert_eq!(changed.transport, RemoteTransport::Ssh);
        assert_ne!(changed.fingerprint, first.fingerprint);
    }

    #[tokio::test]
    async fn endpoint_alias_uses_operation_specific_longest_prefixes() {
        let directory = tempfile::tempdir().expect("temporary repository");
        initialize(directory.path());
        git(
            directory.path(),
            &[
                "remote",
                "add",
                "origin",
                "ssh://git@example.invalid/repo.git",
            ],
        );
        git(
            directory.path(),
            &["config", "lfs.url", "mirror:team/repo.git"],
        );
        git(
            directory.path(),
            &[
                "config",
                "url.ssh://git@fallback.invalid/.insteadOf",
                "mirror:",
            ],
        );
        git(
            directory.path(),
            &[
                "config",
                "url.https://lfs.example.invalid/.insteadOf",
                "mirror:team/",
            ],
        );
        git(
            directory.path(),
            &[
                "config",
                "url.http://upload.example.invalid/.pushInsteadOf",
                "mirror:team/",
            ],
        );

        let download =
            snapshot_lfs_endpoint(directory.path(), "origin", LfsEndpointOperation::Download)
                .await
                .expect("inspect rewritten endpoint");
        assert_eq!(download.transport, RemoteTransport::Https);
        let upload =
            snapshot_lfs_endpoint(directory.path(), "origin", LfsEndpointOperation::Upload)
                .await
                .expect("inspect rewritten upload endpoint");
        assert_eq!(upload.transport, RemoteTransport::Http);
    }

    #[tokio::test]
    async fn upload_endpoint_matches_git_lfs_override_precedence() {
        let directory = tempfile::tempdir().expect("temporary repository");
        initialize(directory.path());
        git(
            directory.path(),
            &[
                "remote",
                "add",
                "origin",
                "ssh://git@example.invalid/repo.git",
            ],
        );
        git(
            directory.path(),
            &[
                "config",
                "remote.origin.lfsurl",
                "ssh://git@shared.invalid/repo.git",
            ],
        );
        let shared =
            snapshot_lfs_endpoint(directory.path(), "origin", LfsEndpointOperation::Upload)
                .await
                .expect("inspect shared LFS endpoint");
        assert_eq!(shared.transport, RemoteTransport::Ssh);

        git(
            directory.path(),
            &[
                "config",
                "remote.origin.lfspushurl",
                "https://remote-push.invalid/lfs",
            ],
        );

        let remote_push =
            snapshot_lfs_endpoint(directory.path(), "origin", LfsEndpointOperation::Upload)
                .await
                .expect("inspect remote upload endpoint");
        assert_eq!(remote_push.transport, RemoteTransport::Https);

        git(
            directory.path(),
            &["config", "lfs.url", "ssh://git@push.invalid/repo.git"],
        );
        let global_push =
            snapshot_lfs_endpoint(directory.path(), "origin", LfsEndpointOperation::Upload)
                .await
                .expect("inspect global upload endpoint");
        assert_eq!(global_push.transport, RemoteTransport::Ssh);
        assert_ne!(global_push.fingerprint, remote_push.fingerprint);

        git(
            directory.path(),
            &["config", "lfs.pushurl", "http://global-push.invalid/lfs"],
        );
        let push_only =
            snapshot_lfs_endpoint(directory.path(), "origin", LfsEndpointOperation::Upload)
                .await
                .expect("inspect push-only endpoint");
        assert_eq!(push_only.transport, RemoteTransport::Http);
        assert_ne!(push_only.fingerprint, global_push.fingerprint);
    }

    #[tokio::test]
    async fn local_dot_remote_has_a_stable_local_endpoint() {
        let directory = tempfile::tempdir().expect("temporary repository");
        initialize(directory.path());

        let endpoint = snapshot_lfs_endpoint(directory.path(), ".", LfsEndpointOperation::Download)
            .await
            .expect("inspect local dot remote");
        assert_eq!(endpoint.transport, RemoteTransport::Local);
    }
}
