//! Terminal utilities for title setting and output management

use std::io::Write;

/// Sets the terminal title to the specified text
pub fn set_terminal_title(title: &str) {
    // ANSI escape sequence to set terminal title
    print!("\x1b]0;{}\x07", title);
}

/// Sets the terminal title and ensures it's flushed to the terminal
pub fn set_terminal_title_and_flush(title: &str) {
    set_terminal_title(title);
    std::io::stdout().flush().unwrap();
}
