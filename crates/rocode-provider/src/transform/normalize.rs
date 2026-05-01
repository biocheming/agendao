use std::collections::HashMap;
use std::fmt;

use crate::{CacheControl, Content, ContentPart, Message};
use serde::{Deserialize, Serialize};

use super::model_config::remap_provider_options;
use crate::cache::{plan_ethnopic_message_breakpoints, EthnopicCachePolicy, EthnopicCacheTtl};

#[derive(Debug, Clone, Copy)]
pub enum ProviderType {
    Ethnopic,
    OpenRouter,
    Bedrock,
    OpenAI,
    Gateway,
    Other,
}

impl ProviderType {
    pub fn from_provider_id(id: &str) -> Self {
        let id_lower = id.to_lowercase();
        if is_ethnopic_compatible_hint(&id_lower) {
            ProviderType::Ethnopic
        } else if id_lower == "openrouter" {
            ProviderType::OpenRouter
        } else if id_lower == "bedrock" || id_lower.contains("bedrock") {
            ProviderType::Bedrock
        } else if id_lower == "gateway" {
            ProviderType::Gateway
        } else if id_lower == "openai" {
            ProviderType::OpenAI
        } else {
            ProviderType::Other
        }
    }

    pub fn supports_caching(&self) -> bool {
        matches!(
            self,
            ProviderType::Ethnopic
                | ProviderType::OpenRouter
                | ProviderType::Bedrock
                | ProviderType::Gateway
        )
    }

    pub fn supports_interleaved_thinking(&self) -> bool {
        matches!(self, ProviderType::Ethnopic | ProviderType::OpenRouter)
    }
}

pub(crate) fn is_ethnopic_compatible_hint(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.contains("anthropic") || lower.contains("ethnopic")
}

pub(crate) fn is_ethnopic_compatible_npm(npm: &str) -> bool {
    let lower = npm.trim().to_ascii_lowercase();
    lower.contains("anthropic") || lower.contains("ethnopic")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningContent {
    pub text: String,
    pub signature: Option<String>,
}

/// The TS source uses `Flag.OPENCODE_EXPERIMENTAL_OUTPUT_TOKEN_MAX || 32_000`.
/// We default to 32_000 to match the TS constant.
pub const OUTPUT_TOKEN_MAX: u64 = 32_000;

pub(super) const WIDELY_SUPPORTED_EFFORTS: &[&str] = &["low", "medium", "high"];
pub(super) const OPENAI_EFFORTS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh"];

/// Maps model ID prefix to provider slug used in providerOptions.
/// Examples:
/// - "amazon/nova-2-lite" -> "bedrock"
/// - "ethnopic-compatible/..." -> "ethnopic"
const SLUG_OVERRIDES: &[(&str, &str)] = &[
    ("amazon", "bedrock"),
    ("ethnopic-compatible", "ethnopic"),
    ("closeai-compatible", "openai"),
    ("openai-compatible", "openai"),
];

pub(super) fn slug_override(key: &str) -> Option<&'static str> {
    SLUG_OVERRIDES
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
}

// ---------------------------------------------------------------------------
// dedup_messages
// ---------------------------------------------------------------------------

/// Remove consecutive duplicate messages (same role and text content).
/// This prevents redundant cache control markers and wasted tokens.
pub fn dedup_messages(messages: &mut Vec<Message>) {
    messages.dedup_by(|b, a| {
        if std::mem::discriminant(&a.role) != std::mem::discriminant(&b.role) {
            return false;
        }
        match (&a.content, &b.content) {
            (Content::Text(t1), Content::Text(t2)) => t1 == t2,
            _ => false,
        }
    });
}

// ---------------------------------------------------------------------------
// apply_caching
// ---------------------------------------------------------------------------

pub fn apply_caching(messages: &mut [Message], provider_type: ProviderType) {
    if !provider_type.supports_caching() {
        return;
    }

    apply_caching_with_policy(messages, provider_type, &EthnopicCachePolicy::default());
}

pub fn apply_caching_with_policy(
    messages: &mut [Message],
    provider_type: ProviderType,
    policy: &EthnopicCachePolicy,
) {
    if !provider_type.supports_caching() || !policy.enabled {
        return;
    }

    let plan = plan_ethnopic_message_breakpoints(messages);
    for idx in plan.message_indices() {
        if let Some(msg) = messages.get_mut(idx) {
            apply_cache_to_message(msg, provider_type, policy);
        }
    }
}

fn apply_cache_to_message(
    message: &mut Message,
    provider_type: ProviderType,
    policy: &EthnopicCachePolicy,
) {
    // TS applyCaching uses providerOptions with multiple provider keys merged via mergeDeep.
    // We replicate that by setting provider_options on the message or its last content part.
    let provider_opts = build_cache_provider_options(policy);

    let provider_id_str = match provider_type {
        ProviderType::Ethnopic => "ethnopic",
        ProviderType::Bedrock => "bedrock",
        _ => "",
    };
    let use_message_level = provider_id_str == "ethnopic" || provider_id_str.contains("bedrock");

    if !use_message_level {
        if let Content::Parts(parts) = &mut message.content {
            if let Some(last_part) = parts.last_mut() {
                let existing = last_part.provider_options.get_or_insert_with(HashMap::new);
                merge_deep_into(existing, &provider_opts);
                return;
            }
        }
    }

    // Fall back to message-level providerOptions
    let existing = message.provider_options.get_or_insert_with(HashMap::new);
    merge_deep_into(existing, &provider_opts);
}

fn build_cache_provider_options(
    policy: &EthnopicCachePolicy,
) -> HashMap<String, serde_json::Value> {
    use serde_json::json;
    let mut opts = HashMap::new();
    let cache_control = cache_control_provider_value(policy);
    opts.insert(
        "ethnopic".to_string(),
        json!({"cacheControl": cache_control.clone()}),
    );
    opts.insert(
        "openrouter".to_string(),
        json!({"cacheControl": cache_control.clone()}),
    );
    opts.insert(
        "bedrock".to_string(),
        json!({"cachePoint": {"type": "default"}}),
    );
    opts.insert(
        "openaiCompatible".to_string(),
        json!({"cache_control": cache_control.clone()}),
    );
    opts.insert(
        "copilot".to_string(),
        json!({"copilot_cache_control": cache_control}),
    );
    opts
}

fn cache_control_provider_value(policy: &EthnopicCachePolicy) -> serde_json::Value {
    use serde_json::{json, Map, Value};
    let mut cache_control = Map::new();
    cache_control.insert("type".to_string(), json!("ephemeral"));
    if policy.ttl == EthnopicCacheTtl::OneHour {
        cache_control.insert("ttl".to_string(), json!("1h"));
    }
    if policy.global_scope {
        cache_control.insert("scope".to_string(), json!("global"));
    }
    Value::Object(cache_control)
}

/// Deep-merge `source` into `target`. For nested JSON objects, recurse; otherwise overwrite.
fn merge_deep_into(
    target: &mut HashMap<String, serde_json::Value>,
    source: &HashMap<String, serde_json::Value>,
) {
    for (k, v) in source {
        if let Some(existing) = target.get_mut(k) {
            if let (Some(existing_obj), Some(new_obj)) = (existing.as_object_mut(), v.as_object()) {
                let mut sub_target: HashMap<String, serde_json::Value> = existing_obj
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                let sub_source: HashMap<String, serde_json::Value> = new_obj
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                merge_deep_into(&mut sub_target, &sub_source);
                *existing = serde_json::Value::Object(sub_target.into_iter().collect());
                continue;
            }
        }
        target.insert(k.clone(), v.clone());
    }
}

// ---------------------------------------------------------------------------
// normalize_messages_for_caching
// ---------------------------------------------------------------------------

pub fn normalize_messages_for_caching(messages: &mut Vec<Message>) {
    messages.retain(|msg| {
        if !matches!(msg.role, crate::Role::Assistant) {
            return true;
        }
        match &msg.content {
            Content::Text(text) => !text.is_empty(),
            Content::Parts(parts) => parts.iter().any(|p| {
                p.text.as_ref().is_some_and(|t| !t.is_empty())
                    || p.tool_use.is_some()
                    || p.tool_result.is_some()
            }),
        }
    });
}

// ---------------------------------------------------------------------------
// apply_interleaved_thinking
// ---------------------------------------------------------------------------

pub fn apply_interleaved_thinking(messages: &mut [Message], provider_type: ProviderType) {
    if !provider_type.supports_interleaved_thinking() {
        return;
    }

    // Reasoning parts are preserved in the message content so that the
    // provider can convert them to the appropriate format (e.g. ethnopic
    // `thinking` blocks).  We only apply cache control hints here.
    for msg in messages.iter_mut() {
        if matches!(msg.role, crate::Role::Assistant) {
            if let Content::Parts(parts) = &mut msg.content {
                let has_reasoning = parts.iter().any(|p| p.content_type == "reasoning");
                if has_reasoning {
                    // Mark the last non-reasoning part with ephemeral cache control
                    if let Some(part) = parts
                        .iter_mut()
                        .rev()
                        .find(|p| p.content_type != "reasoning")
                    {
                        part.cache_control = Some(CacheControl::ephemeral());
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// extract_reasoning_from_response
// ---------------------------------------------------------------------------

pub fn extract_reasoning_from_response(content: &str) -> (Option<String>, String) {
    let thinking_start = content.find("<thinking>");
    let thinking_end = content.find("</thinking>");

    match (thinking_start, thinking_end) {
        (Some(start), Some(end)) if end > start => {
            let reasoning = content[start + 10..end].trim().to_string();
            let rest = format!("{}{}", content[..start].trim(), content[end + 11..].trim());
            (Some(reasoning), rest)
        }
        _ => (None, content.to_string()),
    }
}

// ---------------------------------------------------------------------------
// normalize_messages
// ---------------------------------------------------------------------------

pub fn normalize_messages(
    messages: &mut Vec<Message>,
    provider_type: ProviderType,
    model_id: &str,
    ethnopic_protocol: bool,
) {
    match provider_type {
        ProviderType::Ethnopic => {
            normalize_for_ethnopic_messages(messages);
            normalize_tool_call_ids_ethnopic_family(messages);
        }
        ProviderType::OpenRouter => {
            if ethnopic_protocol {
                normalize_tool_call_ids_ethnopic_family(messages);
            }
            if model_id.to_lowercase().contains("mistral")
                || model_id.to_lowercase().contains("devstral")
            {
                normalize_for_mistral(messages);
            }
        }
        ProviderType::Other => {
            if ethnopic_protocol {
                normalize_tool_call_ids_ethnopic_family(messages);
            } else if model_id.to_lowercase().contains("mistral")
                || model_id.to_lowercase().contains("devstral")
            {
                normalize_for_mistral(messages);
            }
        }
        _ => {}
    }

    // Handle interleaved thinking field (move reasoning to providerOptions)
    normalize_interleaved_field(messages, model_id);
}

/// For models with interleaved thinking that use a specific field
/// (reasoning_content or reasoning_details), move reasoning parts
/// from content into providerOptions.openaiCompatible.<field>.
fn normalize_interleaved_field(_messages: &mut Vec<Message>, _model_id: &str) {
    // This is handled at a higher level via the ModelInfo.interleaved field.
    // The caller should check model.interleaved and pass the field name.
    // For now this is a no-op; the full implementation is in
    // normalize_messages_with_interleaved_field below.
}

/// Normalize messages for models that store reasoning in a specific provider field.
/// Matches the TS: `if (typeof model.capabilities.interleaved === "object" && model.capabilities.interleaved.field)`
pub fn normalize_messages_with_interleaved_field(messages: &mut [Message], field: &str) {
    use serde_json::json;
    for msg in messages.iter_mut() {
        if !matches!(msg.role, crate::Role::Assistant) {
            continue;
        }
        if let Content::Parts(parts) = &mut msg.content {
            let reasoning_text: String = parts
                .iter()
                .filter(|p| p.content_type == "reasoning")
                .filter_map(|p| p.text.as_ref())
                .cloned()
                .collect::<Vec<_>>()
                .join("");

            parts.retain(|p| p.content_type != "reasoning");

            if !reasoning_text.is_empty() {
                let po = msg.provider_options.get_or_insert_with(HashMap::new);
                let compat = po
                    .entry("openaiCompatible".to_string())
                    .or_insert_with(|| json!({}));
                if let Some(obj) = compat.as_object_mut() {
                    obj.insert(field.to_string(), json!(reasoning_text));
                }
            }
        }
    }
}

fn normalize_for_ethnopic_messages(messages: &mut Vec<Message>) {
    // Filter out messages with empty content
    messages.retain(|msg| match &msg.content {
        Content::Text(text) => !text.is_empty(),
        Content::Parts(parts) => parts.iter().any(|p| {
            if p.content_type == "text" || p.content_type == "reasoning" {
                p.text.as_ref().map(|t| !t.is_empty()).unwrap_or(false)
            } else {
                true
            }
        }),
    });

    // Filter out empty text/reasoning parts within messages
    for msg in messages.iter_mut() {
        if let Content::Parts(parts) = &mut msg.content {
            parts.retain(|p| {
                if p.content_type == "text" || p.content_type == "reasoning" {
                    p.text.as_ref().map(|t| !t.is_empty()).unwrap_or(false)
                } else {
                    true
                }
            });
        }
    }
}

fn normalize_tool_call_ids_ethnopic_family(messages: &mut [Message]) {
    for msg in messages.iter_mut() {
        if matches!(msg.role, crate::Role::Assistant | crate::Role::Tool) {
            if let Content::Parts(parts) = &mut msg.content {
                for part in parts.iter_mut() {
                    if let Some(ref mut tool_use) = part.tool_use {
                        tool_use.id = normalize_tool_call_id(&tool_use.id, true);
                    }
                    if let Some(ref mut tool_result) = part.tool_result {
                        tool_result.tool_use_id =
                            normalize_tool_call_id(&tool_result.tool_use_id, true);
                    }
                }
            }
        }
    }
}

fn normalize_for_mistral(messages: &mut Vec<Message>) {
    for msg in messages.iter_mut() {
        if matches!(msg.role, crate::Role::Assistant | crate::Role::Tool) {
            if let Content::Parts(parts) = &mut msg.content {
                for part in parts.iter_mut() {
                    if let Some(ref mut tool_use) = part.tool_use {
                        tool_use.id = normalize_tool_call_id_mistral(&tool_use.id);
                    }
                    if let Some(ref mut tool_result) = part.tool_result {
                        tool_result.tool_use_id =
                            normalize_tool_call_id_mistral(&tool_result.tool_use_id);
                    }
                }
            }
        }
    }

    let mut i = 0;
    while i < messages.len().saturating_sub(1) {
        let current_is_tool = matches!(messages[i].role, crate::Role::Tool);
        let next_is_user = matches!(messages[i + 1].role, crate::Role::User);

        if current_is_tool && next_is_user {
            messages.insert(i + 1, Message::assistant("Done."));
        }
        i += 1;
    }
}

pub(super) fn normalize_tool_call_id(id: &str, allow_underscore: bool) -> String {
    if allow_underscore {
        id.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    } else {
        id.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    }
}

pub(super) fn normalize_tool_call_id_mistral(id: &str) -> String {
    let alphanumeric: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let first_9: String = alphanumeric.chars().take(9).collect();
    format!("{:0<9}", first_9)
}

// ---------------------------------------------------------------------------
// Modality
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Modality {
    Image,
    Audio,
    Video,
    Pdf,
}

impl fmt::Display for Modality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Modality::Image => write!(f, "image"),
            Modality::Audio => write!(f, "audio"),
            Modality::Video => write!(f, "video"),
            Modality::Pdf => write!(f, "pdf"),
        }
    }
}

pub fn mime_to_modality(mime: &str) -> Option<Modality> {
    if mime.starts_with("image/") {
        Some(Modality::Image)
    } else if mime.starts_with("audio/") {
        Some(Modality::Audio)
    } else if mime.starts_with("video/") {
        Some(Modality::Video)
    } else if mime == "application/pdf" {
        Some(Modality::Pdf)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// unsupported_parts
// ---------------------------------------------------------------------------

pub fn unsupported_parts(messages: &mut [Message], supported_modalities: &[Modality]) {
    for msg in messages.iter_mut() {
        if !matches!(msg.role, crate::Role::User) {
            continue;
        }

        if let Content::Parts(parts) = &mut msg.content {
            for part in parts.iter_mut() {
                if part.content_type != "image" && part.content_type != "file" {
                    continue;
                }

                // Check for empty base64 image data
                if part.content_type == "image" {
                    if let Some(ref image_url) = part.image_url {
                        let url_str = &image_url.url;
                        if url_str.starts_with("data:") {
                            // Match data:<mime>;base64,<data>
                            if let Some(comma_pos) = url_str.find(',') {
                                let data_part = &url_str[comma_pos + 1..];
                                if data_part.is_empty() {
                                    *part = ContentPart {
                                        content_type: "text".to_string(),
                                        text: Some("ERROR: Image file is empty or corrupted. Please provide a valid image.".to_string()),
                                        ..Default::default()
                                    };
                                    continue;
                                }
                            }
                        }
                    }
                }

                let mime = if part.content_type == "image" {
                    part.image_url
                        .as_ref()
                        .and_then(|url| {
                            let url_str = url.url.as_str();
                            if url_str.starts_with("data:") {
                                url_str.split(';').next()
                            } else {
                                None
                            }
                        })
                        .map(|s| s.trim_start_matches("data:").to_string())
                        .unwrap_or_default()
                } else {
                    // For file parts, use media_type field
                    part.media_type.clone().unwrap_or_default()
                };

                if let Some(modality) = mime_to_modality(&mime) {
                    if !supported_modalities.contains(&modality) {
                        // Extract filename for error message
                        let name = if let Some(ref filename) = part.filename {
                            format!("\"{}\"", filename)
                        } else {
                            modality.to_string()
                        };
                        let error_msg = format!(
                            "ERROR: Cannot read {} (this model does not support {} input). Inform the user.",
                            name, modality
                        );
                        *part = ContentPart {
                            content_type: "text".to_string(),
                            text: Some(error_msg),
                            ..Default::default()
                        };
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// temperature / topP / topK
// ---------------------------------------------------------------------------

pub fn temperature_for_model(model_id: &str) -> Option<f32> {
    let id = model_id.to_lowercase();
    if id.contains("qwen") {
        return Some(0.55);
    }
    if id.contains("gemini") {
        return Some(1.0);
    }
    if id.contains("glm-4.6") || id.contains("glm-4.7") {
        return Some(1.0);
    }
    if id.contains("minimax-m2") {
        return Some(1.0);
    }
    if id.contains("kimi-k2") {
        if id.contains("thinking") || id.contains("k2.") || id.contains("k2p") {
            return Some(1.0);
        }
        return Some(0.6);
    }
    None
}

pub fn top_p_for_model(model_id: &str) -> Option<f32> {
    let id = model_id.to_lowercase();
    if id.contains("qwen") {
        return Some(1.0);
    }
    if id.contains("minimax-m2")
        || id.contains("kimi-k2.5")
        || id.contains("kimi-k2p5")
        || id.contains("gemini")
    {
        return Some(0.95);
    }
    None
}

pub fn top_k_for_model(model_id: &str) -> Option<u32> {
    let id = model_id.to_lowercase();
    if id.contains("minimax-m2") {
        if id.contains("m2.1") {
            return Some(40);
        }
        return Some(20);
    }
    if id.contains("gemini") {
        return Some(64);
    }
    None
}

// ---------------------------------------------------------------------------
// transform_messages (top-level entry point matching TS `message()`)
// ---------------------------------------------------------------------------

pub fn transform_messages(
    messages: &mut Vec<Message>,
    provider_type: ProviderType,
    model_id: &str,
    supported_modalities: &[Modality],
    npm: &str,
    provider_id: &str,
) {
    unsupported_parts(messages, supported_modalities);

    let id_lower = model_id.to_lowercase();
    let pid_lower = provider_id.to_lowercase();
    let is_ethnopic_like = is_ethnopic_compatible_hint(&pid_lower)
        || is_ethnopic_compatible_hint(&id_lower)
        || is_ethnopic_compatible_npm(npm);

    normalize_messages(messages, provider_type, model_id, is_ethnopic_like);

    // Apply caching for providers/models that follow the ethnopic-compatible
    // family, plus the existing bedrock path, but not gateway.
    if is_ethnopic_like && npm != "@ai-sdk/gateway" {
        apply_caching(messages, provider_type);
    }

    // Remap providerOptions keys from stored providerID to expected SDK key
    remap_provider_options(messages, npm, provider_id);
}
