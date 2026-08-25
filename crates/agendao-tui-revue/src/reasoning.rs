//! Shared TUI reasoning-effort control.
//!
//! The picker is deliberately a small pure model. The server remains the
//! authority for validation and request compilation; this module only owns
//! the visible cycle order used by the prompt bar.

pub const EFFORTS: &[&str] = &[
    "", "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

pub fn label(value: Option<&str>) -> &str {
    match value.unwrap_or("") {
        "" => "auto",
        "none" => "off",
        value => value,
    }
}

pub fn cycle(current: Option<&str>) -> Option<&'static str> {
    let index = EFFORTS
        .iter()
        .position(|value| Some(*value) == current)
        .unwrap_or(0);
    let next = EFFORTS[(index + 1) % EFFORTS.len()];
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_distinguishes_auto_from_disabled() {
        assert_eq!(label(None), "auto");
        assert_eq!(label(Some("")), "auto");
        assert_eq!(label(Some("none")), "off");
        assert_eq!(cycle(None), Some("none"));
        assert_eq!(cycle(Some("none")), Some("minimal"));
    }
}
