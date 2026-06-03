//! Application branding — logo art, name, version, tagline.
//!
//! Shared between CLI and TUI so neither needs to depend on the other
//! for branding constants.

/// Terminal-friendly icon mark derived from the SVG brand symbol.
///
/// This keeps the user's denser block-art silhouette rather than a text logo,
/// so CLI/TUI open with a shape that reads closer to `icons/icon.svg`.
pub const LOGO: &[&str] = &[
    "      ███           ",
    "    ████████             ",
    "  █████        █       ",
    " ████             █ ",
    "█████    ▓▓▓         ",
    "█████    ▓▓▓▓     █     ",
    " █████          █           ",
    "     ████████        ",
    "        ████",
];

pub const fn logo_height() -> usize {
    LOGO.len()
}

/// Return logo lines, each prefixed by `pad`.
pub fn logo_lines(pad: &str) -> Vec<String> {
    LOGO.iter().map(|line| format!("{pad}{line}")).collect()
}
