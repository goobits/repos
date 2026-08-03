pub(crate) mod fs;
pub(crate) mod terminal;

// Public API - utilities used by commands
pub(crate) use fs::compare_repository_locations;
pub use fs::shorten_path;
pub use terminal::{set_terminal_title, set_terminal_title_and_flush};
