use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip)]
    pub variant: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub cache_type: String,
}

impl CacheControl {
    pub fn ephemeral() -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    #[serde(default = "default_content", deserialize_with = "deserialize_content")]
    pub content: Content,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
    #[serde(rename = "providerOptions", skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<HashMap<String, serde_json::Value>>,
}

fn default_content() -> Content {
    Content::Text(String::new())
}

/// Deserialize content that may be null (some closeai-compatible APIs return
/// `"content": null` for tool-call-only responses).
fn deserialize_content<'de, D>(deserializer: D) -> Result<Content, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<Content> = Option::deserialize(deserializer)?;
    Ok(value.unwrap_or_else(|| Content::Text(String::new())))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<ImageUrl>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use: Option<ToolUse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<ToolResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(rename = "providerOptions", skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<HashMap<String, serde_json::Value>>,
}

impl Default for ContentPart {
    fn default() -> Self {
        Self {
            content_type: "text".to_string(),
            text: None,
            image_url: None,
            tool_use: None,
            tool_result: None,
            cache_control: None,
            filename: None,
            media_type: None,
            provider_options: None,
        }
    }
}

impl ContentPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content_type: "text".to_string(),
            text: Some(text.into()),
            ..Default::default()
        }
    }

    pub fn reasoning(text: impl Into<String>) -> Self {
        Self {
            content_type: "reasoning".to_string(),
            text: Some(text.into()),
            ..Default::default()
        }
    }

    pub fn tool_use(
        id: impl Into<String>,
        name: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        Self {
            content_type: "tool_use".to_string(),
            tool_use: Some(ToolUse {
                id: id.into(),
                name: name.into(),
                input,
            }),
            ..Default::default()
        }
    }

    pub fn tool_result(
        tool_use_id: impl Into<String>,
        content: impl Into<String>,
        is_error: Option<bool>,
    ) -> Self {
        Self {
            content_type: "tool_result".to_string(),
            tool_result: Some(ToolResult {
                tool_use_id: tool_use_id.into(),
                content: content.into(),
                is_error,
            }),
            ..Default::default()
        }
    }

    pub fn image_url(
        url: impl Into<String>,
        filename: Option<String>,
        media_type: Option<String>,
    ) -> Self {
        Self {
            content_type: "image_url".to_string(),
            image_url: Some(ImageUrl { url: url.into() }),
            filename,
            media_type,
            ..Default::default()
        }
    }

    pub fn file(
        url: impl Into<String>,
        filename: Option<String>,
        media_type: Option<String>,
    ) -> Self {
        Self {
            content_type: "file".to_string(),
            image_url: Some(ImageUrl { url: url.into() }),
            filename,
            media_type,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: Option<String>,
    pub parameters: serde_json::Value,
}

impl Serialize for ToolDefinition {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut root = serializer.serialize_struct("ToolDefinition", 2)?;
        root.serialize_field("type", "function")?;

        #[derive(Serialize)]
        struct Function<'a> {
            name: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            description: Option<&'a str>,
            parameters: &'a serde_json::Value,
        }

        let function = Function {
            name: &self.name,
            description: self.description.as_deref(),
            parameters: &self.parameters,
        };
        root.serialize_field("function", &function)?;
        root.end()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub index: u32,
    pub message: Message,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: Content::Text(content.into()),
            cache_control: None,
            provider_options: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: Content::Text(content.into()),
            cache_control: None,
            provider_options: None,
        }
    }

    pub fn assistant_parts(parts: Vec<ContentPart>) -> Self {
        Self {
            role: Role::Assistant,
            content: Content::Parts(parts),
            cache_control: None,
            provider_options: None,
        }
    }

    pub fn tool_parts(parts: Vec<ContentPart>) -> Self {
        Self {
            role: Role::Tool,
            content: Content::Parts(parts),
            cache_control: None,
            provider_options: None,
        }
    }

    pub fn assistant_turn(
        reasoning: Option<&str>,
        text: Option<&str>,
        tool_calls: &[ToolUse],
    ) -> Option<Self> {
        let mut parts = Vec::new();

        if let Some(reasoning) = reasoning.filter(|value| !value.is_empty()) {
            parts.push(ContentPart::reasoning(reasoning.to_string()));
        }
        if let Some(text) = text.filter(|value| !value.is_empty()) {
            parts.push(ContentPart::text(text.to_string()));
        }
        for tool_call in tool_calls {
            parts.push(ContentPart::tool_use(
                tool_call.id.clone(),
                tool_call.name.clone(),
                tool_call.input.clone(),
            ));
        }

        if parts.is_empty() {
            None
        } else {
            Some(Self::assistant_parts(parts))
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: Content::Text(content.into()),
            cache_control: None,
            provider_options: None,
        }
    }

    pub fn with_cache_control(mut self, cache_control: CacheControl) -> Self {
        self.cache_control = Some(cache_control);
        self
    }
}

impl ChatRequest {
    pub fn new(model: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            model: model.into(),
            messages,
            max_tokens: None,
            temperature: None,
            top_p: None,
            system: None,
            tools: None,
            stream: Some(true),
            provider_options: None,
            variant: None,
        }
    }

    pub fn with_temperature(mut self, temp: f32) -> Self {
        self.temperature = Some(temp);
        self
    }

    pub fn with_top_p(mut self, top_p: f32) -> Self {
        self.top_p = Some(top_p);
        self
    }

    pub fn with_stream(mut self, stream: bool) -> Self {
        self.stream = Some(stream);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u64) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = Some(tools);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_definition_serializes_to_openai_compatible_format() {
        let tool = ToolDefinition {
            name: "bash".to_string(),
            description: Some("Execute shell commands".to_string()),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "cmd": {"type": "string"}
                },
                "required": ["cmd"]
            }),
        };

        let value = serde_json::to_value(&tool).expect("serialize tool");
        assert_eq!(value["type"], "function");
        assert_eq!(value["function"]["name"], "bash");
        assert_eq!(value["function"]["description"], "Execute shell commands");
        assert_eq!(value["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn assistant_turn_preserves_reasoning_before_tool_calls() {
        let tool_calls = vec![ToolUse {
            id: "tool-call-1".to_string(),
            name: "read".to_string(),
            input: json!({ "path": "/tmp/a" }),
        }];

        let message = Message::assistant_turn(
            Some("need to inspect first"),
            Some("working on it"),
            &tool_calls,
        )
        .expect("assistant turn should exist");

        match message.content {
            Content::Parts(parts) => {
                assert_eq!(parts.len(), 3);
                assert_eq!(parts[0].content_type, "reasoning");
                assert_eq!(parts[0].text.as_deref(), Some("need to inspect first"));
                assert_eq!(parts[1].content_type, "text");
                assert_eq!(parts[1].text.as_deref(), Some("working on it"));
                assert_eq!(parts[2].content_type, "tool_use");
                assert_eq!(
                    parts[2]
                        .tool_use
                        .as_ref()
                        .map(|tool_use| tool_use.name.as_str()),
                    Some("read")
                );
            }
            other => panic!("expected parts content, got {other:?}"),
        }
    }
}
