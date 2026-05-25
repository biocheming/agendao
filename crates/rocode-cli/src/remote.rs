use crate::run::frontend_state_types::CliVisibleTranscript;
use crate::run::{
    cli_apply_live_slot_update, cli_live_slot_commit_suffix, cli_live_slot_has_visible_content,
};
use futures::StreamExt;
use rocode_command::cli_style::CliStyle;
use rocode_command::live_semantic_consumer::LiveSemanticConsumer;
use rocode_command::output_blocks::{
    render_cli_block_rich, BlockTone, MessageBlock, MessagePhase, MessageRole, OutputBlock,
    QueueItemBlock, ReasoningBlock, SchedulerDecisionBlock, SchedulerDecisionField,
    SchedulerDecisionRenderSpec, SchedulerDecisionSection, SchedulerStageBlock, SessionEventBlock,
    SessionEventField, StatusBlock, ToolBlock, ToolPhase,
};
use rocode_command::terminal_presentation::{
    render_terminal_stream_block_semantic, TerminalSemanticStreamRenderState,
    TerminalStreamAccumulator,
};
use rocode_config::schema::ShareMode;
use rocode_runtime_context::ResolvedWorkspaceContext;
use serde::Deserialize;
use std::io::IsTerminal;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::cli::RunOutputFormat;
use crate::util::{parse_bool_env, parse_http_json, server_url};

struct RemoteSemanticRenderState {
    accumulator: TerminalStreamAccumulator,
    semantic: TerminalSemanticStreamRenderState,
    transcript: CliVisibleTranscript,
    is_terminal: bool,
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

fn remote_apply_output_block(
    semantic_state: &mut RemoteSemanticRenderState,
    block: &OutputBlock,
    live_identity: Option<&rocode_types::LiveMessagePartIdentity>,
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
    live_identity: &rocode_types::LiveMessagePartIdentity,
    style: &CliStyle,
) {
    if !LiveSemanticConsumer::is_transcript_bearing_kind(&live_identity.part_kind) {
        return;
    }

    let slot_key = format!("{}:{}", live_identity.message_id, live_identity.part_key);
    if cli_live_slot_has_visible_content(block) {
        let rendered = render_cli_block_rich(block, style);
        let plain = rocode_util::util::color::strip_ansi(&rendered);
        transcript.upsert_live_slot(&slot_key, rendered, plain);
    }

    if live_identity.phase == rocode_types::LivePartPhase::End {
        let suffix_ansi = cli_live_slot_commit_suffix(live_identity, style);
        let suffix_plain = rocode_util::util::color::strip_ansi(&suffix_ansi);
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

#[derive(Debug, Deserialize)]
struct RemoteSessionInfo {
    id: String,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    directory: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteShareInfo {
    url: String,
}

pub(crate) struct RemoteAttachOptions {
    pub base_url: String,
    pub input: String,
    pub command: Option<String>,
    pub continue_last: bool,
    pub session: Option<String>,
    pub fork: bool,
    pub share: bool,
    pub model: Option<String>,
    pub agent: Option<String>,
    pub scheduler_profile: Option<String>,
    pub variant: Option<String>,
    pub format: RunOutputFormat,
    pub title: Option<String>,
    pub directory: Option<String>,
    pub show_thinking: bool,
}

fn remote_show_thinking_from_context(context: &ResolvedWorkspaceContext) -> Option<bool> {
    context
        .config
        .ui_preferences
        .as_ref()
        .and_then(|ui| ui.show_thinking)
}

async fn fetch_remote_workspace_context(
    client: &reqwest::Client,
    base_url: &str,
) -> anyhow::Result<ResolvedWorkspaceContext> {
    let context_endpoint = server_url(base_url, "/workspace/context");
    parse_http_json(client.get(context_endpoint).send().await?).await
}

pub(crate) fn parse_output_block(payload: &serde_json::Value) -> Option<OutputBlock> {
    let kind = payload.get("kind")?.as_str()?;
    match kind {
        "status" => {
            let tone = match payload
                .get("tone")
                .and_then(|v| v.as_str())
                .unwrap_or("normal")
            {
                "title" => BlockTone::Title,
                "muted" => BlockTone::Muted,
                "success" => BlockTone::Success,
                "warning" => BlockTone::Warning,
                "error" => BlockTone::Error,
                _ => BlockTone::Normal,
            };
            let text = payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            Some(OutputBlock::Status(StatusBlock { tone, text }))
        }
        "message" => {
            let role = match payload
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("assistant")
            {
                "user" => MessageRole::User,
                "system" => MessageRole::System,
                _ => MessageRole::Assistant,
            };
            let phase = match payload
                .get("phase")
                .and_then(|v| v.as_str())
                .unwrap_or("delta")
            {
                "start" => MessagePhase::Start,
                "end" => MessagePhase::End,
                "full" => MessagePhase::Full,
                _ => MessagePhase::Delta,
            };
            let text = payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            Some(OutputBlock::Message(MessageBlock { role, phase, text }))
        }
        "tool" => {
            let name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("tool")
                .to_string();
            let phase = match payload
                .get("phase")
                .and_then(|v| v.as_str())
                .unwrap_or("running")
            {
                "start" => ToolPhase::Start,
                "done" | "result" => ToolPhase::Done,
                "error" => ToolPhase::Error,
                _ => ToolPhase::Running,
            };
            let detail = payload
                .get("detail")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Some(OutputBlock::Tool(ToolBlock {
                name,
                phase,
                detail,
                structured: None,
            }))
        }
        "reasoning" => {
            let phase = match payload
                .get("phase")
                .and_then(|v| v.as_str())
                .unwrap_or("delta")
            {
                "start" => MessagePhase::Start,
                "end" => MessagePhase::End,
                "full" => MessagePhase::Full,
                _ => MessagePhase::Delta,
            };
            let text = payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            Some(OutputBlock::Reasoning(ReasoningBlock { phase, text }))
        }
        "session_event" => Some(OutputBlock::SessionEvent(SessionEventBlock {
            event: payload
                .get("event")
                .and_then(|v| v.as_str())
                .unwrap_or("event")
                .to_string(),
            title: payload
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Session Event")
                .to_string(),
            status: payload
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            summary: payload
                .get("summary")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            fields: payload
                .get("fields")
                .and_then(|v| v.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|field| {
                            Some(SessionEventField {
                                label: field.get("label")?.as_str()?.to_string(),
                                value: field.get("value")?.as_str()?.to_string(),
                                tone: field
                                    .get("tone")
                                    .and_then(|value| value.as_str())
                                    .map(str::to_string),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            body: payload
                .get("body")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })),
        "queue_item" => Some(OutputBlock::QueueItem(QueueItemBlock {
            position: payload
                .get("position")
                .and_then(|v| v.as_u64())
                .unwrap_or(1) as usize,
            text: payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        })),
        "scheduler_stage" => Some(OutputBlock::SchedulerStage(Box::new(SchedulerStageBlock {
            stage_id: payload
                .get("stage_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            profile: payload
                .get("profile")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            stage: payload
                .get("stage")
                .and_then(|v| v.as_str())
                .unwrap_or("stage")
                .to_string(),
            title: payload
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Scheduler Stage")
                .to_string(),
            text: payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            stage_index: payload.get("stage_index").and_then(|v| v.as_u64()),
            stage_total: payload.get("stage_total").and_then(|v| v.as_u64()),
            step: payload.get("step").and_then(|v| v.as_u64()),
            status: payload
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            focus: payload
                .get("focus")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            last_event: payload
                .get("last_event")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            waiting_on: payload
                .get("waiting_on")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            estimated_context_tokens: payload
                .get("estimated_context_tokens")
                .and_then(|v| v.as_u64()),
            skill_tree_budget: payload.get("skill_tree_budget").and_then(|v| v.as_u64()),
            skill_tree_truncation_strategy: payload
                .get("skill_tree_truncation_strategy")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            skill_tree_truncated: payload
                .get("skill_tree_truncated")
                .and_then(|v| v.as_bool()),
            retry_attempt: payload.get("retry_attempt").and_then(|v| v.as_u64()),
            activity: payload
                .get("activity")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            loop_budget: payload
                .get("loop_budget")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            available_skill_count: payload
                .get("available_skill_count")
                .and_then(|v| v.as_u64()),
            available_agent_count: payload
                .get("available_agent_count")
                .and_then(|v| v.as_u64()),
            available_category_count: payload
                .get("available_category_count")
                .and_then(|v| v.as_u64()),
            active_skills: payload
                .get("active_skills")
                .and_then(|v| v.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            active_agents: payload
                .get("active_agents")
                .and_then(|v| v.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            active_categories: payload
                .get("active_categories")
                .and_then(|v| v.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            done_agent_count: payload
                .get("done_agent_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            total_agent_count: payload
                .get("total_agent_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            prompt_tokens: payload.get("prompt_tokens").and_then(|v| v.as_u64()),
            context_tokens: payload
                .get("context_tokens")
                .and_then(|v| v.as_u64())
                .or_else(|| payload.get("prompt_tokens").and_then(|v| v.as_u64())),
            completion_tokens: payload.get("completion_tokens").and_then(|v| v.as_u64()),
            reasoning_tokens: payload.get("reasoning_tokens").and_then(|v| v.as_u64()),
            cache_read_tokens: payload.get("cache_read_tokens").and_then(|v| v.as_u64()),
            cache_miss_tokens: payload.get("cache_miss_tokens").and_then(|v| v.as_u64()),
            cache_write_tokens: payload.get("cache_write_tokens").and_then(|v| v.as_u64()),
            decision: parse_scheduler_decision(payload.get("decision")),
            attached_session_id: payload
                .get("attached_session_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        }))),
        _ => None,
    }
}

fn parse_scheduler_decision(payload: Option<&serde_json::Value>) -> Option<SchedulerDecisionBlock> {
    let payload = payload?;
    Some(SchedulerDecisionBlock {
        kind: payload.get("kind")?.as_str()?.to_string(),
        title: payload.get("title")?.as_str()?.to_string(),
        spec: parse_scheduler_decision_spec(payload.get("spec"))?,
        fields: payload
            .get("fields")
            .and_then(|value| value.as_array())
            .map(|fields| {
                fields
                    .iter()
                    .filter_map(|field| {
                        Some(SchedulerDecisionField {
                            label: field.get("label")?.as_str()?.to_string(),
                            value: field.get("value")?.as_str()?.to_string(),
                            tone: field
                                .get("tone")
                                .and_then(|value| value.as_str())
                                .map(|value| value.to_string()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        sections: payload
            .get("sections")
            .and_then(|value| value.as_array())
            .map(|sections| {
                sections
                    .iter()
                    .filter_map(|section| {
                        Some(SchedulerDecisionSection {
                            title: section.get("title")?.as_str()?.to_string(),
                            body: section.get("body")?.as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn parse_scheduler_decision_spec(
    payload: Option<&serde_json::Value>,
) -> Option<SchedulerDecisionRenderSpec> {
    let payload = payload?;
    Some(SchedulerDecisionRenderSpec {
        version: payload.get("version")?.as_str()?.to_string(),
        show_header_divider: payload.get("show_header_divider")?.as_bool()?,
        field_order: payload.get("field_order")?.as_str()?.to_string(),
        field_label_emphasis: payload.get("field_label_emphasis")?.as_str()?.to_string(),
        status_palette: payload.get("status_palette")?.as_str()?.to_string(),
        section_spacing: payload.get("section_spacing")?.as_str()?.to_string(),
        update_policy: payload.get("update_policy")?.as_str()?.to_string(),
    })
}

pub(crate) async fn resolve_remote_session(
    client: &reqwest::Client,
    base_url: &str,
    continue_last: bool,
    session: Option<String>,
    fork: bool,
    title: Option<String>,
    directory: Option<String>,
) -> anyhow::Result<String> {
    let base_id = if let Some(session_id) = session {
        Some(session_id)
    } else if continue_last {
        let list_endpoint = server_url(base_url, "/session?roots=true&limit=100");
        let sessions: Vec<RemoteSessionInfo> =
            parse_http_json(client.get(list_endpoint).send().await?).await?;
        let directory = directory
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        sessions
            .into_iter()
            .find(|s| {
                s.parent_id.is_none()
                    && directory
                        .map(|dir| s.directory.as_deref() == Some(dir))
                        .unwrap_or(true)
            })
            .map(|s| s.id)
    } else {
        None
    };

    if let Some(base_id) = base_id {
        if fork {
            let fork_endpoint = server_url(base_url, &format!("/session/{}/fork", base_id));
            let forked: RemoteSessionInfo = parse_http_json(
                client
                    .post(fork_endpoint)
                    .json(&serde_json::json!({ "message_id": null }))
                    .send()
                    .await?,
            )
            .await?;
            return Ok(forked.id);
        }
        return Ok(base_id);
    }

    let create_endpoint = server_url(base_url, "/session");
    let created: RemoteSessionInfo = parse_http_json(
        client
            .post(create_endpoint)
            .json(&serde_json::json!({
                "title": title,
                "directory": directory
            }))
            .send()
            .await?,
    )
    .await?;
    Ok(created.id)
}

pub(crate) async fn maybe_share_remote_session(
    client: &reqwest::Client,
    base_url: &str,
    session_id: &str,
    share_requested: bool,
) -> anyhow::Result<()> {
    let auto_share_env = std::env::var("ROCODE_AUTO_SHARE")
        .ok()
        .map(|v| parse_bool_env(&v))
        .unwrap_or(false);
    let context = fetch_remote_workspace_context(client, base_url).await?;
    let config_auto = matches!(context.config.share, Some(ShareMode::Auto));

    if !(share_requested || auto_share_env || config_auto) {
        return Ok(());
    }

    let share_endpoint = server_url(base_url, &format!("/session/{}/share", session_id));
    let shared: RemoteShareInfo =
        parse_http_json(client.post(share_endpoint).send().await?).await?;
    println!("~  {}", shared.url);
    Ok(())
}

pub(crate) async fn consume_remote_sse(
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
        if let Ok(context) = fetch_remote_workspace_context(client, base_url).await {
            if let Some(enabled) = remote_show_thinking_from_context(&context) {
                show_thinking.store(enabled, Ordering::SeqCst);
            }
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
            let live_identity: Option<rocode_types::LiveMessagePartIdentity> = parsed
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

pub(crate) async fn run_non_interactive_attach(options: RemoteAttachOptions) -> anyhow::Result<()> {
    let RemoteAttachOptions {
        base_url,
        input,
        command,
        continue_last,
        session,
        fork,
        share,
        model,
        agent,
        scheduler_profile,
        variant,
        format,
        title,
        directory,
        show_thinking,
    } = options;
    let client = reqwest::Client::new();
    let show_thinking = Arc::new(AtomicBool::new(show_thinking));
    let session_id = resolve_remote_session(
        &client,
        &base_url,
        continue_last,
        session,
        fork,
        title,
        directory,
    )
    .await?;
    maybe_share_remote_session(&client, &base_url, &session_id, share).await?;

    let content = if let Some(command_name) = command {
        if input.trim().is_empty() {
            format!("/{}", command_name)
        } else {
            format!("/{} {}", command_name, input)
        }
    } else {
        input
    };

    let endpoint = server_url(&base_url, &format!("/session/{}/stream", session_id));
    let response = client
        .post(endpoint)
        .json(&serde_json::json!({
            "content": content,
            "model": model,
            "agent": agent,
            "scheduler_profile": scheduler_profile,
            "variant": variant,
            "stream": true
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Remote run failed ({}): {}", status, body);
    }

    consume_remote_sse(
        response,
        &client,
        &base_url,
        &session_id,
        format,
        show_thinking,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{parse_output_block, remote_apply_output_block, RemoteSemanticRenderState};
    use crate::run::cli_apply_live_slot_update;
    use crate::run::frontend_state_types::CliVisibleTranscript;
    use rocode_command::cli_style::CliStyle;
    use rocode_command::governance_fixtures::canonical_scheduler_stage_fixture;
    use rocode_command::output_blocks::{
        MessageBlock, MessagePhase, MessageRole, OutputBlock, QueueItemBlock, SchedulerStageBlock,
        StatusBlock,
    };
    use rocode_command::terminal_presentation::render_terminal_stream_block_semantic;

    #[test]
    fn parses_canonical_scheduler_stage_payload() {
        let fixture = canonical_scheduler_stage_fixture();
        let block = parse_output_block(&fixture.payload).expect("scheduler stage block");
        assert_eq!(block, OutputBlock::SchedulerStage(Box::new(fixture.block)));
    }

    #[test]
    fn parses_reasoning_payload() {
        let payload = serde_json::json!({
            "kind": "reasoning",
            "phase": "delta",
            "text": "thinking"
        });
        let block = parse_output_block(&payload).expect("reasoning block");
        assert!(matches!(
            block,
            OutputBlock::Reasoning(reasoning)
                if reasoning.phase == MessagePhase::Delta && reasoning.text == "thinking"
        ));
    }

    #[test]
    fn remote_semantic_render_keeps_single_assistant_header_across_start_delta_end_stream() {
        let style = CliStyle::plain();
        let mut semantic_state = RemoteSemanticRenderState::default();
        let rendered = [
            OutputBlock::Message(MessageBlock::start(MessageRole::Assistant)),
            OutputBlock::Message(MessageBlock::delta(
                MessageRole::Assistant,
                "快速".to_string(),
            )),
            OutputBlock::Message(MessageBlock::delta(
                MessageRole::Assistant,
                "上升".to_string(),
            )),
            OutputBlock::Message(MessageBlock::delta(
                MessageRole::Assistant,
                "期".to_string(),
            )),
            OutputBlock::Message(MessageBlock::end(MessageRole::Assistant)),
        ]
        .into_iter()
        .map(|block| {
            semantic_state
                .accumulator
                .apply_output_block(Some("assistant-1"), &block);
            render_terminal_stream_block_semantic(
                &mut semantic_state.semantic,
                &semantic_state.accumulator,
                &block,
                None,
                &style,
                true,
            )
        })
        .collect::<String>();

        assert_eq!(
            rendered.matches("[message:assistant]").count(),
            1,
            "{rendered}"
        );
        assert_eq!(rendered, "[message:assistant] 快速上升期");
    }

    #[test]
    fn remote_semantic_render_keeps_single_assistant_header_without_explicit_block_id() {
        let style = CliStyle::plain();
        let mut semantic_state = RemoteSemanticRenderState::default();
        let rendered = [
            OutputBlock::Message(MessageBlock::delta(
                MessageRole::Assistant,
                "快速".to_string(),
            )),
            OutputBlock::Message(MessageBlock::delta(
                MessageRole::Assistant,
                "上升".to_string(),
            )),
            OutputBlock::Message(MessageBlock::delta(
                MessageRole::Assistant,
                "期".to_string(),
            )),
        ]
        .into_iter()
        .map(|block| {
            semantic_state.accumulator.apply_output_block(None, &block);
            render_terminal_stream_block_semantic(
                &mut semantic_state.semantic,
                &semantic_state.accumulator,
                &block,
                None,
                &style,
                true,
            )
        })
        .collect::<String>();

        assert_eq!(
            rendered.matches("[message:assistant]").count(),
            1,
            "{rendered}"
        );
        assert_eq!(rendered, "[message:assistant] 快速上升期");
    }

    #[test]
    fn remote_identity_bearing_full_snapshot_replaces_same_slot_without_header_replay() {
        let style = CliStyle::plain();
        let mut transcript = CliVisibleTranscript::new(false);
        let identity = rocode_types::LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: "text/main".to_string(),
            part_kind: rocode_types::LiveMessagePartKind::AssistantText,
            phase: rocode_types::LivePartPhase::Snapshot,
            legacy_block_id: Some("assistant-1".to_string()),
        };

        cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Message(MessageBlock::full(
                MessageRole::Assistant,
                "第一版".to_string(),
            )),
            &identity,
            &style,
        );
        cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Message(MessageBlock::full(
                MessageRole::Assistant,
                "第二版".to_string(),
            )),
            &identity,
            &style,
        );

        let rendered = transcript.rendered_text();
        assert_eq!(
            rendered.matches("[message:assistant]").count(),
            1,
            "{rendered}"
        );
        assert!(rendered.contains("第二版"), "{rendered}");
        assert!(
            !rendered.contains("第一版[message:assistant]"),
            "{rendered}"
        );
        assert!(!rendered.contains("第一版第二版"), "{rendered}");
    }

    #[test]
    fn remote_scheduler_stage_identity_does_not_enter_transcript() {
        let style = CliStyle::plain();
        let mut transcript = CliVisibleTranscript::new(false);
        let identity = rocode_types::LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: "scheduler/stage-1".to_string(),
            part_kind: rocode_types::LiveMessagePartKind::SchedulerStage,
            phase: rocode_types::LivePartPhase::Snapshot,
            legacy_block_id: None,
        };

        cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::SchedulerStage(Box::new(SchedulerStageBlock {
                stage_id: Some("stage-1".to_string()),
                profile: Some("default".to_string()),
                stage: "research".to_string(),
                title: "Research".to_string(),
                text: "planning".to_string(),
                stage_index: Some(1),
                stage_total: Some(3),
                step: Some(1),
                status: Some("running".to_string()),
                focus: None,
                last_event: None,
                waiting_on: None,
                estimated_context_tokens: None,
                skill_tree_budget: None,
                skill_tree_truncation_strategy: None,
                skill_tree_truncated: None,
                retry_attempt: None,
                activity: None,
                loop_budget: None,
                available_skill_count: None,
                available_agent_count: None,
                available_category_count: None,
                active_skills: Vec::new(),
                active_agents: Vec::new(),
                active_categories: Vec::new(),
                done_agent_count: 0,
                total_agent_count: 0,
                prompt_tokens: None,
                context_tokens: None,
                completion_tokens: None,
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_miss_tokens: None,
                cache_write_tokens: None,
                attached_session_id: None,
                decision: None,
            })),
            &identity,
            &style,
        );

        let rendered = transcript.rendered_text();
        assert!(rendered.is_empty(), "{rendered}");
    }

    #[test]
    fn remote_non_terminal_state_rebuilds_from_transcript_authority() {
        let style = CliStyle::plain();
        let mut semantic_state = RemoteSemanticRenderState {
            is_terminal: false,
            ..RemoteSemanticRenderState::default()
        };

        let first = OutputBlock::Message(MessageBlock::delta(
            MessageRole::Assistant,
            "第一版".to_string(),
        ));
        semantic_state
            .accumulator
            .apply_output_block(Some("assistant-1"), &first);
        remote_apply_output_block(&mut semantic_state, &first, None, &style, true);

        let second = OutputBlock::Message(MessageBlock::delta(
            MessageRole::Assistant,
            " 第二段".to_string(),
        ));
        semantic_state
            .accumulator
            .apply_output_block(Some("assistant-1"), &second);
        remote_apply_output_block(&mut semantic_state, &second, None, &style, true);

        assert_eq!(
            semantic_state.transcript.rendered_text(),
            "[message:assistant] 第一版 第二段"
        );
    }

    #[test]
    fn remote_status_blocks_do_not_enter_transcript_authority() {
        let style = CliStyle::plain();
        let mut semantic_state = RemoteSemanticRenderState {
            is_terminal: false,
            ..RemoteSemanticRenderState::default()
        };

        remote_apply_output_block(
            &mut semantic_state,
            &OutputBlock::Status(StatusBlock::warning("retry scheduled")),
            None,
            &style,
            true,
        );

        assert_eq!(semantic_state.transcript.rendered_text(), "");
    }

    #[test]
    fn remote_queue_items_do_not_enter_transcript_authority() {
        let style = CliStyle::plain();
        let mut semantic_state = RemoteSemanticRenderState {
            is_terminal: false,
            ..RemoteSemanticRenderState::default()
        };

        remote_apply_output_block(
            &mut semantic_state,
            &OutputBlock::QueueItem(QueueItemBlock {
                position: 2,
                text: "queued".to_string(),
            }),
            None,
            &style,
            true,
        );

        assert_eq!(semantic_state.transcript.rendered_text(), "");
    }

    #[test]
    fn remote_identity_rewrite_updates_transcript_authority_without_append_replay() {
        let style = CliStyle::plain();
        let mut semantic_state = RemoteSemanticRenderState {
            is_terminal: false,
            ..RemoteSemanticRenderState::default()
        };
        let identity = rocode_types::LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: "text/main".to_string(),
            part_kind: rocode_types::LiveMessagePartKind::AssistantText,
            phase: rocode_types::LivePartPhase::Snapshot,
            legacy_block_id: Some("assistant-1".to_string()),
        };

        remote_apply_output_block(
            &mut semantic_state,
            &OutputBlock::Message(MessageBlock::full(
                MessageRole::Assistant,
                "第一版".to_string(),
            )),
            Some(&identity),
            &style,
            true,
        );

        remote_apply_output_block(
            &mut semantic_state,
            &OutputBlock::Message(MessageBlock::full(
                MessageRole::Assistant,
                "第二版".to_string(),
            )),
            Some(&identity),
            &style,
            true,
        );

        let rendered = semantic_state.transcript.rendered_text();
        assert_eq!(
            rendered.matches("[message:assistant]").count(),
            1,
            "{rendered}"
        );
        assert!(rendered.contains("第二版"), "{rendered}");
        assert!(!rendered.contains("第一版第二版"), "{rendered}");
    }

    #[test]
    fn remote_non_terminal_rewrite_keeps_only_final_consolidated_transcript() {
        let style = CliStyle::plain();
        let mut semantic_state = RemoteSemanticRenderState {
            is_terminal: false,
            ..RemoteSemanticRenderState::default()
        };
        let identity = rocode_types::LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: "text/main".to_string(),
            part_kind: rocode_types::LiveMessagePartKind::AssistantText,
            phase: rocode_types::LivePartPhase::Snapshot,
            legacy_block_id: Some("assistant-1".to_string()),
        };

        remote_apply_output_block(
            &mut semantic_state,
            &OutputBlock::Message(MessageBlock::full(
                MessageRole::Assistant,
                "初稿".to_string(),
            )),
            Some(&identity),
            &style,
            true,
        );
        remote_apply_output_block(
            &mut semantic_state,
            &OutputBlock::Message(MessageBlock::full(
                MessageRole::Assistant,
                "定稿".to_string(),
            )),
            Some(&identity),
            &style,
            true,
        );

        let rendered = semantic_state.transcript.rendered_text();
        assert_eq!(rendered, "[message:assistant] 定稿\n");
    }
}
