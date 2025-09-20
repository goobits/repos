//! File system utilities

/// Shortens long paths for display
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