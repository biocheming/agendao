use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use super::code_block::{CodeBlock, CodeTheme};
use crate::theme::Theme;

pub struct MarkdownRenderer {
    theme: Theme,
    code_theme: CodeTheme,
    concealed: bool,
}

#[derive(Clone, Copy)]
struct ListState {
    ordered: bool,
    next: u64,
}

struct TableRow {
    is_header: bool,
    cells: Vec<Vec<Span<'static>>>,
}

struct TableBuilder {
    alignments: Vec<Alignment>,
    rows: Vec<TableRow>,
    current_row: Vec<Vec<Span<'static>>>,
    in_header: bool,
}

impl TableBuilder {
    fn new(alignments: Vec<Alignment>) -> Self {
        Self {
            alignments,
            rows: Vec::new(),
            current_row: Vec::new(),
            in_header: false,
        }
    }

    fn set_header(&mut self, in_header: bool) {
        self.in_header = in_header;
    }

    fn push_cell(&mut self, cell: Vec<Span<'static>>) {
        self.current_row.push(cell);
    }

    fn finish_row(&mut self) {
        if self.current_row.is_empty() {
            return;
        }
        self.rows.push(TableRow {
            is_header: self.in_header,
            cells: std::mem::take(&mut self.current_row),
        });
    }

    fn render(
        mut self,
        lines: &mut Vec<Line<'static>>,
        theme: &Theme,
        available_width: u16,
    ) {
        self.finish_row();
        if self.rows.is_empty() {
            return;
        }

        let col_count = self
            .rows
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or(0)
            .max(self.alignments.len());
        if col_count == 0 {
            return;
        }

        // Compute per-column content widths.
        let mut content_widths = vec![1usize; col_count];
        for row in &self.rows {
            for (idx, cell) in row.cells.iter().enumerate() {
                content_widths[idx] = content_widths[idx].max(cell_span_width(cell));
            }
        }

        // Table width = sum(content_width + 2 padding) + (col_count + 1) borders.
        let table_total: usize = content_widths.iter().map(|w| w + 2).sum::<usize>() + col_count + 1;
        let avail = available_width.max(1) as usize;

        if table_total <= avail {
            // Fits: render as-is.
            let widths = content_widths;
            Self::emit_box_table(lines, &self.rows, &widths, &self.alignments, theme);
        } else {
            // Compress each column proportionally.
            let border_overhead = 2 * col_count + col_count + 1; // padding + borders
            let content_budget = avail.saturating_sub(border_overhead).max(col_count);
            let compressed: Vec<usize> = content_widths.iter().map(|&w| {
                let share = (w as f64 / table_total.max(1) as f64) * content_budget as f64;
                share.max(3.0).min(w as f64) as usize
            }).collect();

            if compressed.iter().sum::<usize>() + border_overhead <= avail {
                Self::emit_box_table(lines, &self.rows, &compressed, &self.alignments, theme);
            } else {
                // Too narrow even compressed — degrade to stacked.
                Self::emit_stacked_table(lines, &self.rows, theme, available_width.max(1));
            }
        }
    }

    fn emit_box_table(
        lines: &mut Vec<Line<'static>>,
        rows: &[TableRow],
        widths: &[usize],
        alignments: &[Alignment],
        theme: &Theme,
    ) {
        lines.push(table_border_line('┌', '┬', '┐', widths, theme.markdown_horizontal_rule));
        for (idx, row) in rows.iter().enumerate() {
            lines.push(table_row_line(row, widths, alignments, theme));
            let is_header_break =
                row.is_header && rows.get(idx + 1).is_some_and(|next| !next.is_header);
            if is_header_break {
                lines.push(table_border_line('├', '┼', '┤', widths, theme.markdown_horizontal_rule));
            }
        }
        lines.push(table_border_line('└', '┴', '┘', widths, theme.markdown_horizontal_rule));
    }

    /// Narrow-width degradation: render each row as `Header: Value` pairs.
    fn emit_stacked_table(
        lines: &mut Vec<Line<'static>>,
        rows: &[TableRow],
        theme: &Theme,
        avail: u16,
    ) {
        let header_style = Style::default()
            .fg(theme.markdown_heading)
            .add_modifier(Modifier::BOLD);
        let body_style = Style::default().fg(theme.text);
        let dim_style = Style::default().fg(theme.markdown_horizontal_rule);

        let header_labels: Vec<String> = rows
            .iter()
            .find(|r| r.is_header)
            .map(|r| r.cells.iter().map(|c| c.iter().map(|s| s.content.as_ref().to_string()).collect::<Vec<_>>().join(" ")).collect())
            .unwrap_or_default();

        for row in rows {
            if row.is_header {
                continue;
            }
            lines.push(Line::from(Span::styled(
                "─".repeat(avail.min(40) as usize),
                dim_style,
            )));
            for (idx, cell) in row.cells.iter().enumerate() {
                let label = header_labels.get(idx).map(|s| s.as_str()).unwrap_or("?");
                let mut spans: Vec<Span<'static>> = vec![
                    Span::styled(format!("{label}: "), header_style),
                ];
                spans.extend(cell.iter().cloned().map(|s| Span::styled(
                    s.content.to_string(),
                    body_style,
                )));
                lines.push(Line::from(spans));
            }
        }
    }
}

impl MarkdownRenderer {
    pub fn new(theme: Theme) -> Self {
        let code_theme = CodeTheme::from_app_theme(&theme);
        Self {
            theme,
            code_theme,
            concealed: false,
        }
    }

    pub fn with_concealed(mut self, concealed: bool) -> Self {
        self.concealed = concealed;
        self
    }

    pub fn render(&self, text: &str, frame: &mut Frame, area: Rect) {
        let lines = self.to_lines(text, Some(area.width));
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }

    pub fn to_lines(&self, text: &str, available_width: Option<u16>) -> Vec<Line<'static>> {
        let mut options = Options::empty();
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_TASKLISTS);
        options.insert(Options::ENABLE_FOOTNOTES);

        let parser = Parser::new_ext(text, options);

        // Default to 80 columns when caller doesn't know the width.
        // This ensures table compression can activate on narrow terminals
        // even when the call site hasn't been refactored to thread area.width.
        const DEFAULT_CONTENT_WIDTH: u16 = 80;
        let avail = available_width.unwrap_or(DEFAULT_CONTENT_WIDTH);

        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut current: Vec<Span<'static>> = Vec::new();
        let mut styles = vec![Style::default().fg(self.theme.text)];
        let mut list_stack: Vec<ListState> = Vec::new();
        let mut blockquote_depth = 0usize;
        let mut link_urls: Vec<String> = Vec::new();
        let mut code_block: Option<(Option<String>, String)> = None;
        let mut table: Option<TableBuilder> = None;

        for event in parser {
            if let Some((_, code)) = code_block.as_mut() {
                match event {
                    Event::Text(text) | Event::Code(text) => {
                        code.push_str(&text);
                    }
                    Event::SoftBreak | Event::HardBreak => code.push('\n'),
                    Event::End(TagEnd::CodeBlock) => {
                        if let Some((language, content)) = code_block.take() {
                            self.render_code_block(language, &content, &mut lines);
                        }
                        push_blank_line(&mut lines);
                    }
                    _ => {}
                }
                continue;
            }

            match event {
                Event::Start(tag) => match tag {
                    Tag::Paragraph => {}
                    Tag::Heading { level, .. } => {
                        flush_line(&mut lines, &mut current);
                        let style = heading_style(level, &self.theme);
                        styles.push(style);
                        ensure_prefix(&mut current, blockquote_depth, &self.theme);
                        current.push(Span::styled(
                            format!("{} ", "#".repeat(heading_level_to_u8(level) as usize)),
                            style,
                        ));
                    }
                    Tag::BlockQuote(_) => {
                        flush_line(&mut lines, &mut current);
                        blockquote_depth += 1;
                    }
                    Tag::CodeBlock(kind) => {
                        flush_line(&mut lines, &mut current);
                        let language = match kind {
                            CodeBlockKind::Fenced(lang) => {
                                let trimmed = lang.trim();
                                if trimmed.is_empty() {
                                    None
                                } else {
                                    Some(trimmed.to_string())
                                }
                            }
                            CodeBlockKind::Indented => None,
                        };
                        code_block = Some((language, String::new()));
                    }
                    Tag::List(start) => {
                        flush_line(&mut lines, &mut current);
                        list_stack.push(ListState {
                            ordered: start.is_some(),
                            next: start.unwrap_or(1),
                        });
                    }
                    Tag::Item => {
                        flush_line(&mut lines, &mut current);
                        ensure_prefix(&mut current, blockquote_depth, &self.theme);
                        let (prefix, color) = if let Some(state) = list_stack.last_mut() {
                            if state.ordered {
                                let out = format!("{}. ", state.next);
                                state.next += 1;
                                (out, self.theme.markdown_list_enumeration)
                            } else {
                                ("• ".to_string(), self.theme.markdown_list_item)
                            }
                        } else {
                            ("• ".to_string(), self.theme.markdown_list_item)
                        };
                        current.push(Span::styled(prefix, Style::default().fg(color)));
                    }
                    Tag::Emphasis => {
                        let style = current_style(&styles)
                            .fg(self.theme.markdown_emph)
                            .add_modifier(Modifier::ITALIC);
                        styles.push(style);
                    }
                    Tag::Strong => {
                        let style = current_style(&styles)
                            .fg(self.theme.markdown_strong)
                            .add_modifier(Modifier::BOLD);
                        styles.push(style);
                    }
                    Tag::Strikethrough => {
                        let style = current_style(&styles).add_modifier(Modifier::CROSSED_OUT);
                        styles.push(style);
                    }
                    Tag::Link { dest_url, .. } => {
                        link_urls.push(dest_url.to_string());
                        let style = current_style(&styles)
                            .fg(self.theme.markdown_link_text)
                            .add_modifier(Modifier::UNDERLINED);
                        styles.push(style);
                    }
                    Tag::Image { dest_url, .. } => {
                        link_urls.push(dest_url.to_string());
                        let style = current_style(&styles).fg(self.theme.markdown_image);
                        styles.push(style);
                        if table.is_none() {
                            ensure_prefix(&mut current, blockquote_depth, &self.theme);
                        }
                        current.push(Span::styled("[image] ", style));
                    }
                    Tag::Table(alignments) => {
                        flush_line(&mut lines, &mut current);
                        table = Some(TableBuilder::new(alignments));
                    }
                    Tag::TableHead => {
                        if let Some(tbl) = table.as_mut() {
                            tbl.set_header(true);
                        } else {
                            flush_line(&mut lines, &mut current);
                        }
                    }
                    Tag::TableRow => {
                        if table.is_none() {
                            flush_line(&mut lines, &mut current);
                        }
                    }
                    Tag::TableCell => {
                        if table.is_some() {
                            current.clear();
                        } else {
                            ensure_prefix(&mut current, blockquote_depth, &self.theme);
                            if !current.is_empty() {
                                current.push(Span::styled(
                                    " │ ",
                                    Style::default().fg(self.theme.text_muted),
                                ));
                            }
                        }
                    }
                    _ => {}
                },
                Event::End(end) => match end {
                    TagEnd::Paragraph => {
                        flush_line(&mut lines, &mut current);
                        push_blank_line(&mut lines);
                    }
                    TagEnd::Heading(_) => {
                        if styles.len() > 1 {
                            styles.pop();
                        }
                        flush_line(&mut lines, &mut current);
                        push_blank_line(&mut lines);
                    }
                    TagEnd::BlockQuote(_) => {
                        flush_line(&mut lines, &mut current);
                        blockquote_depth = blockquote_depth.saturating_sub(1);
                        push_blank_line(&mut lines);
                    }
                    TagEnd::List(_) => {
                        flush_line(&mut lines, &mut current);
                        list_stack.pop();
                        push_blank_line(&mut lines);
                    }
                    TagEnd::Item => {
                        flush_line(&mut lines, &mut current);
                    }
                    TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                        if styles.len() > 1 {
                            styles.pop();
                        }
                    }
                    TagEnd::Link => {
                        if styles.len() > 1 {
                            styles.pop();
                        }
                        if let Some(url) = link_urls.pop() {
                            current.push(Span::styled(
                                format!(" ({url})"),
                                Style::default().fg(self.theme.markdown_link),
                            ));
                        }
                    }
                    TagEnd::Image => {
                        if styles.len() > 1 {
                            styles.pop();
                        }
                        if let Some(url) = link_urls.pop() {
                            current.push(Span::styled(
                                format!(" ({url})"),
                                Style::default().fg(self.theme.markdown_image_text),
                            ));
                        }
                    }
                    TagEnd::Table => {
                        if let Some(tbl) = table.take() {
                            tbl.render(&mut lines, &self.theme, avail);
                        } else {
                            flush_line(&mut lines, &mut current);
                        }
                        push_blank_line(&mut lines);
                    }
                    TagEnd::TableHead => {
                        if let Some(tbl) = table.as_mut() {
                            tbl.finish_row();
                            tbl.set_header(false);
                        } else {
                            flush_line(&mut lines, &mut current);
                        }
                    }
                    TagEnd::TableRow => {
                        if let Some(tbl) = table.as_mut() {
                            tbl.finish_row();
                        } else {
                            flush_line(&mut lines, &mut current);
                        }
                    }
                    TagEnd::TableCell => {
                        if let Some(tbl) = table.as_mut() {
                            tbl.push_cell(std::mem::take(&mut current));
                        }
                    }
                    _ => {}
                },
                Event::Text(text) => {
                    if table.is_none() {
                        ensure_prefix(&mut current, blockquote_depth, &self.theme);
                    }
                    current.push(Span::styled(text.to_string(), current_style(&styles)));
                }
                Event::Code(text) => {
                    if table.is_none() {
                        ensure_prefix(&mut current, blockquote_depth, &self.theme);
                    }
                    current.push(Span::styled(
                        text.to_string(),
                        Style::default()
                            .fg(self.theme.markdown_code)
                            .bg(self.theme.markdown_code_bg),
                    ));
                }
                Event::SoftBreak => {
                    if table.is_some() {
                        current.push(Span::raw(" "));
                    } else {
                        flush_line(&mut lines, &mut current);
                    }
                }
                Event::HardBreak => {
                    if table.is_some() {
                        current.push(Span::raw(" "));
                    } else {
                        flush_line(&mut lines, &mut current);
                    }
                }
                Event::Rule => {
                    flush_line(&mut lines, &mut current);
                    lines.push(Line::from(Span::styled(
                        "─".repeat(40),
                        Style::default().fg(self.theme.markdown_horizontal_rule),
                    )));
                    push_blank_line(&mut lines);
                }
                Event::Html(html) | Event::InlineHtml(html) => {
                    if table.is_none() {
                        ensure_prefix(&mut current, blockquote_depth, &self.theme);
                    }
                    current.push(Span::styled(
                        html.to_string(),
                        Style::default().fg(self.theme.text_muted),
                    ));
                }
                Event::TaskListMarker(checked) => {
                    if table.is_none() {
                        ensure_prefix(&mut current, blockquote_depth, &self.theme);
                    }
                    let mark = if checked { "[x] " } else { "[ ] " };
                    current.push(Span::styled(
                        mark,
                        Style::default().fg(self.theme.markdown_list_item),
                    ));
                }
                Event::FootnoteReference(label) => {
                    if table.is_none() {
                        ensure_prefix(&mut current, blockquote_depth, &self.theme);
                    }
                    current.push(Span::styled(
                        format!("[^{label}]"),
                        Style::default().fg(self.theme.text_muted),
                    ));
                }
                _ => {}
            }
        }

        flush_line(&mut lines, &mut current);
        while lines.last().is_some_and(line_is_blank) {
            lines.pop();
        }
        lines
    }

    fn render_code_block(
        &self,
        language: Option<String>,
        code: &str,
        lines: &mut Vec<Line<'static>>,
    ) {
        if self.concealed {
            lines.push(Line::from(vec![
                Span::styled("▶ ", Style::default().fg(self.theme.primary)),
                Span::styled(
                    format!(
                        "[{} - {} lines]",
                        language.as_deref().unwrap_or("code"),
                        code.lines().count()
                    ),
                    Style::default().fg(self.theme.text_muted),
                ),
            ]));
            return;
        }

        let code_block = CodeBlock::new(language.clone(), code.to_string());

        lines.push(Line::from(vec![
            Span::styled(
                "╭",
                Style::default().fg(self.theme.markdown_horizontal_rule),
            ),
            Span::styled(
                format!(" {} ", language.as_deref().unwrap_or("code")),
                Style::default().fg(self.theme.markdown_code_block),
            ),
            Span::styled(
                "─".repeat(20),
                Style::default().fg(self.theme.markdown_horizontal_rule),
            ),
        ]));

        for code_spans in code_block.to_lines(&self.code_theme) {
            let mut line_spans = vec![Span::styled(
                "│ ",
                Style::default().fg(self.theme.markdown_horizontal_rule),
            )];
            line_spans.extend(code_spans);
            lines.push(Line::from(line_spans));
        }

        lines.push(Line::from(Span::styled(
            "╰───",
            Style::default().fg(self.theme.markdown_horizontal_rule),
        )));
    }
}

fn table_border_line(
    left: char,
    middle: char,
    right: char,
    widths: &[usize],
    color: ratatui::style::Color,
) -> Line<'static> {
    let mut text = String::new();
    text.push(left);
    for (idx, width) in widths.iter().enumerate() {
        text.push_str(&"─".repeat(width + 2));
        if idx + 1 < widths.len() {
            text.push(middle);
        }
    }
    text.push(right);
    Line::from(Span::styled(text, Style::default().fg(color)))
}

fn table_row_line(
    row: &TableRow,
    widths: &[usize],
    alignments: &[Alignment],
    theme: &Theme,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let border_style = Style::default().fg(theme.markdown_horizontal_rule);
    let content_style = if row.is_header {
        Style::default()
            .fg(theme.markdown_heading)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text)
    };

    spans.push(Span::styled("│", border_style));
    for (idx, width) in widths.iter().enumerate() {
        let cell = row.cells.get(idx).cloned().unwrap_or_default();
        let cell_width = cell_span_width(&cell);
        let padding = width.saturating_sub(cell_width);
        let alignment = alignments.get(idx).copied().unwrap_or(Alignment::None);
        let (left_pad, right_pad) = match alignment {
            Alignment::Right => (padding, 0),
            Alignment::Center => (padding / 2, padding - padding / 2),
            _ => (0, padding),
        };

        spans.push(Span::styled(" ", content_style));
        if left_pad > 0 {
            spans.push(Span::styled(" ".repeat(left_pad), content_style));
        }

        // Emit cell spans, truncating if they exceed the assigned width budget.
        let budget = *width;
        if cell_width <= budget {
            // Fits — no truncation needed.
            if row.is_header {
                for span in cell {
                    spans.push(Span::styled(
                        span.content.to_string(),
                        span.style.fg(theme.markdown_heading).add_modifier(Modifier::BOLD),
                    ));
                }
            } else {
                spans.extend(cell);
            }
        } else {
            // Truncate: emit spans up to budget. Reserve 1 width for ellipsis
            // when we know truncation will happen (cell_width > budget).
            let needs_ellipsis = cell_width > budget;
            let ellipsis = if needs_ellipsis { 1usize } else { 0usize };
            let effective_budget = budget.saturating_sub(ellipsis);
            let mut emitted = 0usize;

            for span in &cell {
                let sw = UnicodeWidthStr::width(span.content.as_ref());
                let remaining = effective_budget.saturating_sub(emitted);
                if remaining == 0 {
                    break;
                }
                if sw <= remaining {
                    // Full span fits within (budget - ellipsis).
                    if row.is_header {
                        spans.push(Span::styled(
                            span.content.to_string(),
                            span.style.fg(theme.markdown_heading).add_modifier(Modifier::BOLD),
                        ));
                    } else {
                        spans.push(span.clone());
                    }
                    emitted += sw;
                } else {
                    // Partial span: take characters by display width.
                    let mut prefix = String::new();
                    let mut prefix_w = 0usize;
                    for ch in span.content.chars() {
                        let ch_w = UnicodeWidthStr::width(ch.to_string().as_str());
                        if prefix_w + ch_w > remaining {
                            break;
                        }
                        prefix.push(ch);
                        prefix_w += ch_w;
                    }
                    if !prefix.is_empty() {
                        if row.is_header {
                            spans.push(Span::styled(
                                prefix,
                                span.style.fg(theme.markdown_heading).add_modifier(Modifier::BOLD),
                            ));
                        } else {
                            spans.push(Span::styled(prefix, span.style));
                        }
                    }
                    emitted = effective_budget; // force break
                }
            }
            if needs_ellipsis {
                spans.push(Span::styled("…", content_style));
            }
        }

        if right_pad > 0 {
            spans.push(Span::styled(" ".repeat(right_pad), content_style));
        }
        spans.push(Span::styled(" ", content_style));
        spans.push(Span::styled("│", border_style));
    }

    Line::from(spans)
}

fn cell_span_width(spans: &[Span<'static>]) -> usize {
    spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum::<usize>()
}

fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn heading_style(level: HeadingLevel, theme: &Theme) -> Style {
    match level {
        HeadingLevel::H1 | HeadingLevel::H2 => Style::default()
            .fg(theme.markdown_heading)
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(theme.markdown_heading)
            .add_modifier(Modifier::BOLD),
    }
}

fn ensure_prefix(spans: &mut Vec<Span<'static>>, depth: usize, theme: &Theme) {
    if !spans.is_empty() {
        return;
    }
    for _ in 0..depth {
        spans.push(Span::styled(
            "│ ",
            Style::default().fg(theme.markdown_block_quote),
        ));
    }
}

fn current_style(styles: &[Style]) -> Style {
    styles
        .last()
        .copied()
        .unwrap_or_else(|| Style::default().fg(ratatui::style::Color::White))
}

fn flush_line(lines: &mut Vec<Line<'static>>, current: &mut Vec<Span<'static>>) {
    if current.is_empty() {
        return;
    }
    lines.push(Line::from(std::mem::take(current)));
}

fn push_blank_line(lines: &mut Vec<Line<'static>>) {
    if lines.last().is_some_and(line_is_blank) {
        return;
    }
    lines.push(Line::from(""));
}

fn line_is_blank(line: &Line<'_>) -> bool {
    line.spans
        .iter()
        .all(|span| span.content.as_ref().trim().is_empty())
}
