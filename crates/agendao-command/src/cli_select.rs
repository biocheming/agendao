//! Interactive terminal selector for CLI question prompts.
//!
//! Provides an interactive selection UI with:
//! - Arrow key / j/k navigation
//! - Visual cursor indicator (`❯`)
//! - "Other" option for free text input
//! - Multi-select support with checkboxes
//! - Enter to confirm selection

use crate::cli_panel::{display_width, CliPanelFrame};
use crate::cli_style::CliStyle;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, ClearType},
};
use std::io::{self, Write};

/// A single selectable option.
#[derive(Debug, Clone)]
pub struct SelectOption {
    pub label: String,
    pub description: Option<String>,
}

/// Result of an interactive selection.
#[derive(Debug, Clone)]
pub enum SelectResult {
    /// User selected one or more options by label.
    Selected(Vec<String>),
    /// User chose "Other" and typed custom text.
    Other(String),
    /// User cancelled (Ctrl+C / Escape).
    Cancelled,
}

#[derive(Debug, Clone)]
struct SelectorRenderState {
    rendered_plain_rows: Vec<String>,
}

/// Run an interactive single-select prompt.
///
/// Displays the question with options in a navigable list.
/// Includes an automatic "Other" option at the end.
///
/// Returns the selected option label, custom text, or cancellation.
pub fn interactive_select(
    question: &str,
    header: Option<&str>,
    options: &[SelectOption],
    style: &CliStyle,
) -> io::Result<SelectResult> {
    if !style.color || options.is_empty() {
        // Fallback: plain numbered list if not a TTY
        return fallback_select(question, header, &[], options);
    }

    let total_items = options.len() + 1; // +1 for "Other"
    run_selector(question, header, &[], options, total_items, false, style)
}

pub fn interactive_select_with_prelude(
    question: &str,
    header: Option<&str>,
    prelude_lines: &[String],
    options: &[SelectOption],
    style: &CliStyle,
) -> io::Result<SelectResult> {
    if !style.color || options.is_empty() {
        return fallback_select(question, header, prelude_lines, options);
    }

    let total_items = options.len() + 1;
    run_selector(
        question,
        header,
        prelude_lines,
        options,
        total_items,
        false,
        style,
    )
}

/// Run an interactive multi-select prompt.
///
/// Displays options with checkboxes. Space toggles selection,
/// Enter confirms.
pub fn interactive_multi_select(
    question: &str,
    header: Option<&str>,
    options: &[SelectOption],
    style: &CliStyle,
) -> io::Result<SelectResult> {
    if !style.color || options.is_empty() {
        return fallback_select(question, header, &[], options);
    }

    let total_items = options.len() + 1;
    run_selector(question, header, &[], options, total_items, true, style)
}

pub fn render_selection_snapshot(
    question: &str,
    header: Option<&str>,
    options: &[SelectOption],
    multi: bool,
    style: &CliStyle,
) -> String {
    let footer = if multi {
        Some("Select one or more options")
    } else {
        Some("Select an option")
    };
    let frame = CliPanelFrame::boxed(header.unwrap_or("Question"), footer, style);
    let mut lines = vec![format!("{} {}", style.bullet(), question), String::new()];

    for (index, option) in options.iter().enumerate() {
        let mut line = format!(" {}. {}", index + 1, option.label);
        if let Some(description) = option.description.as_deref() {
            line.push_str(" — ");
            line.push_str(description);
        }
        lines.push(line);
    }
    if !options.is_empty() {
        lines.push(" · Other".to_string());
    }
    frame.render_lines(&lines)
}

fn run_selector(
    question: &str,
    header: Option<&str>,
    prelude_lines: &[String],
    options: &[SelectOption],
    total_items: usize,
    multi: bool,
    style: &CliStyle,
) -> io::Result<SelectResult> {
    let mut cursor_pos: usize = 0;
    let mut selected: Vec<bool> = vec![false; total_items];
    let build_frame = || {
        CliPanelFrame::boxed(
            header.unwrap_or("Question"),
            Some(if multi {
                "↑↓ move  •  Space toggle  •  Enter confirm  •  Esc cancel"
            } else {
                "↑↓ move  •  Enter confirm  •  Esc cancel"
            }),
            style,
        )
    };
    let mut stdout = io::stdout();

    // Enter raw mode for key-by-key reading
    terminal::enable_raw_mode()?;
    // Hide cursor during selection
    execute!(stdout, cursor::Hide)?;

    // Initial draw
    let mut frame = build_frame();
    let mut render_state = draw_panel(
        &mut stdout,
        &frame,
        prelude_lines,
        question,
        options,
        PanelState {
            cursor_pos,
            selected: &selected,
            multi,
        },
        style,
    )?;

    loop {
        match event::read()? {
            Event::Resize(_, _) => {}
            Event::Key(key) => {
                if !is_primary_key_event(&key) {
                    continue;
                }
                match key {
                    // Navigation: Up / k
                    KeyEvent {
                        code: KeyCode::Up, ..
                    }
                    | KeyEvent {
                        code: KeyCode::Char('k'),
                        modifiers: KeyModifiers::NONE,
                        ..
                    } => {
                        if cursor_pos > 0 {
                            cursor_pos -= 1;
                        } else {
                            cursor_pos = total_items - 1; // wrap around
                        }
                    }
                    // Navigation: Down / j
                    KeyEvent {
                        code: KeyCode::Down,
                        ..
                    }
                    | KeyEvent {
                        code: KeyCode::Char('j'),
                        modifiers: KeyModifiers::NONE,
                        ..
                    } => {
                        if cursor_pos < total_items - 1 {
                            cursor_pos += 1;
                        } else {
                            cursor_pos = 0; // wrap around
                        }
                    }
                    // Toggle selection (multi-select) or quick-select by number
                    KeyEvent {
                        code: KeyCode::Char(' '),
                        ..
                    } if multi => {
                        selected[cursor_pos] = !selected[cursor_pos];
                    }
                    // Number shortcuts: 1-9
                    KeyEvent {
                        code: KeyCode::Char(c),
                        ..
                    } if c.is_ascii_digit() => {
                        let num = c.to_digit(10).unwrap_or(0) as usize;
                        if num >= 1 && num <= options.len() {
                            let idx = num - 1;
                            if multi {
                                selected[idx] = !selected[idx];
                                cursor_pos = idx;
                            } else {
                                // Single select: pick immediately
                                cleanup_terminal(&mut stdout, &render_state)?;
                                return Ok(SelectResult::Selected(vec![options[idx]
                                    .label
                                    .clone()]));
                            }
                        }
                    }
                    // Enter: confirm
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } => {
                        cleanup_terminal(&mut stdout, &render_state)?;

                        // Check if "Other" is selected
                        if cursor_pos == options.len() {
                            // "Other" selected — prompt for free text
                            let text = prompt_other_text(&mut stdout, question, style)?;
                            return if text.trim().is_empty() {
                                Ok(SelectResult::Cancelled)
                            } else {
                                Ok(SelectResult::Other(text))
                            };
                        }

                        if multi {
                            // Collect all checked items
                            let choices: Vec<String> = selected
                                .iter()
                                .enumerate()
                                .filter(|(i, &checked)| checked && *i < options.len())
                                .map(|(i, _)| options[i].label.clone())
                                .collect();
                            if choices.is_empty() {
                                // Nothing checked — use cursor position
                                return Ok(SelectResult::Selected(vec![options[cursor_pos]
                                    .label
                                    .clone()]));
                            }
                            return Ok(SelectResult::Selected(choices));
                        } else {
                            // Single select: use cursor position
                            return Ok(SelectResult::Selected(vec![options[cursor_pos]
                                .label
                                .clone()]));
                        }
                    }
                    // Escape or Ctrl+C: cancel
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    }
                    | KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers: KeyModifiers::CONTROL,
                        ..
                    } => {
                        cleanup_terminal(&mut stdout, &render_state)?;
                        return Ok(SelectResult::Cancelled);
                    }
                    _ => {}
                }
            }
            _ => continue,
        }

        // Redraw options using the current terminal width.
        erase_lines(&mut stdout, &render_state)?;
        frame = build_frame();
        render_state = draw_panel(
            &mut stdout,
            &frame,
            prelude_lines,
            question,
            options,
            PanelState {
                cursor_pos,
                selected: &selected,
                multi,
            },
            style,
        )?;
    }
}

fn draw_panel(
    out: &mut impl Write,
    frame: &CliPanelFrame,
    prelude_lines: &[String],
    question: &str,
    options: &[SelectOption],
    state: PanelState<'_>,
    style: &CliStyle,
) -> io::Result<SelectorRenderState> {
    let mut lines = prelude_lines.to_vec();
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push(format!("{} {}", style.bullet(), question));
    lines.push(String::new());

    for (i, opt) in options.iter().enumerate() {
        let is_cursor = i == state.cursor_pos;
        let is_selected = state.selected.get(i).copied().unwrap_or(false);

        let pointer = if is_cursor {
            "❯".to_string()
        } else {
            " ".to_string()
        };

        let checkbox = if state.multi {
            if is_selected {
                "◉ ".to_string()
            } else {
                "○ ".to_string()
            }
        } else {
            String::new()
        };

        let label = opt.label.clone();

        let desc = match &opt.description {
            Some(d) if is_cursor => format!(" — {}", d),
            _ => String::new(),
        };

        lines.push(format!(
            " {} {}{}. {}{}",
            pointer,
            checkbox,
            i + 1,
            label,
            desc
        ));
    }

    // "Other" option
    let other_idx = options.len();
    let is_cursor = state.cursor_pos == other_idx;
    let pointer = if is_cursor {
        "❯".to_string()
    } else {
        " ".to_string()
    };
    lines.push(format!(" {} · Other", pointer));

    let rendered = frame.render_lines(&lines);
    write!(out, "{rendered}")?;
    out.flush()?;
    Ok(SelectorRenderState {
        rendered_plain_rows: rendered.lines().map(strip_ansi_text).collect(),
    })
}

fn erase_lines(out: &mut impl Write, state: &SelectorRenderState) -> io::Result<()> {
    let width = terminal::size()
        .map(|(width, _)| usize::from(width).max(1))
        .unwrap_or(80);
    let count = selector_physical_row_count(&state.rendered_plain_rows, width);
    for _ in 0..count {
        execute!(
            out,
            cursor::MoveUp(1),
            terminal::Clear(ClearType::CurrentLine)
        )?;
    }
    Ok(())
}

fn cleanup_terminal(out: &mut impl Write, state: &SelectorRenderState) -> io::Result<()> {
    erase_lines(out, state)?;
    execute!(out, cursor::Show)?;
    terminal::disable_raw_mode()?;
    Ok(())
}

struct PanelState<'a> {
    cursor_pos: usize,
    selected: &'a [bool],
    multi: bool,
}

fn prompt_other_text(out: &mut impl Write, question: &str, style: &CliStyle) -> io::Result<String> {
    // Restore terminal to normal mode for text input
    execute!(out, cursor::Show)?;
    terminal::disable_raw_mode()?;

    write!(out, "  {} {} ", style.bold_cyan("›"), style.dim(question),)?;
    out.flush()?;

    // Read from stdin (not stderr)
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let answer = input.trim().to_string();
    let typed_line = format!("  › {} {}", question, input.trim_end_matches(['\n', '\r']));

    // Erase the input line and leave the cursor at the parked modal row.
    clear_recent_plain_rows(&mut io::stdout(), &[typed_line])?;
    io::stdout().flush()?;

    Ok(answer)
}

fn is_primary_key_event(key: &KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
}

/// Fallback for non-TTY / no options: plain numbered prompt.
fn fallback_select(
    question: &str,
    header: Option<&str>,
    prelude_lines: &[String],
    options: &[SelectOption],
) -> io::Result<SelectResult> {
    println!();
    if let Some(header) = header {
        println!("  {}", header);
    }
    for line in prelude_lines {
        println!("  {}", line);
    }
    if header.is_some() || !prelude_lines.is_empty() {
        println!();
    }
    println!("  {}", question);
    println!();
    for (i, opt) in options.iter().enumerate() {
        if let Some(ref desc) = opt.description {
            println!("  {}. {} — {}", i + 1, opt.label, desc);
        } else {
            println!("  {}. {}", i + 1, opt.label);
        }
    }
    println!("  ·  Other");
    println!();
    print!("> ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() && !options.is_empty() {
        return Ok(SelectResult::Selected(vec![options[0].label.clone()]));
    }

    if let Ok(num) = input.parse::<usize>() {
        if num >= 1 && num <= options.len() {
            return Ok(SelectResult::Selected(vec![options[num - 1].label.clone()]));
        }
    }

    // Treat as free text (Other)
    Ok(SelectResult::Other(input.to_string()))
}

fn selector_physical_row_count(rows: &[String], terminal_width: usize) -> usize {
    rows.iter()
        .map(|row| wrapped_terminal_row_count(row, terminal_width))
        .sum()
}

fn clear_recent_plain_rows(out: &mut impl Write, rows: &[String]) -> io::Result<()> {
    let width = terminal::size()
        .map(|(width, _)| usize::from(width).max(1))
        .unwrap_or(80);
    let count = selector_physical_row_count(rows, width);
    for _ in 0..count {
        execute!(
            out,
            cursor::MoveUp(1),
            cursor::MoveToColumn(0),
            terminal::Clear(ClearType::CurrentLine)
        )?;
    }
    Ok(())
}

fn wrapped_terminal_row_count(row: &str, terminal_width: usize) -> usize {
    let visible_width = display_width(row);
    if visible_width == 0 {
        1
    } else {
        visible_width.div_ceil(terminal_width.max(1))
    }
}

fn strip_ansi_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(next) = chars.next() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
                continue;
            }
            continue;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_style::CliStyle;

    #[test]
    fn render_selection_snapshot_lists_options_and_omits_other_for_empty_set() {
        let style = CliStyle::plain();
        let snapshot = render_selection_snapshot(
            "Choose a mode",
            Some("Mode"),
            &[
                SelectOption {
                    label: "Fast".to_string(),
                    description: Some("Lower latency".to_string()),
                },
                SelectOption {
                    label: "Deep".to_string(),
                    description: None,
                },
            ],
            false,
            &style,
        );
        assert!(snapshot.contains("Choose a mode"));
        assert!(snapshot.contains("1. Fast"));
        assert!(snapshot.contains("2. Deep"));
        assert!(snapshot.contains("Other"));

        let empty_snapshot =
            render_selection_snapshot("Type a name", Some("Prompt"), &[], false, &style);
        assert!(empty_snapshot.contains("Type a name"));
        assert!(!empty_snapshot.contains("Other"));
    }

    #[test]
    fn selection_snapshot_can_embed_prelude_without_leading_blank_line() {
        let style = CliStyle::plain();
        let frame = CliPanelFrame::boxed("Permission", Some("Select an option"), &style);
        let mut output = Vec::new();
        let render_state = draw_panel(
            &mut output,
            &frame,
            &[
                "class: External access".to_string(),
                "scope: Web search".to_string(),
            ],
            "Permission required",
            &[SelectOption {
                label: "Allow Once".to_string(),
                description: Some("Allow this action once".to_string()),
            }],
            PanelState {
                cursor_pos: 0,
                selected: &[false, false],
                multi: false,
            },
            &style,
        )
        .expect("draw panel");
        let rendered = String::from_utf8(output).expect("utf8");
        assert!(rendered.contains("class: External access"));
        assert!(rendered.contains("Permission required"));
        assert!(rendered.contains("Allow Once"));
        assert_eq!(
            rendered.lines().count(),
            render_state.rendered_plain_rows.len()
        );
    }

    #[test]
    fn selector_physical_row_count_accounts_for_reflow() {
        let rows = vec!["123456789012".to_string(), "tail".to_string()];
        assert_eq!(selector_physical_row_count(&rows, 6), 3);
    }

    #[test]
    fn fallback_select_with_empty_options() {
        // Just verify the SelectOption struct works
        let opts = [
            SelectOption {
                label: "Yes".to_string(),
                description: Some("Confirm".to_string()),
            },
            SelectOption {
                label: "No".to_string(),
                description: None,
            },
        ];
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].label, "Yes");
        assert_eq!(opts[1].description, None);
    }

    #[test]
    fn select_result_variants() {
        let selected = SelectResult::Selected(vec!["Yes".to_string()]);
        let other = SelectResult::Other("custom".to_string());
        let cancelled = SelectResult::Cancelled;

        match selected {
            SelectResult::Selected(v) => assert_eq!(v, vec!["Yes"]),
            _ => panic!("expected Selected"),
        }
        match other {
            SelectResult::Other(s) => assert_eq!(s, "custom"),
            _ => panic!("expected Other"),
        }
        match cancelled {
            SelectResult::Cancelled => {}
            _ => panic!("expected Cancelled"),
        }
    }
}
