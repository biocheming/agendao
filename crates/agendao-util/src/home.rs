//! Canonical user-level storage root.

use std::path::PathBuf;

/// Resolve the only user-level AgenDao directory.
pub fn agendao_home() -> PathBuf {
    if let Ok(value) = std::env::var("AGENDAO_HOME") {
        let value = value.trim();
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agendao")
}
