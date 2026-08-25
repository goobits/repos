//! Terminal styles shared by nested-status renderers.

pub(super) const RESET: &str = "\x1b[0m";
pub(super) const BOLD_BLUE: &str = "\x1b[1;38;5;75m";
pub(super) const BOLD_PURPLE: &str = "\x1b[1;38;5;141m";
pub(super) const GREEN: &str = "\x1b[1;38;5;114m";
pub(super) const YELLOW: &str = "\x1b[1;38;5;221m";
pub(super) const RED: &str = "\x1b[1;38;5;203m";
pub(super) const DIM: &str = "\x1b[2m";

pub(super) fn paint(color: &str, value: &str) -> String {
    format!("{color}{value}{RESET}")
}
