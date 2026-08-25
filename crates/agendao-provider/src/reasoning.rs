use crate::ReasoningEffort;

/// Clamp an explicit request to a provider-declared wire vocabulary.
///
/// The policy is deliberately monotonic: an unsupported enabled request is
/// mapped to the nearest weaker supported enabled level, never silently to a
/// more expensive level and never to `none`. `None` is preserved only when
/// the provider explicitly advertises it.
pub fn clamp_effort(
    requested: ReasoningEffort,
    supported: &[ReasoningEffort],
) -> Option<ReasoningEffort> {
    if supported.is_empty() {
        return Some(requested);
    }
    if supported.contains(&requested) {
        return Some(requested);
    }
    if !requested.enabled() {
        return supported
            .contains(&ReasoningEffort::None)
            .then_some(ReasoningEffort::None);
    }

    supported
        .iter()
        .copied()
        .filter(|effort| effort.enabled() && effort.rank() <= requested.rank())
        .max_by_key(|effort| effort.rank())
        .or_else(|| {
            supported
                .iter()
                .copied()
                .filter(|effort| effort.enabled())
                .min_by_key(|effort| effort.rank())
        })
}

/// Declared OpenAI-compatible reasoning vocabularies used by the built-in
/// model families. Unknown models intentionally use the conservative common
/// ladder instead of guessing a vendor-specific extension.
pub fn openai_compatible_efforts(model: &str) -> &'static [ReasoningEffort] {
    let model = model.to_ascii_lowercase();
    if model.contains("gpt-5.6") || model.contains("gpt5.6") {
        return &[
            ReasoningEffort::None,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::XHigh,
            ReasoningEffort::Max,
        ];
    }
    if model.contains("deepseek-v4") || model.contains("deepseek_v4") {
        return &[
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Max,
        ];
    }
    if model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.contains("gpt-5")
        || model.contains("codex")
    {
        return &[
            ReasoningEffort::None,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::XHigh,
        ];
    }
    &[
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_down_without_silent_upgrade() {
        let supported = [
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Max,
        ];
        assert_eq!(
            clamp_effort(ReasoningEffort::XHigh, &supported),
            Some(ReasoningEffort::High)
        );
        assert_eq!(
            clamp_effort(ReasoningEffort::Ultra, &supported),
            Some(ReasoningEffort::Max)
        );
    }

    #[test]
    fn disabled_only_survives_when_wire_supports_none() {
        assert_eq!(
            clamp_effort(ReasoningEffort::None, &[ReasoningEffort::Low]),
            None
        );
        assert_eq!(
            clamp_effort(
                ReasoningEffort::None,
                &[ReasoningEffort::None, ReasoningEffort::Low]
            ),
            Some(ReasoningEffort::None)
        );
    }
}
