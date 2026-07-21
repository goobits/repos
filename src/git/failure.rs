//! Structured Git operation failures and actionable remediation.

use super::remote::{RemoteContext, RemoteDirection, RemotePolicyViolation};
use super::status::Status;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GitFailureKind {
    Authentication,
    Diverged,
    Network,
    Timeout,
    TransportPolicy,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GitOperationPhase {
    Fetch,
    LfsPush,
    Push,
    RemoteInspection,
}

impl GitOperationPhase {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::LfsPush => "LFS push",
            Self::Push => "push",
            Self::RemoteInspection => "remote inspection",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GitFailure {
    pub(crate) kind: GitFailureKind,
    pub(crate) phase: GitOperationPhase,
    pub(crate) remote: Option<RemoteContext>,
    pub(crate) message: String,
}

impl GitFailure {
    pub(crate) fn from_policy(violation: RemotePolicyViolation) -> Self {
        let phase = match violation.context.direction {
            RemoteDirection::Fetch => GitOperationPhase::Fetch,
            RemoteDirection::Push => GitOperationPhase::Push,
        };
        let message = violation.message();

        Self {
            kind: GitFailureKind::TransportPolicy,
            phase,
            remote: Some(violation.context),
            message,
        }
    }

    pub(crate) fn from_message(
        phase: GitOperationPhase,
        message: String,
        remote: Option<RemoteContext>,
    ) -> Self {
        let lower = message.to_ascii_lowercase();
        let kind = if lower.contains("ssh-only policy") {
            GitFailureKind::TransportPolicy
        } else if contains_any(
            &lower,
            &[
                "authentication",
                "permission denied",
                "publickey",
                "could not read username",
                "terminal prompts disabled",
            ],
        ) {
            GitFailureKind::Authentication
        } else if lower.contains("diverged") {
            GitFailureKind::Diverged
        } else if lower.contains("timed out") || lower.contains("timeout") {
            GitFailureKind::Timeout
        } else if contains_any(
            &lower,
            &[
                "network",
                "connection reset",
                "could not resolve host",
                "unable to access",
            ],
        ) {
            GitFailureKind::Network
        } else {
            GitFailureKind::Other
        };

        Self {
            kind,
            phase,
            remote,
            message,
        }
    }

    pub(crate) fn reason(&self) -> String {
        match self.kind {
            GitFailureKind::Authentication => {
                format!("authentication failed during {}", self.phase.label())
            }
            GitFailureKind::TransportPolicy => self.remote.as_ref().map_or_else(
                || format!("SSH-only policy blocked {}", self.phase.label()),
                |remote| {
                    format!(
                        "SSH-only policy blocked {} ({})",
                        self.phase.label(),
                        remote.transport.label()
                    )
                },
            ),
            GitFailureKind::Diverged => self.message.clone(),
            GitFailureKind::Network => format!("network error during {}", self.phase.label()),
            GitFailureKind::Timeout => format!("timeout during {}", self.phase.label()),
            GitFailureKind::Other => self.message.clone(),
        }
    }

    pub(crate) fn next_action(&self, repo_path: &str) -> String {
        match self.kind {
            GitFailureKind::Authentication | GitFailureKind::TransportPolicy => {
                if let Some(remote) = &self.remote {
                    if let Some(ssh_url) = &remote.ssh_url {
                        let push_flag = if remote.direction == RemoteDirection::Push {
                            " --push"
                        } else {
                            ""
                        };
                        return format!(
                            "git -C {} remote set-url{push_flag} {} {}",
                            shell_quote(repo_path),
                            shell_quote(&remote.remote),
                            shell_quote(ssh_url)
                        );
                    }

                    return format!("change remote {} to an SSH clone URL", remote.remote);
                }

                "inspect authentication and remote transport".to_string()
            }
            GitFailureKind::Diverged => "repos sync or resolve manually".to_string(),
            GitFailureKind::Network => "retry or inspect remote connectivity".to_string(),
            GitFailureKind::Timeout => "retry with --sequential".to_string(),
            GitFailureKind::Other => "inspect failure".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct GitOperationResult {
    pub(crate) status: Status,
    pub(crate) message: String,
    pub(crate) has_uncommitted: bool,
    pub(crate) failure: Option<GitFailure>,
}

impl GitOperationResult {
    pub(crate) fn new(status: Status, message: String, has_uncommitted: bool) -> Self {
        Self {
            status,
            message,
            has_uncommitted,
            failure: None,
        }
    }

    pub(crate) fn failed(status: Status, failure: GitFailure, has_uncommitted: bool) -> Self {
        Self {
            status,
            message: failure.message.clone(),
            has_uncommitted,
            failure: Some(failure),
        }
    }

    pub(crate) fn into_tuple(self) -> (Status, String, bool) {
        (self.status, self.message, self.has_uncommitted)
    }
}

fn contains_any(value: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| value.contains(pattern))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::{GitFailure, GitFailureKind, GitOperationPhase};
    use crate::git::remote::{RemoteContext, RemoteDirection, RemoteTransport};

    #[test]
    fn classifies_authentication_without_string_parsing_in_reports() {
        let failure = GitFailure::from_message(
            GitOperationPhase::Fetch,
            "authentication failed".to_string(),
            None,
        );

        assert_eq!(failure.kind, GitFailureKind::Authentication);
        assert_eq!(failure.reason(), "authentication failed during fetch");
    }

    #[test]
    fn builds_a_shell_safe_known_host_transport_fix() {
        let failure = GitFailure {
            kind: GitFailureKind::TransportPolicy,
            phase: GitOperationPhase::Fetch,
            remote: Some(RemoteContext {
                remote: "origin".to_string(),
                direction: RemoteDirection::Fetch,
                transport: RemoteTransport::Https,
                identity: Some("github.com/goobits/aw.git".to_string()),
                ssh_url: Some("git@github.com:goobits/aw.git".to_string()),
            }),
            message: "blocked".to_string(),
        };

        assert_eq!(
            failure.next_action("./sketch-api/infra/aw"),
            "git -C './sketch-api/infra/aw' remote set-url 'origin' 'git@github.com:goobits/aw.git'"
        );
    }
}
