use super::{cli_focused_session_id, CliExecutionRuntime, CliFrontendProjection, CliStyle};
use rocode_command::cli_prompt::PromptSession;
use rocode_command::output_blocks::{render_cli_block_rich, OutputBlock};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub(super) struct CliTerminalSurface {
    style: CliStyle,
    frontend_projection: Arc<Mutex<CliFrontendProjection>>,
    prompt_session: Mutex<Option<Arc<PromptSession>>>,
    pub(super) prompt_suspended: AtomicBool,
    busy_flag: Arc<AtomicBool>,
}

impl CliTerminalSurface {
    pub(super) fn new(
        style: CliStyle,
        frontend_projection: Arc<Mutex<CliFrontendProjection>>,
        busy_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            style,
            frontend_projection,
            prompt_session: Mutex::new(None),
            prompt_suspended: AtomicBool::new(false),
            busy_flag,
        }
    }

    pub(super) fn set_prompt_session(&self, prompt_session: Arc<PromptSession>) {
        if let Ok(mut slot) = self.prompt_session.lock() {
            *slot = Some(prompt_session);
        }
    }

    pub(super) fn print_block(&self, block: OutputBlock) -> anyhow::Result<()> {
        self.append_rendered(&render_cli_block_rich(&block, &self.style))?;
        Ok(())
    }

    pub(super) fn print_text(&self, text: &str) -> io::Result<()> {
        self.append_rendered(text)
    }

    pub(super) fn clear_transcript(&self) -> io::Result<()> {
        if let Ok(mut projection) = self.frontend_projection.lock() {
            projection.transcript.clear();
        }
        self.refresh_prompt()
    }

    pub(super) fn replace_transcript(
        &self,
        transcript: super::frontend_state_types::CliVisibleTranscript,
    ) -> io::Result<()> {
        if let Ok(mut projection) = self.frontend_projection.lock() {
            projection.transcript = transcript.clone();
            projection.scroll_offset = 0;
        }

        let prompt = self
            .prompt_session
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned());

        if let Some(prompt_session) = prompt {
            if !self.prompt_suspended.load(Ordering::Relaxed) {
                let _ = prompt_session.suspend();
            }
            let write_result: io::Result<()> = {
                print!("\x1B[2J\x1B[1;1H{}", transcript.rendered_text());
                io::stdout().flush()
            };
            let _ = prompt_session.resume();
            self.prompt_suspended.store(false, Ordering::Relaxed);
            write_result?;
        } else {
            print!("\x1B[2J\x1B[1;1H{}", transcript.rendered_text());
            io::stdout().flush()?;
        }

        self.refresh_prompt()
    }

    /// P3-I: Apply a live slot update and redraw the terminal.
    /// For identity-bearing blocks — updates the slot content in-place, rebuilds
    /// visible output, and triggers a full ANSI redraw.
    pub(super) fn apply_live_slot(
        &self,
        slot_key: &str,
        rendered_ansi: String,
        rendered_plain: String,
    ) -> io::Result<()> {
        if let Ok(mut projection) = self.frontend_projection.lock() {
            projection.transcript.upsert_live_slot(
                slot_key,
                rendered_ansi,
                rendered_plain,
            );
            projection.scroll_offset = 0;
            let snap = projection.transcript.clone();
            drop(projection);
            self.replace_transcript(snap)
        } else {
            Ok(())
        }
    }

    /// P3-I: Commit a live slot to committed (e.g. on message end).
    pub(super) fn commit_live_slot(&self, slot_key: &str) -> io::Result<()> {
        if let Ok(mut projection) = self.frontend_projection.lock() {
            projection.transcript.commit_slot(slot_key);
        }
        Ok(())
    }

    fn append_rendered(&self, rendered: &str) -> io::Result<()> {
        if let Ok(mut projection) = self.frontend_projection.lock() {
            projection.transcript.append_rendered(rendered);
            projection.scroll_offset = 0;
        }
        let prompt = self
            .prompt_session
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned());

        if let Some(prompt_session) = prompt {
            let busy = self.busy_flag.load(Ordering::Relaxed);
            if !self.prompt_suspended.load(Ordering::Relaxed) {
                let _ = prompt_session.suspend();
                self.prompt_suspended.store(true, Ordering::Relaxed);
            }
            let write_result: io::Result<()> = {
                print!("{}", rendered);
                io::stdout().flush()
            };
            if !busy {
                let _ = prompt_session.resume();
                self.prompt_suspended.store(false, Ordering::Relaxed);
            }
            write_result
        } else {
            print!("{}", rendered);
            io::stdout().flush()
        }
    }

    fn refresh_prompt(&self) -> io::Result<()> {
        if let Some(prompt) = self
            .prompt_session
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned())
        {
            prompt.refresh()?;
        }
        Ok(())
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
        Ok(())
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
        surface.print_text(&rendered)
    } else {
        print!("{}", rendered);
        io::stdout().flush()
    }
}

pub(super) fn cli_refresh_prompt(runtime: &CliExecutionRuntime) {
    if let Some(prompt_session) = runtime.prompt_session.as_ref() {
        let _ = prompt_session.refresh();
    }
}
