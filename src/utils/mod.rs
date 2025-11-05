pub(crate) mod fs;
pub(crate) mod terminal;

// Public API - utilities used by commands
pub use fs::shorten_path;
pub use terminal::{set_terminal_title, set_terminal_title_and_flush};
