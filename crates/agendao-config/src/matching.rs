//! Shared matching rules for `disabled_*` configuration lists (skills, tools,
//! plugins). A disable entry is either an exact name or a `prefix/*` category
//! wildcard that matches `prefix` itself and every `prefix/...` descendant.

/// Returns the first pattern in `patterns` that matches `name`, if any.
///
/// Matching rules:
/// - `name` matches a pattern verbatim (exact entry, e.g. `"bash"`).
/// - A pattern ending in `/*` is a category wildcard: `"web/*"` matches
///   `"web"` itself and any `"web/..."` descendant (e.g. `"web/search"`),
///   but not `"webx"`.
///
/// Empty/whitespace-only patterns never match.
pub fn matching_disabled_pattern<'a>(patterns: &'a [String], name: &str) -> Option<&'a str> {
    patterns.iter().find_map(|pattern| {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return None;
        }
        if let Some(prefix) = pattern.strip_suffix("/*") {
            let prefix = prefix.trim_end_matches('/');
            if !prefix.is_empty() && (name == prefix || name.starts_with(prefix)) && {
                name.len() == prefix.len() || name.as_bytes().get(prefix.len()) == Some(&b'/')
            } {
                return Some(pattern);
            }
            return None;
        }
        (pattern == name).then_some(pattern)
    })
}

/// Convenience wrapper: true when any pattern in `patterns` matches `name`.
pub fn is_disabled(patterns: &[String], name: &str) -> bool {
    matching_disabled_pattern(patterns, name).is_some()
}

/// True when the `plugin` map entry named `name` may be loaded, i.e. it is not
/// listed in `Config::disabled_plugins` (exact name or `prefix/*` wildcard).
pub fn plugin_load_allowed(config: &crate::schema::Config, name: &str) -> bool {
    !is_disabled(&config.disabled_plugins, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterns(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn exact_name_matches() {
        let list = patterns(&["bash", "read"]);
        assert_eq!(matching_disabled_pattern(&list, "bash"), Some("bash"));
        assert!(is_disabled(&list, "read"));
        assert!(!is_disabled(&list, "write"));
    }

    #[test]
    fn category_wildcard_matches_prefix_and_descendants() {
        let list = patterns(&["literature-research/*"]);
        assert!(is_disabled(&list, "literature-research"));
        assert!(is_disabled(&list, "literature-research/skills"));
        assert!(is_disabled(&list, "literature-research/skills/semantic-scholar"));
        assert!(!is_disabled(&list, "literature-researchx"));
        assert!(!is_disabled(&list, "other/literature-research"));
    }

    #[test]
    fn wildcard_does_not_match_partial_prefix() {
        let list = patterns(&["web/*"]);
        assert!(!is_disabled(&list, "webx"));
        assert!(!is_disabled(&list, "websearch"));
        assert!(is_disabled(&list, "web"));
        assert!(is_disabled(&list, "web/search"));
    }

    #[test]
    fn empty_and_blank_patterns_never_match() {
        let list = patterns(&["", "   ", "/*"]);
        assert!(!is_disabled(&list, "anything"));
        assert!(!is_disabled(&list, ""));
    }

    #[test]
    fn mixed_exact_and_wildcard_entries() {
        let list = patterns(&["skill_hub", "filesystem_edit/*"]);
        assert!(is_disabled(&list, "skill_hub"));
        assert!(is_disabled(&list, "filesystem_edit/read"));
        assert!(!is_disabled(&list, "filesystem_discovery/glob"));
    }

    #[test]
    fn empty_pattern_list_disables_nothing() {
        assert!(!is_disabled(&[], "bash"));
        assert_eq!(matching_disabled_pattern(&[], "bash"), None);
    }

    #[test]
    fn plugin_load_allowed_honours_disabled_plugins() {
        let mut config = crate::schema::Config {
            disabled_plugins: patterns(&["metrics", "auth/*"]),
            ..Default::default()
        };
        assert!(!plugin_load_allowed(&config, "metrics"));
        assert!(!plugin_load_allowed(&config, "auth/github"));
        assert!(plugin_load_allowed(&config, "filesystem"));
        config.disabled_plugins.clear();
        assert!(plugin_load_allowed(&config, "metrics"));
    }
}
