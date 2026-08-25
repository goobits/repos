//! Batched raw remote-URL configuration inspection.

use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use crate::git::remote::RemoteDirection;
use crate::git::runner::run_git_raw;

#[derive(Debug, Default)]
pub(super) struct ConfiguredRemoteUrls {
    fetch: HashMap<String, Vec<String>>,
    push: HashMap<String, Vec<String>>,
}

impl ConfiguredRemoteUrls {
    pub(super) fn urls(&self, remote: &str, direction: RemoteDirection) -> &[String] {
        let urls = if direction == RemoteDirection::Push {
            &self.push
        } else {
            &self.fetch
        };
        urls.get(remote).map_or(&[], Vec::as_slice)
    }
}

pub(super) async fn inspect_configured_remote_urls(path: &Path) -> Result<ConfiguredRemoteUrls> {
    let output = run_git_raw(
        path,
        &[
            "config",
            "--null",
            "--get-regexp",
            r"^remote\..*\.(url|pushurl)$",
        ],
    )
    .await?;
    if !output.success() {
        if output.exit_code == Some(1) && output.stderr.is_empty() {
            return Ok(ConfiguredRemoteUrls::default());
        }
        let stderr = output.stderr_text();
        anyhow::bail!(
            "{}",
            if stderr.is_empty() {
                format!("git config failed with exit code {:?}", output.exit_code)
            } else {
                stderr
            }
        );
    }

    parse_configured_remote_urls(&output.stdout)
}

pub(super) fn parse_configured_remote_urls(output: &[u8]) -> Result<ConfiguredRemoteUrls> {
    let mut urls = ConfiguredRemoteUrls::default();
    for record in output.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        let record = std::str::from_utf8(record)?;
        let (key, value) = record
            .split_once('\n')
            .ok_or_else(|| anyhow::anyhow!("git config returned a malformed remote URL record"))?;
        let Some(key) = key.strip_prefix("remote.") else {
            continue;
        };
        let Some((remote, suffix)) = key.rsplit_once('.') else {
            continue;
        };
        let target = match suffix {
            "url" => &mut urls.fetch,
            "pushurl" => &mut urls.push,
            _ => continue,
        };
        target
            .entry(remote.to_string())
            .or_default()
            .push(value.to_string());
    }
    Ok(urls)
}
