/// Shared wildcard matching for permission patterns.
/// Supports: `*` (match all), `*suffix`, `prefix*`, `*middle*`, exact match.
pub(crate) fn wildcard_match(text: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    if pattern.starts_with('*') && pattern.ends_with('*') {
        let middle = &pattern[1..pattern.len() - 1];
        return text.contains(middle);
    }

    if let Some(suffix) = pattern.strip_prefix('*') {
        return text.ends_with(suffix);
    }

    if let Some(prefix) = pattern.strip_suffix('*') {
        return text.starts_with(prefix);
    }

    text == pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_star_matches_anything() {
        assert!(wildcard_match("foo", "*"));
        assert!(wildcard_match("", "*"));
    }

    #[test]
    fn wildcard_prefix() {
        assert!(wildcard_match("foo/bar", "foo/*"));
        assert!(!wildcard_match("baz/bar", "foo/*"));
    }

    #[test]
    fn wildcard_suffix() {
        assert!(wildcard_match("foo/bar/baz", "*/baz"));
        assert!(!wildcard_match("foo/bar/qux", "*/baz"));
    }

    #[test]
    fn wildcard_contains() {
        assert!(wildcard_match("foo/bar/baz", "*bar*"));
        assert!(!wildcard_match("foo/qux/baz", "*bar*"));
    }

    #[test]
    fn exact_match() {
        assert!(wildcard_match("foo", "foo"));
        assert!(!wildcard_match("foo", "bar"));
    }
}
