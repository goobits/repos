//! Remote transport inspection and fleet-wide transport policy.

use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use super::operations::run_git;

const TRANSPORT_POLICY_ENV: &str = "REPOS_TRANSPORT_POLICY";
const TRANSPORT_POLICY_CONFIG: &str = "repos.transportPolicy";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransportPolicy {
    Preserve,
    SshOnly,
}

impl TransportPolicy {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "preserve" => Ok(Self::Preserve),
            "ssh-only" => Ok(Self::SshOnly),
            value => Err(anyhow!(
                "invalid transport policy '{value}'; expected 'preserve' or 'ssh-only'"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RemoteDirection {
    Fetch,
    Push,
}

impl RemoteDirection {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Push => "push",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RemoteTransport {
    Http,
    Https,
    Ssh,
    Local,
    Other,
}

impl RemoteTransport {
    pub(crate) fn from_url(url: &str) -> Self {
        let url = url.trim();
        let lower = url.to_ascii_lowercase();

        if lower.starts_with("https://") {
            Self::Https
        } else if lower.starts_with("http://") {
            Self::Http
        } else if lower.starts_with("ssh://")
            || lower.starts_with("git@")
            || lower
                .split_once(':')
                .is_some_and(|(authority, _)| authority.contains('@'))
        {
            Self::Ssh
        } else if lower.starts_with("file://")
            || url.starts_with('/')
            || url.starts_with("./")
            || url.starts_with("../")
        {
            Self::Local
        } else {
            Self::Other
        }
    }

    pub(crate) const fn is_http(self) -> bool {
        matches!(self, Self::Http | Self::Https)
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Http => "HTTP",
            Self::Https => "HTTPS",
            Self::Ssh => "SSH",
            Self::Local => "local",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemotePolicyViolation {
    pub(crate) remote: String,
    pub(crate) direction: RemoteDirection,
    pub(crate) transport: RemoteTransport,
}

impl RemotePolicyViolation {
    pub(crate) fn message(&self) -> String {
        format!(
            "ssh-only policy blocked {}: remote {} uses {}",
            self.direction.label(),
            self.remote,
            self.transport.label()
        )
    }
}

static TRANSPORT_POLICY: OnceLock<std::result::Result<TransportPolicy, String>> = OnceLock::new();

pub(crate) fn transport_policy() -> Result<TransportPolicy> {
    match TRANSPORT_POLICY.get_or_init(resolve_transport_policy) {
        Ok(policy) => Ok(*policy),
        Err(error) => Err(anyhow!(error.clone())),
    }
}

fn resolve_transport_policy() -> std::result::Result<TransportPolicy, String> {
    if let Some(value) = std::env::var_os(TRANSPORT_POLICY_ENV) {
        return TransportPolicy::parse(&value.to_string_lossy()).map_err(|error| error.to_string());
    }

    let output = match Command::new("git")
        .args(["config", "--global", "--get", TRANSPORT_POLICY_CONFIG])
        .output()
    {
        Ok(output) => output,
        Err(_) => return Ok(TransportPolicy::Preserve),
    };

    if !output.status.success() {
        return Ok(TransportPolicy::Preserve);
    }

    TransportPolicy::parse(&String::from_utf8_lossy(&output.stdout))
        .map_err(|error| error.to_string())
}

pub(crate) async fn remote_policy_violation(
    path: &Path,
    remote: &str,
    direction: RemoteDirection,
) -> Result<Option<RemotePolicyViolation>> {
    if transport_policy()? == TransportPolicy::Preserve || remote == "." {
        return Ok(None);
    }

    let mut args = vec!["remote", "get-url"];
    if direction == RemoteDirection::Push {
        args.push("--push");
    }
    args.extend(["--all", remote]);

    let (success, urls, stderr) = run_git(path, &args).await?;
    if !success {
        let detail = if stderr.trim().is_empty() {
            "unknown error"
        } else {
            stderr.trim()
        };
        return Err(anyhow!(
            "could not inspect {direction} URL for remote {remote}: {detail}",
            direction = direction.label()
        ));
    }

    Ok(urls.lines().find_map(|url| {
        let transport = RemoteTransport::from_url(url);
        transport.is_http().then(|| RemotePolicyViolation {
            remote: remote.to_string(),
            direction,
            transport,
        })
    }))
}

#[cfg(test)]
mod tests {
    use super::{RemoteTransport, TransportPolicy};

    #[test]
    fn parses_transport_policy_values() {
        assert_eq!(
            TransportPolicy::parse("preserve").unwrap(),
            TransportPolicy::Preserve
        );
        assert_eq!(
            TransportPolicy::parse("SSH-ONLY").unwrap(),
            TransportPolicy::SshOnly
        );
        assert!(TransportPolicy::parse("automatic").is_err());
    }

    #[test]
    fn classifies_remote_transports_without_exposing_urls() {
        assert_eq!(
            RemoteTransport::from_url("https://token@example.com/team/repo.git"),
            RemoteTransport::Https
        );
        assert_eq!(
            RemoteTransport::from_url("http://example.com/team/repo.git"),
            RemoteTransport::Http
        );
        assert_eq!(
            RemoteTransport::from_url("git@example.com:team/repo.git"),
            RemoteTransport::Ssh
        );
        assert_eq!(
            RemoteTransport::from_url("ssh://git@example.com/team/repo.git"),
            RemoteTransport::Ssh
        );
        assert_eq!(
            RemoteTransport::from_url("../remote.git"),
            RemoteTransport::Local
        );
    }
}
