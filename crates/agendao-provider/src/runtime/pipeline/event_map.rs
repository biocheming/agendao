use crate::driver::StreamingEvent;
use crate::protocol_loader::{ProtocolManifest, StreamingConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MapperKind {
    OpenAi,
    Ethnopic,
    Google,
    Vertex,
}

fn is_ethnopic_compatible_id(id: &str) -> bool {
    let lower = id.trim().to_ascii_lowercase();
    lower.contains("anthropic") || lower.contains("ethnopic")
}

/// Path-driven event mapper for runtime pipeline output.
#[derive(Debug, Clone)]
pub struct PathEventMapper {
    kind: MapperKind,
    content_path: String,
    tool_call_path: String,
    usage_path: String,
}

impl PathEventMapper {
    pub fn openai_defaults() -> Self {
        Self {
            kind: MapperKind::OpenAi,
            content_path: "$.choices[0].delta.content".to_string(),
            tool_call_path: "$.choices[0].delta.tool_calls".to_string(),
            usage_path: "$.usage".to_string(),
        }
    }

    pub fn ethnopic_defaults() -> Self {
        Self {
            kind: MapperKind::Ethnopic,
            content_path: "$.delta.text".to_string(),
            tool_call_path: "$.delta.partial_json".to_string(),
            usage_path: "$.message.usage".to_string(),
        }
    }

    pub fn ethnopic_defaults_compat() -> Self {
        Self::ethnopic_defaults()
    }

    pub fn google_defaults() -> Self {
        Self {
            kind: MapperKind::Google,
            content_path: "$.candidates[0].content.parts[0].text".to_string(),
            tool_call_path: "$.candidates[0].content.parts[0].functionCall".to_string(),
            usage_path: "$.usageMetadata".to_string(),
        }
    }

    pub fn vertex_defaults() -> Self {
        Self {
            kind: MapperKind::Vertex,
            content_path: "$.candidates[0].content.parts[0].text".to_string(),
            tool_call_path: "$.candidates[0].content.parts[0].functionCall".to_string(),
            usage_path: "$.usageMetadata".to_string(),
        }
    }

    pub fn from_manifest(manifest: &ProtocolManifest) -> Self {
        let id = manifest.id.to_ascii_lowercase();
        let mut mapper = if is_ethnopic_compatible_id(&id) {
            Self::ethnopic_defaults()
        } else if id.contains("google-vertex") || id.contains("vertex") {
            Self::vertex_defaults()
        } else if id.contains("google") || id.contains("gemini") {
            Self::google_defaults()
        } else {
            Self::openai_defaults()
        };

        if let Some(streaming) = &manifest.streaming {
            mapper.apply_streaming_config(streaming);
        }

        mapper
    }

    pub fn from_streaming_config(kind: &str, streaming: &StreamingConfig) -> Self {
        let mut mapper = match kind {
            "anthropic" | "ethnopic-compatible" => Self::ethnopic_defaults(),
            "google" => Self::google_defaults(),
            "vertex" => Self::vertex_defaults(),
            _ => Self::openai_defaults(),
        };
        mapper.apply_streaming_config(streaming);
        mapper
    }

    fn apply_streaming_config(&mut self, cfg: &StreamingConfig) {
        if let Some(path) = &cfg.content_path {
            self.content_path = path.clone();
        }
        if let Some(path) = &cfg.tool_call_path {
            self.tool_call_path = path.clone();
        }
        if let Some(path) = &cfg.usage_path {
            self.usage_path = path.clone();
        }
    }

    pub fn map_frame(&self, frame: &serde_json::Value) -> Vec<StreamingEvent> {
        match self.kind {
            MapperKind::OpenAi => self.map_openai(frame),
            MapperKind::Ethnopic => self.map_ethnopic_messages(frame),
            MapperKind::Google | MapperKind::Vertex => self.map_gemini_like(frame),
        }
    }

    fn map_openai(&self, frame: &serde_json::Value) -> Vec<StreamingEvent> {
        let mut events = Vec::new();

        // closeai-compatible reasoning: reasoning_content or reasoning_text in delta
        let delta = frame
            .get("choices")
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("delta"));
        if let Some(delta) = delta {
            let reasoning = delta
                .get("reasoning_content")
                .or_else(|| delta.get("reasoning_text"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if !reasoning.is_empty() {
                events.push(StreamingEvent::ThinkingDelta {
                    thinking: reasoning.to_string(),
                    tool_consideration: None,
                });
            }
        }

        if let Some(content) = resolve_path(frame, &self.content_path).and_then(|v| v.as_str()) {
            if !content.is_empty() {
                events.push(StreamingEvent::PartialContentDelta {
                    content: content.to_string(),
                    sequence_id: None,
                });
            }
        }

        if let Some(tool_calls) =
            resolve_path(frame, &self.tool_call_path).and_then(|v| v.as_array())
        {
            for tool_call in tool_calls {
                let index = tool_call
                    .get("index")
                    .and_then(|v| v.as_u64())
                    .map(|i| i as u32);
                let tool_call_id = tool_call
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("tool-call-{}", index.unwrap_or(0)));

                if let Some(name) = tool_call
                    .get("function")
                    .and_then(|v| v.get("name"))
                    .and_then(|v| v.as_str())
                {
                    if !name.is_empty() {
                        events.push(StreamingEvent::ToolCallStarted {
                            tool_call_id: tool_call_id.clone(),
                            tool_name: name.to_string(),
                            index,
                        });
                    }
                }

                if let Some(arguments) = tool_call
                    .get("function")
                    .and_then(|v| v.get("arguments"))
                    .and_then(|v| v.as_str())
                {
                    if !arguments.is_empty() {
                        events.push(StreamingEvent::PartialToolCall {
                            tool_call_id: tool_call_id.clone(),
                            arguments: arguments.to_string(),
                            index,
                            is_complete: None,
                        });
                    }
                }
            }
        }

        let usage = resolve_path(frame, &self.usage_path).cloned();
        let finish_reason = frame
            .get("choices")
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("finish_reason"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string);

        if usage.is_some() || finish_reason.is_some() {
            events.push(StreamingEvent::Metadata {
                usage,
                finish_reason: finish_reason.clone(),
                stop_reason: None,
            });
        }

        if finish_reason.is_some() {
            events.push(StreamingEvent::StreamEnd { finish_reason });
        }

        events
    }

    fn map_ethnopic_messages(&self, frame: &serde_json::Value) -> Vec<StreamingEvent> {
        let mut events = Vec::new();
        let event_type = frame
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        match event_type {
            "content_block_start" => {
                let block = frame.get("content_block");
                let block_type = block
                    .and_then(|v| v.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if block_type == "thinking" {
                    events.push(StreamingEvent::ThinkingDelta {
                        thinking: String::new(),
                        tool_consideration: None,
                    });
                }
                if block_type == "tool_use" {
                    let index = frame
                        .get("index")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32);
                    let tool_call_id = block
                        .and_then(|v| v.get("id"))
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("tool-call-{}", index.unwrap_or(0)));
                    let tool_name = block
                        .and_then(|v| v.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    events.push(StreamingEvent::ToolCallStarted {
                        tool_call_id,
                        tool_name,
                        index,
                    });
                }
            }
            "content_block_delta" => {
                if let Some(thinking) = frame
                    .get("delta")
                    .and_then(|v| v.get("thinking"))
                    .and_then(|v| v.as_str())
                {
                    if !thinking.is_empty() {
                        events.push(StreamingEvent::ThinkingDelta {
                            thinking: thinking.to_string(),
                            tool_consideration: None,
                        });
                    }
                }

                if let Some(text) = frame
                    .get("delta")
                    .and_then(|v| v.get("text"))
                    .and_then(|v| v.as_str())
                {
                    if !text.is_empty() {
                        events.push(StreamingEvent::PartialContentDelta {
                            content: text.to_string(),
                            sequence_id: None,
                        });
                    }
                }

                if let Some(partial_json) = frame
                    .get("delta")
                    .and_then(|v| v.get("partial_json"))
                    .and_then(|v| v.as_str())
                {
                    if !partial_json.is_empty() {
                        let index = frame
                            .get("index")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as u32);
                        let tool_call_id = frame
                            .get("content_block")
                            .and_then(|v| v.get("id"))
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                            .unwrap_or_else(|| format!("tool-call-{}", index.unwrap_or(0)));
                        events.push(StreamingEvent::PartialToolCall {
                            tool_call_id,
                            arguments: partial_json.to_string(),
                            index,
                            is_complete: None,
                        });
                    }
                }
            }
            "message_start" => {
                let usage = resolve_path(frame, &self.usage_path).cloned();
                if usage.is_some() {
                    events.push(StreamingEvent::Metadata {
                        usage,
                        finish_reason: None,
                        stop_reason: None,
                    });
                }
            }
            "message_delta" => {
                let stop_reason = frame
                    .get("delta")
                    .and_then(|v| v.get("stop_reason"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                if stop_reason.is_some() {
                    events.push(StreamingEvent::Metadata {
                        usage: None,
                        finish_reason: None,
                        stop_reason: stop_reason.clone(),
                    });
                    events.push(StreamingEvent::StreamEnd {
                        finish_reason: stop_reason,
                    });
                }
            }
            "message_stop" => {
                events.push(StreamingEvent::StreamEnd {
                    finish_reason: None,
                });
            }
            "error" => {
                if let Some(error) = frame.get("error") {
                    events.push(StreamingEvent::StreamError {
                        error: error.clone(),
                        event_id: None,
                    });
                }
            }
            _ => {}
        }

        events
    }

    fn map_gemini_like(&self, frame: &serde_json::Value) -> Vec<StreamingEvent> {
        let mut events = Vec::new();

        if let Some(text) = resolve_path(frame, &self.content_path).and_then(|v| v.as_str()) {
            if !text.is_empty() {
                events.push(StreamingEvent::PartialContentDelta {
                    content: text.to_string(),
                    sequence_id: None,
                });
            }
        }

        let usage = resolve_path(frame, &self.usage_path)
            .cloned()
            .or_else(|| frame.get("usage_metadata").cloned());
        let finish_reason = frame
            .get("candidates")
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("finish_reason").or_else(|| v.get("finishReason")))
            .and_then(|v| v.as_str())
            .map(ToString::to_string);

        if usage.is_some() || finish_reason.is_some() {
            events.push(StreamingEvent::Metadata {
                usage,
                finish_reason: finish_reason.clone(),
                stop_reason: None,
            });
        }

        if finish_reason.is_some() {
            events.push(StreamingEvent::StreamEnd { finish_reason });
        }

        events
    }
}

#[cfg(test)]
mod tests {
    use super::{is_ethnopic_compatible_id, PathEventMapper};
    use crate::driver::StreamingEvent;
    use crate::protocol_loader::{
        DecoderConfig, EndpointConfig, ProtocolManifest, StreamingConfig,
    };
    use std::collections::HashMap;

    #[test]
    fn detects_ethnopic_compatible_manifest_ids() {
        assert!(is_ethnopic_compatible_id("anthropic"));
        assert!(is_ethnopic_compatible_id("ethnopic-compatible"));
        assert!(!is_ethnopic_compatible_id("openai-compatible"));
    }

    #[test]
    fn manifest_with_ethnopic_id_uses_messages_defaults() {
        let manifest = ProtocolManifest {
            id: "ethnopic-compatible".to_string(),
            protocol_version: "v1".to_string(),
            endpoint: EndpointConfig {
                base_url: "https://example.invalid".to_string(),
            },
            streaming: Some(StreamingConfig {
                content_path: None,
                tool_call_path: None,
                usage_path: None,
                decoder: DecoderConfig::default(),
            }),
            capabilities: HashMap::new(),
            retry_policy: None,
            rate_limit_headers: None,
        };

        let mapper = PathEventMapper::from_manifest(&manifest);
        let frame = serde_json::json!({
            "type": "content_block_delta",
            "delta": { "text": "hello" },
            "message": { "usage": { "input_tokens": 1, "output_tokens": 2 } }
        });

        let events = mapper.map_frame(&frame);
        assert!(events.iter().any(|event| matches!(
            event,
            StreamingEvent::PartialContentDelta { content, .. } if content == "hello"
        )));
    }
}

#[derive(Debug)]
enum PathPart {
    Key(String),
    Index(usize),
}

fn resolve_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let parts = parse_path(path)?;
    let mut current = value;
    for part in parts {
        match part {
            PathPart::Key(key) => {
                current = current.get(&key)?;
            }
            PathPart::Index(index) => {
                current = current.as_array()?.get(index)?;
            }
        }
    }
    Some(current)
}

fn parse_path(path: &str) -> Option<Vec<PathPart>> {
    let trimmed = path.trim();
    if trimmed == "$" {
        return Some(Vec::new());
    }
    let rest = trimmed.strip_prefix("$.")?;
    let mut parts = Vec::new();

    for segment in rest.split('.') {
        if segment.is_empty() {
            return None;
        }

        let mut cursor = segment;
        if let Some(start) = cursor.find('[') {
            let key = &cursor[..start];
            if !key.is_empty() {
                parts.push(PathPart::Key(key.to_string()));
            }
            cursor = &cursor[start..];
        } else {
            parts.push(PathPart::Key(cursor.to_string()));
            continue;
        }

        while let Some(stripped) = cursor.strip_prefix('[') {
            let end = stripped.find(']')?;
            let index = stripped[..end].parse::<usize>().ok()?;
            parts.push(PathPart::Index(index));
            cursor = &stripped[end + 1..];
        }

        if !cursor.is_empty() {
            return None;
        }
    }

    Some(parts)
}
