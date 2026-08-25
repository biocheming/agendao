//! M7 transcript-detail visibility policy.
//!
//! The policy is deliberately independent of rendering and session I/O.  It
//! only resolves a section/instance pair; callers decide whether the fact is
//! present before exposing a control or summary.  Precedence is explicit:
//! user instance override > section override > global default.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DetailVisibility {
    Collapsed,
    Expanded,
    Hidden,
}

impl Default for DetailVisibility {
    fn default() -> Self {
        Self::Collapsed
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DetailsSection {
    Thinking,
    Tools,
    Todo,
    Subagents,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DetailsPolicy {
    #[serde(default)]
    pub default_mode: DetailVisibility,
    #[serde(default)]
    pub section_overrides: HashMap<DetailsSection, DetailVisibility>,
    /// Instance keys are scoped to the owning SessionStore. Callers use the
    /// transcript block id; switching sessions clears this map.
    #[serde(default)]
    pub instance_overrides: HashMap<String, DetailVisibility>,
}

impl DetailsPolicy {
    pub fn resolve(&self, section: DetailsSection, instance_id: Option<&str>) -> DetailVisibility {
        instance_id
            .filter(|id| !id.is_empty())
            .and_then(|id| self.instance_overrides.get(id).copied())
            .or_else(|| self.section_overrides.get(&section).copied())
            .unwrap_or(self.default_mode)
    }

    pub fn effective(
        &self,
        section: DetailsSection,
        instance_id: Option<&str>,
        user_override: Option<DetailVisibility>,
    ) -> DetailVisibility {
        user_override.unwrap_or_else(|| self.resolve(section, instance_id))
    }

    pub fn cycle_visibility(current: DetailVisibility) -> DetailVisibility {
        match current {
            DetailVisibility::Collapsed => DetailVisibility::Expanded,
            DetailVisibility::Expanded => DetailVisibility::Hidden,
            DetailVisibility::Hidden => DetailVisibility::Collapsed,
        }
    }

    pub fn toggle_section(&mut self, section: DetailsSection) -> DetailVisibility {
        let next = Self::cycle_visibility(self.resolve(section, None));
        self.section_overrides.insert(section, next);
        next
    }

    pub fn cycle_instance(
        &mut self,
        section: DetailsSection,
        instance_id: &str,
    ) -> DetailVisibility {
        if instance_id.is_empty() {
            return self.toggle_section(section);
        }
        let next = Self::cycle_visibility(self.resolve(section, Some(instance_id)));
        self.instance_overrides.insert(instance_id.to_owned(), next);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_is_instance_then_section_then_default() {
        let mut p = DetailsPolicy {
            default_mode: DetailVisibility::Hidden,
            ..Default::default()
        };
        assert_eq!(
            p.resolve(DetailsSection::Thinking, Some("s:t")),
            DetailVisibility::Hidden
        );
        p.section_overrides
            .insert(DetailsSection::Thinking, DetailVisibility::Expanded);
        assert_eq!(
            p.resolve(DetailsSection::Thinking, Some("s:t")),
            DetailVisibility::Expanded
        );
        p.instance_overrides
            .insert("s:t".into(), DetailVisibility::Collapsed);
        assert_eq!(
            p.resolve(DetailsSection::Thinking, Some("s:t")),
            DetailVisibility::Collapsed
        );
    }

    #[test]
    fn empty_or_unknown_instance_falls_back_without_creating_state() {
        let p = DetailsPolicy {
            default_mode: DetailVisibility::Expanded,
            ..Default::default()
        };
        assert_eq!(
            p.resolve(DetailsSection::Subagents, None),
            DetailVisibility::Expanded
        );
        assert_eq!(
            p.resolve(DetailsSection::Subagents, Some("")),
            DetailVisibility::Expanded
        );
        assert_eq!(p.instance_overrides.len(), 0);
    }

    #[test]
    fn all_sections_cycle_deterministically() {
        let mut p = DetailsPolicy::default();
        for section in [
            DetailsSection::Thinking,
            DetailsSection::Tools,
            DetailsSection::Todo,
            DetailsSection::Subagents,
        ] {
            assert_eq!(p.toggle_section(section), DetailVisibility::Expanded);
            assert_eq!(p.toggle_section(section), DetailVisibility::Hidden);
            assert_eq!(p.toggle_section(section), DetailVisibility::Collapsed);
        }
    }

    #[test]
    fn instance_override_is_isolated_by_stable_key() {
        let mut p = DetailsPolicy::default();
        p.cycle_instance(DetailsSection::Tools, "session-a:block-1");
        assert_eq!(
            p.resolve(DetailsSection::Tools, Some("session-a:block-1")),
            DetailVisibility::Expanded
        );
        assert_eq!(
            p.resolve(DetailsSection::Tools, Some("session-b:block-1")),
            DetailVisibility::Collapsed
        );
    }

    #[test]
    fn user_override_wins_over_policy() {
        let mut p = DetailsPolicy::default();
        p.section_overrides
            .insert(DetailsSection::Todo, DetailVisibility::Hidden);
        p.instance_overrides
            .insert("s:todo".into(), DetailVisibility::Expanded);
        assert_eq!(
            p.effective(
                DetailsSection::Todo,
                Some("s:todo"),
                Some(DetailVisibility::Collapsed)
            ),
            DetailVisibility::Collapsed
        );
    }
}
