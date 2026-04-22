//! Enhanced REPL prompt with line editing and history.
//!
//! Uses crossterm raw mode for character-by-character input:
//! - Left/Right arrow keys for cursor movement
//! - Up/Down arrow keys for history navigation
//! - Ctrl+P / Ctrl+N as alternate history navigation
//! - Alt+Up / Alt+Down for cursor movement across wrapped rows
//! - Home/End for start/end of the current visual row
//! - Backspace and Delete
//! - Ctrl+C to cancel current line
//! - Ctrl+D on empty line to exit
//! - Enter to submit
//! - Shift+Enter to insert newline
//! - Tab to apply the best available completion
//! - Ctrl+U to clear line
//! - Ctrl+W to delete word backward

use crate::cli_panel::{
    char_display_width, display_width_between, pad_right_display,
    row_char_index_for_display_column, truncate_display,
};
use crate::cli_style::CliStyle;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, ClearType},
};
use std::io::{self, Write};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

type PromptFrameFactory = Arc<dyn Fn(&str, usize) -> PromptFrame + Send + Sync + 'static>;
type PromptCompletionFactory =
    Arc<dyn Fn(&str, usize) -> Option<PromptCompletion> + Send + Sync + 'static>;

/// Result of reading a prompt line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptResult {
    /// User submitted a line of text.
    Line(String),
    /// User pressed Ctrl+D on an empty line (exit signal).
    Eof,
    /// User pressed Ctrl+C (cancel current input, not exit).
    Interrupt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptSessionEvent {
    Line(String),
    Eof,
    Interrupt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptCompletion {
    pub line: String,
    pub cursor_pos: usize,
}

/// Prompt history buffer.
#[derive(Debug, Clone)]
pub struct PromptHistory {
    entries: Vec<String>,
    max_size: usize,
}

impl PromptHistory {
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: Vec::new(),
            max_size,
        }
    }

    /// Add a new entry to the history (most recent at the end).
    pub fn push(&mut self, line: &str) {
        let line = line.trim().to_string();
        if line.is_empty() {
            return;
        }
        self.entries.retain(|entry| entry != &line);
        self.entries.push(line);
        if self.entries.len() > self.max_size {
            self.entries.remove(0);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&str> {
        self.entries.get(index).map(|entry| entry.as_str())
    }
}

/// Visual prompt frame for interactive CLI input.
#[derive(Debug, Clone)]
pub struct PromptFrame {
    plain_prompt: String,
    header_line: String,
    footer_line: String,
    screen_lines: Vec<String>,
    input_prefix_width: u16,
    inner_width: usize,
    max_visible_rows: usize,
    color: bool,
}

pub struct PromptSession {
    command_tx: mpsc::Sender<PromptSessionCommand>,
    worker: Option<thread::JoinHandle<()>>,
}

enum PromptSessionCommand {
    Refresh(SyncSender<()>),
    Suspend(SyncSender<()>),
    Resume(SyncSender<()>),
    Shutdown(SyncSender<()>),
}

#[derive(Debug, Clone)]
struct PromptRenderState {
    cursor_row_in_view: usize,
    screen_rows: usize,
    frame_height: usize,
}

#[derive(Debug, Clone)]
struct WrappedRow {
    start: usize,
    end: usize,
    text: String,
}

#[derive(Debug, Clone)]
struct WrappedViewport {
    visible_rows: Vec<String>,
    cursor_row: usize,
    cursor_col: usize,
}

impl PromptFrame {
    pub fn boxed(mode_label: &str, model_label: &str, style: &CliStyle) -> Self {
        Self::boxed_with_footer(
            mode_label,
            model_label,
            " Ready  •  Alt+Enter/Ctrl+J newline  •  /help  •  Ctrl+D exit ",
            style,
        )
    }

    pub fn boxed_with_footer(
        mode_label: &str,
        model_label: &str,
        footer_text: &str,
        style: &CliStyle,
    ) -> Self {
        let header_text = truncate_visible(
            &format!(
                " {}{}{} ",
                mode_label.trim(),
                bullet_separator(style),
                model_label.trim()
            ),
            160,
        );
        let footer_text = truncate_visible(footer_text, 160);
        let inner_width = usize::from(style.width.saturating_sub(5)).max(20);
        let chrome_width = inner_width + 2;
        let max_visible_rows = prompt_max_visible_rows();

        let header_content = pad_right(
            &truncate_visible(&header_text, chrome_width),
            chrome_width,
            '─',
        );
        let footer_content = pad_right(
            &truncate_visible(&footer_text, chrome_width),
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

        let footer_line = if style.color {
            format!(
                "{}{}{}",
                style.cyan("╰"),
                style.dim(&footer_content),
                style.cyan("╯")
            )
        } else {
            format!("╰{}╯", footer_content)
        };

        Self {
            plain_prompt: "> ".to_string(),
            header_line,
            footer_line,
            screen_lines: Vec::new(),
            input_prefix_width: 2,
            inner_width,
            max_visible_rows,
            color: style.color,
        }
    }

    pub fn content_width(&self) -> usize {
        self.inner_width
    }

    pub fn with_screen_lines(mut self, lines: Vec<String>) -> Self {
        self.screen_lines = lines;
        self
    }
}

impl PromptSession {
    pub fn spawn(
        frame_factory: PromptFrameFactory,
        completion_factory: Option<PromptCompletionFactory>,
        event_tx: UnboundedSender<PromptSessionEvent>,
    ) -> io::Result<Self> {
        let (command_tx, command_rx) = mpsc::channel::<PromptSessionCommand>();
        let error_event_tx = event_tx.clone();
        let worker = thread::Builder::new()
            .name("rocode-cli-prompt".to_string())
            .spawn(move || {
                if let Err(error) =
                    run_prompt_session(frame_factory, completion_factory, event_tx, command_rx)
                {
                    let _ = error_event_tx.send(PromptSessionEvent::Interrupt);
                    eprintln!("prompt session error: {error}");
                }
            })?;
        Ok(Self {
            command_tx,
            worker: Some(worker),
        })
    }

    pub fn refresh(&self) -> io::Result<()> {
        self.request(PromptSessionCommand::Refresh)
    }

    pub fn suspend(&self) -> io::Result<()> {
        self.request(PromptSessionCommand::Suspend)
    }

    pub fn resume(&self) -> io::Result<()> {
        self.request(PromptSessionCommand::Resume)
    }

    pub fn shutdown(&self) -> io::Result<()> {
        self.request(PromptSessionCommand::Shutdown)
    }

    fn request(
        &self,
        build: impl FnOnce(SyncSender<()>) -> PromptSessionCommand,
    ) -> io::Result<()> {
        let (ack_tx, ack_rx) = mpsc::sync_channel(0);
        self.command_tx
            .send(build(ack_tx))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "prompt session closed"))?;
        ack_rx
            .recv()
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "prompt session ack failed"))
    }
}

impl Drop for PromptSession {
    fn drop(&mut self) {
        let _ = self.shutdown();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_prompt_session(
    frame_factory: PromptFrameFactory,
    completion_factory: Option<PromptCompletionFactory>,
    event_tx: UnboundedSender<PromptSessionEvent>,
    command_rx: mpsc::Receiver<PromptSessionCommand>,
) -> io::Result<()> {
    let mut line = String::new();
    let mut cursor_pos = 0usize;
    let mut preferred_column: Option<usize> = None;
    let mut history = PromptHistory::new(200);
    let mut history_index: Option<usize> = None;
    let mut saved_input = String::new();
    let mut stdout = io::stdout();
    let mut suspend_depth = 0usize;
    let mut frame = frame_factory(&line, cursor_pos);

    terminal::enable_raw_mode()?;
    let mut render_state = Some(render_prompt_frame(
        &mut stdout,
        &frame,
        &line,
        cursor_pos,
        None,
    )?);

    loop {
        while let Ok(command) = command_rx.try_recv() {
            match command {
                PromptSessionCommand::Refresh(ack) => {
                    frame = frame_factory(&line, cursor_pos);
                    if suspend_depth == 0 {
                        render_state = Some(render_prompt_frame(
                            &mut stdout,
                            &frame,
                            &line,
                            cursor_pos,
                            render_state.as_ref(),
                        )?);
                    }
                    let _ = ack.send(());
                }
                PromptSessionCommand::Suspend(ack) => {
                    suspend_depth = suspend_depth.saturating_add(1);
                    if suspend_depth == 1 {
                        if let Some(state) = render_state.take() {
                            dismiss_prompt(&mut stdout, &state)?;
                        }
                        // Disable raw mode so the interactive selector (or any
                        // other consumer) gets exclusive access to the terminal
                        // and the crossterm global event reader.
                        terminal::disable_raw_mode()?;
                    }
                    let _ = ack.send(());
                }
                PromptSessionCommand::Resume(ack) => {
                    frame = frame_factory(&line, cursor_pos);
                    suspend_depth = suspend_depth.saturating_sub(1);
                    if suspend_depth == 0 && render_state.is_none() {
                        // Re-enable raw mode before rendering the prompt.
                        terminal::enable_raw_mode()?;
                        render_state = Some(render_prompt_frame(
                            &mut stdout,
                            &frame,
                            &line,
                            cursor_pos,
                            None,
                        )?);
                    } else if suspend_depth == 0 {
                        render_state = Some(render_prompt_frame(
                            &mut stdout,
                            &frame,
                            &line,
                            cursor_pos,
                            render_state.as_ref(),
                        )?);
                    }
                    let _ = ack.send(());
                }
                PromptSessionCommand::Shutdown(ack) => {
                    if let Some(state) = render_state.take() {
                        dismiss_prompt(&mut stdout, &state)?;
                    }
                    let _ = ack.send(());
                    terminal::disable_raw_mode()?;
                    return Ok(());
                }
            }
        }

        if suspend_depth > 0 {
            thread::sleep(Duration::from_millis(20));
            continue;
        }

        if !event::poll(Duration::from_millis(20))? {
            continue;
        }

        let ev = event::read()?;
        match ev {
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Enter
                    && (key.modifiers.contains(KeyModifiers::SHIFT)
                        || key.modifiers.contains(KeyModifiers::ALT)) =>
            {
                insert_char_at_cursor(&mut line, cursor_pos, '\n');
                cursor_pos += 1;
                preferred_column = None;
                history_index = None;
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('j')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                insert_char_at_cursor(&mut line, cursor_pos, '\n');
                cursor_pos += 1;
                preferred_column = None;
                history_index = None;
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Enter => {
                let submitted = line.clone();
                if !submitted.trim().is_empty() {
                    history.push(&submitted);
                }
                let _ = event_tx.send(PromptSessionEvent::Line(submitted));
                line.clear();
                cursor_pos = 0;
                preferred_column = None;
                history_index = None;
                saved_input.clear();
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                line.clear();
                cursor_pos = 0;
                preferred_column = None;
                history_index = None;
                saved_input.clear();
                let _ = event_tx.send(PromptSessionEvent::Interrupt);
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('d')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if line.is_empty() {
                    if let Some(state) = render_state.take() {
                        dismiss_prompt(&mut stdout, &state)?;
                    }
                    let _ = event_tx.send(PromptSessionEvent::Eof);
                    terminal::disable_raw_mode()?;
                    return Ok(());
                }
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('u')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                line.clear();
                cursor_pos = 0;
                preferred_column = None;
                history_index = None;
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('w')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if cursor_pos > 0 {
                    let chars: Vec<char> = line.chars().collect();
                    let mut new_pos = cursor_pos;
                    while new_pos > 0 && chars[new_pos - 1].is_whitespace() {
                        new_pos -= 1;
                    }
                    while new_pos > 0 && !chars[new_pos - 1].is_whitespace() {
                        new_pos -= 1;
                    }
                    replace_char_range(&mut line, new_pos, cursor_pos, "");
                    cursor_pos = new_pos;
                    preferred_column = None;
                    history_index = None;
                }
            }
            Event::Key(key) if is_history_prev_key(&key) => {
                browse_history_prev(
                    &history,
                    &mut history_index,
                    &mut saved_input,
                    &mut line,
                    &mut cursor_pos,
                );
                preferred_column = None;
            }
            Event::Key(key) if is_history_next_key(&key) => {
                browse_history_next(
                    &history,
                    &mut history_index,
                    &saved_input,
                    &mut line,
                    &mut cursor_pos,
                );
                preferred_column = None;
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Backspace => {
                if cursor_pos > 0 {
                    replace_char_range(&mut line, cursor_pos - 1, cursor_pos, "");
                    cursor_pos -= 1;
                    preferred_column = None;
                    history_index = None;
                }
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Delete => {
                if cursor_pos < line.chars().count() {
                    replace_char_range(&mut line, cursor_pos, cursor_pos + 1, "");
                    preferred_column = None;
                    history_index = None;
                }
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Left => {
                cursor_pos = cursor_pos.saturating_sub(1);
                preferred_column = None;
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Right => {
                if cursor_pos < line.chars().count() {
                    cursor_pos += 1;
                }
                preferred_column = None;
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Home => {
                cursor_pos = move_cursor_home(&line, cursor_pos, frame.inner_width);
                preferred_column = None;
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::End => {
                cursor_pos = move_cursor_end(&line, cursor_pos, frame.inner_width);
                preferred_column = None;
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Tab => {
                if let Some(factory) = completion_factory.as_ref() {
                    if let Some(completion) = factory(&line, cursor_pos) {
                        line = completion.line;
                        cursor_pos = completion.cursor_pos.min(line.chars().count());
                        preferred_column = None;
                        history_index = None;
                    }
                }
            }
            Event::Key(key) if is_vertical_cursor_prev_key(&key) => {
                cursor_pos = move_cursor_vertically(
                    &line,
                    cursor_pos,
                    frame.inner_width,
                    -1,
                    &mut preferred_column,
                );
            }
            Event::Key(key) if is_vertical_cursor_next_key(&key) => {
                cursor_pos = move_cursor_vertically(
                    &line,
                    cursor_pos,
                    frame.inner_width,
                    1,
                    &mut preferred_column,
                );
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && matches!(key.code, KeyCode::Char(_))
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                let ch = match key.code {
                    KeyCode::Char(ch) => ch,
                    _ => {
                        debug_assert!(false, "character input branch received a non-char key");
                        continue;
                    }
                };
                insert_char_at_cursor(&mut line, cursor_pos, ch);
                cursor_pos += 1;
                preferred_column = None;
                history_index = None;
            }
            _ => {}
        }

        // Determine if we need a full redraw or just cursor update
        let needs_redraw = !matches!(ev, Event::Key(key) if is_cursor_only_key(&key));

        if needs_redraw {
            frame = frame_factory(&line, cursor_pos);
            render_state = Some(render_prompt_frame(
                &mut stdout,
                &frame,
                &line,
                cursor_pos,
                render_state.as_ref(),
            )?);
        } else if let Some(state) = render_state.as_mut() {
            let viewport =
                wrapped_viewport(&line, cursor_pos, frame.inner_width, frame.max_visible_rows);
            let row_delta = state.cursor_row_in_view as i16 - viewport.cursor_row as i16;
            if row_delta > 0 {
                execute!(stdout, cursor::MoveUp(row_delta as u16))?;
            } else if row_delta < 0 {
                execute!(stdout, cursor::MoveDown((-row_delta) as u16))?;
            }
            execute!(
                stdout,
                cursor::MoveToColumn(frame.input_prefix_width + viewport.cursor_col as u16)
            )?;
            state.cursor_row_in_view = viewport.cursor_row;
            stdout.flush()?;
        }
    }
}

/// Read a single line from the terminal with editing and history support.
///
/// If the terminal is not a TTY, falls back to plain `stdin.read_line()`.
pub fn read_prompt_line(
    frame: &PromptFrame,
    history: &PromptHistory,
    style: &CliStyle,
) -> io::Result<PromptResult> {
    if !style.color {
        return read_plain_line(&frame.plain_prompt);
    }

    read_raw_line(frame, history)
}

/// Read a single inline prompt line with raw editing and history support.
///
/// Unlike [`read_prompt_line`], this keeps the interaction in a native
/// single-line CLI form instead of rendering the boxed prompt chrome.
pub fn read_inline_prompt_line(
    prompt_str: &str,
    history: &PromptHistory,
    style: &CliStyle,
) -> io::Result<PromptResult> {
    if !style.color {
        return read_plain_line(prompt_str);
    }

    let max_prompt_width = usize::from(style.width).saturating_sub(8).clamp(4, 32);
    let visible_prompt = if prompt_str.chars().count() > max_prompt_width {
        let mut truncated = prompt_str
            .chars()
            .take(max_prompt_width.saturating_sub(1))
            .collect::<String>();
        truncated.push(' ');
        truncated
    } else {
        prompt_str.to_string()
    };
    let prompt_width = visible_prompt.chars().count();
    let content_width = usize::from(style.width).saturating_sub(prompt_width).max(8);
    read_raw_inline_line(&visible_prompt, prompt_width, content_width, history)
}

fn read_plain_line(prompt_str: &str) -> io::Result<PromptResult> {
    print!("{}", prompt_str);
    io::stdout().flush()?;

    let mut input = String::new();
    let bytes = io::stdin().read_line(&mut input)?;
    if bytes == 0 {
        return Ok(PromptResult::Eof);
    }
    Ok(PromptResult::Line(
        input
            .trim_end_matches('\n')
            .trim_end_matches('\r')
            .to_string(),
    ))
}

fn read_raw_line(frame: &PromptFrame, history: &PromptHistory) -> io::Result<PromptResult> {
    let mut line = String::new();
    let mut cursor_pos = 0usize;
    let mut preferred_column: Option<usize> = None;
    let mut history_index: Option<usize> = None;
    let mut saved_input = String::new();
    let mut stdout = io::stdout();

    terminal::enable_raw_mode()?;
    let mut render_state = render_prompt_frame(&mut stdout, frame, &line, cursor_pos, None)?;

    let result = loop {
        let ev = event::read()?;
        match ev {
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Enter
                    && (key.modifiers.contains(KeyModifiers::SHIFT)
                        || key.modifiers.contains(KeyModifiers::ALT)) =>
            {
                insert_char_at_cursor(&mut line, cursor_pos, '\n');
                cursor_pos += 1;
                preferred_column = None;
                history_index = None;
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('j')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                insert_char_at_cursor(&mut line, cursor_pos, '\n');
                cursor_pos += 1;
                preferred_column = None;
                history_index = None;
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Enter => {
                dismiss_prompt(&mut stdout, &render_state)?;
                break PromptResult::Line(line);
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                dismiss_prompt(&mut stdout, &render_state)?;
                break PromptResult::Interrupt;
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('d')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if line.is_empty() {
                    dismiss_prompt(&mut stdout, &render_state)?;
                    break PromptResult::Eof;
                }
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('u')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                line.clear();
                cursor_pos = 0;
                preferred_column = None;
                history_index = None;
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('w')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if cursor_pos > 0 {
                    let chars: Vec<char> = line.chars().collect();
                    let mut new_pos = cursor_pos;
                    while new_pos > 0 && chars[new_pos - 1].is_whitespace() {
                        new_pos -= 1;
                    }
                    while new_pos > 0 && !chars[new_pos - 1].is_whitespace() {
                        new_pos -= 1;
                    }
                    replace_char_range(&mut line, new_pos, cursor_pos, "");
                    cursor_pos = new_pos;
                    preferred_column = None;
                    history_index = None;
                }
            }
            Event::Key(key) if is_history_prev_key(&key) => {
                browse_history_prev(
                    history,
                    &mut history_index,
                    &mut saved_input,
                    &mut line,
                    &mut cursor_pos,
                );
                preferred_column = None;
            }
            Event::Key(key) if is_history_next_key(&key) => {
                browse_history_next(
                    history,
                    &mut history_index,
                    &saved_input,
                    &mut line,
                    &mut cursor_pos,
                );
                preferred_column = None;
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Backspace => {
                if cursor_pos > 0 {
                    replace_char_range(&mut line, cursor_pos - 1, cursor_pos, "");
                    cursor_pos -= 1;
                    preferred_column = None;
                    history_index = None;
                }
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Delete => {
                if cursor_pos < line.chars().count() {
                    replace_char_range(&mut line, cursor_pos, cursor_pos + 1, "");
                    preferred_column = None;
                    history_index = None;
                }
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Left => {
                cursor_pos = cursor_pos.saturating_sub(1);
                preferred_column = None;
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Right => {
                if cursor_pos < line.chars().count() {
                    cursor_pos += 1;
                }
                preferred_column = None;
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Home => {
                cursor_pos = move_cursor_home(&line, cursor_pos, frame.inner_width);
                preferred_column = None;
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::End => {
                cursor_pos = move_cursor_end(&line, cursor_pos, frame.inner_width);
                preferred_column = None;
            }
            Event::Key(key) if is_vertical_cursor_prev_key(&key) => {
                cursor_pos = move_cursor_vertically(
                    &line,
                    cursor_pos,
                    frame.inner_width,
                    -1,
                    &mut preferred_column,
                );
            }
            Event::Key(key) if is_vertical_cursor_next_key(&key) => {
                cursor_pos = move_cursor_vertically(
                    &line,
                    cursor_pos,
                    frame.inner_width,
                    1,
                    &mut preferred_column,
                );
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && matches!(key.code, KeyCode::Char(_))
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                let ch = match key.code {
                    KeyCode::Char(ch) => ch,
                    _ => {
                        debug_assert!(false, "character input branch received a non-char key");
                        continue;
                    }
                };
                insert_char_at_cursor(&mut line, cursor_pos, ch);
                cursor_pos += 1;
                preferred_column = None;
                history_index = None;
            }
            _ => {}
        }

        render_state =
            render_prompt_frame(&mut stdout, frame, &line, cursor_pos, Some(&render_state))?;
    };

    terminal::disable_raw_mode()?;
    Ok(result)
}

fn read_raw_inline_line(
    prompt_str: &str,
    prompt_width: usize,
    content_width: usize,
    history: &PromptHistory,
) -> io::Result<PromptResult> {
    let mut line = String::new();
    let mut cursor_pos = 0usize;
    let mut history_index: Option<usize> = None;
    let mut saved_input = String::new();
    let mut stdout = io::stdout();

    terminal::enable_raw_mode()?;
    let mut render_state = render_inline_prompt(
        &mut stdout,
        prompt_str,
        prompt_width,
        content_width,
        &line,
        cursor_pos,
        None,
    )?;

    let result = loop {
        let ev = event::read()?;
        match ev {
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('j')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                write!(stdout, "\r\n")?;
                stdout.flush()?;
                break PromptResult::Line(line);
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Enter => {
                write!(stdout, "\r\n")?;
                stdout.flush()?;
                break PromptResult::Line(line);
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                dismiss_inline_prompt(&mut stdout, &render_state)?;
                writeln!(stdout)?;
                stdout.flush()?;
                break PromptResult::Interrupt;
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('d')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if line.is_empty() {
                    dismiss_inline_prompt(&mut stdout, &render_state)?;
                    writeln!(stdout)?;
                    stdout.flush()?;
                    break PromptResult::Eof;
                }
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('u')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                line.clear();
                cursor_pos = 0;
                history_index = None;
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && key.code == KeyCode::Char('w')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if cursor_pos > 0 {
                    let chars: Vec<char> = line.chars().collect();
                    let mut new_pos = cursor_pos;
                    while new_pos > 0 && chars[new_pos - 1].is_whitespace() {
                        new_pos -= 1;
                    }
                    while new_pos > 0 && !chars[new_pos - 1].is_whitespace() {
                        new_pos -= 1;
                    }
                    replace_char_range(&mut line, new_pos, cursor_pos, "");
                    cursor_pos = new_pos;
                    history_index = None;
                }
            }
            Event::Key(key) if is_history_prev_key(&key) => {
                browse_history_prev(
                    history,
                    &mut history_index,
                    &mut saved_input,
                    &mut line,
                    &mut cursor_pos,
                );
            }
            Event::Key(key) if is_history_next_key(&key) => {
                browse_history_next(
                    history,
                    &mut history_index,
                    &saved_input,
                    &mut line,
                    &mut cursor_pos,
                );
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Backspace => {
                if cursor_pos > 0 {
                    replace_char_range(&mut line, cursor_pos - 1, cursor_pos, "");
                    cursor_pos -= 1;
                    history_index = None;
                }
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Delete => {
                if cursor_pos < line.chars().count() {
                    replace_char_range(&mut line, cursor_pos, cursor_pos + 1, "");
                    history_index = None;
                }
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Left => {
                cursor_pos = cursor_pos.saturating_sub(1);
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Right => {
                if cursor_pos < line.chars().count() {
                    cursor_pos += 1;
                }
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::Home => {
                cursor_pos = 0;
            }
            Event::Key(key) if is_primary_key_event(&key) && key.code == KeyCode::End => {
                cursor_pos = line.chars().count();
            }
            Event::Key(key)
                if is_primary_key_event(&key)
                    && matches!(key.code, KeyCode::Char(_))
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                let ch = match key.code {
                    KeyCode::Char(ch) => ch,
                    _ => {
                        debug_assert!(false, "character input branch received a non-char key");
                        continue;
                    }
                };
                insert_char_at_cursor(&mut line, cursor_pos, ch);
                cursor_pos += 1;
                history_index = None;
            }
            _ => {}
        }

        render_state = render_inline_prompt(
            &mut stdout,
            prompt_str,
            prompt_width,
            content_width,
            &line,
            cursor_pos,
            Some(&render_state),
        )?;
    };

    terminal::disable_raw_mode()?;
    Ok(result)
}

fn render_inline_prompt<W: Write>(
    stdout: &mut W,
    prompt_str: &str,
    prompt_width: usize,
    content_width: usize,
    line: &str,
    cursor_pos: usize,
    previous_state: Option<&PromptRenderState>,
) -> io::Result<PromptRenderState> {
    let _ = previous_state;

    let (rows, row_index, cursor_col) = current_wrapped_position(line, cursor_pos, content_width);
    let row = rows
        .get(row_index)
        .map(|row| row.text.as_str())
        .unwrap_or("");
    write!(stdout, "\r\x1b[2K{}{}", prompt_str, row)?;

    execute!(
        stdout,
        cursor::MoveToColumn((prompt_width + cursor_col) as u16)
    )?;
    stdout.flush()?;

    Ok(PromptRenderState {
        cursor_row_in_view: 0,
        screen_rows: 0,
        frame_height: 1,
    })
}

fn dismiss_inline_prompt<W: Write>(stdout: &mut W, state: &PromptRenderState) -> io::Result<()> {
    let _ = state;
    write!(stdout, "\r\x1b[2K")?;
    stdout.flush()?;
    Ok(())
}

fn render_prompt_frame<W: Write>(
    stdout: &mut W,
    frame: &PromptFrame,
    line: &str,
    cursor_pos: usize,
    previous_state: Option<&PromptRenderState>,
) -> io::Result<PromptRenderState> {
    let viewport = wrapped_viewport(line, cursor_pos, frame.inner_width, frame.max_visible_rows);
    let screen_rows = frame.screen_lines.len();
    // Total lines the frame will occupy: screen + prompt header + visible_rows + footer.
    let new_frame_height = screen_rows + viewport.visible_rows.len() + 2;

    if let Some(state) = previous_state {
        // Move cursor from the active input row back to the top of the full frame.
        let lines_up = (state.cursor_row_in_view + state.screen_rows + 1) as u16;
        execute!(
            stdout,
            cursor::MoveToColumn(0),
            cursor::MoveUp(lines_up),
            terminal::Clear(ClearType::FromCursorDown)
        )?;
        // If the new frame is taller than the old one, the additional \r\n's will
        // push the header off-screen (terminal scrolls).  Reserve extra rows below
        // the current cursor to prevent that.
        if new_frame_height > state.frame_height {
            let extra = (new_frame_height - state.frame_height) as u16;
            for _ in 0..extra {
                writeln!(stdout)?;
            }
            execute!(stdout, cursor::MoveUp(extra))?;
        }
    } else {
        // First render: reserve vertical space so the terminal won't need to scroll
        // when we draw. Print (new_frame_height - 1) newlines to push content up,
        // then move back to where the header should go.
        execute!(stdout, cursor::MoveToColumn(0))?;
        let reserve = new_frame_height.saturating_sub(1) as u16;
        if reserve > 0 {
            for _ in 0..reserve {
                writeln!(stdout)?;
            }
            execute!(stdout, cursor::MoveUp(reserve))?;
        }
    }

    for (index, row) in frame.screen_lines.iter().enumerate() {
        if index > 0 {
            write!(stdout, "\r\n")?;
        }
        write!(
            stdout,
            "\r{}{}",
            row,
            terminal::Clear(ClearType::UntilNewLine)
        )?;
    }
    write!(
        stdout,
        "{}\r{}{}",
        if frame.screen_lines.is_empty() {
            ""
        } else {
            "\r\n"
        },
        frame.header_line,
        terminal::Clear(ClearType::UntilNewLine),
    )?;
    for row in &viewport.visible_rows {
        write!(
            stdout,
            "\r\n\r{}{}",
            compose_input_row(frame, row),
            terminal::Clear(ClearType::UntilNewLine),
        )?;
    }
    write!(
        stdout,
        "\r\n\r{}{}",
        frame.footer_line,
        terminal::Clear(ClearType::UntilNewLine),
    )?;
    execute!(
        stdout,
        cursor::MoveUp((viewport.visible_rows.len() - viewport.cursor_row) as u16),
        cursor::MoveToColumn(frame.input_prefix_width + viewport.cursor_col as u16)
    )?;
    stdout.flush()?;

    Ok(PromptRenderState {
        cursor_row_in_view: viewport.cursor_row,
        screen_rows,
        frame_height: new_frame_height,
    })
}

fn dismiss_prompt<W: Write>(stdout: &mut W, state: &PromptRenderState) -> io::Result<()> {
    execute!(
        stdout,
        cursor::MoveToColumn(0),
        cursor::MoveUp((state.cursor_row_in_view + state.screen_rows + 1) as u16),
        terminal::Clear(ClearType::FromCursorDown),
        cursor::MoveToColumn(0)
    )?;
    stdout.flush()?;
    Ok(())
}

fn is_primary_key_event(key: &KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
}

fn is_history_prev_key(key: &KeyEvent) -> bool {
    is_primary_key_event(key)
        && ((key.code == KeyCode::Up && key.modifiers == KeyModifiers::NONE)
            || (key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL)))
}

fn is_history_next_key(key: &KeyEvent) -> bool {
    is_primary_key_event(key)
        && ((key.code == KeyCode::Down && key.modifiers == KeyModifiers::NONE)
            || (key.code == KeyCode::Char('n') && key.modifiers.contains(KeyModifiers::CONTROL)))
}

fn is_vertical_cursor_prev_key(key: &KeyEvent) -> bool {
    is_primary_key_event(key) && key.code == KeyCode::Up && key.modifiers == KeyModifiers::ALT
}

fn is_vertical_cursor_next_key(key: &KeyEvent) -> bool {
    is_primary_key_event(key) && key.code == KeyCode::Down && key.modifiers == KeyModifiers::ALT
}

fn is_cursor_only_key(key: &KeyEvent) -> bool {
    if !is_primary_key_event(key) {
        return false;
    }

    match key.code {
        KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => {
            key.modifiers == KeyModifiers::NONE
        }
        KeyCode::Up => is_vertical_cursor_prev_key(key),
        KeyCode::Down => is_vertical_cursor_next_key(key),
        _ => false,
    }
}

fn compose_input_row(frame: &PromptFrame, visible_line: &str) -> String {
    let content = pad_right(visible_line, frame.inner_width, ' ');
    if frame.color {
        format!("{} {} {}", "\x1b[36m│\x1b[0m", content, "\x1b[36m│\x1b[0m")
    } else {
        format!("│ {} │", content)
    }
}

fn wrapped_rows(text: &str, width: usize) -> Vec<WrappedRow> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let row_width = width.max(1);
    let mut rows = Vec::new();
    let mut row_start = 0usize;
    let mut row_display_width = 0usize;
    let mut index = 0usize;

    while index < len {
        if chars[index] == '\n' {
            rows.push(make_wrapped_row(&chars, row_start, index));
            row_start = index + 1;
            row_display_width = 0;
            index += 1;
            continue;
        }

        let ch_width = char_display_width(chars[index]);
        if row_display_width > 0 && row_display_width + ch_width > row_width {
            rows.push(make_wrapped_row(&chars, row_start, index));
            row_start = index;
            row_display_width = 0;
            continue;
        }

        row_display_width += ch_width;
        index += 1;
    }

    rows.push(make_wrapped_row(&chars, row_start, len));
    if rows.is_empty() {
        rows.push(make_wrapped_row(&chars, 0, 0));
    }
    rows
}

fn make_wrapped_row(chars: &[char], start: usize, end: usize) -> WrappedRow {
    WrappedRow {
        start,
        end,
        text: chars[start.min(chars.len())..end.min(chars.len())]
            .iter()
            .collect(),
    }
}

fn current_wrapped_position(
    text: &str,
    cursor_pos: usize,
    width: usize,
) -> (Vec<WrappedRow>, usize, usize) {
    let rows = wrapped_rows(text, width);
    let clamped_cursor = cursor_pos.min(text.chars().count());
    let row_index = rows
        .iter()
        .rposition(|row| row.start <= clamped_cursor)
        .unwrap_or(0);
    let row = &rows[row_index];
    let col = display_width_between(text, row.start, clamped_cursor.min(row.end));
    (rows, row_index, col)
}

fn wrapped_viewport(
    text: &str,
    cursor_pos: usize,
    width: usize,
    max_visible_rows: usize,
) -> WrappedViewport {
    let (rows, row_index, col) = current_wrapped_position(text, cursor_pos, width);
    let visible_rows_count = rows.len().min(max_visible_rows.max(1));
    let visible_start_row = row_index
        .saturating_add(1)
        .saturating_sub(visible_rows_count);
    let visible_end_row = visible_start_row + visible_rows_count;
    let visible_rows = rows[visible_start_row..visible_end_row]
        .iter()
        .map(|row| row.text.clone())
        .collect::<Vec<_>>();

    WrappedViewport {
        visible_rows,
        cursor_row: row_index.saturating_sub(visible_start_row),
        cursor_col: col,
    }
}

fn move_cursor_vertically(
    text: &str,
    cursor_pos: usize,
    width: usize,
    delta_rows: isize,
    preferred_column: &mut Option<usize>,
) -> usize {
    let (rows, row_index, current_col) = current_wrapped_position(text, cursor_pos, width);
    let preferred = preferred_column.get_or_insert(current_col);
    let target_row = if delta_rows < 0 {
        row_index.saturating_sub(delta_rows.unsigned_abs())
    } else {
        row_index
            .saturating_add(delta_rows as usize)
            .min(rows.len().saturating_sub(1))
    };
    let row = &rows[target_row];
    row_char_index_for_display_column(text, row.start, row.end, *preferred)
}

fn move_cursor_home(text: &str, cursor_pos: usize, width: usize) -> usize {
    let (rows, row_index, _) = current_wrapped_position(text, cursor_pos, width);
    rows[row_index].start
}

fn move_cursor_end(text: &str, cursor_pos: usize, width: usize) -> usize {
    let (rows, row_index, _) = current_wrapped_position(text, cursor_pos, width);
    rows[row_index].end
}

fn browse_history_prev(
    history: &PromptHistory,
    history_index: &mut Option<usize>,
    saved_input: &mut String,
    line: &mut String,
    cursor_pos: &mut usize,
) {
    if history.is_empty() {
        return;
    }
    match *history_index {
        None => {
            *saved_input = line.clone();
            let idx = history.len() - 1;
            *history_index = Some(idx);
            *line = history.get(idx).unwrap_or_default().to_string();
        }
        Some(idx) if idx > 0 => {
            let new_idx = idx - 1;
            *history_index = Some(new_idx);
            *line = history.get(new_idx).unwrap_or_default().to_string();
        }
        _ => {}
    }
    *cursor_pos = line.chars().count();
}

fn browse_history_next(
    history: &PromptHistory,
    history_index: &mut Option<usize>,
    saved_input: &str,
    line: &mut String,
    cursor_pos: &mut usize,
) {
    if let Some(idx) = *history_index {
        if idx + 1 < history.len() {
            let new_idx = idx + 1;
            *history_index = Some(new_idx);
            *line = history.get(new_idx).unwrap_or_default().to_string();
        } else {
            *history_index = None;
            *line = saved_input.to_string();
        }
        *cursor_pos = line.chars().count();
    }
}

fn insert_char_at_cursor(text: &mut String, cursor_pos: usize, ch: char) {
    let byte_pos = char_index_to_byte_offset(text, cursor_pos);
    text.insert(byte_pos, ch);
}

fn replace_char_range(text: &mut String, start: usize, end: usize, replacement: &str) {
    let byte_start = char_index_to_byte_offset(text, start);
    let byte_end = char_index_to_byte_offset(text, end);
    text.replace_range(byte_start..byte_end, replacement);
}

fn char_index_to_byte_offset(text: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }
    text.char_indices()
        .nth(char_index)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len())
}

fn prompt_max_visible_rows() -> usize {
    match terminal::size() {
        Ok((_, rows)) => usize::from(rows.saturating_sub(10)).clamp(3, 12),
        Err(_) => 6,
    }
}

fn truncate_visible(text: &str, max_width: usize) -> String {
    truncate_display(text, max_width)
}

fn pad_right(text: &str, width: usize, fill: char) -> String {
    pad_right_display(text, width, fill)
}

fn bullet_separator(style: &CliStyle) -> String {
    if style.color {
        "  •  ".to_string()
    } else {
        " | ".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_push_deduplicates() {
        let mut history = PromptHistory::new(100);
        history.push("hello");
        history.push("world");
        history.push("hello");

        assert_eq!(history.len(), 2);
        assert_eq!(history.get(0), Some("world"));
        assert_eq!(history.get(1), Some("hello"));
    }

    #[test]
    fn history_push_trims_whitespace() {
        let mut history = PromptHistory::new(100);
        history.push("  hello  ");
        assert_eq!(history.get(0), Some("hello"));
    }

    #[test]
    fn history_push_ignores_empty() {
        let mut history = PromptHistory::new(100);
        history.push("");
        history.push("   ");
        assert!(history.is_empty());
    }

    #[test]
    fn history_respects_max_size() {
        let mut history = PromptHistory::new(3);
        history.push("a");
        history.push("b");
        history.push("c");
        history.push("d");

        assert_eq!(history.len(), 3);
        assert_eq!(history.get(0), Some("b"));
        assert_eq!(history.get(1), Some("c"));
        assert_eq!(history.get(2), Some("d"));
    }

    #[test]
    fn prompt_result_variants() {
        let line = PromptResult::Line("test".to_string());
        let eof = PromptResult::Eof;
        let interrupt = PromptResult::Interrupt;

        assert_eq!(line, PromptResult::Line("test".to_string()));
        assert_eq!(eof, PromptResult::Eof);
        assert_eq!(interrupt, PromptResult::Interrupt);
    }

    #[test]
    fn boxed_prompt_frame_uses_full_terminal_width_budget() {
        let style = CliStyle::plain();
        let frame = PromptFrame::boxed("Preset prometheus", "Model auto", &style);
        assert_eq!(frame.content_width(), 75);
        assert!(frame.header_line.contains("Preset prometheus"));
        assert!(frame.footer_line.contains("Alt+Enter/Ctrl+J newline"));
    }

    #[test]
    fn wrapped_viewport_soft_wraps_and_keeps_cursor_visible() {
        let viewport = wrapped_viewport("abcdefghijklmnopqrstuvwxyz", 25, 10, 4);
        assert_eq!(
            viewport.visible_rows,
            vec!["abcdefghij", "klmnopqrst", "uvwxyz"]
        );
        assert_eq!(viewport.cursor_row, 2);
        assert_eq!(viewport.cursor_col, 5);
    }

    #[test]
    fn wrapped_viewport_respects_explicit_newlines() {
        let viewport = wrapped_viewport("abc\ndef\n\nghi", 8, 10, 6);
        assert_eq!(viewport.visible_rows, vec!["abc", "def", "", "ghi"]);
        assert_eq!(viewport.cursor_row, 2);
        assert_eq!(viewport.cursor_col, 0);
    }

    #[test]
    fn move_cursor_vertically_preserves_preferred_column() {
        let mut preferred = None;
        let text = "abc\ndefgh\nxy";
        let pos = move_cursor_vertically(text, 7, 10, -1, &mut preferred);
        assert_eq!(pos, 3);
        let pos = move_cursor_vertically(text, pos, 10, 1, &mut preferred);
        assert_eq!(pos, 7);
    }

    #[test]
    fn history_navigation_restores_saved_input() {
        let mut history = PromptHistory::new(10);
        history.push("first");
        history.push("second");

        let mut history_index = None;
        let mut saved_input = String::new();
        let mut line = "draft".to_string();
        let mut cursor_pos = line.chars().count();

        browse_history_prev(
            &history,
            &mut history_index,
            &mut saved_input,
            &mut line,
            &mut cursor_pos,
        );
        assert_eq!(history_index, Some(1));
        assert_eq!(saved_input, "draft");
        assert_eq!(line, "second");
        assert_eq!(cursor_pos, 6);

        browse_history_prev(
            &history,
            &mut history_index,
            &mut saved_input,
            &mut line,
            &mut cursor_pos,
        );
        assert_eq!(history_index, Some(0));
        assert_eq!(line, "first");

        browse_history_next(
            &history,
            &mut history_index,
            &saved_input,
            &mut line,
            &mut cursor_pos,
        );
        assert_eq!(history_index, Some(1));
        assert_eq!(line, "second");

        browse_history_next(
            &history,
            &mut history_index,
            &saved_input,
            &mut line,
            &mut cursor_pos,
        );
        assert_eq!(history_index, None);
        assert_eq!(line, "draft");
        assert_eq!(cursor_pos, 5);
    }

    #[test]
    fn move_cursor_home_and_end_use_visual_rows() {
        let text = "abcdefghijXYZ";
        assert_eq!(move_cursor_home(text, 12, 5), 10);
        assert_eq!(move_cursor_end(text, 12, 5), 13);
    }

    #[test]
    fn primary_key_event_only_accepts_press() {
        let press = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        let repeat =
            KeyEvent::new_with_kind(KeyCode::Char('a'), KeyModifiers::NONE, KeyEventKind::Repeat);

        assert!(is_primary_key_event(&press));
        assert!(!is_primary_key_event(&release));
        assert!(!is_primary_key_event(&repeat));
    }

    #[test]
    fn plain_arrows_drive_history_and_alt_arrows_move_cursor() {
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let ctrl_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        let ctrl_n = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL);
        let alt_up = KeyEvent::new(KeyCode::Up, KeyModifiers::ALT);
        let alt_down = KeyEvent::new(KeyCode::Down, KeyModifiers::ALT);

        assert!(is_history_prev_key(&up));
        assert!(is_history_prev_key(&ctrl_p));
        assert!(is_history_next_key(&down));
        assert!(is_history_next_key(&ctrl_n));
        assert!(!is_history_prev_key(&alt_up));
        assert!(!is_history_next_key(&alt_down));

        assert!(is_vertical_cursor_prev_key(&alt_up));
        assert!(is_vertical_cursor_next_key(&alt_down));
        assert!(!is_vertical_cursor_prev_key(&up));
        assert!(!is_vertical_cursor_next_key(&down));
    }

    #[test]
    fn only_alt_vertical_motion_skips_full_redraw() {
        let left = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        let alt_up = KeyEvent::new(KeyCode::Up, KeyModifiers::ALT);
        let alt_down = KeyEvent::new(KeyCode::Down, KeyModifiers::ALT);

        assert!(is_cursor_only_key(&left));
        assert!(is_cursor_only_key(&alt_up));
        assert!(is_cursor_only_key(&alt_down));
        assert!(!is_cursor_only_key(&up));
    }

    #[test]
    fn render_state_tracks_full_frame_height() {
        let frame = PromptFrame {
            plain_prompt: "> ".to_string(),
            header_line: "header".to_string(),
            footer_line: "footer".to_string(),
            screen_lines: Vec::new(),
            input_prefix_width: 2,
            inner_width: 5,
            max_visible_rows: 12,
            color: false,
        };
        let mut output = Vec::new();

        let state = render_prompt_frame(&mut output, &frame, "abcdefghijk", 11, None)
            .expect("prompt frame renders");

        assert_eq!(state.cursor_row_in_view, 2);
        assert_eq!(state.frame_height, 5);
    }

    #[test]
    fn render_state_clears_from_prompt_header_not_above_it() {
        let state = PromptRenderState {
            cursor_row_in_view: 0,
            screen_rows: 0,
            frame_height: 3,
        };

        assert_eq!(state.cursor_row_in_view + 1, 1);
        assert_ne!(
            state.frame_height.saturating_sub(1),
            state.cursor_row_in_view + 1
        );
    }

    #[test]
    fn render_frame_reserves_space_on_first_draw() {
        // First render (previous_state=None) should emit newlines to reserve
        // vertical space, preventing terminal scroll from hiding the header.
        let frame = PromptFrame {
            plain_prompt: "> ".to_string(),
            header_line: "HDR".to_string(),
            footer_line: "FTR".to_string(),
            screen_lines: Vec::new(),
            input_prefix_width: 2,
            inner_width: 5,
            max_visible_rows: 12,
            color: false,
        };
        let mut output = Vec::new();
        let state =
            render_prompt_frame(&mut output, &frame, "hello", 5, None).expect("render succeeds");

        // frame_height = 1 header + 1 content row + 1 footer = 3
        assert_eq!(state.frame_height, 3);

        let text = String::from_utf8_lossy(&output);
        // The first render should contain the header and footer
        assert!(text.contains("HDR"));
        assert!(text.contains("FTR"));
    }

    #[test]
    fn redraw_with_growth_reserves_extra_space() {
        // When the new frame is taller than the old one (text wraps to a new row),
        // extra newlines are emitted before drawing to prevent scroll-induced
        // header duplication.
        let frame = PromptFrame {
            plain_prompt: "> ".to_string(),
            header_line: "HDR".to_string(),
            footer_line: "FTR".to_string(),
            screen_lines: Vec::new(),
            input_prefix_width: 2,
            inner_width: 5,
            max_visible_rows: 12,
            color: false,
        };

        // Simulate previous state: 1 content row → frame_height = 3
        let prev = PromptRenderState {
            cursor_row_in_view: 0,
            screen_rows: 0,
            frame_height: 3,
        };
        let mut output = Vec::new();
        // Now the text wraps to 3 rows → frame_height = 5 (growth of 2)
        let state = render_prompt_frame(&mut output, &frame, "abcdefghijklmno", 15, Some(&prev))
            .expect("render succeeds");

        assert_eq!(state.frame_height, 5);
        assert_eq!(state.cursor_row_in_view, 2);
    }

    #[test]
    fn wrapped_viewport_accounts_for_fullwidth_characters() {
        let viewport = wrapped_viewport("你好 ", 3, 4, 4);
        assert_eq!(viewport.visible_rows, vec!["你好", " "]);
        assert_eq!(viewport.cursor_row, 1);
        assert_eq!(viewport.cursor_col, 1);
    }

    #[test]
    fn render_frame_handles_fullwidth_wrap_growth() {
        let frame = PromptFrame {
            plain_prompt: "> ".to_string(),
            header_line: "HDR".to_string(),
            footer_line: "FTR".to_string(),
            screen_lines: Vec::new(),
            input_prefix_width: 2,
            inner_width: 4,
            max_visible_rows: 12,
            color: false,
        };
        let previous = PromptRenderState {
            cursor_row_in_view: 0,
            screen_rows: 0,
            frame_height: 3,
        };
        let mut output = Vec::new();

        let state = render_prompt_frame(&mut output, &frame, "你好 ", 3, Some(&previous))
            .expect("render succeeds");

        assert_eq!(state.frame_height, 4);
        assert_eq!(state.cursor_row_in_view, 1);
    }

    #[test]
    fn display_padding_uses_terminal_cell_width() {
        assert_eq!(pad_right("你", 4, ' ').as_str(), "你  ");
    }
}
