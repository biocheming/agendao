//! Application branding — logo art, name, version, tagline.
//!
//! Shared between CLI and TUI so neither needs to depend on the other
//! for branding constants.

/// Terminal-friendly wordmark shared by CLI and TUI.
pub const LOGO: &[&str] = &[
    "█▀█ █▀▀ █▀▀ █▄░█ █▀▄ █▀█ █▀█",
    "█▀█░█▀█░█▀▀░█░▀█░█░█░█▀█░█░█",
    "▀ ▀ ▀▀▀ ▀▀▀ ▀  ▀ ▀▀  ▀ ▀ ▀▀▀",
];

pub const fn logo_height() -> usize {
    LOGO.len()
}

/// Return logo lines, each prefixed by `pad`.
pub fn logo_lines(pad: &str) -> Vec<String> {
    LOGO.iter().map(|line| format!("{pad}{line}")).collect()
}
