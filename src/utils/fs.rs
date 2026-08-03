//! File system utilities

use std::cmp::Ordering;
use std::path::Path;

/// Orders repository results by their location, using the display name only as
/// a deterministic tie-breaker.
pub(crate) fn compare_repository_locations(
    left_path: impl AsRef<Path>,
    left_repository: &str,
    right_path: impl AsRef<Path>,
    right_repository: &str,
) -> Ordering {
    left_path
        .as_ref()
        .cmp(right_path.as_ref())
        .then_with(|| left_repository.cmp(right_repository))
}

/// Shortens long paths for display
#[must_use]
pub fn shorten_path(path: &str, max_length: usize) -> String {
    if path.len() <= max_length {
        return path.to_string();
    }

    let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if components.len() <= 2 {
        // Too few components to shorten meaningfully
        return path.to_string();
    }

    // Keep last 2 components with ellipsis prefix
    let prefix = if path.starts_with("./") { "./" } else { "" };
    format!(
        "{}.../{}/{}",
        prefix,
        components[components.len() - 2],
        components[components.len() - 1]
    )
}
