use std::collections::HashMap;

use crate::cli_style::CliStyle;
use crate::live_semantic_consumer::{LiveContentMode, LiveSemanticConsumer, SemanticAction};
use crate::output_blocks::{
    render_cli_block_rich, MessageBlock as OutputMessageBlock, MessagePhase,
    MessageRole as OutputMessageRole, OutputBlock, ReasoningBlock as OutputReasoningBlock,
};
use agendao_types::{LiveMessagePartIdentity, LivePartPhase};

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalToolResultInfo {
    pub output: String,
    pub is_error: bool,
    pub title: Option<String>,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalToolState {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalStreamRenderState {
    assistant_open: bool,
    assistant_visible: bool,
    reasoning_open: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalSemanticStreamRenderState {
    boundary: TerminalStreamRenderState,
    live_consumer: LiveSemanticConsumer,
}

pub fn render_terminal_stream_block_with_state(
    state: &mut TerminalStreamRenderState,
    block: &OutputBlock,
    style: &CliStyle,
) -> String {
    match block {
        OutputBlock::Message(message) if message.role == OutputMessageRole::Assistant => {
            render_terminal_assistant_block(state, message, style)
        }
        OutputBlock::Reasoning(reasoning) => {
            render_terminal_reasoning_block(state, reasoning, style)
        }
        _ => {
            let mut out = render_terminal_stream_boundary_prefix(state);
            out.push_str(&render_cli_block_rich(block, style));
            out
        }
    }
}

fn render_terminal_assistant_block(
    state: &mut TerminalStreamRenderState,
    message: &OutputMessageBlock,
    style: &CliStyle,
) -> String {
    match message.phase {
        MessagePhase::Start => {
            state.assistant_open = true;
            String::new()
        }
        MessagePhase::Delta => {
            let mut out = String::new();
            if state.reasoning_open {
                out.push('\n');
                state.reasoning_open = false;
                state.assistant_visible = false;
            }
            state.assistant_open = true;
            if !state.assistant_visible {
                out.push_str(&render_cli_block_rich(
                    &OutputBlock::Message(OutputMessageBlock::start(OutputMessageRole::Assistant)),
                    style,
                ));
                state.assistant_visible = true;
            }
            out.push_str(&render_cli_block_rich(
                &OutputBlock::Message(message.clone()),
                style,
            ));
            out
        }
        MessagePhase::End => {
            let mut out = String::new();
            if state.reasoning_open {
                out.push('\n');
                state.reasoning_open = false;
            }
            if state.assistant_visible {
                out.push_str(&render_cli_block_rich(
                    &OutputBlock::Message(OutputMessageBlock::end(OutputMessageRole::Assistant)),
                    style,
                ));
            }
            state.assistant_open = false;
            state.assistant_visible = false;
            out
        }
        MessagePhase::Full => {
            let mut out = render_terminal_stream_boundary_prefix(state);
            out.push_str(&render_cli_block_rich(
                &OutputBlock::Message(message.clone()),
                style,
            ));
            state.assistant_open = false;
            state.assistant_visible = false;
            out
        }
    }
}

fn render_terminal_reasoning_block(
    state: &mut TerminalStreamRenderState,
    reasoning: &OutputReasoningBlock,
    style: &CliStyle,
) -> String {
    match reasoning.phase {
        MessagePhase::Start => {
            let mut out = String::new();
            if state.assistant_open && state.assistant_visible {
                out.push('\n');
                state.assistant_visible = false;
            }
            state.reasoning_open = true;
            out.push_str(&render_cli_block_rich(
                &OutputBlock::Reasoning(OutputReasoningBlock::start()),
                style,
            ));
            out
        }
        MessagePhase::Delta => {
            if !state.reasoning_open {
                state.reasoning_open = true;
                let mut out = render_cli_block_rich(
                    &OutputBlock::Reasoning(OutputReasoningBlock::start()),
                    style,
                );
                out.push_str(&render_cli_block_rich(
                    &OutputBlock::Reasoning(reasoning.clone()),
                    style,
                ));
                return out;
            }
            render_cli_block_rich(&OutputBlock::Reasoning(reasoning.clone()), style)
        }
        MessagePhase::End => {
            if !state.reasoning_open {
                return String::new();
            }
            state.reasoning_open = false;
            render_cli_block_rich(&OutputBlock::Reasoning(OutputReasoningBlock::end()), style)
        }
        MessagePhase::Full => {
            let mut out = String::new();
            if state.assistant_open && state.assistant_visible {
                out.push('\n');
                state.assistant_visible = false;
            }
            out.push_str(&render_cli_block_rich(
                &OutputBlock::Reasoning(reasoning.clone()),
                style,
            ));
            state.reasoning_open = false;
            out
        }
    }
}

fn render_terminal_stream_boundary_prefix(state: &mut TerminalStreamRenderState) -> String {
    let mut out = String::new();
    if state.reasoning_open {
        out.push('\n');
        state.reasoning_open = false;
    }
    if state.assistant_open && state.assistant_visible {
        out.push('\n');
        state.assistant_visible = false;
    }
    out
}

fn render_semantic_reasoning_start(
    state: &mut TerminalSemanticStreamRenderState,
    style: &CliStyle,
) -> String {
    let rendered = render_cli_block_rich(
        &OutputBlock::Reasoning(OutputReasoningBlock::start()),
        style,
    );
    let mut out = String::new();
    if state.boundary.assistant_open
        && state.boundary.assistant_visible
        && !rendered.starts_with('\n')
    {
        out.push('\n');
    }
    state.boundary.assistant_visible = false;
    state.boundary.reasoning_open = true;
    out.push_str(&rendered);
    out
}

fn render_semantic_reasoning_rewrite(
    state: &mut TerminalSemanticStreamRenderState,
    text: &str,
    style: &CliStyle,
) -> String {
    if text.is_empty() || state.boundary.reasoning_open {
        return String::new();
    }

    let mut out = render_semantic_reasoning_start(state, style);
    out.push_str(&render_terminal_stream_block_with_state(
        &mut state.boundary,
        &OutputBlock::Reasoning(OutputReasoningBlock::delta(text.to_string())),
        style,
    ));
    out
}

fn render_semantic_assistant_text_rewrite(
    boundary: &mut TerminalStreamRenderState,
    text: &str,
    style: &CliStyle,
) -> String {
    let mut out = render_terminal_stream_boundary_prefix(boundary);
    if !boundary.assistant_visible {
        out.push_str(&render_cli_block_rich(
            &OutputBlock::Message(OutputMessageBlock::start(OutputMessageRole::Assistant)),
            style,
        ));
        boundary.assistant_open = true;
        boundary.assistant_visible = true;
    }
    out.push_str(&render_cli_block_rich(
        &OutputBlock::Message(OutputMessageBlock::delta(
            OutputMessageRole::Assistant,
            text.to_string(),
        )),
        style,
    ));
    out
}

pub fn render_terminal_semantic_action(
    state: &mut TerminalSemanticStreamRenderState,
    action: &SemanticAction,
    style: &CliStyle,
) -> String {
    match action {
        SemanticAction::NoOp
        | SemanticAction::ToolBoundary
        | SemanticAction::ToolCallStarted { .. }
        | SemanticAction::ToolCallCompleted { .. } => String::new(),
        SemanticAction::OpenAssistant { text } | SemanticAction::ReplaceTextFull { text } => {
            render_semantic_assistant_text_rewrite(&mut state.boundary, text, style)
        }
        SemanticAction::AppendTextDelta { text } => render_terminal_stream_block_with_state(
            &mut state.boundary,
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                text.clone(),
            )),
            style,
        ),
        SemanticAction::OpenReasoning { text } => {
            let mut out = render_semantic_reasoning_start(state, style);
            if !text.is_empty() {
                out.push_str(&render_terminal_stream_block_with_state(
                    &mut state.boundary,
                    &OutputBlock::Reasoning(OutputReasoningBlock::delta(text.clone())),
                    style,
                ));
            }
            out
        }
        SemanticAction::AppendReasoningDelta { text } => render_terminal_stream_block_with_state(
            &mut state.boundary,
            &OutputBlock::Reasoning(OutputReasoningBlock::delta(text.clone())),
            style,
        ),
        SemanticAction::ReplaceReasoningFull { text } => {
            render_semantic_reasoning_rewrite(state, text, style)
        }
        SemanticAction::CloseReasoning => render_terminal_stream_block_with_state(
            &mut state.boundary,
            &OutputBlock::Reasoning(OutputReasoningBlock::end()),
            style,
        ),
    }
}

pub fn render_terminal_stream_block_semantic(
    state: &mut TerminalSemanticStreamRenderState,
    block: &OutputBlock,
    live_identity: Option<&LiveMessagePartIdentity>,
    style: &CliStyle,
) -> String {
    let Some(live_identity) = live_identity else {
        return render_terminal_stream_block_with_state(&mut state.boundary, block, style);
    };

    let mode = match live_identity.phase {
        LivePartPhase::Append => LiveContentMode::Delta,
        LivePartPhase::Start | LivePartPhase::Snapshot | LivePartPhase::End => {
            LiveContentMode::Snapshot
        }
    };
    let block_text = match block {
        OutputBlock::Message(message) => Some(message.text.as_str()),
        OutputBlock::Reasoning(reasoning) => Some(reasoning.text.as_str()),
        _ => None,
    };
    let action = state.live_consumer.consume(block_text, live_identity, mode);

    let mut out = render_terminal_semantic_action(state, &action, style);
    if matches!(
        action,
        SemanticAction::ToolBoundary | SemanticAction::ToolCallCompleted { .. }
    ) {
        out.push_str(&render_terminal_stream_block_with_state(
            &mut state.boundary,
            block,
            style,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_types::{LiveMessagePartKind, LivePartPhase};

    fn identity(phase: LivePartPhase) -> LiveMessagePartIdentity {
        LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: "text:0".to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase,
        }
    }

    #[test]
    fn typed_snapshots_emit_only_growth() {
        let style = CliStyle::plain();
        let mut state = TerminalSemanticStreamRenderState::default();
        let first = OutputBlock::Message(OutputMessageBlock::full(
            OutputMessageRole::Assistant,
            "hello".to_string(),
        ));
        let second = OutputBlock::Message(OutputMessageBlock::full(
            OutputMessageRole::Assistant,
            "hello world".to_string(),
        ));

        let first_rendered = render_terminal_stream_block_semantic(
            &mut state,
            &first,
            Some(&identity(LivePartPhase::Snapshot)),
            &style,
        );
        let second_rendered = render_terminal_stream_block_semantic(
            &mut state,
            &second,
            Some(&identity(LivePartPhase::Snapshot)),
            &style,
        );

        assert!(first_rendered.contains("hello"));
        assert!(second_rendered.contains(" world"));
        assert!(!second_rendered.contains("hello world"));
    }

    #[test]
    fn ordinary_blocks_render_without_live_identity() {
        let style = CliStyle::plain();
        let mut state = TerminalSemanticStreamRenderState::default();
        let block = OutputBlock::Message(OutputMessageBlock::full(
            OutputMessageRole::User,
            "question".to_string(),
        ));

        let rendered = render_terminal_stream_block_semantic(&mut state, &block, None, &style);
        assert!(rendered.contains("question"));
    }
}
