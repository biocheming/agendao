//! Application branding — logo art, name, version, tagline.
//!
//! Shared between CLI and TUI so neither needs to depend on the other
//! for branding constants.

/// Terminal-friendly two-panel wordmark shared by CLI and TUI.
/// Left panel: pixel-art logo (18 chars wide). Right panel: text.
pub const LOGO: &[&str] = &[
    "╭────────────────┬────────────────────────╮",
    "│░░░░▒▒▒▒████░░░░│                        │",
    "│░▒▒▒▒▒▒▒███████░│        AGENDAO         │",
    "│░░░░▒●▒▒██○█░░░░│    The Dao of Agent    │",
    "│░░░░░▒▒▒███░░░░░│                        │",
    "╰────────────────┴────────────────────────╯",
];

pub const fn logo_height() -> usize {
    LOGO.len()
}

/// Return logo lines, each prefixed by `pad`.
pub fn logo_lines(pad: &str) -> Vec<String> {
    LOGO.iter().map(|line| format!("{pad}{line}")).collect()
}
