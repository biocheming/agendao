pub const CONTEXT_PRESSURE_WARNING_PERCENT: u64 = 80;
pub const CONTEXT_PRESSURE_AUTO_COMPACT_SOON_PERCENT: u64 = 90;
pub const CONTEXT_PRESSURE_CRITICAL_PERCENT: u64 = 95;

pub const CONTEXT_PRESSURE_WARNING_LABEL: &str = "warning";
pub const CONTEXT_PRESSURE_AUTO_COMPACT_SOON_LABEL: &str = "auto-compact soon";
pub const CONTEXT_PRESSURE_CRITICAL_LABEL: &str = "compact now";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextPressure {
    Normal,
    Warning,
    AutoCompactSoon,
    Critical,
}

impl ContextPressure {
    pub fn from_percent(percent: u64) -> Self {
        match percent {
            percent if percent >= CONTEXT_PRESSURE_CRITICAL_PERCENT => Self::Critical,
            percent if percent >= CONTEXT_PRESSURE_AUTO_COMPACT_SOON_PERCENT => {
                Self::AutoCompactSoon
            }
            percent if percent >= CONTEXT_PRESSURE_WARNING_PERCENT => Self::Warning,
            _ => Self::Normal,
        }
    }

    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Critical => Some(CONTEXT_PRESSURE_CRITICAL_LABEL),
            Self::AutoCompactSoon => Some(CONTEXT_PRESSURE_AUTO_COMPACT_SOON_LABEL),
            Self::Warning => Some(CONTEXT_PRESSURE_WARNING_LABEL),
            Self::Normal => None,
        }
    }

    pub fn is_warning_or_higher(self) -> bool {
        self >= Self::Warning
    }

    pub fn is_critical(self) -> bool {
        self == Self::Critical
    }
}

pub fn context_usage_percent(used: u64, limit: u64) -> Option<u64> {
    if limit == 0 {
        return None;
    }
    Some(((used as f64 / limit as f64) * 100.0).round() as u64)
}

pub fn context_usage_bar(percent: Option<u64>, width: usize) -> String {
    let safe_percent = percent.unwrap_or(0).min(100) as usize;
    let mut filled = ((safe_percent * width) + 50) / 100;
    if safe_percent > 0 && filled == 0 {
        filled = 1;
    }
    format!(
        "[{}{}]",
        "\u{2588}".repeat(filled),
        "\u{2591}".repeat(width.saturating_sub(filled))
    )
}

pub fn current_context_tokens_from_sources(
    usage_context_tokens: Option<u64>,
    active_stage_context_tokens: Option<u64>,
) -> Option<u64> {
    let usage_context_tokens = usage_context_tokens.filter(|tokens| *tokens > 0);
    let active_stage_context_tokens = active_stage_context_tokens.filter(|tokens| *tokens > 0);

    match (usage_context_tokens, active_stage_context_tokens) {
        (Some(usage), Some(active_stage)) => Some(usage.max(active_stage)),
        (Some(usage), None) => Some(usage),
        (None, Some(active_stage)) => Some(active_stage),
        (None, None) => None,
    }
}

pub fn context_pressure_for_percent(percent: Option<u64>) -> ContextPressure {
    percent
        .map(ContextPressure::from_percent)
        .unwrap_or(ContextPressure::Normal)
}

pub fn context_pressure_label(percent: Option<u64>) -> Option<&'static str> {
    context_pressure_for_percent(percent).label()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_pressure_thresholds_are_shared_contract() {
        assert_eq!(context_pressure_label(Some(79)), None);
        assert_eq!(context_pressure_label(Some(80)), Some("warning"));
        assert_eq!(context_pressure_label(Some(89)), Some("warning"));
        assert_eq!(context_pressure_label(Some(90)), Some("auto-compact soon"));
        assert_eq!(context_pressure_label(Some(94)), Some("auto-compact soon"));
        assert_eq!(context_pressure_label(Some(95)), Some("compact now"));
    }

    #[test]
    fn context_usage_percent_uses_rounded_percentage() {
        assert_eq!(context_usage_percent(12_450, 200_000), Some(6));
        assert_eq!(context_usage_percent(0, 0), None);
    }

    #[test]
    fn context_usage_bar_clamps_and_preserves_nonzero_visibility() {
        assert_eq!(context_usage_bar(Some(0), 5), "[░░░░░]");
        assert_eq!(context_usage_bar(Some(1), 5), "[█░░░░]");
        assert_eq!(context_usage_bar(Some(50), 5), "[███░░]");
        assert_eq!(context_usage_bar(Some(140), 5), "[█████]");
    }

    #[test]
    fn current_context_tokens_prefers_live_max_without_cumulative_fallback() {
        assert_eq!(current_context_tokens_from_sources(Some(0), Some(0)), None);
        assert_eq!(
            current_context_tokens_from_sources(Some(80), None),
            Some(80)
        );
        assert_eq!(
            current_context_tokens_from_sources(None, Some(90)),
            Some(90)
        );
        assert_eq!(
            current_context_tokens_from_sources(Some(80), Some(90)),
            Some(90)
        );
        assert_eq!(
            current_context_tokens_from_sources(Some(120), Some(90)),
            Some(120)
        );
    }
}
