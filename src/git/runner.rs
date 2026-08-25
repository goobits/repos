//! Typed execution for Git subprocesses.

use anyhow::Result;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

use super::remote::{transport_policy, TransportPolicy};

const GIT_OPERATION_TIMEOUT_SECS: u64 = 180;

/// Complete Git subprocess output, including the exit code and unmodified bytes.
#[derive(Debug)]
pub(crate) struct GitCommandOutput {
    pub(crate) exit_code: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

impl GitCommandOutput {
    pub(crate) fn success(&self) -> bool {
        self.exit_code == Some(0)
    }

    pub(crate) fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout)
            .trim_end_matches(['\r', '\n'])
            .to_string()
    }

    pub(crate) fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).trim().to_string()
    }
}

/// Runs Git while retaining its exit code and byte-exact output.
pub(crate) async fn run_git_raw(path: &Path, args: &[&str]) -> Result<GitCommandOutput> {
    let mut last_error = None;
    let is_network = is_network_operation(args);
    let max_retries = if is_network { 3 } else { 1 };

    for attempt in 1..=max_retries {
        let mut command = Command::new("git");
        command.kill_on_drop(true);
        if is_network && transport_policy()? == TransportPolicy::SshOnly {
            command.args([
                "-c",
                "credential.helper=",
                "-c",
                "credential.interactive=false",
            ]);
        }
        if std::env::var_os("GIT_SSH_COMMAND").is_none() {
            command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
        }

        let result = tokio::time::timeout(
            Duration::from_secs(GIT_OPERATION_TIMEOUT_SECS),
            command
                .args(args)
                .current_dir(path)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GCM_INTERACTIVE", "never")
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                let command_output = GitCommandOutput {
                    exit_code: output.status.code(),
                    stdout: output.stdout,
                    stderr: output.stderr,
                };
                if command_output.success()
                    || !is_transient_network_error(&command_output.stderr_text())
                {
                    return Ok(command_output);
                }
                last_error = Some(anyhow::anyhow!(
                    "Git command failed: {}",
                    command_output.stderr_text()
                ));
            }
            Ok(Err(error)) => {
                if attempt == max_retries {
                    return Err(error.into());
                }
                last_error = Some(error.into());
            }
            Err(_) => {
                if attempt == max_retries {
                    anyhow::bail!(
                        "Git operation timed out after {GIT_OPERATION_TIMEOUT_SECS} seconds"
                    );
                }
                last_error = Some(anyhow::anyhow!("Git operation timed out"));
            }
        }

        if attempt < max_retries {
            tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Unknown error during Git operation")))
}

/// Compatibility adapter for callers that only need text and success/failure.
pub(crate) async fn run_git(path: &Path, args: &[&str]) -> Result<(bool, String, String)> {
    let output = run_git_raw(path, args).await?;
    Ok((output.success(), output.stdout_text(), output.stderr_text()))
}

fn is_network_operation(args: &[&str]) -> bool {
    args.iter()
        .any(|arg| matches!(*arg, "push" | "pull" | "fetch" | "clone" | "ls-remote"))
}

fn is_transient_network_error(stderr: &str) -> bool {
    let error = stderr.to_lowercase();
    error.contains("could not resolve host")
        || error.contains("connection reset")
        || error.contains("network is unreachable")
        || error.contains("operation timed out")
        || error.contains("temporary failure")
}
