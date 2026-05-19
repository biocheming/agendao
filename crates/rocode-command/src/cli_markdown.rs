//! Terminal markdown renderer for CLI output.
//!
//! Converts markdown text to ANSI-styled terminal output.
//! Designed for both full-text rendering and streaming (delta) mode.

use crate::cli_style::CliStyle;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Render a complete markdown string to ANSI-styled terminal text.
pub fn render_markdown(text: &str, style: &CliStyle) -> String {
    if !style.color {
        return text.to_string();
    }
    let mut renderer = MarkdownRenderer::new(style);
    renderer.push(text);
    renderer.finish()
}

/// Streaming markdown renderer that accumulates delta text and renders
/// complete markdown blocks. Incomplete trailing content is buffered
/// until more text arrives or `finish()` is called.
pub struct MarkdownStreamer<'a> {
    buffer: String,
    rendered_up_to: usize,
    style: &'a CliStyle,
    continuation_prefix: String,
    at_line_start: bool,
}

impl<'a> MarkdownStreamer<'a> {
    pub fn new(style: &'a CliStyle) -> Self {
        Self {
            buffer: String::new(),
            rendered_up_to: 0,
            style,
            continuation_prefix: String::new(),
            at_line_start: false,
        }
    }

    pub fn with_continuation_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.continuation_prefix = prefix.into();
        self
    }

    /// Push a new delta chunk. Returns any newly renderable output.
    pub fn push(&mut self, delta: &str) -> String {
        self.buffer.push_str(delta);
        let rendered = if !self.style.color {
            self.try_take_complete_lines()
        } else {
            self.try_render_complete()
        };
        self.apply_continuation_prefix(&rendered)
    }

    /// Flush remaining buffer and render everything.
    pub fn finish(&mut self) -> String {
        let remaining = self.buffer[self.rendered_up_to..].to_string();
        self.rendered_up_to = self.buffer.len();
        if remaining.is_empty() {
            return String::new();
        }
        let rendered = if !self.style.color {
            remaining
        } else {
            let mut renderer = MarkdownRenderer::new(self.style);
            renderer.push(&remaining);
            renderer.finish()
        };
        self.apply_continuation_prefix(&rendered)
    }

    /// Try to render complete lines from the buffer.
    /// We render line-by-line to handle streaming safely — complete lines
    /// are rendered with markdown, the trailing incomplete line is held.
    fn try_render_complete(&mut self) -> String {
        let pending = &self.buffer[self.rendered_up_to..];

        // Find the last newline in the pending content
        let last_newline = pending.rfind('\n');
        let Some(last_nl_offset) = last_newline else {
            // No complete line yet — hold everything
            return String::new();
        };

        let complete = &pending[..=last_nl_offset];
        self.rendered_up_to += complete.len();

        let mut renderer = MarkdownRenderer::new(self.style);
        renderer.push(complete);
        renderer.finish()
    }

    fn try_take_complete_lines(&mut self) -> String {
        let pending = &self.buffer[self.rendered_up_to..];
        let Some(last_nl_offset) = pending.rfind('\n') else {
            return String::new();
        };
        let complete = &pending[..=last_nl_offset];
        self.rendered_up_to += complete.len();
        complete.to_string()
    }

    fn apply_continuation_prefix(&mut self, rendered: &str) -> String {
        if rendered.is_empty() {
            return String::new();
        }

        if self.continuation_prefix.is_empty() {
            self.at_line_start = rendered.ends_with('\n');
            return rendered.to_string();
        }

        let mut out = String::with_capacity(rendered.len() + self.continuation_prefix.len() * 2);
        for ch in rendered.chars() {
            if self.at_line_start && ch != '\n' {
                out.push_str(&self.continuation_prefix);
                self.at_line_start = false;
            }
            out.push(ch);
            if ch == '\n' {
                self.at_line_start = true;
            }
        }
        out
    }
}

/// Internal markdown-to-ANSI renderer using pulldown-cmark events.
struct MarkdownRenderer<'a> {
    style: &'a CliStyle,
    output: String,
    /// Stack of active inline styles (bold, italic, etc.)
    emphasis_depth: usize,
    strong_depth: usize,
    in_code_block: bool,
    code_block_lang: Option<String>,
    code_block_buf: String,
    in_heading: bool,
    heading_level: u8,
    heading_buf: String,
    list_depth: usize,
    ordered_counters: Vec<u64>,
    list_item_stack: Vec<ListItemFrame>,
    /// Track if we already emitted a blank line before a block
    last_was_blank: bool,
    in_block_quote: bool,
    block_quote_buf: String,
    // Table rendering state.
    in_table: bool,
    in_table_head: bool,
    table_alignments: Vec<pulldown_cmark::Alignment>,
    table_rows: Vec<Vec<String>>,
    table_current_row: Vec<String>,
    table_current_cell: String,
}

struct ListItemFrame {
    prefix: String,
    prefix_width: usize,
    content: String,
    emitted: bool,
}

impl<'a> MarkdownRenderer<'a> {
    fn new(style: &'a CliStyle) -> Self {
        Self {
            style,
            output: String::new(),
            emphasis_depth: 0,
            strong_depth: 0,
            in_code_block: false,
            code_block_lang: None,
            code_block_buf: String::new(),
            in_heading: false,
            heading_level: 0,
            heading_buf: String::new(),
            list_depth: 0,
            ordered_counters: Vec::new(),
            list_item_stack: Vec::new(),
            last_was_blank: true,
            in_block_quote: false,
            block_quote_buf: String::new(),
            in_table: false,
            in_table_head: false,
            table_alignments: Vec::new(),
            table_rows: Vec::new(),
            table_current_row: Vec::new(),
            table_current_cell: String::new(),
        }
    }

    fn push(&mut self, text: &str) {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TABLES);

        let parser = Parser::new_ext(text, opts);
        for event in parser {
            self.handle_event(event);
        }
    }

    fn finish(self) -> String {
        self.output
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.handle_start(tag),
            Event::End(tag) => self.handle_end(tag),
            Event::Text(text) => self.handle_text(&text),
            Event::Code(code) => self.handle_inline_code(&code),
            Event::SoftBreak => self.handle_soft_break(),
            Event::HardBreak => self.handle_hard_break(),
            Event::Rule => self.handle_rule(),
            _ => {}
        }
    }

    fn handle_start(&mut self, tag: Tag) {
        match tag {
            Tag::Heading { level, .. } => {
                self.in_heading = true;
                self.heading_level = level as u8;
                self.heading_buf.clear();
            }
            Tag::Paragraph => {
                if !self.in_list_item()
                    && !self.in_heading
                    && !self.in_block_quote
                    && !self.last_was_blank
                {
                    self.output.push('\n');
                }
            }
            Tag::CodeBlock(kind) => {
                self.in_code_block = true;
                self.code_block_buf.clear();
                self.code_block_lang = match kind {
                    CodeBlockKind::Fenced(lang) => {
                        let lang = lang.to_string();
                        if lang.is_empty() {
                            None
                        } else {
                            Some(lang)
                        }
                    }
                    CodeBlockKind::Indented => None,
                };
            }
            Tag::List(start) => {
                self.flush_open_list_item();
                self.list_depth += 1;
                if let Some(n) = start {
                    self.ordered_counters.push(n);
                } else {
                    self.ordered_counters.push(0); // 0 = unordered
                }
                if self.list_depth == 1 && !self.last_was_blank {
                    self.output.push('\n');
                }
            }
            Tag::Item => {
                let indent = "  ".repeat(self.list_depth.saturating_sub(1));
                let is_ordered = self.ordered_counters.last().copied().unwrap_or(0);
                let (prefix, prefix_width) = if is_ordered > 0 {
                    let counter = is_ordered;
                    if let Some(current) = self.ordered_counters.last_mut() {
                        *current += 1;
                    }
                    let label = format!("{counter}. ");
                    (
                        format!("{indent}{}", self.style.markdown_list_enumeration(&label)),
                        indent.chars().count() + label.chars().count(),
                    )
                } else {
                    (
                        format!("{indent}{} ", self.style.markdown_list_item("•")),
                        indent.chars().count() + 2,
                    )
                };
                self.list_item_stack.push(ListItemFrame {
                    prefix,
                    prefix_width,
                    content: String::new(),
                    emitted: false,
                });
            }
            Tag::BlockQuote(_) => {
                self.in_block_quote = true;
                self.block_quote_buf.clear();
            }
            Tag::Emphasis => {
                self.emphasis_depth += 1;
            }
            Tag::Strong => {
                self.strong_depth += 1;
            }
            Tag::Strikethrough => {}
            Tag::Link { .. } => {}
            Tag::Table(alignments) => {
                self.in_table = true;
                self.table_alignments = alignments;
                self.table_rows.clear();
                self.table_current_row.clear();
                self.table_current_cell.clear();
            }
            Tag::TableHead => {
                self.in_table_head = true;
            }
            Tag::TableRow => {
                self.table_current_row.clear();
            }
            Tag::TableCell => {
                self.table_current_cell.clear();
            }
            _ => {}
        }
    }

    fn handle_end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.in_heading = false;
                let text = std::mem::take(&mut self.heading_buf);
                let rendered = match self.heading_level {
                    1 => format!(
                        "\n{}\n",
                        self.style.markdown_heading(&format!("# {}", text))
                    ),
                    2 => format!(
                        "\n{}\n",
                        self.style.markdown_heading(&format!("## {}", text))
                    ),
                    _ => format!(
                        "\n{}\n",
                        self.style.markdown_heading(&format!(
                            "{} {}",
                            "#".repeat(self.heading_level as usize),
                            text
                        ))
                    ),
                };
                self.output.push_str(&rendered);
                self.last_was_blank = false;
            }
            TagEnd::Paragraph => {
                if !self.in_list_item() && !self.in_block_quote {
                    self.output.push('\n');
                    self.last_was_blank = true;
                }
            }
            TagEnd::CodeBlock => {
                self.in_code_block = false;
                let code = std::mem::take(&mut self.code_block_buf);
                let lang_label = self.code_block_lang.take();
                self.render_code_block(&code, lang_label.as_deref());
                self.last_was_blank = false;
            }
            TagEnd::List(_) => {
                self.list_depth = self.list_depth.saturating_sub(1);
                self.ordered_counters.pop();
                self.last_was_blank = true;
            }
            TagEnd::Item => {
                if let Some(item) = self.list_item_stack.pop() {
                    if !item.content.is_empty() || !item.emitted {
                        self.render_list_item(item);
                    }
                }
            }
            TagEnd::BlockQuote(_) => {
                self.in_block_quote = false;
                let text = std::mem::take(&mut self.block_quote_buf);
                for line in text.lines() {
                    self.output.push_str(&format!(
                        "  {} {}\n",
                        self.style.markdown_block_quote("│"),
                        self.style.markdown_block_quote(line)
                    ));
                }
                self.last_was_blank = false;
            }
            TagEnd::Emphasis => {
                self.emphasis_depth = self.emphasis_depth.saturating_sub(1);
            }
            TagEnd::Strong => {
                self.strong_depth = self.strong_depth.saturating_sub(1);
            }
            TagEnd::Table => {
                self.in_table = false;
                self.render_table();
            }
            TagEnd::TableHead => {
                self.in_table_head = false;
            }
            TagEnd::TableRow => {
                if !self.table_current_cell.is_empty() {
                    self.table_current_row.push(std::mem::take(&mut self.table_current_cell));
                }
                if !self.table_current_row.is_empty() {
                    self.table_rows.push(std::mem::take(&mut self.table_current_row));
                }
            }
            TagEnd::TableCell => {
                self.table_current_row.push(std::mem::take(&mut self.table_current_cell));
            }
            _ => {}
        }
    }

    fn handle_text(&mut self, text: &str) {
        if self.in_table {
            self.table_current_cell.push_str(text);
            return;
        }
        if self.in_code_block {
            self.code_block_buf.push_str(text);
            return;
        }

        if self.in_heading {
            self.heading_buf.push_str(text);
            return;
        }

        if self.in_block_quote {
            self.block_quote_buf.push_str(text);
            return;
        }

        if self.in_list_item() {
            let styled = self.apply_inline_style(text);
            self.append_list_item_segment(&styled);
            return;
        }

        let styled = self.apply_inline_style(text);
        self.output.push_str(&styled);
        self.last_was_blank = false;
    }

    fn handle_inline_code(&mut self, code: &str) {
        if self.in_table {
            self.table_current_cell.push('`');
            self.table_current_cell.push_str(code);
            self.table_current_cell.push('`');
            return;
        }
        if self.in_heading {
            self.heading_buf.push_str(code);
            return;
        }
        if self.in_block_quote {
            self.block_quote_buf.push('`');
            self.block_quote_buf.push_str(code);
            self.block_quote_buf.push('`');
            return;
        }

        let rendered = self.style.markdown_code(&format!("`{}`", code));
        if self.in_list_item() {
            self.append_list_item_segment(&rendered);
        } else {
            self.output.push_str(&rendered);
        }
        self.last_was_blank = false;
    }

    fn handle_soft_break(&mut self) {
        if self.in_table {
            self.table_current_cell.push(' ');
        } else if self.in_code_block {
            self.code_block_buf.push('\n');
        } else if self.in_heading {
            self.heading_buf.push(' ');
        } else if self.in_block_quote {
            self.block_quote_buf.push('\n');
        } else if self.in_list_item() {
            self.append_list_item_segment("\n");
        } else {
            self.output.push('\n');
        }
    }

    fn handle_hard_break(&mut self) {
        if self.in_table {
            self.table_current_cell.push(' ');
        } else if self.in_code_block {
            self.code_block_buf.push('\n');
        } else if self.in_list_item() {
            self.append_list_item_segment("\n");
        } else {
            self.output.push('\n');
        }
    }

    fn handle_rule(&mut self) {
        self.output.push_str(&self.style.hr());
        self.output.push('\n');
        self.last_was_blank = true;
    }

    fn render_table(&mut self) {
        if self.table_rows.is_empty() {
            return;
        }
        let col_count = self.table_rows.iter().map(|r| r.len()).max().unwrap_or(0)
            .max(self.table_alignments.len());
        if col_count == 0 {
            return;
        }

        // Compute column widths from content (display width).
        let mut widths = vec![3usize; col_count];
        for row in &self.table_rows {
            for (i, cell) in row.iter().enumerate() {
                let dw = unicode_width::UnicodeWidthStr::width(cell.as_str()) + 2; // padding
                widths[i] = widths[i].max(dw);
            }
        }

        // Constrain to terminal width.
        let max_width = (self.style.width.max(1) as usize).min(72);
        let border_overhead = col_count + 1;
        let content_budget = max_width.saturating_sub(border_overhead);
        let total_content: usize = widths.iter().sum();
        if total_content > content_budget {
            let scale = content_budget as f64 / total_content.max(1) as f64;
            for w in &mut widths {
                *w = ((*w as f64 * scale).max(5.0) as usize).min(*w);
            }
        }

        let border_line = |left: char, mid: char, right: char| -> String {
            let mut s = String::new();
            s.push(left);
            for (i, w) in widths.iter().enumerate() {
                s.push_str(&"─".repeat(*w));
                if i + 1 < widths.len() { s.push(mid); }
            }
            s.push(right);
            s.push('\n');
            s
        };

        let style = self.style;
        let row_line = |row: &[String]| -> String {
            let mut s = String::new();
            s.push('│');
            for i in 0..widths.len() {
                let cell = row.get(i).map(|c| c.as_str()).unwrap_or("");
                let w = widths[i].saturating_sub(2);
                let cell_w = unicode_width::UnicodeWidthStr::width(cell);
                if cell_w <= w {
                    let pad = w - cell_w;
                    s.push(' ');
                    s.push_str(cell);
                    s.push_str(&" ".repeat(pad));
                    s.push(' ');
                } else {
                    s.push(' ');
                    let budget = w.saturating_sub(1); // reserve for ellipsis
                    let mut visible = String::new();
                    let mut visible_w = 0usize;
                    for ch in cell.chars() {
                        let ch_w = unicode_width::UnicodeWidthStr::width(ch.to_string().as_str());
                        if visible_w + ch_w > budget { break; }
                        visible.push(ch);
                        visible_w += ch_w;
                    }
                    s.push_str(&visible);
                    s.push('…');
                    s.push(' ');
                }
                s.push('│');
            }
            s.push('\n');
            s
        };

        self.output.push('\n');
        self.output.push_str(&border_line('┌', '┬', '┐'));
        for (i, row) in self.table_rows.iter().enumerate() {
            self.output.push_str(&style.dim(&row_line(row)));
            if i == 0 {
                self.output.push_str(&style.dim(&border_line('├', '┼', '┤')));
            }
        }
        self.output.push_str(&style.dim(&border_line('└', '┴', '┘')));
        self.output.push('\n');
        self.last_was_blank = true;
    }

    fn apply_inline_style(&self, text: &str) -> String {
        if self.strong_depth > 0 {
            self.style.bold(text)
        } else if self.emphasis_depth > 0 {
            self.style.markdown_emph(text)
        } else {
            text.to_string()
        }
    }

    fn in_list_item(&self) -> bool {
        !self.list_item_stack.is_empty()
    }

    fn append_list_item_segment(&mut self, text: &str) {
        if let Some(item) = self.list_item_stack.last_mut() {
            item.content.push_str(text);
        }
    }

    fn flush_open_list_item(&mut self) {
        let Some(item) = self.list_item_stack.last_mut() else {
            return;
        };
        if item.emitted || item.content.is_empty() {
            return;
        }

        let flushed = ListItemFrame {
            prefix: item.prefix.clone(),
            prefix_width: item.prefix_width,
            content: std::mem::take(&mut item.content),
            emitted: false,
        };
        item.emitted = true;
        self.render_list_item(flushed);
    }

    fn render_list_item(&mut self, item: ListItemFrame) {
        let continuation_indent = " ".repeat(item.prefix_width);
        for (index, line) in item.content.split('\n').enumerate() {
            let prefix = if index == 0 && !item.emitted {
                item.prefix.as_str()
            } else {
                continuation_indent.as_str()
            };
            self.output.push_str(&format!("  {}{}\n", prefix, line));
        }
        self.last_was_blank = false;
    }

    fn render_code_block(&mut self, code: &str, lang: Option<&str>) {
        let header = if let Some(lang) = lang {
            format!(
                "  {} {}\n",
                self.style.markdown_hr("```"),
                self.style.markdown_hr(lang)
            )
        } else {
            format!("  {}\n", self.style.markdown_hr("```"))
        };
        self.output.push_str(&header);

        for line in code.lines() {
            self.output
                .push_str(&format!("  {}\n", self.style.markdown_code(line)));
        }

        self.output
            .push_str(&format!("  {}\n", self.style.markdown_hr("```")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_heading() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_markdown("# Hello World", &style);
        assert!(out.contains("# Hello World"));
        // Should have ANSI codes
        assert!(out.contains("\x1b["));
    }

    #[test]
    fn renders_bold_text() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_markdown("This is **bold** text", &style);
        assert!(out.contains("bold"));
        // Bold uses ANSI \x1b[1m
        assert!(out.contains("\x1b[1m"));
    }

    #[test]
    fn renders_inline_code() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_markdown("Use `cargo test` to run", &style);
        assert!(out.contains("`cargo test`"));
        assert!(out.contains("\x1b[38;2;220;220;170m"));
    }

    #[test]
    fn renders_code_block() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_markdown("```rust\nfn main() {}\n```", &style);
        assert!(out.contains("fn main() {}"));
        assert!(out.contains("rust"));
    }

    #[test]
    fn renders_unordered_list() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_markdown("- item one\n- item two\n- item three", &style);
        assert!(out.contains("item one"));
        assert!(out.contains("item two"));
        assert!(out.contains("item three"));
    }

    #[test]
    fn renders_ordered_list() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_markdown("1. first\n2. second\n3. third", &style);
        assert!(out.contains("first"));
        assert!(out.contains("second"));
        assert!(out.contains("third"));
    }

    #[test]
    fn list_item_keeps_inline_code_and_text_together() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_markdown("- 示例：`git`, `ls`, `npm`", &style);
        assert!(out.contains("示例："));
        assert!(out.contains("`git`"));
        assert!(out.contains("`ls`"));
        assert!(out.contains("`npm`"));
        assert!(!out.contains("• ,"));
    }

    #[test]
    fn list_does_not_insert_extra_blank_line_between_items_and_following_text() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_markdown("- one\n- two\nnext", &style);
        assert!(out.contains("one\n  "));
        assert!(!out.contains("two\n\nnext"));
        assert!(!out.contains("two\n\n\n"));
    }

    #[test]
    fn plain_mode_returns_raw_text() {
        let style = CliStyle::plain();
        let out = render_markdown("# Hello **World**", &style);
        assert_eq!(out, "# Hello **World**");
        assert!(!out.contains("\x1b["));
    }

    #[test]
    fn streamer_buffers_incomplete_lines() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let mut streamer = MarkdownStreamer::new(&style);

        // Push incomplete line — should buffer
        let out1 = streamer.push("Hello ");
        assert!(out1.is_empty());

        // Push more — still no newline
        let out2 = streamer.push("World");
        assert!(out2.is_empty());

        // Push newline — now it should render
        let out3 = streamer.push("\n");
        assert!(out3.contains("Hello World"));
    }

    #[test]
    fn streamer_finish_flushes_remaining() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let mut streamer = MarkdownStreamer::new(&style);

        streamer.push("trailing text");
        let out = streamer.finish();
        assert!(out.contains("trailing text"));
    }

    #[test]
    fn streamer_applies_continuation_prefix_after_newline() {
        let style = CliStyle::plain();
        let mut streamer = MarkdownStreamer::new(&style).with_continuation_prefix("  ");

        let out1 = streamer.push("hello\nworld\n");
        assert_eq!(out1, "hello\n  world\n");

        let out2 = streamer.push("next");
        assert_eq!(out2, "");

        let out3 = streamer.finish();
        assert_eq!(out3, "  next");
    }

    #[test]
    fn renders_block_quote() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_markdown("> quoted text", &style);
        assert!(out.contains("quoted text"));
        assert!(out.contains("│"));
    }

    #[test]
    fn renders_horizontal_rule() {
        let style = CliStyle {
            color: true,
            width: 80,
        };
        let out = render_markdown("---", &style);
        assert!(out.contains("─"));
    }

    // ── Table rendering tests ──────────────────────────────────────────

    #[test]
    fn renders_markdown_table_with_box_borders() {
        let style = CliStyle { color: true, width: 80 };
        let out = render_markdown("| A | B |\n| --- | --- |\n| 1 | 2 |\n", &style);
        assert!(out.contains('┌'), "table missing top-left corner");
        assert!(out.contains('┐'), "table missing top-right corner");
        assert!(out.contains("│"), "table missing vertical border");
        assert!(out.contains("1"), "table missing cell content");
        assert!(out.contains("2"), "table missing cell content");
    }

    #[test]
    fn table_inline_code_preserved_in_cell() {
        let style = CliStyle { color: true, width: 80 };
        let out = render_markdown("| cmd |\n| --- |\n| `ls -la` |\n", &style);
        assert!(out.contains("ls -la"), "inline code must appear in cell");
    }

    #[test]
    fn table_sparse_row_pads_empty_cells() {
        let style = CliStyle { color: true, width: 80 };
        // Row with fewer cells than the header — must still render with aligned borders.
        let out = render_markdown("| A | B |\n| --- | --- |\n| only_one |\n", &style);
        // The closing ┘ should be at the correct position, not shifted left.
        assert!(out.contains('┘'), "table must close properly with sparse row");
    }

    #[test]
    fn table_narrow_width_compresses_columns() {
        let narrow = CliStyle { color: true, width: 25 };
        // 3 columns with very long content in narrow terminal — must truncate.
        let out = render_markdown(
            "| AAAAAA | BBBBBB | CCCCCC |\n| --- | --- | --- |\n| xxxxxx | yyyyyy | zzzzzz |\n",
            &narrow,
        );
        // In 25 cols with 3 wide columns, borders still close correctly.
        assert!(out.contains('┘'), "table must close properly in narrow terminal");
        assert!(!out.is_empty(), "table must produce output");
    }

    #[test]
    fn table_cjk_width_accounted() {
        let style = CliStyle { color: true, width: 80 };
        let out = render_markdown(
            "| 名称 | 值 |\n| --- | --- |\n| 你好 | 世界 |\n",
            &style,
        );
        // The CJK chars should appear, not break the border.
        assert!(out.contains("你好"), "CJK cell content must appear");
        assert!(out.contains('┘'), "table borders must not break with CJK");
    }
}
