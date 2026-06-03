use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Provider Options
// ---------------------------------------------------------------------------

/// Include values for the Responses API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResponsesIncludeValue {
    #[serde(rename = "web_search_call.action.sources")]
    WebSearchCallActionSources,
    #[serde(rename = "code_interpreter_call.outputs")]
    CodeInterpreterCallOutputs,
    #[serde(rename = "computer_call_output.output.image_url")]
    ComputerCallOutputImageUrl,
    #[serde(rename = "file_search_call.results")]
    FileSearchCallResults,
    #[serde(rename = "message.input_image.image_url")]
    MessageInputImageUrl,
    #[serde(rename = "message.output_text.logprobs")]
    MessageOutputTextLogprobs,
    #[serde(rename = "reasoning.encrypted_content")]
    ReasoningEncryptedContent,
}

/// Service tier options.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServiceTier {
    Auto,
    Flex,
    Priority,
}

/// Text verbosity options.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TextVerbosity {
    Low,
    Medium,
    High,
}

/// Reasoning effort levels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

/// Reasoning summary mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningSummary {
    Auto,
    Concise,
    Detailed,
}

/// Provider-specific options for the OpenAI Responses API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ResponsesProviderOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<ResponsesIncludeValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Return log probabilities. `true` = max (20), or a number 1..=20.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<LogprobsSetting>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_calls: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<ServiceTier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict_json_schema: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_verbosity: Option<TextVerbosity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

/// Logprobs can be `true` (use max=20) or a specific number 1..=20.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LogprobsSetting {
    Enabled(bool),
    TopN(u8),
}

impl LogprobsSetting {
    pub fn top_logprobs(&self) -> Option<u8> {
        match self {
            LogprobsSetting::Enabled(true) => Some(TOP_LOGPROBS_MAX),
            LogprobsSetting::Enabled(false) => None,
            LogprobsSetting::TopN(n) => Some(*n),
        }
    }
}

pub const TOP_LOGPROBS_MAX: u8 = 20;

// ---------------------------------------------------------------------------
// Model Configuration
// ---------------------------------------------------------------------------

/// Determines model capabilities based on model ID.
#[derive(Debug, Clone)]
pub struct ResponsesModelConfig {
    pub is_reasoning_model: bool,
    pub system_message_mode: SystemMessageMode,
    pub required_auto_truncation: bool,
    pub supports_flex_processing: bool,
    pub supports_priority_processing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemMessageMode {
    System,
    Developer,
    Remove,
}

/// Determine model capabilities from model ID string.
/// Mirrors the TS `getResponsesModelConfig()`.
pub fn get_responses_model_config(model_id: &str) -> ResponsesModelConfig {
    let supports_flex = model_id.starts_with("o3")
        || model_id.starts_with("o4-mini")
        || (model_id.starts_with("gpt-5") && !model_id.starts_with("gpt-5-chat"));

    let supports_priority = model_id.starts_with("gpt-4")
        || model_id.starts_with("gpt-5-mini")
        || (model_id.starts_with("gpt-5")
            && !model_id.starts_with("gpt-5-nano")
            && !model_id.starts_with("gpt-5-chat"))
        || model_id.starts_with("o3")
        || model_id.starts_with("o4-mini");

    let defaults = ResponsesModelConfig {
        is_reasoning_model: false,
        system_message_mode: SystemMessageMode::System,
        required_auto_truncation: false,
        supports_flex_processing: supports_flex,
        supports_priority_processing: supports_priority,
    };

    // gpt-5-chat models are non-reasoning
    if model_id.starts_with("gpt-5-chat") {
        return defaults;
    }

    // o series reasoning models, gpt-5, codex-, computer-use
    if model_id.starts_with('o')
        || model_id.starts_with("gpt-5")
        || model_id.starts_with("codex-")
        || model_id.starts_with("computer-use")
    {
        if model_id.starts_with("o1-mini") || model_id.starts_with("o1-preview") {
            return ResponsesModelConfig {
                is_reasoning_model: true,
                system_message_mode: SystemMessageMode::Remove,
                ..defaults
            };
        }
        return ResponsesModelConfig {
            is_reasoning_model: true,
            system_message_mode: SystemMessageMode::Developer,
            ..defaults
        };
    }

    // gpt models (non-reasoning)
    defaults
}

// ---------------------------------------------------------------------------
// Finish Reason Mapping
// ---------------------------------------------------------------------------

/// Maps OpenAI Responses API incomplete_details.reason to a finish reason.
/// Mirrors TS `mapOpenAIResponseFinishReason()`.
pub fn map_openai_response_finish_reason(
    finish_reason: Option<&str>,
    has_function_call: bool,
) -> FinishReason {
    match finish_reason {
        None => {
            if has_function_call {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }
        }
        Some("max_output_tokens") => FinishReason::Length,
        Some("content_filter") => FinishReason::ContentFilter,
        Some(_) => {
            if has_function_call {
                FinishReason::ToolCalls
            } else {
                FinishReason::Unknown
            }
        }
    }
}

/// Maps OpenAI Compatible chat finish_reason strings.
/// Mirrors TS `mapOpenAICompatibleFinishReason()`.
pub fn map_openai_compatible_finish_reason(finish_reason: Option<&str>) -> FinishReason {
    match finish_reason {
        Some("stop") => FinishReason::Stop,
        Some("length") => FinishReason::Length,
        Some("content_filter") => FinishReason::ContentFilter,
        Some("function_call") | Some("tool_calls") => FinishReason::ToolCalls,
        _ => FinishReason::Unknown,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
    Error,
    Unknown,
}

// ---------------------------------------------------------------------------
// Responses API Input Types
// ---------------------------------------------------------------------------

/// Input items for the Responses API request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesInputItem {
    FunctionCall(ResponsesFunctionCall),
    FunctionCallOutput(ResponsesFunctionCallOutput),
    LocalShellCall(ResponsesLocalShellCall),
    LocalShellCallOutput(ResponsesLocalShellCallOutput),
    Reasoning(ResponsesReasoning),
    ItemReference(ResponsesItemReference),
    /// System or developer message (role-based, not type-tagged).
    #[serde(untagged)]
    RoleMessage(ResponsesRoleMessage),
}

/// We need a custom serialization approach since the input items mix
/// tagged and untagged variants. Use `serde_json::Value` as the wire type.
pub type ResponsesInput = Vec<serde_json::Value>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesRoleMessage {
    pub role: String, // "system" | "developer" | "user" | "assistant"
    pub content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesFunctionCall {
    #[serde(rename = "type")]
    pub item_type: String, // "function_call"
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesFunctionCallOutput {
    #[serde(rename = "type")]
    pub item_type: String, // "function_call_output"
    pub call_id: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesLocalShellCall {
    #[serde(rename = "type")]
    pub item_type: String, // "local_shell_call"
    pub id: Option<String>,
    pub call_id: String,
    pub action: LocalShellAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalShellAction {
    #[serde(rename = "type")]
    pub action_type: String, // "exec"
    pub command: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesLocalShellCallOutput {
    #[serde(rename = "type")]
    pub item_type: String, // "local_shell_call_output"
    pub call_id: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesReasoning {
    #[serde(rename = "type")]
    pub item_type: String, // "reasoning"
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
    pub summary: Vec<ReasoningSummaryText>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningSummaryText {
    #[serde(rename = "type")]
    pub text_type: String, // "summary_text"
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesItemReference {
    #[serde(rename = "type")]
    pub item_type: String, // "item_reference"
    pub id: String,
}

// ---------------------------------------------------------------------------
// Responses API Response / Output Types
// ---------------------------------------------------------------------------

/// Usage information from the Responses API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponsesUsage {
    pub input_tokens: u64,
    #[serde(default)]
    pub input_tokens_details: Option<InputTokensDetails>,
    pub output_tokens: u64,
    #[serde(default)]
    pub output_tokens_details: Option<OutputTokensDetails>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InputTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutputTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
}

// ---------------------------------------------------------------------------
// Streaming Chunk Types (12+ discriminated types)
// ---------------------------------------------------------------------------

/// All possible chunk types from the Responses API SSE stream.
/// Mirrors the TS `openaiResponsesChunkSchema` union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponsesStreamChunk {
    /// `response.output_text.delta`
    #[serde(rename = "response.output_text.delta")]
    TextDelta {
        item_id: String,
        delta: String,
        #[serde(default)]
        logprobs: Option<Vec<LogprobEntry>>,
    },

    /// `response.created`
    #[serde(rename = "response.created")]
    ResponseCreated { response: ResponseCreatedData },

    /// `response.completed` or `response.incomplete`
    #[serde(rename = "response.completed")]
    ResponseCompleted { response: ResponseFinishedData },
    #[serde(rename = "response.incomplete")]
    ResponseIncomplete { response: ResponseFinishedData },

    /// `response.output_item.added`
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        output_index: usize,
        item: OutputItemAddedItem,
    },

    /// `response.output_item.done`
    #[serde(rename = "response.output_item.done")]
    OutputItemDone {
        output_index: usize,
        item: OutputItemDoneItem,
    },

    /// `response.function_call_arguments.delta`
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta {
        item_id: String,
        output_index: usize,
        delta: String,
    },

    /// `response.image_generation_call.partial_image`
    #[serde(rename = "response.image_generation_call.partial_image")]
    ImageGenerationPartialImage {
        item_id: String,
        output_index: usize,
        partial_image_b64: String,
    },

    /// `response.code_interpreter_call_code.delta`
    #[serde(rename = "response.code_interpreter_call_code.delta")]
    CodeInterpreterCodeDelta {
        item_id: String,
        output_index: usize,
        delta: String,
    },

    /// `response.code_interpreter_call_code.done`
    #[serde(rename = "response.code_interpreter_call_code.done")]
    CodeInterpreterCodeDone {
        item_id: String,
        output_index: usize,
        code: String,
    },

    /// `response.output_text.annotation.added`
    #[serde(rename = "response.output_text.annotation.added")]
    AnnotationAdded { annotation: AnnotationItem },

    /// `response.reasoning_summary_part.added`
    #[serde(rename = "response.reasoning_summary_part.added")]
    ReasoningSummaryPartAdded {
        item_id: String,
        summary_index: usize,
    },

    /// `response.reasoning_summary_text.delta`
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryTextDelta {
        item_id: String,
        summary_index: usize,
        delta: String,
    },

    /// `error`
    #[serde(rename = "error")]
    Error {
        code: String,
        message: String,
        #[serde(default)]
        param: Option<String>,
        #[serde(default)]
        sequence_number: Option<u64>,
    },

    /// Fallback for unknown chunk types.
    #[serde(other)]
    Unknown,
}

// ---------------------------------------------------------------------------
// Sub-types for streaming chunks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogprobEntry {
    pub token: String,
    pub logprob: f64,
    pub top_logprobs: Vec<TopLogprob>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopLogprob {
    pub token: String,
    pub logprob: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseCreatedData {
    pub id: String,
    pub created_at: u64,
    pub model: String,
    #[serde(default)]
    pub service_tier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFinishedData {
    #[serde(default)]
    pub incomplete_details: Option<IncompleteDetails>,
    pub usage: ResponsesUsage,
    #[serde(default)]
    pub service_tier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncompleteDetails {
    pub reason: String,
}

/// Items that can appear in `response.output_item.added`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutputItemAddedItem {
    #[serde(rename = "message")]
    Message { id: String },
    #[serde(rename = "reasoning")]
    Reasoning {
        id: String,
        #[serde(default)]
        encrypted_content: Option<String>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
    },
    #[serde(rename = "web_search_call")]
    WebSearchCall {
        id: String,
        status: String,
        #[serde(default)]
        action: Option<serde_json::Value>,
    },
    #[serde(rename = "computer_call")]
    ComputerCall { id: String, status: String },
    #[serde(rename = "file_search_call")]
    FileSearchCall { id: String },
    #[serde(rename = "image_generation_call")]
    ImageGenerationCall { id: String },
    #[serde(rename = "code_interpreter_call")]
    CodeInterpreterCall {
        id: String,
        container_id: String,
        #[serde(default)]
        code: Option<String>,
        #[serde(default)]
        outputs: Option<Vec<CodeInterpreterOutput>>,
        #[serde(default)]
        status: Option<String>,
    },
}

/// Items that can appear in `response.output_item.done`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutputItemDoneItem {
    #[serde(rename = "message")]
    Message { id: String },
    #[serde(rename = "reasoning")]
    Reasoning {
        id: String,
        #[serde(default)]
        encrypted_content: Option<String>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
        #[serde(default)]
        status: Option<String>,
    },
    #[serde(rename = "web_search_call")]
    WebSearchCall {
        id: String,
        status: String,
        #[serde(default)]
        action: Option<serde_json::Value>,
    },
    #[serde(rename = "file_search_call")]
    FileSearchCall {
        id: String,
        #[serde(default)]
        queries: Option<Vec<String>>,
        #[serde(default)]
        results: Option<Vec<FileSearchResult>>,
    },
    #[serde(rename = "code_interpreter_call")]
    CodeInterpreterCall {
        id: String,
        #[serde(default)]
        code: Option<String>,
        container_id: String,
        #[serde(default)]
        outputs: Option<Vec<CodeInterpreterOutput>>,
    },
    #[serde(rename = "image_generation_call")]
    ImageGenerationCall { id: String, result: String },
    #[serde(rename = "local_shell_call")]
    LocalShellCall {
        id: String,
        call_id: String,
        action: LocalShellAction,
    },
    #[serde(rename = "computer_call")]
    ComputerCall {
        id: String,
        #[serde(default)]
        status: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CodeInterpreterOutput {
    #[serde(rename = "logs")]
    Logs { logs: String },
    #[serde(rename = "image")]
    Image { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchResult {
    #[serde(default)]
    pub attributes: HashMap<String, serde_json::Value>,
    pub file_id: String,
    pub filename: String,
    pub score: f64,
    pub text: String,
}

/// Annotation types from `response.output_text.annotation.added`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnnotationItem {
    #[serde(rename = "url_citation")]
    UrlCitation { url: String, title: String },
    #[serde(rename = "file_citation")]
    FileCitation {
        file_id: String,
        #[serde(default)]
        filename: Option<String>,
        #[serde(default)]
        index: Option<u64>,
        #[serde(default)]
        start_index: Option<u64>,
        #[serde(default)]
        end_index: Option<u64>,
        #[serde(default)]
        quote: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Streaming State Management
// ---------------------------------------------------------------------------

/// Tracks state for an ongoing tool call during streaming.
#[derive(Debug, Clone)]
pub struct OngoingToolCall {
    pub tool_name: String,
    pub tool_call_id: String,
    pub code_interpreter: Option<CodeInterpreterState>,
}

#[derive(Debug, Clone)]
pub struct CodeInterpreterState {
    pub container_id: String,
}

/// Tracks active reasoning by output_index.
/// GitHub Copilot rotates encrypted item IDs on every event,
/// so we track by output_index instead of item_id.
#[derive(Debug, Clone)]
pub struct ActiveReasoning {
    /// The item.id from output_item.added
    pub canonical_id: String,
    pub encrypted_content: Option<String>,
    pub summary_parts: Vec<usize>,
}

// ---------------------------------------------------------------------------
// Metadata Extractor Trait
// ---------------------------------------------------------------------------

/// Extracts provider-specific metadata from API responses.
/// Mirrors the TS `MetadataExtractor` type.
pub trait MetadataExtractor: Send + Sync {
    /// Extract metadata from a complete (non-streaming) response body.
    fn extract_metadata(
        &self,
        parsed_body: &serde_json::Value,
    ) -> Option<HashMap<String, serde_json::Value>>;

    /// Create a stream extractor for processing chunks.
    fn create_stream_extractor(&self) -> Box<dyn StreamMetadataExtractor>;
}

/// Processes individual chunks and builds final metadata from accumulated stream data.
pub trait StreamMetadataExtractor: Send + Sync {
    fn process_chunk(&mut self, parsed_chunk: &serde_json::Value);
    fn build_metadata(&self) -> Option<HashMap<String, serde_json::Value>>;
}

// ---------------------------------------------------------------------------
// Response Metadata
// ---------------------------------------------------------------------------

/// Metadata extracted from a response, including token details.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Vec<Vec<LogprobEntry>>>,
}

/// Completion token details (reasoning, prediction accepted/rejected).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompletionTokenDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
    #[serde(default)]
    pub accepted_prediction_tokens: Option<u64>,
    #[serde(default)]
    pub rejected_prediction_tokens: Option<u64>,
}

/// Prompt token details (cached tokens).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptTokenDetails {
    #[serde(default)]
    pub cached_tokens: Option<u64>,
}

/// Get response metadata from a chat completion response.
/// Mirrors TS `getResponseMetadata()`.
pub fn get_response_metadata(
    id: Option<&str>,
    model: Option<&str>,
    created: Option<u64>,
) -> ResponseMetadata {
    ResponseMetadata {
        response_id: id.map(|s| s.to_string()),
        model_id: model.map(|s| s.to_string()),
        timestamp: created,
        service_tier: None,
        logprobs: None,
    }
}
