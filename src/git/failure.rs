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
    use crate::git::remote::{
        RemoteContext, RemoteDirection, RemotePolicyViolation, RemoteTransport,
    };

    #[test]
    fn classifies_failure_messages_and_formats_reasons() {
        let cases = [
            (
                "authentication failed",
                GitFailureKind::Authentication,
                "authentication failed during fetch",
            ),
            (
                "Permission denied (publickey)",
                GitFailureKind::Authentication,
                "authentication failed during fetch",
            ),
            (
                "diverged: 2 ahead / 3 behind",
                GitFailureKind::Diverged,
                "diverged: 2 ahead / 3 behind",
            ),
            (
                "Git operation timed out",
                GitFailureKind::Timeout,
                "timeout during fetch",
            ),
            (
                "could not resolve host",
                GitFailureKind::Network,
                "network error during fetch",
            ),
            (
                "unexpected failure",
                GitFailureKind::Other,
                "unexpected failure",
            ),
        ];

        for (message, expected_kind, expected_reason) in cases {
            let failure =
                GitFailure::from_message(GitOperationPhase::Fetch, message.to_string(), None);

            assert_eq!(failure.kind, expected_kind, "{message}");
            assert_eq!(failure.reason(), expected_reason, "{message}");
        }
    }

    #[test]
    fn maps_policy_violation_to_push_failure() {
        let failure = GitFailure::from_policy(RemotePolicyViolation {
            context: RemoteContext {
                remote: "origin".to_string(),
                direction: RemoteDirection::Push,
                transport: RemoteTransport::Https,
                identity: Some("github.com/goobits/aw.git".to_string()),
                ssh_url: Some("git@github.com:goobits/aw.git".to_string()),
            },
        });

        assert_eq!(failure.kind, GitFailureKind::TransportPolicy);
        assert_eq!(failure.phase, GitOperationPhase::Push);
        assert_eq!(failure.reason(), "SSH-only policy blocked push (HTTPS)");
    }

    #[test]
    fn builds_shell_safe_push_fix_for_known_host() {
        let failure = GitFailure {
            kind: GitFailureKind::TransportPolicy,
            phase: GitOperationPhase::Push,
            remote: Some(RemoteContext {
                remote: "origin".to_string(),
                direction: RemoteDirection::Push,
                transport: RemoteTransport::Https,
                identity: Some("github.com/goobits/aw.git".to_string()),
                ssh_url: Some("git@github.com:goobits/aw.git".to_string()),
            }),
            message: "blocked".to_string(),
        };

        assert_eq!(
            failure.next_action("./sketch-api/O'Brien/aw"),
            "git -C './sketch-api/O'\\''Brien/aw' remote set-url --push 'origin' 'git@github.com:goobits/aw.git'"
        );
    }

    #[test]
    fn avoids_guessing_ssh_url_for_unknown_host() {
        let failure = GitFailure {
            kind: GitFailureKind::Authentication,
            phase: GitOperationPhase::Fetch,
            remote: Some(RemoteContext {
                remote: "upstream".to_string(),
                direction: RemoteDirection::Fetch,
                transport: RemoteTransport::Https,
                identity: Some("code.example.com/team/repo.git".to_string()),
                ssh_url: None,
            }),
            message: "authentication failed".to_string(),
        };

        assert_eq!(
            failure.next_action("./repo"),
            "change remote upstream to an SSH clone URL"
        );
    }
}
