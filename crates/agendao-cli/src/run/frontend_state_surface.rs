use super::{
    cli_focused_session_id, cli_prompt_aux_line, CliExecutionRuntime, CliFrontendProjection,
    CliPromptChrome, CliStyle,
};
use agendao_command_render::output_blocks::{render_cli_block_rich, OutputBlock};
use agendao_command_runtime::cli_prompt::PromptSession;
use std::io::{self, Write};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MIN_PROMPT_REFRESH_INTERVAL: Duration = Duration::from_millis(33);

pub(super) struct CliTerminalSurface {
    style: CliStyle,
    frontend_projection: Arc<Mutex<CliFrontendProjection>>,
    busy_flag: Mutex<Option<Arc<AtomicBool>>>,
    prompt_chrome: Mutex<Option<Arc<CliPromptChrome>>>,
    prompt_session: Mutex<Option<Arc<PromptSession>>>,
    last_prompt_snapshot: Mutex<Option<CliPromptRefreshSnapshot>>,
    last_prompt_refresh_at: Mutex<Option<Instant>>,
    prompt_refresh_pending: AtomicBool,
    pub(super) prompt_suspended: AtomicBool,
    cursor_hidden: AtomicBool,
    #[cfg(test)]
    emitted_render_count: AtomicUsize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliPromptRefreshSnapshot {
    mode_label: String,
    model_label: String,
    screen_lines: Vec<String>,
    footer_text: String,
}

// P0-3: Output sink hierarchy (documented; structural convergence in P1).
//
// Canonical sink — transcript-bearing output should go through here:
//   append_rendered(text) — updates projection.transcript AND emits to stdout.
//
// Special-purpose paths (exceptions, not alternatives):
//   print_ephemeral_text() — overlay output (lists, status), not transcript.
//   print_block() — routes to prompt aux lane or append_rendered.
//   refresh_prompt() — prompt redraw, never transcript output.
//
// Legacy bypass (still active, marked for convergence):
//   Direct print!() in sse.rs — when terminal_surface is None
//     (pipe/non-interactive). Also does projection.transcript.append_rendered()
//     so the authority is updated, but the output path bypasses the surface.
impl CliTerminalSurface {
    pub(super) fn new(
        style: CliStyle,
        frontend_projection: Arc<Mutex<CliFrontendProjection>>,
    ) -> Self {
        Self {
            style,
            frontend_projection,
            busy_flag: Mutex::new(None),
            prompt_chrome: Mutex::new(None),
            prompt_session: Mutex::new(None),
            last_prompt_snapshot: Mutex::new(None),
            last_prompt_refresh_at: Mutex::new(None),
            prompt_refresh_pending: AtomicBool::new(false),
            prompt_suspended: AtomicBool::new(false),
            cursor_hidden: AtomicBool::new(false),
            #[cfg(test)]
            emitted_render_count: AtomicUsize::new(0),
        }
    }

    pub(super) fn set_busy_flag(&self, busy_flag: Arc<AtomicBool>) {
        if let Ok(mut slot) = self.busy_flag.lock() {
            *slot = Some(busy_flag);
        }
    }

    pub(super) fn set_prompt_chrome(&self, prompt_chrome: Arc<CliPromptChrome>) {
        if let Ok(mut slot) = self.prompt_chrome.lock() {
            *slot = Some(prompt_chrome);
        }
    }

    pub(super) fn set_prompt_session(&self, prompt_session: Arc<PromptSession>) {
        if let Ok(mut slot) = self.prompt_session.lock() {
            *slot = Some(prompt_session);
        }
    }

    pub(super) fn suspend_modal_prompt(&self) -> io::Result<bool> {
        if self.prompt_suspended.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let prompt = self
            .prompt_session
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned());
        if let Some(prompt_session) = prompt {
            // Modal/question interactions must not leave the prompt frame parked
            // in terminal history. Hide it completely, then restore after.
            prompt_session.suspend()?;
            self.prompt_suspended.store(true, Ordering::Relaxed);
            self.invalidate_prompt_snapshot();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(super) fn resume_modal_prompt(&self, suspended_by_surface: bool) -> io::Result<()> {
        if !suspended_by_surface {
            return Ok(());
        }
        if let Some(prompt_session) = self
            .prompt_session
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned())
        {
            prompt_session.resume()?;
        }
        self.prompt_suspended.store(false, Ordering::Relaxed);
        self.invalidate_prompt_snapshot();
        self.refresh_prompt_with_policy(true)
    }

    pub(super) fn print_block(&self, block: OutputBlock) -> anyhow::Result<()> {
        if self
            .prompt_session
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned())
            .is_some()
            && self.apply_prompt_aux_block(&block)?
        {
            self.refresh_prompt()?;
            return Ok(());
        }
        self.append_rendered(&render_cli_block_rich(&block, &self.style))?;
        Ok(())
    }

    pub(super) fn print_ephemeral_text(&self, text: &str) -> io::Result<()> {
        let prompt = self
            .prompt_session
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned());
        if let Some(prompt_session) = prompt {
            if self.prompt_suspended.load(Ordering::Relaxed) {
                print!("{}", text);
                io::stdout().flush()?;
                return Ok(());
            }
            prompt_session.suspend()?;
            self.prompt_suspended.store(true, Ordering::Relaxed);
            self.invalidate_prompt_snapshot();
            print!("{}", text);
            io::stdout().flush()?;
            prompt_session.resume()?;
            self.prompt_suspended.store(false, Ordering::Relaxed);
            self.invalidate_prompt_snapshot();
            self.refresh_prompt_with_policy(true)?;
            Ok(())
        } else {
            print!("{}", text);
            io::stdout().flush()?;
            self.sync_cursor_visibility()
        }
    }

    pub(super) fn print_rendered_stream(&self, rendered: &str) -> io::Result<()> {
        if rendered.is_empty() {
            return Ok(());
        }
        self.append_rendered(rendered)
    }

    pub(super) fn print_rendered_passthrough(&self, rendered: &str) -> io::Result<()> {
        if rendered.is_empty() {
            return Ok(());
        }
        self.emit_rendered(rendered)
    }

    pub(super) fn clear_transcript(&self) -> io::Result<()> {
        if let Ok(mut projection) = self.frontend_projection.lock() {
            projection.transcript.clear();
            projection.prompt_lanes.clear();
        }
        self.refresh_prompt()
    }

    fn append_rendered(&self, rendered: &str) -> io::Result<()> {
        if let Ok(mut projection) = self.frontend_projection.lock() {
            projection.transcript.append_rendered(rendered);
            projection.scroll_offset = 0;
        }
        self.emit_rendered(rendered)
    }

    fn emit_rendered(&self, rendered: &str) -> io::Result<()> {
        #[cfg(test)]
        self.emitted_render_count.fetch_add(1, Ordering::Relaxed);
        let prompt = self
            .prompt_session
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned());
        if let Some(prompt_session) = prompt {
            if self.prompt_suspended.load(Ordering::Relaxed) {
                print!("{}", rendered);
                io::stdout().flush()?;
                return self.sync_cursor_visibility();
            }
            prompt_session.suspend()?;
            self.prompt_suspended.store(true, Ordering::Relaxed);
            self.invalidate_prompt_snapshot();
            print!("{}", rendered);
            io::stdout().flush()?;
            prompt_session.resume()?;
            self.prompt_suspended.store(false, Ordering::Relaxed);
            self.invalidate_prompt_snapshot();
            self.refresh_prompt_with_policy(true)
        } else {
            print!("{}", rendered);
            io::stdout().flush()?;
            self.sync_cursor_visibility()
        }
    }

    pub(super) fn refresh_prompt(&self) -> io::Result<()> {
        self.refresh_prompt_with_policy(false)
    }

    fn refresh_prompt_with_policy(&self, force: bool) -> io::Result<()> {
        let prompt_chrome = self
            .prompt_chrome
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned());
        let next_snapshot = self
            .frontend_projection
            .lock()
            .ok()
            .map(|projection| prompt_refresh_snapshot(&projection, prompt_chrome.as_deref()));
        if let Some(next_snapshot) = next_snapshot {
            if let Ok(mut cached) = self.last_prompt_snapshot.lock() {
                if !force && cached.as_ref() == Some(&next_snapshot) {
                    return self.sync_cursor_visibility();
                }
                if !force
                    && self
                        .prompt_session
                        .lock()
                        .ok()
                        .and_then(|slot| slot.as_ref().cloned())
                        .is_some()
                {
                    let now = Instant::now();
                    if let Ok(mut last_refresh_at) = self.last_prompt_refresh_at.lock() {
                        if last_refresh_at.as_ref().is_some_and(|last| {
                            now.duration_since(*last) < MIN_PROMPT_REFRESH_INTERVAL
                        }) {
                            self.prompt_refresh_pending.store(true, Ordering::Relaxed);
                            return self.sync_cursor_visibility();
                        }
                        *last_refresh_at = Some(now);
                    }
                }
                *cached = Some(next_snapshot);
            }
        }
        if let Some(prompt) = self
            .prompt_session
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned())
        {
            prompt.refresh()?;
            if let Ok(mut last_refresh_at) = self.last_prompt_refresh_at.lock() {
                *last_refresh_at = Some(Instant::now());
            }
            self.prompt_refresh_pending.store(false, Ordering::Relaxed);
        }
        self.sync_cursor_visibility()?;
        Ok(())
    }

    fn invalidate_prompt_snapshot(&self) {
        if let Ok(mut cached) = self.last_prompt_snapshot.lock() {
            *cached = None;
        }
        if let Ok(mut last_refresh_at) = self.last_prompt_refresh_at.lock() {
            *last_refresh_at = None;
        }
        self.prompt_refresh_pending.store(false, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(super) fn emitted_render_count(&self) -> usize {
        self.emitted_render_count.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(super) fn has_prompt_snapshot(&self) -> bool {
        self.last_prompt_snapshot
            .lock()
            .ok()
            .and_then(|snapshot| snapshot.as_ref().cloned())
            .is_some()
    }

    #[cfg(test)]
    pub(super) fn is_cursor_hidden(&self) -> bool {
        self.cursor_hidden.load(Ordering::Relaxed)
    }

    fn apply_prompt_aux_block(&self, block: &OutputBlock) -> io::Result<bool> {
        let Some(line) = cli_prompt_aux_line(block) else {
            return Ok(false);
        };
        if let Ok(mut projection) = self.frontend_projection.lock() {
            projection.prompt_lanes.push_aux_line(line.lane, &line.text);
        }
        Ok(true)
    }

    pub(super) fn ensure_prompt_visible(&self) -> io::Result<()> {
        if self.prompt_suspended.swap(false, Ordering::Relaxed) {
            if let Some(prompt_session) = self
                .prompt_session
                .lock()
                .ok()
                .and_then(|slot| slot.as_ref().cloned())
            {
                let _ = prompt_session.resume();
            }
        }
        if self.prompt_refresh_pending.load(Ordering::Relaxed) {
            self.refresh_prompt_with_policy(true)?;
        }
        self.sync_cursor_visibility()?;
        Ok(())
    }

    fn sync_cursor_visibility(&self) -> io::Result<()> {
        let should_hide = self
            .busy_flag
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned())
            .is_some_and(|busy_flag| busy_flag.load(Ordering::SeqCst));
        let was_hidden = self.cursor_hidden.load(Ordering::Relaxed);
        if should_hide == was_hidden {
            return Ok(());
        }
        print!(
            "{}",
            if should_hide {
                "\x1b[?25l"
            } else {
                "\x1b[?25h"
            }
        );
        io::stdout().flush()?;
        self.cursor_hidden.store(should_hide, Ordering::Relaxed);
        Ok(())
    }
}

impl Drop for CliTerminalSurface {
    fn drop(&mut self) {
        if self.cursor_hidden.load(Ordering::Relaxed) {
            print!("\x1b[?25h");
            let _ = io::stdout().flush();
            self.cursor_hidden.store(false, Ordering::Relaxed);
        }
    }
}

fn prompt_refresh_snapshot(
    projection: &CliFrontendProjection,
    prompt_chrome: Option<&CliPromptChrome>,
) -> CliPromptRefreshSnapshot {
    let (mode_label, model_label) = prompt_chrome
        .map(CliPromptChrome::snapshot_labels)
        .unwrap_or_else(|| ("Agent build".to_string(), "Model auto".to_string()));
    CliPromptRefreshSnapshot {
        mode_label,
        model_label,
        screen_lines: super::frontend_state_prompt::cli_prompt_lane_screen_lines_from_projection(
            projection,
        ),
        footer_text: projection.footer_text(),
    }
}

pub(super) fn cli_copy_target_transcript(runtime: &CliExecutionRuntime) -> Option<String> {
    if let Some(focused_session_id) = cli_focused_session_id(runtime) {
        return runtime
            .attached_session_transcripts
            .lock()
            .ok()
            .and_then(|transcripts| {
                transcripts
                    .get(&focused_session_id)
                    .map(super::frontend_state_types::CliVisibleTranscript::rendered_text)
            });
    }

    runtime
        .root_session_transcript
        .lock()
        .ok()
        .map(|transcript| transcript.rendered_text())
}

fn render_cli_list(
    title: &str,
    footer: Option<&str>,
    lines: &[String],
    style: &CliStyle,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "\r\n  {} {}\r\n",
        style.bold_cyan(style.bullet()),
        style.bold(title),
    ));
    if lines.is_empty() {
        out.push_str(&format!("    {}\r\n", style.dim("(none)")));
    } else {
        for line in lines {
            out.push_str(&format!("    {}\r\n", line));
        }
    }
    if let Some(footer) = footer {
        out.push_str(&format!("    {}\r\n", style.dim(footer)));
    }
    out.push_str("\r\n");
    out
}

pub(super) fn print_cli_list_on_surface(
    runtime: Option<&CliExecutionRuntime>,
    title: &str,
    footer: Option<&str>,
    lines: &[String],
    style: &CliStyle,
) -> io::Result<()> {
    let rendered = render_cli_list(title, footer, lines, style);
    if let Some(surface) = runtime.and_then(|runtime| runtime.terminal_surface.as_ref()) {
        surface.print_ephemeral_text(&rendered)
    } else {
        print!("{}", rendered);
        io::stdout().flush()
    }
}

pub(super) fn cli_refresh_prompt(runtime: &CliExecutionRuntime) {
    if let Some(surface) = runtime.terminal_surface.as_ref() {
        let _ = surface.refresh_prompt();
    } else if let Some(prompt_session) = runtime.prompt_session.as_ref() {
        let _ = prompt_session.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::frontend_state_prompt::CliPromptChrome;

    #[test]
    fn prompt_refresh_snapshot_changes_only_when_visible_prompt_surface_changes() {
        let mut projection = CliFrontendProjection::default();
        let before = prompt_refresh_snapshot(&projection, None);
        let repeated = prompt_refresh_snapshot(&projection, None);

        assert_eq!(before, repeated);

        projection.run_tail = Some(super::super::frontend_state_types::CliRunTailState {
            status: "running".to_string(),
            detail: Some("Current stage: Research".to_string()),
        });

        let after = prompt_refresh_snapshot(&projection, None);

        assert_ne!(before, after);
    }

    #[test]
    fn prompt_refresh_snapshot_includes_prompt_header_labels() {
        let projection = Arc::new(Mutex::new(CliFrontendProjection::default()));
        let style = CliStyle::plain();
        let first_chrome =
            CliPromptChrome::from_labels("Agent build", "Model a", projection.clone(), &style);
        let second_chrome =
            CliPromptChrome::from_labels("Preset review", "Model b", projection.clone(), &style);

        let first =
            prompt_refresh_snapshot(&projection.lock().expect("projection"), Some(&first_chrome));
        let second = prompt_refresh_snapshot(
            &projection.lock().expect("projection"),
            Some(&second_chrome),
        );

        assert_ne!(first, second);
        assert_eq!(first.mode_label, "Agent build");
        assert_eq!(second.model_label, "Model b");
    }

    #[test]
    fn prompt_aux_status_updates_lane_without_touching_transcript() {
        let projection = Arc::new(Mutex::new(CliFrontendProjection::default()));
        let surface = CliTerminalSurface::new(CliStyle::plain(), projection.clone());

        let applied = surface
            .apply_prompt_aux_block(&OutputBlock::Status(
                agendao_command_render::output_blocks::StatusBlock::warning("retry scheduled"),
            ))
            .expect("apply prompt aux");

        assert!(applied, "status block should map to prompt aux lane");

        let locked = projection.lock().expect("projection");
        assert_eq!(
            locked.prompt_lanes.warning_lines,
            vec!["Warning: retry scheduled".to_string()]
        );
        assert_eq!(locked.transcript.rendered_text(), "");
    }

    #[test]
    fn suspend_and_resume_modal_prompt_without_session_keep_surface_ready() {
        let projection = Arc::new(Mutex::new(CliFrontendProjection::default()));
        let surface = CliTerminalSurface::new(CliStyle::plain(), projection);

        let suspended = surface
            .suspend_modal_prompt()
            .expect("suspend modal prompt");
        assert!(!suspended);
        assert!(!surface.prompt_suspended.load(Ordering::Relaxed));

        surface
            .resume_modal_prompt(false)
            .expect("resume modal prompt");
        assert!(!surface.prompt_suspended.load(Ordering::Relaxed));
    }

    #[test]
    fn append_rendered_updates_transcript_even_without_prompt_owned_viewport() {
        let projection = Arc::new(Mutex::new(CliFrontendProjection::default()));
        let surface = CliTerminalSurface::new(CliStyle::plain(), projection.clone());

        surface
            .print_rendered_stream("alpha\nbeta\n")
            .expect("append rendered stream");

        let locked = projection.lock().expect("projection");
        assert_eq!(locked.transcript.rendered_text(), "alpha\nbeta\n");
    }

    #[test]
    fn busy_surface_hides_terminal_cursor_until_idle() {
        let projection = Arc::new(Mutex::new(CliFrontendProjection::default()));
        let surface = CliTerminalSurface::new(CliStyle::plain(), projection);
        let busy_flag = Arc::new(AtomicBool::new(true));
        surface.set_busy_flag(busy_flag.clone());

        surface.refresh_prompt().expect("refresh while busy");
        assert!(surface.is_cursor_hidden());

        busy_flag.store(false, Ordering::SeqCst);
        surface.refresh_prompt().expect("refresh while idle");
        assert!(!surface.is_cursor_hidden());
    }
}
