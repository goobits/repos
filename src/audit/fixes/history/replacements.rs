//! Safe secret-replacement planning for history rewrites.

use super::HistoryRewritePlan;
use crate::audit::scanner::ScannedSecret;
use anyhow::{anyhow, bail, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

const MAX_REPLACEMENT_LITERAL_BYTES: usize = 16 * 1024;
const BLOB_READ_CHUNK_BYTES: usize = 8 * 1024;

pub(super) fn add_secret_to_plan(
    plan: &mut HistoryRewritePlan,
    secret: ScannedSecret,
) -> Result<()> {
    let file_path = secret.finding.file_path;
    validate_repository_path(&file_path)?;

    let mut values = Vec::new();
    values.extend(secret.raw);
    if secret.secret_parts.is_empty() {
        // RawV2 can be synthetic. Historical blob inspection below verifies
        // that every planned value exists or falls back to path removal.
        values.extend(secret.raw_v2);
    } else {
        // Multipart detectors can join RawV2 synthetically. Prefer the source
        // components and redact them only in blobs for the reported path.
        values.extend(secret.secret_parts);
    }
    if values.is_empty()
        || values
            .iter()
            .any(|value| validate_secret_value(value).is_err())
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

pub(super) async fn collect_replacement_blobs(
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
        let path_replacements = replacements_for_path(repo_path, &path, &values).await?;
        let observed_values = path_replacements
            .values()
            .flat_map(|values| values.iter().cloned())
            .collect::<BTreeSet<_>>();
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

async fn replacements_for_path(
    repo_path: &Path,
    path: &str,
    values: &BTreeSet<String>,
) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let pathspec = format!(":(literal){path}");
    let mut rev_list = Command::new("git")
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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = rev_list
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Git history listing stdout was unavailable"))?;
    let mut stderr = rev_list
        .stderr
        .take()
        .ok_or_else(|| anyhow!("Git history listing stderr was unavailable"))?;
    let stderr_task = tokio::spawn(async move {
        let mut output = Vec::new();
        stderr.read_to_end(&mut output).await.map(|_| output)
    });

    let mut replacements = BTreeMap::new();
    let mut matched_path = false;
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await? {
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
        matched_path = true;
        let matches = matching_blob_values(repo_path, object_id, values).await?;
        if !matches.is_empty() {
            replacements.insert(object_id.to_string(), matches);
        }
    }

    let status = rev_list.wait().await?;
    let stderr = stderr_task.await??;
    if !status.success() {
        bail!(
            "resolving secret-containing history blobs failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        );
    }
    if !matched_path {
        bail!("could not resolve historical blobs for secret path {path}");
    }
    Ok(replacements)
}

async fn matching_blob_values(
    repo_path: &Path,
    object_id: &str,
    values: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    let mut cat_file = Command::new("git")
        .args(["cat-file", "blob", object_id])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdout = cat_file
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Git blob stdout was unavailable"))?;
    let mut stderr = cat_file
        .stderr
        .take()
        .ok_or_else(|| anyhow!("Git blob stderr was unavailable"))?;
    let stderr_task = tokio::spawn(async move {
        let mut output = Vec::new();
        stderr.read_to_end(&mut output).await.map(|_| output)
    });

    let max_value_len = values.iter().map(String::len).max().unwrap_or(1);
    let mut matches = BTreeSet::new();
    let mut window = Vec::with_capacity(BLOB_READ_CHUNK_BYTES + max_value_len);
    let mut chunk = [0u8; BLOB_READ_CHUNK_BYTES];
    loop {
        let read = stdout.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        window.extend_from_slice(&chunk[..read]);
        for value in values {
            if !matches.contains(value) && contains_bytes(&window, value.as_bytes()) {
                matches.insert(value.clone());
            }
        }
        let overlap = max_value_len.saturating_sub(1);
        if window.len() > overlap {
            let keep_from = window.len() - overlap;
            window.drain(..keep_from);
        }
    }

    let status = cat_file.wait().await?;
    let stderr = stderr_task.await??;
    if !status.success() {
        bail!(
            "reading a secret-containing historical blob failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        );
    }
    Ok(matches)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

pub(super) fn encode_replacement_blobs(
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

pub(super) fn blob_replacement_callback(plan_path: &Path) -> Result<String> {
    let plan_path = plan_path
        .to_str()
        .ok_or_else(|| anyhow!("temporary rewrite plan path was not valid UTF-8"))?;
    let encoded_path = hex_encode(plan_path.as_bytes());
    Ok(format!(
        concat!(
            "global _repos_plan\n",
            "try:\n",
            "    patterns_by_id = _repos_plan\n",
            "except NameError:\n",
            "    import json\n",
            "    plan_path = bytes.fromhex('{encoded_path}').decode('utf-8')\n",
            "    with open(plan_path, encoding='utf-8') as stream:\n",
            "        encoded_plan = json.load(stream)\n",
            "    _repos_plan = {{\n",
            "        object_id.encode('ascii'): [bytes.fromhex(value) for value in values]\n",
            "        for object_id, values in encoded_plan.items()\n",
            "    }}\n",
            "    patterns_by_id = _repos_plan\n",
            "patterns = patterns_by_id.get(blob.original_id)\n",
            "if patterns:\n",
            "    for pattern in patterns:\n",
            "        blob.data = blob.data.replace(pattern, b'REDACTED')"
        ),
        encoded_path = encoded_path
    ))
}

pub(super) fn validate_repository_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.len() > MAX_REPLACEMENT_LITERAL_BYTES
        || path.contains(['\n', '\r', '\0'])
    {
        bail!("secret file path cannot be represented safely for git filter-repo");
    }
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

pub(super) fn validate_secret_value(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_REPLACEMENT_LITERAL_BYTES {
        bail!("secret value is empty or too large for a bounded history rewrite");
    }
    Ok(())
}
