use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

pub fn fuzzy_match(query: &str, target: &str) -> Option<i32> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Some(0);
    }

    let pattern = Pattern::parse(trimmed, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut utf32_buf = Vec::new();
    pattern
        .score(Utf32Str::new(target, &mut utf32_buf), &mut matcher)
        .map(|score| score.min(i32::MAX as u32) as i32)
}
