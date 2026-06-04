#![cfg(feature = "terminal-ui")]

//! CLI terminal style primitives — colors, icons, indentation.
//!
//! Provides ANSI escape-based styling that automatically degrades to plain
//! text when stdout is not a terminal (pipe / file redirect).

/// Terminal style context. Constructed once at CLI startup and threaded
/// through all rendering calls.
#[derive(Debug, Clone)]
pub struct CliStyle {
    /// Whether ANSI escape sequences are supported.
    pub color: bool,
    /// Terminal width in columns (0 = unknown).
    pub width: u16,
}

impl CliStyle {
    /// Auto-detect based on stdout.
    pub fn detect() -> Self {
        use std::io::IsTerminal;
        let is_tty = std::io::stdout().is_terminal();
        let width = if is_tty {
            terminal_width().unwrap_or(80)
        } else {
            80
        };
        Self {
            color: is_tty,
            width,
        }
    }

    /// Force plain text (for tests / `--format json`).
    pub fn plain() -> Self {
        Self {
            color: false,
            width: 80,
        }
    }

    /// Preserve semantic styling while refreshing the current terminal width.
    pub fn with_live_width(&self) -> Self {
        let width = if self.color {
            terminal_width().unwrap_or(self.width.max(1))
        } else {
            self.width.max(1)
        };
        Self {
            color: self.color,
            width,
        }
    }

    pub fn bold(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[1m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn dim(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[2m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn green(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[32m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn red(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[31m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn cyan(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[36m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn yellow(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[33m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn bold_green(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[1;32m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn bold_red(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[1;31m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn bold_cyan(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[1;36m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn bold_yellow(&self, text: &str) -> String {
        if self.color {
            format!("\x1b[1;33m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn rgb(&self, text: &str, red: u8, green: u8, blue: u8) -> String {
        if self.color {
            format!("\x1b[38;2;{red};{green};{blue}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    pub fn bold_rgb(&self, text: &str, red: u8, green: u8, blue: u8) -> String {
        if self.color {
            format!("\x1b[1;38;2;{red};{green};{blue}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    pub fn markdown_heading(&self, text: &str) -> String {
        self.bold_rgb(text, 100, 200, 255)
    }

    pub fn markdown_list_item(&self, text: &str) -> String {
        self.rgb(text, 100, 180, 255)
    }

    pub fn markdown_list_enumeration(&self, text: &str) -> String {
        self.rgb(text, 100, 255, 255)
    }

    pub fn markdown_code(&self, text: &str) -> String {
        self.rgb(text, 220, 220, 170)
    }

    pub fn markdown_emph(&self, text: &str) -> String {
        self.rgb(text, 255, 200, 80)
    }

    pub fn markdown_block_quote(&self, text: &str) -> String {
        self.rgb(text, 255, 200, 80)
    }

    pub fn markdown_link_text(&self, text: &str) -> String {
        self.rgb(text, 100, 255, 255)
    }

    pub fn markdown_link(&self, text: &str) -> String {
        self.rgb(text, 100, 180, 255)
    }

    pub fn markdown_hr(&self, text: &str) -> String {
        self.rgb(text, 80, 80, 80)
    }

    pub fn bullet(&self) -> &str {
        if self.color {
            "●"
        } else {
            "*"
        }
    }

    pub fn check(&self) -> &str {
        if self.color {
            "✔"
        } else {
            "[ok]"
        }
    }

    pub fn cross(&self) -> &str {
        if self.color {
            "✗"
        } else {
            "[err]"
        }
    }

    pub fn square(&self) -> &str {
        if self.color {
            "◼"
        } else {
            "[..]"
        }
    }

    pub fn warning_icon(&self) -> &str {
        if self.color {
            "⚠"
        } else {
            "[!]"
        }
    }

    pub fn tree_mid(&self) -> &str {
        if self.color {
            "│"
        } else {
            "|"
        }
    }

    pub fn tree_end(&self) -> &str {
        if self.color {
            "⎿"
        } else {
            "└"
        }
    }

    pub fn indent(&self, text: &str, spaces: usize) -> String {
        let prefix = " ".repeat(spaces);
        text.lines()
            .map(|line| format!("{}{}", prefix, line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn hr(&self) -> String {
        let w = (self.width as usize).min(72);
        let line = "─".repeat(w);
        self.markdown_hr(&line)
    }

    pub fn collapse(&self, text: &str, head: usize, tail: usize) -> String {
        let lines: Vec<&str> = text.lines().collect();
        let total = lines.len();
        let visible = head + tail;
        if total <= visible + 1 {
            return text.to_string();
        }
        let hidden = total - visible;
        let mut out = Vec::with_capacity(visible + 1);
        for line in &lines[..head] {
            out.push(line.to_string());
        }
        out.push(self.dim(&format!("… +{} lines", hidden)));
        for line in &lines[total - tail..] {
            out.push(line.to_string());
        }
        out.join("\n")
    }

    pub fn collapse_with_width(
        &self,
        text: &str,
        head: usize,
        tail: usize,
        max_width: Option<u16>,
    ) -> String {
        let max_w = max_width
            .or(Some(self.width))
            .map(|w| (w as usize).saturating_sub(6))
            .unwrap_or(200);

        let lines: Vec<&str> = text.lines().collect();
        let total = lines.len();
        let visible = head + tail;

        let truncate_line = |line: &str| -> String {
            if max_w > 0 && line.chars().count() > max_w {
                let truncated: String = line.chars().take(max_w).collect();
                format!("{}…", truncated)
            } else {
                line.to_string()
            }
        };

        if total <= visible + 1 {
            return lines
                .iter()
                .map(|line| truncate_line(line))
                .collect::<Vec<_>>()
                .join("\n");
        }

        let hidden = total - visible;
        let mut out = Vec::with_capacity(visible + 1);
        for line in &lines[..head] {
            out.push(truncate_line(line));
        }
        out.push(self.dim(&format!("… +{} lines", hidden)));
        for line in &lines[total - tail..] {
            out.push(truncate_line(line));
        }
        out.join("\n")
    }

    pub fn truncate_to_width(&self, text: &str) -> String {
        let max_w = (self.width as usize).saturating_sub(4);
        if max_w == 0 || text.chars().count() <= max_w {
            return text.to_string();
        }
        let truncated: String = text.chars().take(max_w).collect();
        format!("{}…", truncated)
    }
}

fn terminal_width() -> Option<u16> {
    crossterm::terminal::size()
        .ok()
        .map(|(width, _)| width.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_style_no_ansi() {
        let s = CliStyle::plain();
        assert_eq!(s.bold("hello"), "hello");
        assert_eq!(s.green("ok"), "ok");
        assert_eq!(s.bullet(), "*");
        assert_eq!(s.check(), "[ok]");
    }

    #[test]
    fn color_style_has_ansi() {
        let s = CliStyle {
            color: true,
            width: 80,
        };
        assert!(s.bold("hello").contains("\x1b[1m"));
        assert!(s.green("ok").contains("\x1b[32m"));
        assert_eq!(s.bullet(), "●");
        assert_eq!(s.check(), "✔");
    }

    #[test]
    fn collapse_short_text_unchanged() {
        let s = CliStyle::plain();
        let text = "line1\nline2\nline3";
        assert_eq!(s.collapse(text, 3, 2), text);
    }

    #[test]
    fn collapse_long_text() {
        let s = CliStyle::plain();
        let lines: Vec<String> = (1..=20).map(|i| format!("line{}", i)).collect();
        let text = lines.join("\n");
        let collapsed = s.collapse(&text, 3, 2);
        assert!(collapsed.contains("… +15 lines"));
        assert!(collapsed.starts_with("line1\n"));
        assert!(collapsed.ends_with("line20"));
    }

    #[test]
    fn indent_adds_prefix() {
        let s = CliStyle::plain();
        let result = s.indent("a\nb\nc", 4);
        assert_eq!(result, "    a\n    b\n    c");
    }

    #[test]
    fn truncate_to_width_short_unchanged() {
        let s = CliStyle {
            color: false,
            width: 80,
        };
        assert_eq!(s.truncate_to_width("hello"), "hello");
    }

    #[test]
    fn truncate_to_width_long_truncated() {
        let s = CliStyle {
            color: false,
            width: 20,
        };
        let long = "a".repeat(30);
        let result = s.truncate_to_width(&long);
        assert!(result.len() < 30);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn collapse_with_width_truncates_long_lines() {
        let s = CliStyle {
            color: false,
            width: 30,
        };
        let lines: Vec<String> = (1..=5)
            .map(|i| format!("line{} {}", i, "x".repeat(50)))
            .collect();
        let text = lines.join("\n");
        let collapsed = s.collapse_with_width(&text, 3, 2, None);
        for line in collapsed.lines() {
            assert!(line.chars().count() <= 30, "line too long: {}", line);
        }
    }
}
