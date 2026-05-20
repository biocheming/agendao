use std::collections::HashMap;

use crate::cli_style::CliStyle;
use crate::live_semantic_consumer::{LiveSemanticConsumer, SemanticAction};
use crate::output_blocks::{
    render_cli_block_rich, MessageBlock as OutputMessageBlock, MessagePhase,
    MessageRole as OutputMessageRole, OutputBlock, ReasoningBlock as OutputReasoningBlock,
    ToolBlock as OutputToolBlock, ToolPhase,
};
use crate::terminal_tool_cli_render::{
    render_cli_file_lines, render_cli_image_lines, render_cli_tool_lines,
};
use rocode_types::LiveMessagePartIdentity;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalMessageRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalMessage {
    pub id: String,
    pub role: TerminalMessageRole,
    pub parts: Vec<TerminalMessagePart>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TerminalMessagePart {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    File {
        path: String,
        mime: String,
    },
    Image {
        url: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        id: String,
        result: String,
        is_error: bool,
        title: Option<String>,
        metadata: Option<HashMap<String, serde_json::Value>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum TerminalAssistantSegment {
    Spacer,
    Text {
        part_index: usize,
        text: String,
    },
    Reasoning {
        part_index: usize,
        text: String,
    },
    ToolCall {
        part_index: usize,
        id: String,
        name: String,
        arguments: String,
        state: TerminalToolState,
        result: Option<TerminalToolResultInfo>,
    },
    File {
        part_index: usize,
        path: String,
        mime: String,
    },
    Image {
        part_index: usize,
        url: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerminalStreamAccumulator {
    messages: Vec<TerminalMessage>,
    message_index: HashMap<String, usize>,
    next_generated_id: u64,
}

impl TerminalStreamAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn messages(&self) -> &[TerminalMessage] {
        &self.messages
    }

    pub fn message(&self, id: &str) -> Option<&TerminalMessage> {
        self.message_index
            .get(id)
            .and_then(|index| self.messages.get(*index))
    }

    pub fn into_messages(self) -> Vec<TerminalMessage> {
        self.messages
    }

    pub fn apply_output_block(&mut self, block_id: Option<&str>, block: &OutputBlock) -> bool {
        match block {
            OutputBlock::Message(message) => {
                self.apply_message_block(block_id, message);
                true
            }
            OutputBlock::Reasoning(reasoning) => {
                self.apply_reasoning_block(block_id, reasoning);
                true
            }
            OutputBlock::Tool(tool) => {
                self.apply_tool_block(block_id, tool);
                true
            }
            _ => false,
        }
    }

    fn apply_message_block(&mut self, block_id: Option<&str>, block: &OutputMessageBlock) {
        let role = match block.role {
            OutputMessageRole::User => TerminalMessageRole::User,
            OutputMessageRole::Assistant => TerminalMessageRole::Assistant,
            OutputMessageRole::System => TerminalMessageRole::System,
        };
        let pos = self.ensure_message_for_block(block_id, role.clone());
        let Some(message) = self.messages.get_mut(pos) else {
            return;
        };

        match block.phase {
            MessagePhase::Start => {
                message.role = role;
                message
                    .parts
                    .retain(|part| !matches!(part, TerminalMessagePart::Text { .. }));
            }
            MessagePhase::Delta => {
                if let Some(TerminalMessagePart::Text { text }) = message
                    .parts
                    .iter_mut()
                    .rev()
                    .find(|part| matches!(part, TerminalMessagePart::Text { .. }))
                {
                    text.push_str(&block.text);
                } else if !block.text.is_empty() {
                    message.parts.push(TerminalMessagePart::Text {
                        text: block.text.clone(),
                    });
                }
            }
            MessagePhase::Full => {
                message.role = role;
                let prior_text = message.parts.iter().rev().find_map(|part| match part {
                    TerminalMessagePart::Text { text } => Some(text.clone()),
                    _ => None,
                });
                message
                    .parts
                    .retain(|part| !matches!(part, TerminalMessagePart::Text { .. }));
                if !block.text.is_empty() {
                    let text = match prior_text {
                        Some(existing) if block.text.starts_with(&existing) => block.text.clone(),
                        Some(existing) if !existing.is_empty() => {
                            let mut merged = existing;
                            merged.push_str(&block.text);
                            merged
                        }
                        _ => block.text.clone(),
                    };
                    message.parts.push(TerminalMessagePart::Text {
                        text,
                    });
                }
            }
            MessagePhase::End => {}
        }
    }

    fn apply_reasoning_block(&mut self, block_id: Option<&str>, block: &OutputReasoningBlock) {
        let pos = self.ensure_reasoning_target(block_id);
        let Some(message) = self.messages.get_mut(pos) else {
            return;
        };

        match block.phase {
            MessagePhase::Start => {
                let has_reasoning = message
                    .parts
                    .iter()
                    .any(|part| matches!(part, TerminalMessagePart::Reasoning { .. }));
                if !has_reasoning {
                    message.parts.push(TerminalMessagePart::Reasoning {
                        text: String::new(),
                    });
                }
            }
            MessagePhase::Delta => {
                if let Some(TerminalMessagePart::Reasoning { text }) = message
                    .parts
                    .iter_mut()
                    .rev()
                    .find(|part| matches!(part, TerminalMessagePart::Reasoning { .. }))
                {
                    text.push_str(&block.text);
                } else if !block.text.is_empty() {
                    message.parts.push(TerminalMessagePart::Reasoning {
                        text: block.text.clone(),
                    });
                }
            }
            MessagePhase::Full => {
                message
                    .parts
                    .retain(|part| !matches!(part, TerminalMessagePart::Reasoning { .. }));
                if !block.text.is_empty() {
                    message.parts.push(TerminalMessagePart::Reasoning {
                        text: block.text.clone(),
                    });
                }
            }
            MessagePhase::End => {}
        }
    }

    fn apply_tool_block(&mut self, block_id: Option<&str>, block: &OutputToolBlock) {
        let tool_call_id = block_id
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.generate_message_id("tool"));
        let pos = self.ensure_message_for_block(block_id, TerminalMessageRole::Assistant);
        let Some(message) = self.messages.get_mut(pos) else {
            return;
        };

        match block.phase {
            ToolPhase::Start | ToolPhase::Running => {
                let arguments = block.detail.clone().unwrap_or_default();
                if let Some(TerminalMessagePart::ToolCall {
                    name, arguments: existing, ..
                }) = message.parts.iter_mut().find(|part| {
                    matches!(part, TerminalMessagePart::ToolCall { id, .. } if *id == tool_call_id)
                }) {
                    *name = block.name.clone();
                    *existing = arguments;
                } else {
                    message.parts.push(TerminalMessagePart::ToolCall {
                        id: tool_call_id,
                        name: block.name.clone(),
                        arguments,
                    });
                }
            }
            ToolPhase::Done | ToolPhase::Error => {
                let result = block.detail.clone().unwrap_or_default();
                let is_error = matches!(block.phase, ToolPhase::Error);
                let title = Some(block.name.clone());
                if let Some(TerminalMessagePart::ToolResult {
                    result: existing,
                    is_error: existing_is_error,
                    title: existing_title,
                    ..
                }) = message.parts.iter_mut().find(|part| {
                    matches!(
                        part,
                        TerminalMessagePart::ToolResult { id, .. } if *id == tool_call_id
                    )
                }) {
                    *existing = result;
                    *existing_is_error = is_error;
                    *existing_title = title;
                } else {
                    message.parts.push(TerminalMessagePart::ToolResult {
                        id: tool_call_id,
                        result,
                        is_error,
                        title,
                        metadata: None,
                    });
                }
            }
        }
    }

    fn ensure_reasoning_target(&mut self, block_id: Option<&str>) -> usize {
        if let Some(message_id) = block_id.filter(|value| !value.is_empty()) {
            return self.ensure_message_for_block(Some(message_id), TerminalMessageRole::Assistant);
        }

        let generated_id = self.generate_message_id("reasoning");
        self.ensure_message_for_block(Some(&generated_id), TerminalMessageRole::Assistant)
    }

    fn ensure_message_for_block(
        &mut self,
        block_id: Option<&str>,
        role: TerminalMessageRole,
    ) -> usize {
        if let Some(message_id) = block_id.filter(|value| !value.is_empty()) {
            if let Some(index) = self.message_index.get(message_id).copied() {
                return index;
            }

            let index = self.messages.len();
            self.messages.push(TerminalMessage {
                id: message_id.to_string(),
                role,
                parts: Vec::new(),
            });
            self.message_index.insert(message_id.to_string(), index);
            return index;
        }

        let generated_id = self.generate_message_id(match role {
            TerminalMessageRole::Assistant => "assistant",
            TerminalMessageRole::User => "user",
            TerminalMessageRole::System => "system",
            TerminalMessageRole::Tool => "tool",
        });
        let index = self.messages.len();
        self.messages.push(TerminalMessage {
            id: generated_id.clone(),
            role,
            parts: Vec::new(),
        });
        self.message_index.insert(generated_id, index);
        index
    }

    fn generate_message_id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}_{}", self.next_generated_id);
        self.next_generated_id = self.next_generated_id.saturating_add(1);
        id
    }
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
    current_message_id: Option<String>,
    part_states: HashMap<usize, TerminalSemanticPartState>,
    live_consumer: LiveSemanticConsumer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TerminalSemanticPartState {
    Text { emitted_text: String },
    Reasoning { started: bool, emitted_text: String },
    ToolCall { started: bool, completed: bool },
    File,
    Image,
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
            if !state.assistant_open {
                state.assistant_open = true;
            }
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

fn render_semantic_text_lines(
    boundary: &mut TerminalStreamRenderState,
    lines: &[String],
) -> String {
    if lines.is_empty() {
        return String::new();
    }

    let mut out = render_terminal_stream_boundary_prefix(boundary);
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
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
        | SemanticAction::LegacyPassThrough
        | SemanticAction::ToolBoundary
        | SemanticAction::ToolCallStarted { .. }
        | SemanticAction::ToolCallCompleted { .. } => String::new(),
        SemanticAction::OpenAssistant { text } => {
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
        SemanticAction::ReplaceTextFull { text } => {
            render_semantic_assistant_text_rewrite(&mut state.boundary, text, style)
        }
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
        SemanticAction::ReplaceReasoningFull { text } => render_terminal_stream_block_with_state(
            &mut state.boundary,
            &OutputBlock::Reasoning(OutputReasoningBlock::full(text.clone())),
            style,
        ),
        SemanticAction::CloseReasoning => render_terminal_stream_block_with_state(
            &mut state.boundary,
            &OutputBlock::Reasoning(OutputReasoningBlock::end()),
            style,
        ),
    }
}

fn semantic_delta_suffix<'a>(emitted_text: &str, current_text: &'a str) -> Option<&'a str> {
    current_text.strip_prefix(emitted_text)
}

pub fn render_terminal_stream_block_semantic(
    state: &mut TerminalSemanticStreamRenderState,
    accumulator: &TerminalStreamAccumulator,
    block: &OutputBlock,
    live_identity: Option<&LiveMessagePartIdentity>,
    style: &CliStyle,
    show_thinking: bool,
) -> String {
    if live_identity.is_some() {
        let block_text = match block {
            OutputBlock::Message(message) => Some(message.text.as_str()),
            OutputBlock::Reasoning(reasoning) => Some(reasoning.text.as_str()),
            _ => None,
        };
        let action = state.live_consumer.consume(block_text, live_identity);
        if !matches!(action, SemanticAction::LegacyPassThrough) {
            let mut out = render_terminal_semantic_action(state, &action, style);
            // ToolBoundary is a state signal — still render the tool block.
            if matches!(
                action,
                SemanticAction::ToolBoundary | SemanticAction::ToolCallCompleted { .. }
            ) {
                out.push_str(&render_terminal_stream_block_with_state(
                    &mut state.boundary, block, style,
                ));
            }
            return out;
        }
    }

    let is_semantic_block = match block {
        OutputBlock::Message(message) => message.role == OutputMessageRole::Assistant,
        OutputBlock::Reasoning(_) | OutputBlock::Tool(_) => true,
        _ => false,
    };
    if !is_semantic_block {
        return render_terminal_stream_block_with_state(&mut state.boundary, block, style);
    }

    let Some((assistant_idx, message)) = accumulator
        .messages()
        .iter()
        .enumerate()
        .rev()
        .find(|(_, message)| matches!(message.role, TerminalMessageRole::Assistant))
    else {
        return render_terminal_stream_block_with_state(&mut state.boundary, block, style);
    };

    if state.current_message_id.as_deref() != Some(message.id.as_str()) {
        state.current_message_id = Some(message.id.clone());
        state.part_states.clear();
    }

    let tool_results = collect_assistant_tool_results(accumulator.messages(), assistant_idx);
    let running_tool_call = message.parts.iter().find_map(|part| match part {
        TerminalMessagePart::ToolCall { id, .. } if !tool_results.contains_key(id) => {
            Some(id.as_str())
        }
        _ => None,
    });
    let segments =
        compose_assistant_segments(message, &tool_results, running_tool_call, show_thinking);
    let mut out = String::new();

    for segment in segments {
        match segment {
            TerminalAssistantSegment::Spacer => {}
            TerminalAssistantSegment::Text { part_index, text } => {
                let entry = state
                    .part_states
                    .entry(part_index)
                    .or_insert_with(|| TerminalSemanticPartState::Text {
                        emitted_text: String::new(),
                    });
                let TerminalSemanticPartState::Text { emitted_text } = entry else {
                    continue;
                };
                if let Some(delta) = semantic_delta_suffix(emitted_text, &text) {
                    if delta.is_empty() {
                        continue;
                    }
                    out.push_str(&render_terminal_stream_block_with_state(
                        &mut state.boundary,
                        &OutputBlock::Message(OutputMessageBlock::delta(
                            OutputMessageRole::Assistant,
                            delta,
                        )),
                        style,
                    ));
                    *emitted_text = text;
                } else {
                    out.push_str(&render_semantic_assistant_text_rewrite(
                        &mut state.boundary,
                        &text,
                        style,
                    ));
                    *emitted_text = text;
                }
            }
            TerminalAssistantSegment::Reasoning { part_index, text } => {
                let mut emit_start = false;
                let entry = state.part_states.entry(part_index).or_insert(
                    TerminalSemanticPartState::Reasoning {
                        started: false,
                        emitted_text: String::new(),
                    },
                );
                let TerminalSemanticPartState::Reasoning {
                    started,
                    emitted_text,
                } = entry
                else {
                    continue;
                };
                if !*started {
                    emit_start = true;
                }
                let prior_text = emitted_text.clone();
                if emit_start {
                    out.push_str(&render_semantic_reasoning_start(state, style));
                }
                let entry = state.part_states.entry(part_index).or_insert(
                    TerminalSemanticPartState::Reasoning {
                        started: false,
                        emitted_text: String::new(),
                    },
                );
                let TerminalSemanticPartState::Reasoning {
                    started,
                    emitted_text,
                } = entry
                else {
                    continue;
                };
                if emit_start {
                    *started = true;
                }
                if let Some(delta) = semantic_delta_suffix(&prior_text, &text) {
                    if delta.is_empty() {
                        continue;
                    }
                    out.push_str(&render_terminal_stream_block_with_state(
                        &mut state.boundary,
                        &OutputBlock::Reasoning(OutputReasoningBlock::delta(delta)),
                        style,
                    ));
                    *emitted_text = text;
                } else {
                    out.push_str(&render_terminal_stream_block_with_state(
                        &mut state.boundary,
                        &OutputBlock::Reasoning(OutputReasoningBlock::full(text.clone())),
                        style,
                    ));
                    *emitted_text = text;
                }
            }
            TerminalAssistantSegment::ToolCall {
                part_index,
                name,
                arguments,
                state: tool_state,
                result,
                ..
            } => {
                let entry = state.part_states.entry(part_index).or_insert(
                    TerminalSemanticPartState::ToolCall {
                        started: false,
                        completed: false,
                    },
                );
                let TerminalSemanticPartState::ToolCall { started, completed } = entry else {
                    continue;
                };
                if !*started {
                    let lines =
                        render_cli_tool_lines(&name, &arguments, tool_state, None, false, style);
                    out.push_str(&render_semantic_text_lines(&mut state.boundary, &lines));
                    *started = true;
                }
                if !*completed {
                    if let Some(info) = result {
                        let lines = render_cli_tool_lines(
                            &name,
                            &arguments,
                            tool_state,
                            Some(&info),
                            true,
                            style,
                        );
                        out.push_str(&render_semantic_text_lines(&mut state.boundary, &lines));
                        *completed = true;
                    } else if matches!(
                        tool_state,
                        TerminalToolState::Failed | TerminalToolState::Completed
                    ) {
                        *completed = true;
                    }
                }
            }
            TerminalAssistantSegment::File {
                part_index,
                path,
                mime,
            } => {
                if matches!(
                    state.part_states.get(&part_index),
                    Some(TerminalSemanticPartState::File)
                ) {
                    continue;
                }
                let lines = render_cli_file_lines(&path, &mime, style);
                out.push_str(&render_semantic_text_lines(&mut state.boundary, &lines));
                state
                    .part_states
                    .insert(part_index, TerminalSemanticPartState::File);
            }
            TerminalAssistantSegment::Image { part_index, url } => {
                if matches!(
                    state.part_states.get(&part_index),
                    Some(TerminalSemanticPartState::Image)
                ) {
                    continue;
                }
                let lines = render_cli_image_lines(&url, style);
                out.push_str(&render_semantic_text_lines(&mut state.boundary, &lines));
                state
                    .part_states
                    .insert(part_index, TerminalSemanticPartState::Image);
            }
        }
    }

    out
}

pub fn is_tool_result_carrier(message: &TerminalMessage) -> bool {
    if !matches!(message.role, TerminalMessageRole::Tool) {
        return false;
    }

    let mut has_tool_result = false;
    for part in &message.parts {
        match part {
            TerminalMessagePart::ToolResult { .. } => has_tool_result = true,
            TerminalMessagePart::Text { text } | TerminalMessagePart::Reasoning { text }
                if text.trim().is_empty() => {}
            _ => return false,
        }
    }

    has_tool_result
}

pub fn collect_assistant_tool_results(
    messages: &[TerminalMessage],
    assistant_idx: usize,
) -> HashMap<String, TerminalToolResultInfo> {
    let mut tool_results = HashMap::new();

    for (idx, message) in messages.iter().enumerate().skip(assistant_idx) {
        if idx > assistant_idx && matches!(message.role, TerminalMessageRole::Assistant) {
            break;
        }

        for part in &message.parts {
            if let TerminalMessagePart::ToolResult {
                id,
                result,
                is_error,
                title,
                metadata,
            } = part
            {
                tool_results.insert(
                    id.clone(),
                    TerminalToolResultInfo {
                        output: result.clone(),
                        is_error: *is_error,
                        title: title.clone(),
                        metadata: metadata.clone(),
                    },
                );
            }
        }
    }

    tool_results
}

pub fn compose_assistant_segments(
    message: &TerminalMessage,
    tool_results: &HashMap<String, TerminalToolResultInfo>,
    running_tool_call: Option<&str>,
    show_thinking: bool,
) -> Vec<TerminalAssistantSegment> {
    let mut segments = Vec::new();

    for (part_index, part) in message.parts.iter().enumerate() {
        match part {
            TerminalMessagePart::Text { text } => {
                segments.push(TerminalAssistantSegment::Text {
                    part_index,
                    text: text.clone(),
                });
            }
            TerminalMessagePart::Reasoning { text } => {
                if !show_thinking {
                    continue;
                }
                segments.push(TerminalAssistantSegment::Reasoning {
                    part_index,
                    text: text.clone(),
                });
            }
            TerminalMessagePart::ToolCall {
                id,
                name,
                arguments,
            } => {
                let state = if let Some(info) = tool_results.get(id) {
                    if info.is_error {
                        TerminalToolState::Failed
                    } else {
                        TerminalToolState::Completed
                    }
                } else if running_tool_call == Some(id.as_str()) {
                    TerminalToolState::Running
                } else {
                    TerminalToolState::Pending
                };
                segments.push(TerminalAssistantSegment::ToolCall {
                    part_index,
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                    state,
                    result: tool_results.get(id).cloned(),
                });
            }
            TerminalMessagePart::ToolResult { .. } => {}
            TerminalMessagePart::File { path, mime } => {
                segments.push(TerminalAssistantSegment::File {
                    part_index,
                    path: path.clone(),
                    mime: mime.clone(),
                });
            }
            TerminalMessagePart::Image { url } => {
                segments.push(TerminalAssistantSegment::Image {
                    part_index,
                    url: url.clone(),
                });
            }
        }
    }

    segments.sort_by_key(assistant_segment_semantic_key);
    segments
}

fn assistant_segment_semantic_key(segment: &TerminalAssistantSegment) -> (u8, usize) {
    match segment {
        TerminalAssistantSegment::Reasoning { part_index, .. } => (1, *part_index),
        TerminalAssistantSegment::ToolCall { part_index, .. } => (2, *part_index),
        TerminalAssistantSegment::File { part_index, .. }
        | TerminalAssistantSegment::Image { part_index, .. } => (3, *part_index),
        TerminalAssistantSegment::Text { part_index, .. } => (4, *part_index),
        TerminalAssistantSegment::Spacer => (5, usize::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocode_types::{LiveMessagePartIdentity, LiveMessagePartKind, LivePartPhase};

    fn live_identity(
        message_id: &str,
        part_key: &str,
        part_kind: LiveMessagePartKind,
        phase: LivePartPhase,
    ) -> LiveMessagePartIdentity {
        LiveMessagePartIdentity {
            message_id: message_id.to_string(),
            part_key: part_key.to_string(),
            part_kind,
            phase,
            legacy_block_id: Some(format!("{message_id}:{part_key}")),
        }
    }

    fn message(
        id: &str,
        role: TerminalMessageRole,
        parts: Vec<TerminalMessagePart>,
    ) -> TerminalMessage {
        TerminalMessage {
            id: id.to_string(),
            role,
            parts,
        }
    }

    #[test]
    fn tool_result_carrier_is_detected() {
        let msg = message(
            "tool-msg",
            TerminalMessageRole::Tool,
            vec![TerminalMessagePart::ToolResult {
                id: "call-1".to_string(),
                result: "ok".to_string(),
                is_error: false,
                title: None,
                metadata: None,
            }],
        );
        assert!(is_tool_result_carrier(&msg));
    }

    #[test]
    fn assistant_collects_tool_results_until_next_assistant() {
        let messages = vec![
            message("user-1", TerminalMessageRole::User, vec![]),
            message(
                "assistant-1",
                TerminalMessageRole::Assistant,
                vec![TerminalMessagePart::ToolCall {
                    id: "call-1".to_string(),
                    name: "ls".to_string(),
                    arguments: r#"{"path":"."}"#.to_string(),
                }],
            ),
            message(
                "tool-1",
                TerminalMessageRole::Tool,
                vec![TerminalMessagePart::ToolResult {
                    id: "call-1".to_string(),
                    result: "file_a\nfile_b".to_string(),
                    is_error: false,
                    title: None,
                    metadata: None,
                }],
            ),
            message(
                "assistant-2",
                TerminalMessageRole::Assistant,
                vec![TerminalMessagePart::ToolCall {
                    id: "call-2".to_string(),
                    name: "read".to_string(),
                    arguments: r#"{"file_path":"README.md"}"#.to_string(),
                }],
            ),
            message(
                "tool-2",
                TerminalMessageRole::Tool,
                vec![TerminalMessagePart::ToolResult {
                    id: "call-2".to_string(),
                    result: "readme".to_string(),
                    is_error: false,
                    title: None,
                    metadata: None,
                }],
            ),
        ];

        let first_results = collect_assistant_tool_results(&messages, 1);
        assert!(first_results.contains_key("call-1"));
        assert!(!first_results.contains_key("call-2"));
    }

    #[test]
    fn assistant_segments_insert_spacers_between_text_reasoning_and_tools() {
        let message = message(
            "assistant-1",
            TerminalMessageRole::Assistant,
            vec![
                TerminalMessagePart::Text {
                    text: "one".to_string(),
                },
                TerminalMessagePart::Reasoning {
                    text: "think".to_string(),
                },
                TerminalMessagePart::ToolCall {
                    id: "call-1".to_string(),
                    name: "websearch".to_string(),
                    arguments: "{}".to_string(),
                },
                TerminalMessagePart::Text {
                    text: "two".to_string(),
                },
            ],
        );

        let segments = compose_assistant_segments(&message, &HashMap::new(), Some("call-1"), true);

        assert!(matches!(
            segments.as_slice(),
            [
                TerminalAssistantSegment::Reasoning { .. },
                TerminalAssistantSegment::ToolCall {
                    state: TerminalToolState::Running,
                    ..
                },
                TerminalAssistantSegment::Text { .. },
                TerminalAssistantSegment::Text { .. }
            ]
        ));
    }

    #[test]
    fn accumulator_preserves_reasoning_when_assistant_message_starts() {
        let mut accumulator = TerminalStreamAccumulator::new();

        assert!(accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Reasoning(OutputReasoningBlock::start())
        ));
        assert!(accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Reasoning(OutputReasoningBlock::delta("thinking..."))
        ));
        assert!(accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::start(OutputMessageRole::Assistant))
        ));

        let message = accumulator
            .message("assistant-1")
            .expect("assistant message should exist");
        assert!(message.parts.iter().any(|part| {
            matches!(
                part,
                TerminalMessagePart::Reasoning { text } if text == "thinking..."
            )
        }));
    }

    #[test]
    fn accumulator_creates_distinct_tool_message_when_tool_has_no_parent_message_id() {
        let mut accumulator = TerminalStreamAccumulator::new();

        assert!(accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "answer"
            ))
        ));
        assert!(accumulator.apply_output_block(
            Some("tool-1"),
            &OutputBlock::Tool(OutputToolBlock::start("websearch"))
        ));
        assert!(accumulator.apply_output_block(
            Some("tool-1"),
            &OutputBlock::Tool(OutputToolBlock::done(
                "websearch",
                Some("query finished".to_string())
            ))
        ));

        let message = accumulator
            .message("assistant-1")
            .expect("assistant message should exist");
        assert!(message.parts.iter().any(|part| {
            matches!(
                part,
                TerminalMessagePart::Text { text } if text == "answer"
            )
        }));
        assert!(
            accumulator
                .message("tool-1")
                .is_some_and(|tool_message| tool_message.parts.iter().any(|part| matches!(
                    part,
                    TerminalMessagePart::ToolCall { id, name, .. }
                        if id == "tool-1" && name == "websearch"
                )))
        );
        assert!(
            accumulator
                .message("tool-1")
                .is_some_and(|tool_message| tool_message.parts.iter().any(|part| matches!(
                    part,
                    TerminalMessagePart::ToolResult {
                        id,
                        result,
                        is_error,
                        ..
                    } if id == "tool-1" && result == "query finished" && !is_error
                )))
        );
    }

    #[test]
    fn accumulator_creates_new_reasoning_message_without_id() {
        let mut accumulator = TerminalStreamAccumulator::new();

        assert!(accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "answer"
            ))
        ));
        assert!(accumulator.apply_output_block(
            None,
            &OutputBlock::Reasoning(OutputReasoningBlock::delta("thinking"))
        ));

        let message = accumulator
            .message("assistant-1")
            .expect("assistant message should exist");
        assert!(message.parts.iter().any(|part| {
            matches!(
                part,
                TerminalMessagePart::Text { text } if text == "answer"
            )
        }));
        assert!(
            accumulator
                .messages()
                .iter()
                .any(|message| matches!(message.role, TerminalMessageRole::Assistant)
                    && message.id != "assistant-1"
                    && message.parts.iter().any(|part| matches!(
                        part,
                        TerminalMessagePart::Reasoning { text } if text == "thinking"
                    )))
        );
    }

    #[test]
    fn accumulator_creates_new_assistant_delta_message_without_id() {
        let mut accumulator = TerminalStreamAccumulator::new();

        assert!(accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "previous answer"
            ))
        ));
        assert!(accumulator.apply_output_block(
            Some("assistant-2"),
            &OutputBlock::Message(OutputMessageBlock::start(OutputMessageRole::Assistant))
        ));
        assert!(accumulator.apply_output_block(
            None,
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "new answer"
            ))
        ));

        let previous = accumulator
            .message("assistant-1")
            .expect("previous assistant should exist");

        let previous_text = previous
            .parts
            .iter()
            .filter_map(|part| match part {
                TerminalMessagePart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(previous_text, "previous answer");
        assert!(
            accumulator
                .messages()
                .iter()
                .any(|message| matches!(message.role, TerminalMessageRole::Assistant)
                    && message.id != "assistant-1"
                    && message.id != "assistant-2"
                    && message.parts.iter().any(|part| matches!(
                        part,
                        TerminalMessagePart::Text { text } if text == "new answer"
                    )))
        );
    }

    #[test]
    fn compose_assistant_segments_orders_reasoning_and_tools_before_final_text() {
        let msg = message(
            "assistant-1",
            TerminalMessageRole::Assistant,
            vec![
                TerminalMessagePart::Text {
                    text: "final answer".to_string(),
                },
                TerminalMessagePart::Reasoning {
                    text: "thinking".to_string(),
                },
                TerminalMessagePart::ToolCall {
                    id: "tool-1".to_string(),
                    name: "search".to_string(),
                    arguments: "{}".to_string(),
                },
            ],
        );

        let segments = compose_assistant_segments(&msg, &HashMap::new(), None, true)
            .into_iter()
            .map(|segment| match segment {
                TerminalAssistantSegment::Reasoning { .. } => "reasoning",
                TerminalAssistantSegment::ToolCall { .. } => "tool",
                TerminalAssistantSegment::Text { .. } => "text",
                TerminalAssistantSegment::File { .. } => "file",
                TerminalAssistantSegment::Image { .. } => "image",
                TerminalAssistantSegment::Spacer => "spacer",
            })
            .collect::<Vec<_>>();

        assert_eq!(segments, vec!["reasoning", "tool", "text"]);
    }

    #[test]
    fn accumulator_merges_same_message_full_chunks_until_cumulative_snapshot_arrives() {
        let mut accumulator = TerminalStreamAccumulator::new();

        assert!(accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::full(
                OutputMessageRole::Assistant,
                "现在".to_string()
            ))
        ));
        assert!(accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::full(
                OutputMessageRole::Assistant,
                "我已".to_string()
            ))
        ));
        assert!(accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::full(
                OutputMessageRole::Assistant,
                "掌握".to_string()
            ))
        ));

        let intermediate = accumulator
            .message("assistant-1")
            .expect("assistant message should exist");
        let intermediate_text = intermediate
            .parts
            .iter()
            .filter_map(|part| match part {
                TerminalMessagePart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(intermediate_text, "现在我已掌握");

        assert!(accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::full(
                OutputMessageRole::Assistant,
                "现在我已掌握充分信息".to_string()
            ))
        ));

        let final_message = accumulator
            .message("assistant-1")
            .expect("assistant message should exist");
        let final_text = final_message
            .parts
            .iter()
            .filter_map(|part| match part {
                TerminalMessagePart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(final_text, "现在我已掌握充分信息");
    }

    #[test]
    fn stream_render_state_moves_assistant_start_after_reasoning_boundary() {
        let style = CliStyle::plain();
        let mut state = TerminalStreamRenderState::default();

        let assistant_start = render_terminal_stream_block_with_state(
            &mut state,
            &OutputBlock::Message(OutputMessageBlock::start(OutputMessageRole::Assistant)),
            &style,
        );
        let reasoning_start = render_terminal_stream_block_with_state(
            &mut state,
            &OutputBlock::Reasoning(OutputReasoningBlock::start()),
            &style,
        );
        let reasoning_delta = render_terminal_stream_block_with_state(
            &mut state,
            &OutputBlock::Reasoning(OutputReasoningBlock::delta("thinking")),
            &style,
        );
        let assistant_delta = render_terminal_stream_block_with_state(
            &mut state,
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "answer",
            )),
            &style,
        );

        assert_eq!(assistant_start, "");
        assert_eq!(reasoning_start, "\n[thinking]\n│ ");
        assert_eq!(reasoning_delta, "thinking");
        assert_eq!(assistant_delta, "\n[message:assistant] answer");
    }

    #[test]
    fn stream_render_state_inserts_newline_before_tool_when_assistant_end_is_missing() {
        let style = CliStyle::plain();
        let mut state = TerminalStreamRenderState::default();

        let assistant_start = render_terminal_stream_block_with_state(
            &mut state,
            &OutputBlock::Message(OutputMessageBlock::start(OutputMessageRole::Assistant)),
            &style,
        );
        let assistant_delta = render_terminal_stream_block_with_state(
            &mut state,
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "answer",
            )),
            &style,
        );
        let tool_start = render_terminal_stream_block_with_state(
            &mut state,
            &OutputBlock::Tool(OutputToolBlock::start("websearch")),
            &style,
        );

        assert_eq!(assistant_start, "");
        assert_eq!(assistant_delta, "[message:assistant] answer");
        assert_eq!(tool_start, "\n[tool:start] websearch\n");
    }

    #[test]
    fn semantic_stream_renderer_groups_reasoning_between_assistant_segments() {
        let style = CliStyle::plain();
        let mut accumulator = TerminalStreamAccumulator::new();
        let mut state = TerminalSemanticStreamRenderState::default();

        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "answer",
            )),
        );
        let text = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "answer",
            )),
            None,
            &style,
            true,
        );

        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Reasoning(OutputReasoningBlock::start()),
        );
        let reasoning_start = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Reasoning(OutputReasoningBlock::start()),
            None,
            &style,
            true,
        );

        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Reasoning(OutputReasoningBlock::delta("thinking")),
        );
        let reasoning_delta = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Reasoning(OutputReasoningBlock::delta("thinking")),
            None,
            &style,
            true,
        );

        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                " done",
            )),
        );
        let trailing_text = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                " done",
            )),
            None,
            &style,
            true,
        );

        assert_eq!(text, "[message:assistant] answer");
        assert_eq!(reasoning_start, "\n[thinking]\n│ ");
        assert_eq!(reasoning_delta, "thinking");
        assert_eq!(trailing_text, "\n[message:assistant]  done");
    }

    #[test]
    fn semantic_stream_renderer_emits_reasoning_header_only_once_for_multiple_deltas() {
        let style = CliStyle::plain();
        let mut accumulator = TerminalStreamAccumulator::new();
        let mut state = TerminalSemanticStreamRenderState::default();

        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Reasoning(OutputReasoningBlock::start()),
        );
        let reasoning_start = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Reasoning(OutputReasoningBlock::start()),
            None,
            &style,
            true,
        );

        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Reasoning(OutputReasoningBlock::delta("alpha")),
        );
        let delta_one = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Reasoning(OutputReasoningBlock::delta("alpha")),
            None,
            &style,
            true,
        );

        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Reasoning(OutputReasoningBlock::delta(" beta")),
        );
        let delta_two = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Reasoning(OutputReasoningBlock::delta(" beta")),
            None,
            &style,
            true,
        );

        assert_eq!(reasoning_start, "\n[thinking]\n│ ");
        assert_eq!(delta_one, "alpha");
        assert_eq!(delta_two, " beta");
    }

    #[test]
    fn semantic_stream_renderer_uses_segment_order_for_tool_start_and_result() {
        let style = CliStyle::plain();
        let mut accumulator = TerminalStreamAccumulator::new();
        let mut state = TerminalSemanticStreamRenderState::default();

        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "answer",
            )),
        );
        let text = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "answer",
            )),
            None,
            &style,
            true,
        );

        accumulator.apply_output_block(
            Some("tool-1"),
            &OutputBlock::Tool(OutputToolBlock::running(
                "websearch",
                r#"{"query":"青岛天气"}"#,
            )),
        );
        let tool_start = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Tool(OutputToolBlock::running(
                "websearch",
                r#"{"query":"青岛天气"}"#,
            )),
            None,
            &style,
            true,
        );

        accumulator.apply_output_block(
            Some("tool-1"),
            &OutputBlock::Tool(OutputToolBlock::done(
                "websearch",
                Some("晴 18C".to_string()),
            )),
        );
        let tool_done = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Tool(OutputToolBlock::done(
                "websearch",
                Some("晴 18C".to_string()),
            )),
            None,
            &style,
            true,
        );

        assert_eq!(text, "[message:assistant] answer");
        assert_eq!(tool_start, "\n◌ ◈ websearch  \"青岛天气\"\n");
        assert_eq!(tool_done, "● ◈ websearch  \"青岛天气\"\n晴 18C\n");
    }

    #[test]
    fn semantic_stream_renderer_renders_shared_task_body_items() {
        let style = CliStyle::plain();
        let mut accumulator = TerminalStreamAccumulator::new();
        let mut state = TerminalSemanticStreamRenderState::default();

        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Tool(OutputToolBlock::running(
                "task",
                r###"{"category":"visual-engineering","prompt":"## 1. TASK\nRedesign page\n- [ ] 修改 t2.html"}"###,
            )),
        );
        let task_start = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Tool(OutputToolBlock::running(
                "task",
                r###"{"category":"visual-engineering","prompt":"## 1. TASK\nRedesign page\n- [ ] 修改 t2.html"}"###,
            )),
            None,
            &style,
            true,
        );

        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Tool(OutputToolBlock::done(
                "task",
                Some(
                    "task_id: abc123\ntask_status: completed\n<task_result>\n## Summary\n- [x] 修改 t2.html\nDone.\n</task_result>"
                        .to_string(),
                ),
            )),
        );
        let task_done = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Tool(OutputToolBlock::done(
                "task",
                Some(
                    "task_id: abc123\ntask_status: completed\n<task_result>\n## Summary\n- [x] 修改 t2.html\nDone.\n</task_result>"
                        .to_string(),
                ),
            )),
            None,
            &style,
            true,
        );

        assert!(task_start.contains("◌ # task"));
        assert!(task_start.contains("Delegating task to subagent"));
        assert!(task_start.contains("Checklist (1 items):"));

        assert!(task_done.contains("● # task"));
        assert!(task_done.contains("Task ID: abc123"));
        assert!(task_done.contains("Checklist (1 items):"));
        assert!(task_done.contains("## Summary"));
        assert!(task_done.contains("Done."));
    }

    #[test]
    fn semantic_stream_renderer_uses_shared_file_and_image_items() {
        let style = CliStyle::plain();
        let mut accumulator = TerminalStreamAccumulator::new();
        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "see attachments",
            )),
        );
        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::full(
                OutputMessageRole::Assistant,
                "see attachments",
            )),
        );
        if let Some(message) = accumulator
            .messages
            .iter_mut()
            .find(|message| message.id == "assistant-1")
        {
            message.parts.push(TerminalMessagePart::File {
                path: "/tmp/demo.png".to_string(),
                mime: "image/png".to_string(),
            });
            message.parts.push(TerminalMessagePart::Image {
                url: "data:image/png;base64,QUJDRA==".to_string(),
            });
        }

        let mut state = TerminalSemanticStreamRenderState::default();
        let rendered = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Message(OutputMessageBlock::full(
                OutputMessageRole::Assistant,
                "see attachments",
            )),
            None,
            &style,
            true,
        );

        assert!(rendered.contains("[file] /tmp/demo.png"));
        assert!(rendered.contains("type: image/png"));
        assert!(rendered.contains("[image] inline image"));
        assert!(rendered.contains("size: 4 B"));
    }

    #[test]
    fn semantic_stream_renderer_handles_unicode_after_stale_ascii_prefix_state() {
        let style = CliStyle::plain();
        let mut accumulator = TerminalStreamAccumulator::new();
        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::full(
                OutputMessageRole::Assistant,
                "中国".to_string(),
            )),
        );

        let mut state = TerminalSemanticStreamRenderState {
            boundary: TerminalStreamRenderState::default(),
            current_message_id: Some("assistant-1".to_string()),
            part_states: HashMap::from([(
                0,
                TerminalSemanticPartState::Text {
                    emitted_text: "a".to_string(),
                },
            )]),
            live_consumer: LiveSemanticConsumer::default(),
        };

        let rendered = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Message(OutputMessageBlock::full(
                OutputMessageRole::Assistant,
                "中国".to_string(),
            )),
            None,
            &style,
            true,
        );

        assert_eq!(rendered, "[message:assistant] 中国");
    }

    #[test]
    fn semantic_stream_renderer_keeps_single_assistant_header_across_many_text_deltas() {
        let style = CliStyle::plain();
        let mut accumulator = TerminalStreamAccumulator::new();
        let mut state = TerminalSemanticStreamRenderState::default();

        let deltas = [
            "## ",
            "\n",
            "五",
            "、",
            "授权",
            "发明专利",
            "\n",
            "| 序号 | 专利名称 |\n",
        ];

        let mut rendered = String::new();
        for delta in deltas {
            accumulator.apply_output_block(
                Some("assistant-1"),
                &OutputBlock::Message(OutputMessageBlock::delta(
                    OutputMessageRole::Assistant,
                    delta,
                )),
            );
            rendered.push_str(&render_terminal_stream_block_semantic(
                &mut state,
                &accumulator,
                &OutputBlock::Message(OutputMessageBlock::delta(
                    OutputMessageRole::Assistant,
                    delta,
                )),
                None,
                &style,
                true,
            ));
        }

        assert_eq!(rendered.matches("[message:assistant]").count(), 1);
        assert!(rendered.contains("五、授权发明专利"));
        assert!(rendered.contains("| 序号 | 专利名称 |"));
    }

    #[test]
    fn semantic_stream_renderer_handles_five_assistant_messages_separated_by_four_tool_cycles() {
        let style = CliStyle::plain();
        let mut accumulator = TerminalStreamAccumulator::new();
        let mut state = TerminalSemanticStreamRenderState::default();
        let mut rendered = String::new();

        let mut push = |id: &str, block: OutputBlock| {
            accumulator.apply_output_block(Some(id), &block);
            rendered.push_str(&render_terminal_stream_block_semantic(
                &mut state,
                &accumulator,
                &block,
                None,
                &style,
                true,
            ));
        };

        for step in 1..=4 {
            let assistant_id = format!("assistant-{step}");
            let tool_id = format!("tool-{step}");

            push(
                &assistant_id,
                OutputBlock::Reasoning(OutputReasoningBlock::start()),
            );
            push(
                &assistant_id,
                OutputBlock::Reasoning(OutputReasoningBlock::delta(format!(
                    "thinking {step}"
                ))),
            );
            push(
                &assistant_id,
                OutputBlock::Tool(OutputToolBlock::start("websearch")),
            );
            push(
                &tool_id,
                OutputBlock::Tool(OutputToolBlock::done(
                    "websearch",
                    Some(format!("result {step}")),
                )),
            );
            push(
                &assistant_id,
                OutputBlock::Reasoning(OutputReasoningBlock::end()),
            );
            push(
                &assistant_id,
                OutputBlock::Message(OutputMessageBlock::end(OutputMessageRole::Assistant)),
            );
        }

        let final_id = "assistant-5";
        for delta in [
            "快速",
            "上升",
            "期",
            "，",
            "从副研究员晋升为正高级工程师",
            "，",
            "作为通讯作者发表了包括 ",
            "*Nucleic Acids Research*",
            "、",
            "*J. Med. Chem.*",
            " 等在内的约 20 篇论文",
        ] {
            push(
                final_id,
                OutputBlock::Message(OutputMessageBlock::delta(
                    OutputMessageRole::Assistant,
                    delta,
                )),
            );
        }

        assert_eq!(
            rendered.matches("[message:assistant]").count(),
            1,
            "{rendered}"
        );
        assert!(rendered.contains("快速上升期"), "{rendered}");
        assert!(rendered.contains("*Nucleic Acids Research*"), "{rendered}");
        assert!(rendered.contains("*J. Med. Chem.*"), "{rendered}");
    }

    #[test]
    fn semantic_stream_renderer_reopens_assistant_header_after_non_prefix_text_rewrite() {
        let style = CliStyle::plain();
        let mut accumulator = TerminalStreamAccumulator::new();
        let mut state = TerminalSemanticStreamRenderState::default();
        let mut rendered = String::new();

        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "快速".to_string(),
            )),
        );
        rendered.push_str(&render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "快速".to_string(),
            )),
            None,
            &style,
            true,
        ));

        // Simulate an upstream rewrite where the currently accumulated text is
        // no longer prefixed by the previously emitted text.
        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::full(
                OutputMessageRole::Assistant,
                "上升期".to_string(),
            )),
        );
        rendered.push_str(&render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Message(OutputMessageBlock::full(
                OutputMessageRole::Assistant,
                "上升期".to_string(),
            )),
            None,
            &style,
            true,
        ));

        accumulator.apply_output_block(
            Some("assistant-1"),
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "，".to_string(),
            )),
        );
        rendered.push_str(&render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &OutputBlock::Message(OutputMessageBlock::delta(
                OutputMessageRole::Assistant,
                "，".to_string(),
            )),
            None,
            &style,
            true,
        ));

        assert_eq!(
            rendered.matches("[message:assistant]").count(),
            1,
            "{rendered}"
        );
        assert!(rendered.contains("[message:assistant] 快速"), "{rendered}");
        assert!(rendered.contains("上升期"), "{rendered}");
        assert!(rendered.ends_with('，'), "{rendered}");
    }

    #[test]
    fn semantic_stream_renderer_uses_identity_driven_path_for_assistant_snapshot_growth() {
        let style = CliStyle::plain();
        let mut accumulator = TerminalStreamAccumulator::new();
        let mut state = TerminalSemanticStreamRenderState::default();
        let identity = live_identity(
            "assistant-1",
            "text/main",
            LiveMessagePartKind::AssistantText,
            LivePartPhase::Snapshot,
        );

        let first = OutputBlock::Message(OutputMessageBlock::full(
            OutputMessageRole::Assistant,
            "快速".to_string(),
        ));
        accumulator.apply_output_block(Some("assistant-1"), &first);
        let first_rendered = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &first,
            Some(&identity),
            &style,
            true,
        );

        let second = OutputBlock::Message(OutputMessageBlock::full(
            OutputMessageRole::Assistant,
            "快速上升期".to_string(),
        ));
        accumulator.apply_output_block(Some("assistant-1"), &second);
        let second_rendered = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &second,
            Some(&identity),
            &style,
            true,
        );

        assert_eq!(first_rendered, "[message:assistant] 快速");
        assert_eq!(second_rendered, "上升期");
    }

    #[test]
    fn semantic_stream_renderer_uses_identity_driven_reasoning_open_append_and_close() {
        let style = CliStyle::plain();
        let mut accumulator = TerminalStreamAccumulator::new();
        let mut state = TerminalSemanticStreamRenderState::default();
        let identity = live_identity(
            "assistant-1",
            "reasoning/main",
            LiveMessagePartKind::AssistantReasoning,
            LivePartPhase::Snapshot,
        );

        let start = OutputBlock::Reasoning(OutputReasoningBlock::full("thinking".to_string()));
        accumulator.apply_output_block(Some("assistant-1"), &start);
        let start_rendered = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &start,
            Some(&identity),
            &style,
            true,
        );

        let append = OutputBlock::Reasoning(OutputReasoningBlock::full(
            "thinking more".to_string(),
        ));
        accumulator.apply_output_block(Some("assistant-1"), &append);
        let append_rendered = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &append,
            Some(&identity),
            &style,
            true,
        );

        let end_identity = live_identity(
            "assistant-1",
            "reasoning/main",
            LiveMessagePartKind::AssistantReasoning,
            LivePartPhase::End,
        );
        let end = OutputBlock::Reasoning(OutputReasoningBlock::end());
        let end_rendered = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &end,
            Some(&end_identity),
            &style,
            true,
        );

        assert_eq!(start_rendered, "\n[thinking]\n│ thinking");
        assert_eq!(append_rendered, " more");
        assert_eq!(end_rendered, "\n");
    }

    #[test]
    fn semantic_stream_renderer_renders_tool_result_after_identity_completion_action() {
        let style = CliStyle::plain();
        let mut accumulator = TerminalStreamAccumulator::new();
        let mut state = TerminalSemanticStreamRenderState::default();

        let tool_start = OutputBlock::Tool(OutputToolBlock::running(
            "websearch",
            r#"{"query":"青岛天气"}"#,
        ));
        let tool_start_identity = live_identity(
            "assistant-1",
            "tool_call/tool-1",
            LiveMessagePartKind::ToolCall,
            LivePartPhase::Start,
        );
        accumulator.apply_output_block(Some("tool-1"), &tool_start);
        let start_rendered = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &tool_start,
            Some(&tool_start_identity),
            &style,
            true,
        );

        let tool_done = OutputBlock::Tool(OutputToolBlock::done(
            "websearch",
            Some("晴 18C".to_string()),
        ));
        let tool_done_identity = live_identity(
            "assistant-1",
            "tool_result/tool-1",
            LiveMessagePartKind::ToolResult,
            LivePartPhase::End,
        );
        accumulator.apply_output_block(Some("tool-1"), &tool_done);
        let done_rendered = render_terminal_stream_block_semantic(
            &mut state,
            &accumulator,
            &tool_done,
            Some(&tool_done_identity),
            &style,
            true,
        );

        assert!(
            start_rendered.contains("websearch"),
            "tool start should still render through semantic path: {start_rendered}"
        );
        assert!(
            done_rendered.contains("websearch"),
            "tool completion should still render tool header: {done_rendered}"
        );
        assert!(
            done_rendered.contains("晴 18C"),
            "tool completion should not be swallowed by semantic completion action: {done_rendered}"
        );
    }
}
