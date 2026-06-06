use agendao_command_render::cli_style::CliStyle;
use agendao_command_render::live_semantic_consumer::LiveSemanticConsumer;
use agendao_command_render::output_blocks::{render_cli_block_rich, OutputBlock};
use agendao_command_render::terminal_presentation::{
    render_terminal_stream_block_semantic, TerminalSemanticStreamRenderState,
    TerminalStreamAccumulator,
};
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
    pub(super) accumulator: TerminalStreamAccumulator,
    pub(super) semantic: TerminalSemanticStreamRenderState,
    pub(super) transcript: CliVisibleTranscript,
    pub(super) is_terminal: bool,
}

impl RemoteSemanticRenderState {
    fn new() -> Self {
        let is_terminal = std::io::stdout().is_terminal();
        Self {
            accumulator: TerminalStreamAccumulator::default(),
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

pub(super) fn remote_apply_output_block(
    semantic_state: &mut RemoteSemanticRenderState,
    block: &OutputBlock,
    live_identity: Option<&agendao_types::LiveMessagePartIdentity>,
    style: &CliStyle,
    show_thinking: bool,
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

    let rendered = render_terminal_stream_block_semantic(
        &mut semantic_state.semantic,
        &semantic_state.accumulator,
        block,
        None,
        style,
        show_thinking,
    );
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

pub(super) async fn consume_remote_sse(
    response: reqwest::Response,
    client: &reqwest::Client,
    base_url: &str,
    session_id: &str,
    format: RunOutputFormat,
    show_thinking: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut current_event: Option<String> = None;
    let mut current_data: Vec<String> = Vec::new();
    let mut semantic_state = RemoteSemanticRenderState::new();

    loop {
        let Some(chunk) = StreamExt::next(&mut stream).await else {
            break;
        };
        let chunk = chunk?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find('\n') {
            let mut line = buffer[..pos].to_string();
            buffer = buffer[pos + 1..].to_string();
            if line.ends_with('\r') {
                line.pop();
            }
            if line.is_empty() {
                let data = current_data.join("\n");
                dispatch_remote_sse_event(
                    client,
                    base_url,
                    &show_thinking,
                    session_id,
                    &format,
                    &mut semantic_state,
                    current_event.take(),
                    data,
                )
                .await?;
                current_data.clear();
                continue;
            }
            if let Some(event) = line.strip_prefix("event:") {
                current_event = Some(event.trim().to_string());
            } else if let Some(data) = line.strip_prefix("data:") {
                current_data.push(data.trim_start().to_string());
            }
        }
    }

    if !current_data.is_empty() {
        dispatch_remote_sse_event(
            client,
            base_url,
            &show_thinking,
            session_id,
            &format,
            &mut semantic_state,
            current_event.take(),
            current_data.join("\n"),
        )
        .await?;
    }

    if !matches!(format, RunOutputFormat::Json) && !semantic_state.is_terminal {
        print!("{}", semantic_state.transcript.rendered_text());
        io::stdout().flush()?;
    }

    Ok(())
}

async fn dispatch_remote_sse_event(
    client: &reqwest::Client,
    base_url: &str,
    show_thinking: &Arc<AtomicBool>,
    session_id: &str,
    format: &RunOutputFormat,
    semantic_state: &mut RemoteSemanticRenderState,
    event_name: Option<String>,
    data: String,
) -> anyhow::Result<()> {
    if data.trim().is_empty() {
        return Ok(());
    }

    let parsed: serde_json::Value =
        serde_json::from_str(&data).unwrap_or_else(|_| serde_json::json!({ "raw": data }));
    let event_type = event_name
        .or_else(|| {
            parsed
                .get("type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "message".to_string());

    if event_type == "config.updated" {
        if let Some(enabled) = refresh_show_thinking_from_context(client, base_url).await {
            show_thinking.store(enabled, Ordering::SeqCst);
        }
    }

    if matches!(format, &RunOutputFormat::Json) {
        let mut output = serde_json::Map::new();
        output.insert(
            "type".to_string(),
            serde_json::Value::String(event_type.clone()),
        );
        output.insert(
            "timestamp".to_string(),
            serde_json::json!(chrono::Utc::now().timestamp_millis()),
        );
        output.insert(
            "sessionID".to_string(),
            serde_json::Value::String(session_id.to_string()),
        );
        match parsed {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    output.insert(k, v);
                }
            }
            other => {
                output.insert("data".to_string(), other);
            }
        }
        println!("{}", serde_json::Value::Object(output));
        return Ok(());
    }

    if event_type == "output_block" {
        let payload = parsed.get("block").unwrap_or(&parsed);
        if let Some(block) = parse_output_block(payload) {
            if matches!(block, OutputBlock::Reasoning(_)) && !show_thinking.load(Ordering::SeqCst) {
                return Ok(());
            }
            let style = CliStyle::detect();
            let block_id = parsed.get("id").and_then(|value| value.as_str());
            semantic_state
                .accumulator
                .apply_output_block(block_id, &block);
            let live_identity: Option<agendao_types::LiveMessagePartIdentity> = parsed
                .get("live_identity")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            let transcript_identity = live_identity.as_ref().filter(|identity| {
                LiveSemanticConsumer::is_transcript_bearing_kind(&identity.part_kind)
            });
            remote_apply_output_block(
                semantic_state,
                &block,
                transcript_identity.or(live_identity.as_ref()),
                &style,
                show_thinking.load(Ordering::SeqCst),
            );
            remote_emit_transcript(semantic_state)?;
        }
        return Ok(());
    }

    if event_type.as_str() == "error" {
        let message = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .or_else(|| parsed.get("message").and_then(|v| v.as_str()))
            .unwrap_or("unknown remote stream error");
        eprintln!("\nError: {}", message);
    }
    Ok(())
}
