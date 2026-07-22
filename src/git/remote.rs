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
pub(crate) struct RemoteContext {
    pub(crate) remote: String,
    pub(crate) direction: RemoteDirection,
    pub(crate) transport: RemoteTransport,
    pub(crate) identity: Option<String>,
    pub(crate) ssh_url: Option<String>,
}

impl RemoteContext {
    pub(crate) fn display(&self) -> String {
        self.identity.as_ref().map_or_else(
            || format!("{} ({})", self.remote, self.transport.label()),
            |identity| format!("{} ({}, {identity})", self.remote, self.transport.label()),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemotePolicyViolation {
    pub(crate) context: RemoteContext,
}

impl RemotePolicyViolation {
    pub(crate) fn message(&self) -> String {
        format!(
            "ssh-only policy blocked {}: remote {} uses {}",
            self.context.direction.label(),
            self.context.remote,
            self.context.transport.label()
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
    if remote == "." {
        return Ok(None);
    }

    let contexts = inspect_remote(path, remote, direction).await?;
    policy_violation(&contexts)
}

pub(crate) async fn inspect_remote(
    path: &Path,
    remote: &str,
    direction: RemoteDirection,
) -> Result<Vec<RemoteContext>> {
    if remote == "." {
        return Ok(vec![RemoteContext {
            remote: remote.to_string(),
            direction,
            transport: RemoteTransport::Local,
            identity: None,
            ssh_url: None,
        }]);
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

    Ok(urls
        .lines()
        .map(|url| {
            let transport = RemoteTransport::from_url(url);
            let (identity, ssh_url) = safe_remote_details(url, transport);
            RemoteContext {
                remote: remote.to_string(),
                direction,
                transport,
                identity,
                ssh_url,
            }
        })
        .collect())
}

pub(crate) fn policy_violation(
    contexts: &[RemoteContext],
) -> Result<Option<RemotePolicyViolation>> {
    Ok(policy_violation_for(transport_policy()?, contexts))
}

fn policy_violation_for(
    policy: TransportPolicy,
    contexts: &[RemoteContext],
) -> Option<RemotePolicyViolation> {
    if policy == TransportPolicy::Preserve {
        return None;
    }

    contexts.iter().find_map(|context| {
        context.transport.is_http().then(|| RemotePolicyViolation {
            context: context.clone(),
        })
    })
}

fn safe_remote_details(url: &str, transport: RemoteTransport) -> (Option<String>, Option<String>) {
    match transport {
        RemoteTransport::Http | RemoteTransport::Https => safe_http_details(url),
        RemoteTransport::Ssh => (safe_ssh_identity(url), None),
        RemoteTransport::Local | RemoteTransport::Other => (None, None),
    }
}

fn safe_http_details(url: &str) -> (Option<String>, Option<String>) {
    let Some((_, remainder)) = url.trim().split_once("://") else {
        return (None, None);
    };
    let (authority, path) = remainder.split_once('/').unwrap_or((remainder, ""));
    let safe_authority = authority.rsplit('@').next().unwrap_or(authority);
    let (host, has_explicit_port) = host_and_port(safe_authority);
    let path = path
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_matches('/');

    if host.is_empty() || path.is_empty() {
        return (None, None);
    }

    let identity = format!("{safe_authority}/{path}");
    let standard_ssh_host = !has_explicit_port
        && matches!(
            host.to_ascii_lowercase().as_str(),
            "github.com" | "gitlab.com" | "bitbucket.org"
        );
    let ssh_url = standard_ssh_host.then(|| format!("git@{host}:{path}"));

    (Some(identity), ssh_url)
}

fn host_and_port(authority: &str) -> (&str, bool) {
    if let Some(bracketed) = authority.strip_prefix('[') {
        if let Some(bracket_end) = bracketed.find(']') {
            let host = &bracketed[..bracket_end];
            let suffix = &bracketed[bracket_end + 1..];
            return (host, suffix.starts_with(':'));
        }
        return (authority, false);
    }

    authority
        .rsplit_once(':')
        .map_or((authority, false), |(host, _)| (host, true))
}

fn safe_ssh_identity(url: &str) -> Option<String> {
    let url = url.trim();
    if let Some((_, remainder)) = url.split_once("://") {
        let (authority, path) = remainder.split_once('/')?;
        let host = authority.rsplit('@').next()?.split(':').next()?;
        let path = path.trim_matches('/');
        return (!host.is_empty() && !path.is_empty()).then(|| format!("{host}/{path}"));
    }

    let (authority, path) = url.split_once(':')?;
    let host = authority.rsplit('@').next()?;
    (!host.is_empty() && !path.is_empty()).then(|| format!("{host}/{path}"))
}

#[cfg(test)]
mod tests {
    use super::{
        policy_violation_for, safe_http_details, safe_ssh_identity, RemoteContext, RemoteDirection,
        RemoteTransport, TransportPolicy,
    };

    fn context(transport: RemoteTransport) -> RemoteContext {
        RemoteContext {
            remote: "origin".to_string(),
            direction: RemoteDirection::Fetch,
            transport,
            identity: None,
            ssh_url: None,
        }
    }

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
        let cases = [
            (
                "https://token@example.com/team/repo.git",
                RemoteTransport::Https,
            ),
            ("http://example.com/team/repo.git", RemoteTransport::Http),
            ("git@example.com:team/repo.git", RemoteTransport::Ssh),
            ("ssh://git@example.com/team/repo.git", RemoteTransport::Ssh),
            ("../remote.git", RemoteTransport::Local),
            ("custom-transport", RemoteTransport::Other),
        ];

        for (url, expected) in cases {
            assert_eq!(RemoteTransport::from_url(url), expected, "{url}");
        }
    }

    #[test]
    fn applies_transport_policy_to_every_transport() {
        for transport in [
            RemoteTransport::Http,
            RemoteTransport::Https,
            RemoteTransport::Ssh,
            RemoteTransport::Local,
            RemoteTransport::Other,
        ] {
            let contexts = [context(transport)];
            assert!(
                policy_violation_for(TransportPolicy::Preserve, &contexts).is_none(),
                "preserve must allow {transport:?}"
            );
            assert_eq!(
                policy_violation_for(TransportPolicy::SshOnly, &contexts).is_some(),
                transport.is_http(),
                "unexpected SSH-only decision for {transport:?}"
            );
        }
    }

    #[test]
    fn finds_http_transport_after_an_allowed_url() {
        let contexts = [
            context(RemoteTransport::Ssh),
            context(RemoteTransport::Https),
        ];

        let violation = policy_violation_for(TransportPolicy::SshOnly, &contexts)
            .expect("later HTTPS URL must still be blocked");

        assert_eq!(violation.context.transport, RemoteTransport::Https);
    }

    #[test]
    fn sanitizes_remote_identity_and_builds_known_host_ssh_urls() {
        let cases = [
            (
                "https://secret@github.com/goobits/repos.git?token=hidden",
                "github.com/goobits/repos.git",
                Some("git@github.com:goobits/repos.git"),
            ),
            (
                "https://gitlab.com/goobits/repos.git#fragment",
                "gitlab.com/goobits/repos.git",
                Some("git@gitlab.com:goobits/repos.git"),
            ),
            (
                "http://bitbucket.org/goobits/repos.git",
                "bitbucket.org/goobits/repos.git",
                Some("git@bitbucket.org:goobits/repos.git"),
            ),
            (
                "https://code.example.com/goobits/repos.git",
                "code.example.com/goobits/repos.git",
                None,
            ),
            (
                "https://github.com:8443/goobits/repos.git",
                "github.com:8443/goobits/repos.git",
                None,
            ),
        ];

        for (url, expected_identity, expected_ssh) in cases {
            let (identity, ssh_url) = safe_http_details(url);
            assert_eq!(identity.as_deref(), Some(expected_identity), "{url}");
            assert_eq!(ssh_url.as_deref(), expected_ssh, "{url}");
            assert!(!identity.unwrap().contains("secret"), "{url}");
        }

        assert_eq!(
            safe_ssh_identity("git@github.com:goobits/repos.git").as_deref(),
            Some("github.com/goobits/repos.git")
        );
    }
}
