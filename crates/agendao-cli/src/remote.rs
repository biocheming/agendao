mod output_block_parse;
mod session_attach;
pub(crate) mod stream_consume;
mod transcript;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::cli::RunOutputFormat;
use serde::Deserialize;
use session_attach::{maybe_share_remote_session, resolve_remote_session};
use stream_consume::consume_remote_events;

fn server_url(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

async fn parse_http_json<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> anyhow::Result<T> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        anyhow::bail!("Request failed ({}): {}", status, body);
    }
    Ok(serde_json::from_str(&body)?)
}

pub(super) struct RemoteAttachOptions {
    pub base_url: String,
    pub input: String,
    pub command: Option<String>,
    pub continue_last: bool,
    pub session: Option<String>,
    pub fork: bool,
    pub share: bool,
    pub model: Option<String>,
    pub agent: Option<String>,
    pub scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
    pub variant: Option<String>,
    pub format: RunOutputFormat,
    pub title: Option<String>,
    pub directory: Option<String>,
    pub show_thinking: bool,
}

pub(super) async fn run_non_interactive_attach(options: RemoteAttachOptions) -> anyhow::Result<()> {
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
        scheduler,
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

    // Subscribe before submitting the prompt so short runs cannot finish before
    // the CLI has attached to the canonical frontend event authority.
    let events_endpoint = server_url(&base_url, "/event");
    let events_response = client
        .get(events_endpoint)
        .query(&[("session", session_id.as_str()), ("tier", "cli")])
        .send()
        .await?;
    if !events_response.status().is_success() {
        let status = events_response.status();
        let body = events_response.text().await.unwrap_or_default();
        anyhow::bail!("Remote event subscription failed ({}): {}", status, body);
    }

    let prompt_endpoint = server_url(&base_url, &format!("/session/{}/prompt", session_id));
    let response = client
        .post(prompt_endpoint)
        .json(&serde_json::json!({
            "message": content,
            "model": model,
            "agent": agent,
            "scheduler": scheduler,
            "variant": variant,
            "ingress_source": "cli"
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Remote run failed ({}): {}", status, body);
    }

    consume_remote_events(events_response, &client, &base_url, format, show_thinking).await
}

#[cfg(test)]
mod tests {
    use super::output_block_parse::parse_output_block;
    use super::stream_consume::{remote_apply_output_block, RemoteSemanticRenderState};
    use super::transcript::{cli_apply_live_slot_update, CliVisibleTranscript};
    use agendao_command_render::cli_style::CliStyle;
    use agendao_command_render::output_blocks::{
        MessageBlock, MessagePhase, MessageRole, OutputBlock, QueueItemBlock, StatusBlock,
    };
    use agendao_command_render::terminal_presentation::render_terminal_stream_block_semantic;

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
            render_terminal_stream_block_semantic(
                &mut semantic_state.semantic,
                &block,
                None,
                &style,
            )
        })
        .collect::<String>();

        assert_eq!(
            rendered.matches("[message:assistant]").count(),
            1,
            "{rendered}"
        );
        assert_eq!(rendered, "[message:assistant] 快速上升期\n");
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
            render_terminal_stream_block_semantic(
                &mut semantic_state.semantic,
                &block,
                None,
                &style,
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
        let identity = agendao_types::LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: agendao_types::LiveMessagePartKind::AssistantText,
            phase: agendao_types::LivePartPhase::Snapshot,
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
        remote_apply_output_block(&mut semantic_state, &first, None, &style, true);

        let second = OutputBlock::Message(MessageBlock::delta(
            MessageRole::Assistant,
            " 第二段".to_string(),
        ));
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
        let identity = agendao_types::LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: agendao_types::LiveMessagePartKind::AssistantText,
            phase: agendao_types::LivePartPhase::Snapshot,
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
        let identity = agendao_types::LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: agendao_types::LiveMessagePartKind::AssistantText,
            phase: agendao_types::LivePartPhase::Snapshot,
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
