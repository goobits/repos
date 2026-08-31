//! History-rewrite planning, execution, verification, and repository safety.

use super::{
    ensure_command_success, write_private_temp_file, FixOptions, HISTORY_PUBLICATION_GUIDANCE,
};
use crate::audit::hygiene::scanner::check_large_files;
use crate::audit::scanner::{scan_repository_secrets, ScannedSecret, TruffleScanMode};
use crate::git::operations::{get_upstream_push_target, run_git};
use crate::git::remote::{inspect_remote, policy_violation, RemoteDirection};
use anyhow::{anyhow, bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use tokio::process::Command;

#[derive(Default)]
struct HistoryRewritePlan {
    remove_paths: BTreeSet<String>,
    replacements_by_path: BTreeMap<String, BTreeSet<String>>,
    large_path_count: usize,
    secret_finding_count: usize,
}

impl HistoryRewritePlan {
    fn is_empty(&self) -> bool {
        self.remove_paths.is_empty() && self.replacement_literals().is_empty()
    }

    fn replacement_literals(&self) -> BTreeSet<String> {
        self.replacements_by_path
            .iter()
            .filter(|(path, _)| !self.remove_paths.contains(*path))
            .flat_map(|(_, values)| values.iter().cloned())
            .collect()
    }
}

pub(super) async fn rewrite_history(repo_path: &Path, options: &FixOptions) -> Result<String> {
    let mut plan = build_history_rewrite_plan(repo_path, options).await?;
    let replacements = if plan.replacement_literals().is_empty() {
        BTreeMap::new()
    } else {
        collect_replacement_blobs(repo_path, &mut plan).await?
    };
    if plan.is_empty() {
        return Ok("No matching history findings remain".to_string());
    }

    if options.dry_run {
        return Ok(format!(
            "[DRY RUN] Would remove {} paths and redact {} secret values",
            plan.remove_paths.len(),
            plan.replacement_literals().len()
        ));
    }
    check_history_rewrite_tools().await?;

    let replacements_file = if replacements.is_empty() {
        None
    } else {
        let encoded = encode_replacement_blobs(replacements);
        let contents = serde_json::to_string(&encoded)?;
        Some(write_private_temp_file(
            "filter-repo-secret-blobs",
            &contents,
        )?)
    };
    let blob_callback = replacements_file
        .as_ref()
        .map(|file| blob_replacement_callback(&file.path))
        .transpose()?;
    let paths_file = if plan.remove_paths.is_empty() {
        None
    } else {
        let contents = plan
            .remove_paths
            .iter()
            .map(|path| format!("literal:{path}\n"))
            .collect::<String>();
        Some(write_private_temp_file("filter-repo-paths", &contents)?)
    };

    let backup_bundle = create_backup_bundle(repo_path).await?;
    eprintln!(
        "    Full-ref backup created at: {}",
        backup_bundle.display()
    );

    let mut command = Command::new("git");
    command.arg("filter-repo");
    if let Some(callback) = &blob_callback {
        command.arg("--blob-callback").arg(callback);
    }
    if let Some(file) = &paths_file {
        command
            .arg("--invert-paths")
            .arg("--paths-from-file")
            .arg(&file.path);
    }
    let output = command
        .arg("--force")
        .current_dir(repo_path)
        .output()
        .await?;
    if !output.status.success() {
        bail!(
            "git filter-repo failed: {}\nRecovery bundle: {}",
            String::from_utf8_lossy(&output.stderr).trim(),
            backup_bundle.display()
        );
    }

    if options.fix_secrets {
        let remaining = scan_repository_secrets(repo_path, TruffleScanMode::Offline)
            .await
            .with_context(|| {
                format!(
                    "post-rewrite secret verification failed; recovery bundle: {}",
                    backup_bundle.display()
                )
            })?;
        if !remaining.is_empty() {
            bail!(
                "post-rewrite verification still found {} secrets; recovery bundle: {}",
                remaining.len(),
                backup_bundle.display()
            );
        }
    }
    if options.fix_large {
        let remaining = check_large_files(repo_path).await.with_context(|| {
            format!(
                "post-rewrite large-file verification failed; recovery bundle: {}",
                backup_bundle.display()
            )
        })?;
        if !remaining.is_empty() {
            bail!(
                "post-rewrite verification still found {} large objects; recovery bundle: {}",
                remaining.len(),
                backup_bundle.display()
            );
        }
    }

    Ok(format!(
        "Rewrote history for {} large paths and {} secret findings\n    {}\n    Recovery bundle: {}",
        plan.large_path_count,
        plan.secret_finding_count,
        HISTORY_PUBLICATION_GUIDANCE,
        backup_bundle.display()
    ))
}

async fn build_history_rewrite_plan(
    repo_path: &Path,
    options: &FixOptions,
) -> Result<HistoryRewritePlan> {
    let mut plan = HistoryRewritePlan::default();
    if options.fix_large {
        for violation in check_large_files(repo_path).await? {
            validate_repository_path(&violation.file_path)?;
            plan.remove_paths.insert(violation.file_path.clone());
        }
        plan.large_path_count = plan.remove_paths.len();
    }

    if options.fix_secrets {
        let secrets = scan_repository_secrets(repo_path, TruffleScanMode::Offline).await?;
        plan.secret_finding_count = secrets.len();
        for secret in secrets {
            add_secret_to_plan(&mut plan, secret)?;
        }
    }

    Ok(plan)
}

fn add_secret_to_plan(plan: &mut HistoryRewritePlan, secret: ScannedSecret) -> Result<()> {
    let file_path = secret.finding.file_path;
    validate_repository_path(&file_path)?;

    let mut values = Vec::new();
    values.extend(secret.raw);
    if secret.secret_parts.is_empty() {
        // RawV2 is the best available complete representation for detectors
        // without structured parts. It is validated against historical blob
        // contents before use; a synthetic/nonexistent value causes the path
        // to be removed instead.
        values.extend(secret.raw_v2);
    } else {
        // Multipart detectors can use Raw as an identifier and RawV2 as a
        // synthetic joined value. Redact every reported source component in
        // the affected path rather than trusting that joined value.
        values.extend(secret.secret_parts);
    }
    if values.is_empty()
        || values
            .iter()
            .any(|value| validate_filter_literal(value, "secret value").is_err())
    {
        plan.remove_paths.insert(file_path.clone());
        plan.replacements_by_path.remove(&file_path);
    } else if !plan.remove_paths.contains(&file_path) {
        plan.replacements_by_path
            .entry(file_path)
            .or_default()
            .extend(values);
    }
    Ok(())
}

async fn collect_replacement_blobs(
    repo_path: &Path,
    plan: &mut HistoryRewritePlan,
) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let mut replacements = BTreeMap::<String, BTreeSet<String>>::new();
    let planned = plan.replacements_by_path.clone();
    let mut fallback_paths = BTreeSet::new();
    for (path, values) in planned {
        if plan.remove_paths.contains(&path) {
            continue;
        }
        let pathspec = format!(":(literal){path}");
        let output = Command::new("git")
            .args([
                "-c",
                "core.quotePath=false",
                "rev-list",
                "--objects",
                "--all",
                "--",
                &pathspec,
            ])
            .current_dir(repo_path)
            .output()
            .await?;
        ensure_command_success(&output, "resolving secret-containing history blobs")?;
        let stdout = String::from_utf8(output.stdout)?;
        let mut object_ids = BTreeSet::new();
        for line in stdout.lines() {
            let Some((object_id, object_path)) = line.split_once(' ') else {
                continue;
            };
            if object_path != path {
                continue;
            }
            if !matches!(object_id.len(), 40 | 64)
                || !object_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                bail!("git returned an invalid historical blob identifier");
            }
            object_ids.insert(object_id.to_string());
        }
        if object_ids.is_empty() {
            bail!("could not resolve historical blobs for secret path {path}");
        }

        let mut path_replacements = BTreeMap::<String, BTreeSet<String>>::new();
        let mut observed_values = BTreeSet::new();
        for object_id in object_ids {
            let output = Command::new("git")
                .args(["cat-file", "blob", &object_id])
                .current_dir(repo_path)
                .output()
                .await?;
            ensure_command_success(&output, "reading a secret-containing historical blob")?;
            for value in &values {
                if contains_bytes(&output.stdout, value.as_bytes()) {
                    path_replacements
                        .entry(object_id.clone())
                        .or_default()
                        .insert(value.clone());
                    observed_values.insert(value.clone());
                }
            }
        }

        if observed_values != values {
            // A detector may synthesize RawV2 or omit a multipart component.
            // Removing the affected path is the only fail-closed rewrite when
            // every planned credential component cannot be located verbatim.
            fallback_paths.insert(path);
            continue;
        }
        for (object_id, values) in path_replacements {
            replacements.entry(object_id).or_default().extend(values);
        }
    }

    for path in fallback_paths {
        plan.replacements_by_path.remove(&path);
        plan.remove_paths.insert(path);
    }
    Ok(replacements)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn encode_replacement_blobs(
    replacements: BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<String, Vec<String>> {
    replacements
        .into_iter()
        .map(|(object_id, values)| {
            let mut values = values.into_iter().collect::<Vec<_>>();
            values
                .sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
            (
                object_id,
                values
                    .into_iter()
                    .map(|value| hex_encode(value.as_bytes()))
                    .collect(),
            )
        })
        .collect()
}

fn hex_encode(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn blob_replacement_callback(plan_path: &Path) -> Result<String> {
    let plan_path = plan_path
        .to_str()
        .ok_or_else(|| anyhow!("temporary rewrite plan path was not valid UTF-8"))?;
    let encoded_path = hex_encode(plan_path.as_bytes());
    Ok(format!(
        concat!(
            "if not hasattr(callback, '_repos_plan'):\n",
            "    import json\n",
            "    plan_path = bytes.fromhex('{encoded_path}').decode('utf-8')\n",
            "    with open(plan_path, encoding='utf-8') as stream:\n",
            "        encoded_plan = json.load(stream)\n",
            "    callback._repos_plan = {{\n",
            "        object_id.encode('ascii'): [bytes.fromhex(value) for value in values]\n",
            "        for object_id, values in encoded_plan.items()\n",
            "    }}\n",
            "patterns = callback._repos_plan.get(blob.original_id)\n",
            "if patterns:\n",
            "    for pattern in patterns:\n",
            "        blob.data = blob.data.replace(pattern, b'REDACTED')"
        ),
        encoded_path = encoded_path
    ))
}

fn validate_repository_path(path: &str) -> Result<()> {
    validate_filter_literal(path, "secret file path")?;
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("audit returned an unsafe repository-relative file path");
    }
    Ok(())
}

fn validate_filter_literal(value: &str, kind: &str) -> Result<()> {
    if value.is_empty() || value.contains(['\n', '\r', '\0']) || value.contains("==>") {
        bail!("{kind} cannot be represented safely for git filter-repo");
    }
    Ok(())
}

async fn create_backup_bundle(repo_path: &Path) -> Result<PathBuf> {
    let common_dir_output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(repo_path)
        .output()
        .await?;
    ensure_command_success(&common_dir_output, "resolving the common Git directory")?;
    let common_dir_text = String::from_utf8(common_dir_output.stdout)?;
    let common_dir = PathBuf::from(common_dir_text.trim());
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        repo_path.join(common_dir)
    };
    let common_dir = fs::canonicalize(&common_dir)
        .with_context(|| format!("resolving Git directory {}", common_dir.display()))?;
    let backup_root = common_dir.join("repos-backups");
    fs::create_dir_all(&backup_root)?;

    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let mut backup_dir = None;
    for attempt in 0..100 {
        let candidate = backup_root.join(format!(
            "audit-{timestamp}-{}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&candidate) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&candidate, fs::Permissions::from_mode(0o700))?;
                }
                backup_dir = Some(candidate);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    let backup_dir = backup_dir.ok_or_else(|| anyhow!("failed to create backup directory"))?;
    let bundle_path = backup_dir.join("before.bundle");
    let create = Command::new("git")
        .arg("bundle")
        .arg("create")
        .arg(&bundle_path)
        .args(["--all", "HEAD"])
        .current_dir(repo_path)
        .output()
        .await?;
    ensure_command_success(&create, "creating full-ref history backup")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bundle_path, fs::Permissions::from_mode(0o600))?;
    }
    let verify = Command::new("git")
        .arg("bundle")
        .arg("verify")
        .arg(&bundle_path)
        .current_dir(repo_path)
        .output()
        .await?;
    ensure_command_success(&verify, "verifying full-ref history backup")?;
    Ok(bundle_path)
}

pub(super) async fn check_repository_safety(repo_path: &Path, options: &FixOptions) -> Result<()> {
    if options.dry_run {
        return Ok(());
    }

    let status = run_git(repo_path, &["status", "--porcelain"]).await?;
    if !status.0 {
        return Err(anyhow!("git status failed: {}", status.2));
    }
    if !status.1.is_empty() {
        return Err(anyhow!(
            "Repository has uncommitted changes:\n{}\n\n\
             Please commit or stash changes before running fixes.\n\
             Run: git stash push -m \"Before repos fix\"",
            status.1
        ));
    }

    if options.fix_large || options.fix_secrets {
        let remotes = run_git(repo_path, &["remote"]).await?;
        if !remotes.0 {
            return Err(anyhow!("remote inspection failed: {}", remotes.2));
        }
        if remotes.1.trim().is_empty() {
            return Ok(());
        }

        let branch = run_git(repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
        if !branch.0 || branch.1 == "HEAD" {
            return Err(anyhow!(
                "History rewrite requires an attached branch with a configured upstream: {}",
                branch.2
            ));
        }
        let Some((remote, _)) = get_upstream_push_target(repo_path, &branch.1).await? else {
            return Err(anyhow!(
                "History rewrite requires a configured upstream for branch {}",
                branch.1
            ));
        };

        let remote_contexts = inspect_remote(repo_path, &remote, RemoteDirection::Fetch).await?;
        if let Some(violation) = policy_violation(&remote_contexts)? {
            bail!(violation.message());
        }

        let fetch = run_git(
            repo_path,
            &["fetch", "--quiet", "--prune", "--tags", "--", &remote],
        )
        .await?;
        if !fetch.0 {
            return Err(anyhow!("git fetch failed: {}", fetch.2));
        }

        let counts = crate::git::ancestry::ahead_behind(repo_path).await?;
        if counts.behind > 0 {
            return Err(anyhow!(
                "Repository is {} commits behind remote.\nPull changes first: git pull",
                counts.behind
            ));
        }
        if counts.ahead > 0 {
            eprintln!(
                "⚠️  Warning: Repository is {} commits ahead of remote.\n   {}",
                counts.ahead, HISTORY_PUBLICATION_GUIDANCE,
            );
        }
    }

    Ok(())
}

async fn check_history_rewrite_tools() -> Result<()> {
    let filter_repo = Command::new("git")
        .args(["filter-repo", "--version"])
        .output()
        .await;
    if !filter_repo.is_ok_and(|output| output.status.success()) {
        bail!(
            "git-filter-repo is required to rewrite history. Install it with a trusted package manager (for example, brew install git-filter-repo on macOS)"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        add_secret_to_plan, blob_replacement_callback, collect_replacement_blobs,
        create_backup_bundle, encode_replacement_blobs, validate_filter_literal,
        validate_repository_path, HistoryRewritePlan,
    };
    use crate::audit::scanner::{ScannedSecret, SecretFinding, SecretVerification};
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;
    use std::process::Command;

    fn scanned_secret(path: &str, raw: Option<&str>) -> ScannedSecret {
        ScannedSecret {
            finding: SecretFinding {
                detector_name: "Test".to_string(),
                verified: false,
                file_path: path.to_string(),
            },
            verification: SecretVerification::Unverified,
            raw: raw.map(str::to_string),
            raw_v2: None,
            secret_parts: Vec::new(),
        }
    }

    #[test]
    fn filter_literals_reject_expression_injection() {
        for value in ["", "line\nbreak", "line\rbreak", "nul\0byte", "a==>b"] {
            assert!(validate_filter_literal(value, "test value").is_err());
        }
    }

    #[test]
    fn repository_paths_must_be_relative_and_safe() {
        assert!(validate_repository_path("config/secrets.env").is_ok());
        assert!(validate_repository_path("../outside").is_err());
        assert!(validate_repository_path("/absolute").is_err());
    }

    #[test]
    fn mixed_secret_plan_combines_replacement_and_file_removal() {
        let mut plan = HistoryRewritePlan::default();
        add_secret_to_plan(&mut plan, scanned_secret("safe.env", Some("safe-token")))
            .expect("safe replacement");
        add_secret_to_plan(&mut plan, scanned_secret("remove.env", Some("old-token")))
            .expect("initial replacement");
        add_secret_to_plan(&mut plan, scanned_secret("remove.env", None))
            .expect("missing raw fallback");
        add_secret_to_plan(&mut plan, scanned_secret("unsafe.env", Some("multi\nline")))
            .expect("unsafe raw fallback");

        assert_eq!(
            plan.remove_paths,
            BTreeSet::from(["remove.env".to_string(), "unsafe.env".to_string()])
        );
        assert_eq!(
            plan.replacement_literals(),
            BTreeSet::from(["safe-token".to_string()])
        );
    }

    #[test]
    fn multipart_secret_plan_uses_source_parts_instead_of_synthetic_raw_v2() {
        let mut plan = HistoryRewritePlan::default();
        add_secret_to_plan(
            &mut plan,
            ScannedSecret {
                finding: SecretFinding {
                    detector_name: "AWS".to_string(),
                    verified: false,
                    file_path: "secret.env".to_string(),
                },
                verification: SecretVerification::Unverified,
                raw: Some("AKIAEXAMPLE".to_string()),
                raw_v2: Some("AKIAEXAMPLE:super-secret".to_string()),
                secret_parts: vec!["AKIAEXAMPLE".to_string(), "super-secret".to_string()],
            },
        )
        .expect("multipart replacement plan");

        assert_eq!(
            plan.replacement_literals(),
            BTreeSet::from(["AKIAEXAMPLE".to_string(), "super-secret".to_string()])
        );
        assert!(!plan
            .replacement_literals()
            .contains("AKIAEXAMPLE:super-secret"));
    }

    #[test]
    fn blob_replacements_are_encoded_longest_first() {
        let encoded = encode_replacement_blobs(BTreeMap::from([(
            "object".to_string(),
            BTreeSet::from(["abc".to_string(), "abcd".to_string()]),
        )]));

        assert_eq!(
            encoded.get("object"),
            Some(&vec!["61626364".to_string(), "616263".to_string()])
        );
    }

    #[tokio::test]
    async fn synthetic_raw_v2_falls_back_to_path_removal() {
        let directory = tempfile::tempdir().expect("temporary repository");
        for args in [
            vec!["init"],
            vec!["config", "user.name", "Test User"],
            vec!["config", "user.email", "test@example.com"],
        ] {
            let output = Command::new("git")
                .args(args)
                .current_dir(directory.path())
                .output()
                .expect("run git setup");
            assert!(output.status.success());
        }
        std::fs::write(
            directory.path().join("secret.env"),
            "access_key=AKIAEXAMPLE\nsecret_key=super-secret\n",
        )
        .expect("write secret fixture");
        for args in [vec!["add", "secret.env"], vec!["commit", "-m", "Secret"]] {
            let output = Command::new("git")
                .args(args)
                .current_dir(directory.path())
                .output()
                .expect("commit fixture");
            assert!(output.status.success());
        }

        let mut plan = HistoryRewritePlan::default();
        add_secret_to_plan(
            &mut plan,
            ScannedSecret {
                finding: SecretFinding {
                    detector_name: "AWS".to_string(),
                    verified: false,
                    file_path: "secret.env".to_string(),
                },
                verification: SecretVerification::Unverified,
                raw: Some("AKIAEXAMPLE".to_string()),
                raw_v2: Some("AKIAEXAMPLE:super-secret".to_string()),
                secret_parts: Vec::new(),
            },
        )
        .expect("ambiguous replacement plan");

        let replacements = collect_replacement_blobs(directory.path(), &mut plan)
            .await
            .expect("resolve historical blobs");
        assert!(replacements.is_empty());
        assert_eq!(
            plan.remove_paths,
            BTreeSet::from(["secret.env".to_string()])
        );
        assert!(plan.replacement_literals().is_empty());
    }

    #[test]
    fn replacement_callback_reads_only_the_private_blob_plan() {
        let callback = blob_replacement_callback(Path::new("/tmp/private-plan.json"))
            .expect("callback construction");
        assert!(callback.contains("2f746d702f707269766174652d706c616e2e6a736f6e"));
        assert!(callback.contains("blob.original_id"));
        assert!(!callback.contains("shared-value"));
    }

    #[test]
    fn missing_complete_raw_removes_only_the_affected_path() {
        let mut plan = HistoryRewritePlan::default();
        add_secret_to_plan(&mut plan, scanned_secret("secret.env", None))
            .expect("safe file-removal fallback");

        assert_eq!(
            plan.remove_paths,
            BTreeSet::from(["secret.env".to_string()])
        );
        assert!(plan.replacement_literals().is_empty());
    }

    #[tokio::test]
    async fn backup_bundle_captures_and_verifies_repository_history() {
        let directory = tempfile::tempdir().expect("temporary repository");
        for args in [
            vec!["init"],
            vec!["config", "user.name", "Test User"],
            vec!["config", "user.email", "test@example.com"],
        ] {
            let output = Command::new("git")
                .args(args)
                .current_dir(directory.path())
                .output()
                .expect("run git setup");
            assert!(output.status.success());
        }
        std::fs::write(directory.path().join("README.md"), "backup me").expect("write fixture");
        for args in [vec!["add", "README.md"], vec!["commit", "-m", "Initial"]] {
            let output = Command::new("git")
                .args(args)
                .current_dir(directory.path())
                .output()
                .expect("commit fixture");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        for args in [vec!["branch", "backup-branch"], vec!["tag", "backup-tag"]] {
            let output = Command::new("git")
                .args(args)
                .current_dir(directory.path())
                .output()
                .expect("create backup ref");
            assert!(output.status.success());
        }

        let bundle = create_backup_bundle(directory.path())
            .await
            .expect("create backup bundle");
        assert!(bundle.exists());
        let verify = Command::new("git")
            .arg("bundle")
            .arg("verify")
            .arg(&bundle)
            .current_dir(directory.path())
            .output()
            .expect("verify backup bundle");
        assert!(verify.status.success());
        let heads = Command::new("git")
            .arg("bundle")
            .arg("list-heads")
            .arg(&bundle)
            .current_dir(directory.path())
            .output()
            .expect("list bundle refs");
        assert!(heads.status.success());
        let heads = String::from_utf8(heads.stdout).expect("UTF-8 bundle refs");
        assert!(heads.contains("refs/heads/backup-branch"), "{heads}");
        assert!(heads.contains("refs/tags/backup-tag"), "{heads}");
        assert!(heads.lines().any(|line| line.ends_with(" HEAD")), "{heads}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&bundle)
                    .expect("bundle metadata")
                    .permissions()
                    .mode()
                    & 0o077,
                0
            );
        }
    }
}
