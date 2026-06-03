use crate::cli_style::CliStyle;
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone)]
pub struct CliPanelFrame {
    header_line: String,
    footer_line: Option<String>,
    inner_width: usize,
    color: bool,
}

impl CliPanelFrame {
    pub fn boxed(title: &str, footer: Option<&str>, style: &CliStyle) -> Self {
        let style = style.with_live_width();
        let inner_width = usize::from(style.width.saturating_sub(5)).clamp(24, 160);
        let chrome_width = inner_width + 2;
        let header_content = pad_right_display(
            &truncate_display(&format!(" {} ", title.trim()), chrome_width),
            chrome_width,
            '─',
        );
        let header_line = if style.color {
            format!(
                "{}{}{}",
                style.cyan("╭"),
                style.bold_cyan(&header_content),
                style.cyan("╮")
            )
        } else {
            format!("╭{}╮", header_content)
        };

        let footer_line = footer.map(|text| {
            let content = pad_right_display(
                &truncate_display(&format!(" {} ", text.trim()), chrome_width),
                chrome_width,
                '─',
            );
            if style.color {
                format!(
                    "{}{}{}",
                    style.cyan("╰"),
                    style.dim(&content),
                    style.cyan("╯")
                )
            } else {
                format!("╰{}╯", content)
            }
        });

        Self {
            header_line,
            footer_line,
            inner_width,
            color: style.color,
        }
    }

    pub fn content_width(&self) -> usize {
        self.inner_width
    }

    pub fn render_lines(&self, lines: &[String]) -> String {
        let mut out = String::new();
        out.push_str(&self.header_line);
        out.push_str("\r\n");

        let rows = if lines.is_empty() {
            vec![String::new()]
        } else {
            wrap_lines(lines, self.inner_width)
        };

        for row in rows {
            out.push_str(&compose_row(&row, self.inner_width, self.color));
            out.push_str("\r\n");
        }

        if let Some(footer) = &self.footer_line {
            out.push_str(footer);
            out.push_str("\r\n");
        } else if self.color {
            out.push_str(&format!(
                "{}{}{}\r\n",
                "\x1b[36m╰\x1b[0m",
                "─".repeat(self.inner_width + 2),
                "\x1b[36m╯\x1b[0m"
            ));
        } else {
            out.push_str(&format!("╰{}╯\r\n", "─".repeat(self.inner_width + 2)));
        }

        out
    }

    pub fn rendered_line_count(&self, lines: &[String]) -> usize {
        let body_rows = if lines.is_empty() {
            1
        } else {
            wrap_lines(lines, self.inner_width).len()
        };
        1 + body_rows + 1
    }
}

fn compose_row(text: &str, width: usize, color: bool) -> String {
    let content = pad_right_display(text, width, ' ');
    if color {
        format!("{} {} {}", "\x1b[36m│\x1b[0m", content, "\x1b[36m│\x1b[0m")
    } else {
        format!("│ {} │", content)
    }
}

fn wrap_lines(lines: &[String], width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in lines {
        let wrapped = wrap_display_text(line, width);
        if wrapped.is_empty() {
            out.push(String::new());
        } else {
            out.extend(wrapped);
        }
    }
    out
}

pub fn wrap_display_text(text: &str, width: usize) -> Vec<String> {
    let row_width = width.max(1);
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }

    let mut rows = Vec::new();
    let mut row = String::new();
    let mut row_display_width = 0usize;

    for ch in chars {
        if ch == '\n' {
            rows.push(row);
            row = String::new();
            row_display_width = 0;
            continue;
        }

        let ch_width = char_display_width(ch);
        if row_display_width > 0 && row_display_width + ch_width > row_width {
            rows.push(row);
            row = String::new();
            row_display_width = 0;
        }
        row.push(ch);
        row_display_width += ch_width;
    }

    rows.push(row);
    rows
}

pub fn char_display_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0).max(1)
}

pub fn display_width(text: &str) -> usize {
    text.chars().map(char_display_width).sum()
}

pub fn display_width_between(text: &str, start: usize, end: usize) -> usize {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(char_display_width)
        .sum()
}

pub fn row_char_index_for_display_column(
    text: &str,
    row_start: usize,
    row_end: usize,
    target_col: usize,
) -> usize {
    let row_chars: Vec<char> = text
        .chars()
        .skip(row_start)
        .take(row_end.saturating_sub(row_start))
        .collect();
    let mut consumed_width = 0usize;
    let mut char_index = row_start;

    for ch in row_chars {
        let ch_width = char_display_width(ch);
        if consumed_width + ch_width > target_col {
            break;
        }
        consumed_width += ch_width;
        char_index += 1;
    }

    char_index
}

pub fn truncate_display(text: &str, max_width: usize) -> String {
    if display_width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 1 {
        return "…".to_string();
    }
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = char_display_width(ch);
        if width + ch_width + 1 > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out.push('…');
    out
}

pub fn pad_right_display(text: &str, width: usize, fill: char) -> String {
    let current = display_width(text);
    if current >= width {
        return text.to_string();
    }
    format!("{}{}", text, fill.to_string().repeat(width - current))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_display_respects_fullwidth() {
        assert_eq!(truncate_display("你好世界", 5), "你好…");
    }

    #[test]
    fn pad_right_display_respects_fullwidth() {
        assert_eq!(pad_right_display("你", 4, ' '), "你  ");
    }

    #[test]
    fn panel_wraps_fullwidth_rows() {
        let rows = wrap_display_text("你好 world", 6);
        assert_eq!(rows, vec!["你好 w", "orld"]);
    }

    #[test]
    fn rendered_panel_does_not_leak_untracked_leading_blank_line() {
        let style = CliStyle {
            color: false,
            width: 40,
        };
        let frame = CliPanelFrame::boxed("Question", Some("footer"), &style);
        let lines = vec!["line one".to_string(), "line two".to_string()];
        let rendered = frame.render_lines(&lines);

        assert!(!rendered.starts_with("\r\n"));
        assert_eq!(rendered.lines().count(), frame.rendered_line_count(&lines));
    }
}
