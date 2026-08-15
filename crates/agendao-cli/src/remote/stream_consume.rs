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

fn remote_emit_transcript(semantic_state: &mut RemoteSemanticRenderState) -> io::Result<()> {
    if !semantic_state.is_terminal {
        return Ok(());
    }
    print!(
        "\x1B[2J\x1B[1;1H{}",
        semantic_state.transcript.rendered_text()
    );
    io::stdout().flush()
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
    // Detected once per stream: `detect()` does an is_terminal check plus a
    // terminal-width ioctl, which is wasteful per SSE event. Trade-off: a
    // terminal resize mid-stream no longer updates the render width.
    let style = CliStyle::detect();
    let dispatch_context = RemoteEventContext {
        client,
        base_url,
        show_thinking: &show_thinking,
        format: &format,
        style: &style,
    };

    loop {
        let Some(chunk) = StreamExt::next(&mut stream).await else {
            break;
        };
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
                let transcript_identity = live_identity.as_ref().filter(|identity| {
                    LiveSemanticConsumer::is_transcript_bearing_kind(&identity.part_kind)
                });
                remote_apply_output_block(
                    semantic_state,
                    &block,
                    transcript_identity.or(live_identity.as_ref()),
                    style,
                    show_thinking.load(Ordering::SeqCst),
                );
                remote_emit_transcript(semantic_state)?;
            }
        }
        FrontendEvent::SessionError { error, .. } => {
            eprintln!("\nError: {error}");
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
