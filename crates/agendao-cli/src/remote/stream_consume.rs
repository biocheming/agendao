use agendao_command_render::cli_style::CliStyle;
use agendao_command_render::live_semantic_consumer::LiveSemanticConsumer;
use agendao_command_render::output_blocks::{render_cli_block_rich, OutputBlock};
use agendao_command_render::terminal_presentation::{
    render_terminal_stream_block_semantic, TerminalSemanticStreamRenderState,
};
use agendao_server_core::frontend_events::FrontendEvent;
use futures::StreamExt;
use std::io::IsTerminal;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::cli::RunOutputFormat;

use super::output_block_parse::parse_output_block;
use super::session_attach::refresh_show_thinking_from_context;
use super::transcript::{
    cli_apply_live_slot_update, cli_live_slot_commit_suffix, cli_live_slot_has_visible_content,
    CliVisibleTranscript,
};

pub(super) struct RemoteSemanticRenderState {
    pub(super) semantic: TerminalSemanticStreamRenderState,
    pub(super) transcript: CliVisibleTranscript,
    pub(super) is_terminal: bool,
}

impl RemoteSemanticRenderState {
    fn new() -> Self {
        let is_terminal = std::io::stdout().is_terminal();
        Self {
            semantic: TerminalSemanticStreamRenderState::default(),
            transcript: CliVisibleTranscript::new(is_terminal),
            is_terminal,
        }
    }
}

impl Default for RemoteSemanticRenderState {
    fn default() -> Self {
        Self::new()
    }
}

struct RemoteEventContext<'a> {
    client: &'a reqwest::Client,
    base_url: &'a str,
    show_thinking: &'a Arc<AtomicBool>,
    format: &'a RunOutputFormat,
    style: &'a CliStyle,
}

pub(super) fn remote_apply_output_block(
    semantic_state: &mut RemoteSemanticRenderState,
    block: &OutputBlock,
    live_identity: Option<&agendao_types::LiveMessagePartIdentity>,
    style: &CliStyle,
    _show_thinking: bool,
) {
    if let Some(identity) = live_identity {
        if !semantic_state.is_terminal {
            remote_apply_non_terminal_live_slot_update(
                &mut semantic_state.transcript,
                block,
                identity,
                style,
            );
            return;
        }
        cli_apply_live_slot_update(&mut semantic_state.transcript, block, identity, style);
        return;
    }

    if matches!(block, OutputBlock::Status(_) | OutputBlock::QueueItem(_)) {
        return;
    }

    let rendered =
        render_terminal_stream_block_semantic(&mut semantic_state.semantic, block, None, style);
    semantic_state.transcript.append_committed(&rendered);
}

fn remote_apply_non_terminal_live_slot_update(
    transcript: &mut CliVisibleTranscript,
    block: &OutputBlock,
    live_identity: &agendao_types::LiveMessagePartIdentity,
    style: &CliStyle,
) {
    if !LiveSemanticConsumer::is_transcript_bearing_kind(&live_identity.part_kind) {
        return;
    }

    let slot_key = format!("{}:{}", live_identity.message_id, live_identity.part_key);
    if cli_live_slot_has_visible_content(block) {
        let rendered = render_cli_block_rich(block, style);
        let plain = agendao_util::util::color::strip_ansi(&rendered);
        transcript.upsert_live_slot(&slot_key, rendered, plain);
    }

    if live_identity.phase == agendao_types::LivePartPhase::End {
        let suffix_ansi = cli_live_slot_commit_suffix(live_identity, style);
        let suffix_plain = agendao_util::util::color::strip_ansi(&suffix_ansi);
        transcript.finalize_live_slot(&slot_key, suffix_ansi, suffix_plain);
    }
}

fn remote_emit_transcript(
    semantic_state: &mut RemoteSemanticRenderState,
    style: &CliStyle,
) -> io::Result<()> {
    if !semantic_state.is_terminal {
        return Ok(());
    }
    // Incremental update: erase+reprint only the live-slot tail instead of
    // clearing the screen and reprinting the whole transcript per event.
    // The erase moves up by the row count recorded when the tail was last
    // printed; a width change since then forces a full clear+reprint inside
    // `incremental_screen_update` because reflowed rows make the stored
    // count unreliable.
    print!(
        "{}",
        semantic_state
            .transcript
            .incremental_screen_update(style.width)
    );
    io::stdout().flush()
}

/// How long a non-interactive run waits for an interactive answer
/// (permission/question) before giving up. Matches the server-side
/// permission timeout.
fn user_wait_timeout() -> std::time::Duration {
    std::env::var("AGENDAO_CLI_USER_WAIT_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(300))
}

#[derive(Default)]
struct UserWaitState {
    /// `Some` while a permission/question is pending and unanswered.
    waiting_since: Option<std::time::Instant>,
}

impl UserWaitState {
    fn wait_expired(&self) -> bool {
        self.waiting_since
            .is_some_and(|since| since.elapsed() >= user_wait_timeout())
    }

    fn remaining(&self) -> Option<std::time::Duration> {
        self.waiting_since
            .map(|since| user_wait_timeout().saturating_sub(since.elapsed()))
    }

    fn bail_if_expired(&self) -> anyhow::Result<()> {
        if self.wait_expired() {
            anyhow::bail!(
                "timed out waiting for an interactive answer. \
                 The run needed a permission or question answered outside this \
                 non-interactive session; open the Web UI or `agendao tui` to \
                 answer it and run the prompt again"
            );
        }
        Ok(())
    }
}

pub(super) async fn consume_remote_events(
    response: reqwest::Response,
    client: &reqwest::Client,
    base_url: &str,
    format: RunOutputFormat,
    show_thinking: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut current_data: Vec<String> = Vec::new();
    let mut semantic_state = RemoteSemanticRenderState::new();
    let mut saw_active = false;
    let mut user_wait = UserWaitState::default();
    // Base style detected once per stream (is_terminal + ioctl); only the
    // terminal OutputBlockAppended branch refreshes the width per event via
    // `with_live_width()` so a mid-stream terminal resize re-renders
    // correctly without querying the terminal size for unrelated events.
    let style = CliStyle::detect();
    let dispatch_context = RemoteEventContext {
        client,
        base_url,
        show_thinking: &show_thinking,
        format: &format,
        style: &style,
    };

    loop {
        let chunk = match user_wait.remaining() {
            Some(remaining) => {
                match tokio::time::timeout(remaining, StreamExt::next(&mut stream)).await {
                    Ok(chunk) => chunk,
                    Err(_) => {
                        user_wait.bail_if_expired()?;
                        continue;
                    }
                }
            }
            None => StreamExt::next(&mut stream).await,
        };
        let Some(chunk) = chunk else {
            break;
        };
        user_wait.bail_if_expired()?;
        let chunk = chunk?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find('\n') {
            // Drain the consumed line in place instead of re-copying the
            // whole remaining buffer per line.
            let mut line: String = buffer.drain(..=pos).collect();
            line.pop(); // trailing '\n'
            if line.ends_with('\r') {
                line.pop();
            }
            if line.is_empty() {
                let data = current_data.join("\n");
                if dispatch_remote_event(
                    &dispatch_context,
                    &mut semantic_state,
                    &mut saw_active,
                    &mut user_wait,
                    data,
                )
                .await?
                {
                    finish_remote_output(&mut semantic_state, &format)?;
                    return Ok(());
                }
                current_data.clear();
                continue;
            }
            if let Some(data) = line.strip_prefix("data:") {
                current_data.push(data.trim_start().to_string());
            }
        }
    }

    if !current_data.is_empty() {
        dispatch_remote_event(
            &dispatch_context,
            &mut semantic_state,
            &mut saw_active,
            &mut user_wait,
            current_data.join("\n"),
        )
        .await?;
    }

    finish_remote_output(&mut semantic_state, &format)?;
    anyhow::bail!("Remote event stream closed before the session became idle")
}

fn finish_remote_output(
    semantic_state: &mut RemoteSemanticRenderState,
    format: &RunOutputFormat,
) -> io::Result<()> {
    if !matches!(format, RunOutputFormat::Json) && !semantic_state.is_terminal {
        print!("{}", semantic_state.transcript.rendered_text());
        io::stdout().flush()?;
    }
    Ok(())
}

async fn dispatch_remote_event(
    context: &RemoteEventContext<'_>,
    semantic_state: &mut RemoteSemanticRenderState,
    saw_active: &mut bool,
    user_wait: &mut UserWaitState,
    data: String,
) -> anyhow::Result<bool> {
    let RemoteEventContext {
        client,
        base_url,
        show_thinking,
        format,
        style,
    } = *context;
    if data.trim().is_empty() {
        return Ok(false);
    }

    let event: FrontendEvent = serde_json::from_str(&data)?;
    if matches!(event, FrontendEvent::ConfigUpdated) {
        if let Some(enabled) = refresh_show_thinking_from_context(client, base_url).await {
            show_thinking.store(enabled, Ordering::SeqCst);
        }
    }

    if matches!(format, &RunOutputFormat::Json) {
        println!("{}", serde_json::to_string(&event)?);
    }

    match event {
        FrontendEvent::OutputBlockAppended {
            block: payload,
            live_identity,
            ..
        } if !matches!(format, &RunOutputFormat::Json) => {
            if let Some(block) = parse_output_block(&payload) {
                if matches!(block, OutputBlock::Reasoning(_))
                    && !show_thinking.load(Ordering::SeqCst)
                {
                    return Ok(false);
                }
                // Only terminal transcript rendering needs the live width
                // (markdown, tool previews, and collapse all read
                // `style.width`, and the terminal may have been resized
                // since the last event). One live style is shared by
                // content rendering and redraw so both agree on the width.
                let live_style = style.with_live_width();
                let transcript_identity = live_identity.as_ref().filter(|identity| {
                    LiveSemanticConsumer::is_transcript_bearing_kind(&identity.part_kind)
                });
                remote_apply_output_block(
                    semantic_state,
                    &block,
                    transcript_identity.or(live_identity.as_ref()),
                    &live_style,
                    show_thinking.load(Ordering::SeqCst),
                );
                remote_emit_transcript(semantic_state, &live_style)?;
            }
        }
        FrontendEvent::SessionError { error, .. } => {
            eprintln!("\nError: {error}");
        }
        FrontendEvent::PermissionUpsert { session_id, .. } => {
            if user_wait.waiting_since.is_none() {
                eprintln!(
                    "\nSession {session_id} is waiting for a permission decision. \
                     This non-interactive run cannot answer it; open the Web UI or run \
                     `agendao tui -s {session_id}` to approve or deny. \
                     Waiting up to {}s before giving up…",
                    user_wait_timeout().as_secs()
                );
            }
            user_wait
                .waiting_since
                .get_or_insert(std::time::Instant::now());
        }
        FrontendEvent::QuestionUpsert { session_id, .. } => {
            if user_wait.waiting_since.is_none() {
                eprintln!(
                    "\nSession {session_id} is waiting for an answer to a question. \
                     This non-interactive run cannot answer it; open the Web UI or run \
                     `agendao tui -s {session_id}` to respond. \
                     Waiting up to {}s before giving up…",
                    user_wait_timeout().as_secs()
                );
            }
            user_wait
                .waiting_since
                .get_or_insert(std::time::Instant::now());
        }
        FrontendEvent::PermissionRemoved { .. } | FrontendEvent::QuestionRemoved { .. } => {
            user_wait.waiting_since = None;
        }
        FrontendEvent::SessionRuntimeReplaced { runtime, .. } => {
            if runtime.run_status == agendao_api::SessionRunStatusKind::Idle {
                return Ok(*saw_active);
            }
            *saw_active = true;
        }
        _ => {}
    }
    Ok(false)
}
