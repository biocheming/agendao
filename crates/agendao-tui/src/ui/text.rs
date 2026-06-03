use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

/// Build a text line from terminal cells while removing placeholder cells
/// that follow multi-width graphemes (e.g. CJK full-width characters).
pub fn line_from_cells<'a, I>(symbols: I) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    let mut line = String::new();
    let mut skip_cells = 0usize;

    for symbol in symbols {
        if skip_cells > 0 {
            skip_cells -= 1;
            continue;
        }

        line.push_str(symbol);
        let width = UnicodeWidthStr::width(symbol).max(1);
        if width > 1 {
            skip_cells = width - 1;
        }
    }

    line
}

/// Strip session left gutter decorations (e.g. "│", "│┃") from a rendered line.
pub fn strip_session_gutter_line(line: &str) -> String {
    let mut chars = line.char_indices().peekable();

    while let Some((_i, ch)) = chars.peek().copied() {
        if ch == ' ' {
            chars.next();
        } else {
            break;
        }
    }

    let Some((i, first)) = chars.peek().copied() else {
        return line.to_string();
    };
    if !matches!(first, '│' | '┃') {
        return line.to_string();
    }
    chars.next();
    let mut idx = i + first.len_utf8();

    if let Some((i, second)) = chars.peek().copied() {
        if matches!(second, '│' | '┃') {
            chars.next();
            idx = i + second.len_utf8();
        }
    }

    if let Some((i, ch)) = chars.peek().copied() {
        if ch == ' ' {
            let mut lookahead = chars.clone();
            lookahead.next();
            let next_is_space = lookahead.next().map(|(_, c)| c == ' ').unwrap_or(false);
            if !next_is_space {
                idx = i + ch.len_utf8();
            }
        }
    }

    line[idx..].to_string()
}

pub fn strip_session_gutter(text: &str) -> String {
    text.lines()
        .map(strip_session_gutter_line)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn truncate(text: &str, max_width: usize) -> String {
    let width = UnicodeWidthStr::width(text);
    if width <= max_width {
        return text.to_string();
    }

    let mut result = String::new();
    let mut current_width = 0;

    for ch in text.chars() {
        let ch_str = ch.to_string();
        let ch_width = UnicodeWidthStr::width(ch_str.as_str());
        if current_width + ch_width + 3 > max_width {
            break;
        }
        result.push(ch);
        current_width += ch_width;
    }

    result.push_str("...");
    result
}

pub fn pad_left(text: &str, width: usize) -> String {
    let text_width = text.width();
    if text_width >= width {
        return text.to_string();
    }
    format!("{}{}", " ".repeat(width - text_width), text)
}

pub fn pad_right(text: &str, width: usize) -> String {
    let text_width = text.width();
    if text_width >= width {
        return text.to_string();
    }
    format!("{}{}", text, " ".repeat(width - text_width))
}

pub fn center_text(text: &str, width: usize) -> String {
    let text_width = text.width();
    if text_width >= width {
        return text.to_string();
    }
    let left_pad = (width - text_width) / 2;
    format!("{}{}", " ".repeat(left_pad), text)
}

pub fn highlight_text<'a>(text: &'a str, color: ratatui::style::Color) -> Span<'a> {
    Span::styled(text, ratatui::style::Style::default().fg(color))
}

#[cfg(test)]
mod tests {
    use super::{line_from_cells, strip_session_gutter_line};

    #[test]
    fn line_from_cells_collapses_wide_char_placeholders() {
        let cells = vec!["A", "你", " ", "B"];
        assert_eq!(line_from_cells(cells), "A你B");
    }

    #[test]
    fn line_from_cells_preserves_real_spaces() {
        // ["你", " "] includes the full-width placeholder.
        // The next " " is a real content space that must remain.
        let cells = vec!["你", " ", " ", "好", " "];
        assert_eq!(line_from_cells(cells), "你 好");
    }

    #[test]
    fn strip_session_gutter_removes_visual_bars() {
        assert_eq!(strip_session_gutter_line("  │┃ 你看一下"), "你看一下");
        assert_eq!(strip_session_gutter_line("│◯ → ls"), "◯ → ls");
        assert_eq!(
            strip_session_gutter_line("│    {\"path\":\".\"}"),
            "    {\"path\":\".\"}"
        );
        assert_eq!(strip_session_gutter_line("no gutter"), "no gutter");
    }
}
