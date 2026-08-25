//! Text formatting and sanitization helpers for fleet statistics.

use std::path::Path;

use crate::core::config::{
    ERROR_MESSAGE_MAX_LENGTH, ERROR_MESSAGE_TRUNCATE_LENGTH, TIMEOUT_SECONDS_DISPLAY,
};

pub(crate) fn format_relative_repo_path(path: &str) -> String {
    let repo_path = Path::new(path);
    if repo_path.is_absolute() {
        let relative = std::env::current_dir()
            .ok()
            .and_then(|cwd| repo_path.strip_prefix(cwd).ok())
            .map(Path::to_path_buf);
        let Some(relative) = relative else {
            return repo_path.to_string_lossy().into_owned();
        };
        return prefix_relative_path(&relative);
    }

    prefix_relative_path(repo_path)
}

fn prefix_relative_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if value == "." || value.starts_with("./") {
        value.to_string()
    } else {
        format!("./{value}")
    }
}

pub(crate) fn truncate_text(value: &str, width: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= width {
        return value.to_string();
    }

    if width <= 1 {
        return "…".to_string();
    }

    let mut truncated = value.chars().take(width - 1).collect::<String>();
    truncated.push('…');
    truncated
}

pub(super) fn pluralize(count: u64, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 {
        singular
    } else {
        plural
    }
}

pub(super) fn parse_commit_count(message: &str) -> Option<u64> {
    message.split_whitespace().next()?.parse::<u64>().ok()
}

/// Cleans and formats error messages for display.
pub(crate) fn clean_error_message(error: &str) -> String {
    let cleaned = error
        .replace('\n', " ")
        .replace('\r', "")
        .replace('\t', " ");
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = redact_http_url_secrets(&cleaned);

    if cleaned.contains("repository moved") {
        if cleaned.contains("email privacy") {
            "repo moved + email privacy".to_string()
        } else {
            "repo moved".to_string()
        }
    } else if cleaned.contains("email privacy") {
        "email privacy restriction".to_string()
    } else if cleaned.contains("timed out") {
        if cleaned.contains(&TIMEOUT_SECONDS_DISPLAY.to_string()) {
            format!("timeout ({TIMEOUT_SECONDS_DISPLAY}s)")
        } else {
            "timeout".to_string()
        }
    } else if lower_contains_any(
        &cleaned,
        &[
            "authentication",
            "permission denied",
            "publickey",
            "could not read username",
            "terminal prompts disabled",
        ],
    ) {
        "authentication failed".to_string()
    } else if cleaned.contains("conflict") || cleaned.contains("diverged") {
        "merge conflict".to_string()
    } else if cleaned.contains("Connection") || cleaned.contains("network") {
        "network error".to_string()
    } else if cleaned.chars().count() > ERROR_MESSAGE_MAX_LENGTH {
        format!(
            "{}...",
            cleaned
                .chars()
                .take(ERROR_MESSAGE_TRUNCATE_LENGTH)
                .collect::<String>()
        )
    } else {
        cleaned
    }
}

fn redact_http_url_secrets(value: &str) -> String {
    let mut redacted = String::with_capacity(value.len());
    let mut remaining = value;

    while let Some(start) = [remaining.find("https://"), remaining.find("http://")]
        .into_iter()
        .flatten()
        .min()
    {
        redacted.push_str(&remaining[..start]);
        remaining = &remaining[start..];

        let end = remaining
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '\'' | '"' | ')' | ']' | '>' | ',')
            })
            .unwrap_or(remaining.len());
        let url = &remaining[..end];
        redacted.push_str(&redact_http_url(url));
        remaining = &remaining[end..];
    }

    redacted.push_str(remaining);
    redacted
}

fn redact_http_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://").map(|index| index + 3) else {
        return url.to_string();
    };
    let remainder = &url[scheme_end..];
    let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let path = &remainder[authority_end..];
    let safe_path_end = path.find(['?', '#']).unwrap_or(path.len());

    format!("{}{}{}", &url[..scheme_end], host, &path[..safe_path_end])
}

fn lower_contains_any(value: &str, patterns: &[&str]) -> bool {
    let lower = value.to_lowercase();
    patterns.iter().any(|pattern| lower.contains(pattern))
}

/// Gets the list of changed files in a repository using Git porcelain output.
pub(in crate::core) fn get_repo_changes(
    repo_path: &str,
) -> std::result::Result<Vec<String>, std::io::Error> {
    use std::process::Command;

    let path = Path::new(repo_path);
    let output = Command::new("git")
        .args([
            "status",
            "--porcelain=v1",
            "--untracked-files=normal",
            "--ignore-submodules=dirty",
        ])
        .current_dir(path)
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let status_output = String::from_utf8_lossy(&output.stdout);
    let mut changes = Vec::new();
    const MAX_FILES: usize = 10;

    for (index, line) in status_output.lines().enumerate() {
        if index >= MAX_FILES {
            let remaining = status_output.lines().count() - MAX_FILES;
            changes.push(format!("... and {remaining} more"));
            break;
        }
        if !line.is_empty() {
            changes.push(line.to_string());
        }
    }

    Ok(changes)
}
