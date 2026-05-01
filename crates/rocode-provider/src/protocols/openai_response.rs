use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use super::openai_tool_recovery::parse_tool_call_input;
use super::openai_usage::{raw_usage_to_usage, RawUsage};
use crate::{ChatResponse, Choice, Message, ProviderError, Role};

#[derive(Debug, Deserialize)]
pub(super) struct RawChatResponse {
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) choices: Vec<RawChoice>,
    #[serde(default)]
    pub(super) usage: Option<RawUsage>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawChoice {
    #[serde(default)]
    pub(super) index: Option<u32>,
    #[serde(default)]
    pub(super) message: Option<RawMessage>,
    #[serde(default)]
    pub(super) finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawMessage {
    #[serde(default)]
    pub(super) role: Option<String>,
    #[serde(default)]
    pub(super) content: Option<String>,
    #[serde(default)]
    pub(super) tool_calls: Option<Vec<RawToolCall>>,
    #[serde(default, rename = "reasoning_text")]
    pub(super) _reasoning_text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawToolCall {
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(default)]
    pub(super) function: Option<RawFunction>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawFunction {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) arguments: Option<String>,
}

impl RawChatResponse {
    /// Convert the lenient wire format into our internal `ChatResponse`.
    pub(super) fn into_chat_response(self) -> ChatResponse {
        let choices = self
            .choices
            .into_iter()
            .map(|c| {
                let raw_msg = c.message.unwrap_or(RawMessage {
                    role: None,
                    content: None,
                    tool_calls: None,
                    _reasoning_text: None,
                });

                let mut parts: Vec<crate::ContentPart> = Vec::new();

                if let Some(text) = &raw_msg.content {
                    if !text.is_empty() {
                        parts.push(crate::ContentPart {
                            content_type: "text".to_string(),
                            text: Some(text.clone()),
                            ..Default::default()
                        });
                    }
                }

                if let Some(reasoning) = &raw_msg._reasoning_text {
                    if !reasoning.is_empty() {
                        parts.insert(
                            0,
                            crate::ContentPart {
                                content_type: "reasoning".to_string(),
                                text: Some(reasoning.clone()),
                                ..Default::default()
                            },
                        );
                    }
                }

                if let Some(tool_calls) = &raw_msg.tool_calls {
                    for tc in tool_calls {
                        let func = tc.function.as_ref();
                        let name = func.and_then(|f| f.name.as_deref()).unwrap_or("");
                        let args_str = func.and_then(|f| f.arguments.as_deref()).unwrap_or("{}");
                        let input = parse_tool_call_input(name, args_str);
                        let id = tc
                            .id
                            .clone()
                            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                        parts.push(crate::ContentPart {
                            content_type: "tool_use".to_string(),
                            tool_use: Some(crate::ToolUse {
                                id,
                                name: name.to_string(),
                                input,
                            }),
                            ..Default::default()
                        });
                    }
                }

                let content = if parts.is_empty() {
                    crate::Content::Text(raw_msg.content.unwrap_or_default())
                } else if parts.len() == 1 && parts[0].content_type == "text" {
                    crate::Content::Text(parts.remove(0).text.unwrap_or_default())
                } else {
                    crate::Content::Parts(parts)
                };

                Choice {
                    index: c.index.unwrap_or(0),
                    message: Message {
                        role: match raw_msg.role.as_deref() {
                            Some("assistant") | None => Role::Assistant,
                            Some("system") => Role::System,
                            Some("user") => Role::User,
                            Some("tool") => Role::Tool,
                            _ => Role::Assistant,
                        },
                        content,
                        cache_control: None,
                        provider_options: None,
                    },
                    finish_reason: c.finish_reason,
                }
            })
            .collect();

        let usage = self.usage.map(raw_usage_to_usage);

        ChatResponse {
            id: self.id.unwrap_or_default(),
            model: self.model.unwrap_or_default(),
            choices,
            usage,
        }
    }
}

/// Reassemble SSE `data:` chunks into a single `RawChatResponse`.
/// Some closeai-compatible providers return SSE even for non-streaming requests.
pub(super) fn reassemble_sse_chunks(body: &str) -> Result<RawChatResponse, ProviderError> {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut finish_reason: Option<String> = None;
    let mut usage: Option<RawUsage> = None;
    let mut tool_calls: HashMap<u32, (Option<String>, Option<String>, String)> = HashMap::new();

    for line in body.lines() {
        let line = line.trim();
        if !line.starts_with("data:") {
            continue;
        }
        let data = line[5..].trim();
        if data == "[DONE]" {
            break;
        }
        let chunk: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) {
            for choice in choices {
                if let Some(delta) = choice.get("delta") {
                    if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                        content.push_str(text);
                    }
                    let reasoning_val = delta
                        .get("reasoning_content")
                        .or_else(|| delta.get("reasoning_text"))
                        .and_then(|v| v.as_str());
                    if let Some(r) = reasoning_val {
                        reasoning.push_str(r);
                    }
                    if let Some(tcs) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                        for tc in tcs {
                            let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            let entry =
                                tool_calls.entry(idx).or_insert((None, None, String::new()));
                            if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                                entry.0 = Some(id.to_string());
                            }
                            if let Some(func) = tc.get("function") {
                                if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                                    entry.1 = Some(name.to_string());
                                }
                                if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                                    entry.2.push_str(args);
                                }
                            }
                        }
                    }
                }
                if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                    finish_reason = Some(fr.to_string());
                }
            }
        }
        if let Some(u) = chunk.get("usage") {
            usage = serde_json::from_value(u.clone()).ok();
        }
    }

    let raw_tool_calls: Option<Vec<RawToolCall>> = if tool_calls.is_empty() {
        None
    } else {
        let mut sorted: Vec<_> = tool_calls.into_iter().collect();
        sorted.sort_by_key(|(idx, _)| *idx);
        Some(
            sorted
                .into_iter()
                .map(|(_idx, (id, name, args))| RawToolCall {
                    id,
                    function: Some(RawFunction {
                        name,
                        arguments: Some(args),
                    }),
                })
                .collect(),
        )
    };

    Ok(RawChatResponse {
        id: None,
        model: None,
        choices: vec![RawChoice {
            index: Some(0),
            message: Some(RawMessage {
                role: Some("assistant".to_string()),
                content: if content.is_empty() {
                    None
                } else {
                    Some(content)
                },
                tool_calls: raw_tool_calls,
                _reasoning_text: if reasoning.is_empty() {
                    None
                } else {
                    Some(reasoning)
                },
            }),
            finish_reason,
        }],
        usage,
    })
}
