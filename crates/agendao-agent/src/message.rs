use serde::{Deserialize, Serialize};

/// An attachment carried by an `AgentMessage` for multimodal content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    /// URL of the attachment — data URL, http(s) URL, or file URL.
    pub url: String,
    /// MIME type, e.g. "image/png", "audio/wav".
    pub mime: String,
    /// Optional original filename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub role: MessageRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reasoning: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<ToolResult>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub name: String,
    pub content: String,
    pub is_error: bool,
}

impl AgentMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
            reasoning: String::new(),
            tool_result: None,
            tool_calls: Vec::new(),
            attachments: Vec::new(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            reasoning: String::new(),
            tool_result: None,
            tool_calls: Vec::new(),
            attachments: Vec::new(),
        }
    }

    /// Create a user message with file/image attachments.
    pub fn user_with_attachments(content: impl Into<String>, attachments: Vec<Attachment>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            reasoning: String::new(),
            tool_result: None,
            tool_calls: Vec::new(),
            attachments,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            reasoning: String::new(),
            tool_result: None,
            tool_calls: Vec::new(),
            attachments: Vec::new(),
        }
    }

    pub fn assistant_with_tools(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            reasoning: String::new(),
            tool_result: None,
            tool_calls,
            attachments: Vec::new(),
        }
    }

    pub fn tool_result(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        let content = content.into();
        Self {
            role: MessageRole::Tool,
            content: content.clone(),
            reasoning: String::new(),
            tool_result: Some(ToolResult {
                tool_call_id: tool_call_id.into(),
                name: name.into(),
                content,
                is_error,
            }),
            tool_calls: vec![],
            attachments: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub messages: Vec<AgentMessage>,
}

impl Conversation {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    pub fn with_system_prompt(prompt: impl Into<String>) -> Self {
        let mut conv = Self::new();
        conv.messages.push(AgentMessage::system(prompt));
        conv
    }

    pub fn add_user_message(&mut self, content: impl Into<String>) {
        self.messages.push(AgentMessage::user(content));
    }

    pub fn add_user_message_with_attachments(
        &mut self,
        content: impl Into<String>,
        attachments: Vec<Attachment>,
    ) {
        self.messages
            .push(AgentMessage::user_with_attachments(content, attachments));
    }

    pub fn add_assistant_message(&mut self, content: impl Into<String>) {
        self.messages.push(AgentMessage::assistant(content));
    }

    pub fn add_assistant_message_with_reasoning(
        &mut self,
        reasoning: impl Into<String>,
        content: impl Into<String>,
    ) {
        let mut message = AgentMessage::assistant(content);
        message.reasoning = reasoning.into();
        self.messages.push(message);
    }

    pub fn add_assistant_message_with_tools(
        &mut self,
        content: impl Into<String>,
        tool_calls: Vec<ToolCall>,
    ) {
        self.messages
            .push(AgentMessage::assistant_with_tools(content, tool_calls));
    }

    pub fn add_assistant_message_with_reasoning_and_tools(
        &mut self,
        reasoning: impl Into<String>,
        content: impl Into<String>,
        tool_calls: Vec<ToolCall>,
    ) {
        let mut message = AgentMessage::assistant_with_tools(content, tool_calls);
        message.reasoning = reasoning.into();
        self.messages.push(message);
    }

    pub fn add_tool_result(
        &mut self,
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) {
        self.messages.push(AgentMessage::tool_result(
            tool_call_id,
            name,
            content,
            is_error,
        ));
    }

    pub fn to_provider_messages(&self) -> Vec<agendao_provider::Message> {
        self.messages
            .iter()
            .map(|m| match &m.role {
                MessageRole::System => agendao_provider::Message::system(&m.content),
                MessageRole::User => {
                    if m.attachments.is_empty() {
                        agendao_provider::Message::user(&m.content)
                    } else {
                        let mut parts = Vec::new();
                        if !m.content.is_empty() {
                            parts.push(agendao_provider::ContentPart {
                                content_type: "text".to_string(),
                                text: Some(m.content.clone()),
                                image_url: None,
                                tool_use: None,
                                tool_result: None,
                                cache_control: None,
                                filename: None,
                                media_type: None,
                                provider_options: None,
                            });
                        }
                        for att in &m.attachments {
                            if att.mime.starts_with("image/") {
                                parts.push(agendao_provider::ContentPart {
                                    content_type: "image_url".to_string(),
                                    text: None,
                                    image_url: Some(agendao_provider::ImageUrl {
                                        url: att.url.clone(),
                                    }),
                                    tool_use: None,
                                    tool_result: None,
                                    cache_control: None,
                                    filename: att.filename.clone(),
                                    media_type: Some(att.mime.clone()),
                                    provider_options: None,
                                });
                            } else {
                                parts.push(agendao_provider::ContentPart {
                                    content_type: "text".to_string(),
                                    text: Some(format!(
                                        "[File: {} ({})]",
                                        att.filename.as_deref().unwrap_or("unnamed"),
                                        att.mime
                                    )),
                                    image_url: None,
                                    tool_use: None,
                                    tool_result: None,
                                    cache_control: None,
                                    filename: att.filename.clone(),
                                    media_type: Some(att.mime.clone()),
                                    provider_options: None,
                                });
                            }
                        }
                        agendao_provider::Message {
                            role: agendao_provider::Role::User,
                            content: agendao_provider::Content::Parts(parts),
                            cache_control: None,
                            provider_options: None,
                        }
                    }
                }
                MessageRole::Assistant => {
                    if m.tool_calls.is_empty() && m.reasoning.is_empty() {
                        agendao_provider::Message::assistant(&m.content)
                    } else {
                        let mut parts = Vec::new();
                        if !m.reasoning.is_empty() {
                            parts.push(agendao_provider::ContentPart {
                                content_type: "reasoning".to_string(),
                                text: Some(m.reasoning.clone()),
                                image_url: None,
                                tool_use: None,
                                tool_result: None,
                                cache_control: None,
                                filename: None,
                                media_type: None,
                                provider_options: None,
                            });
                        }
                        if !m.content.is_empty() {
                            parts.push(agendao_provider::ContentPart {
                                content_type: "text".to_string(),
                                text: Some(m.content.clone()),
                                image_url: None,
                                tool_use: None,
                                tool_result: None,
                                cache_control: None,
                                filename: None,
                                media_type: None,
                                provider_options: None,
                            });
                        }
                        for call in &m.tool_calls {
                            parts.push(agendao_provider::ContentPart {
                                content_type: "tool_use".to_string(),
                                text: None,
                                image_url: None,
                                tool_use: Some(agendao_provider::ToolUse {
                                    id: call.id.clone(),
                                    name: call.name.clone(),
                                    input: call.arguments.clone(),
                                }),
                                tool_result: None,
                                cache_control: None,
                                filename: None,
                                media_type: None,
                                provider_options: None,
                            });
                        }
                        agendao_provider::Message {
                            role: agendao_provider::Role::Assistant,
                            content: agendao_provider::Content::Parts(parts),
                            cache_control: None,
                            provider_options: None,
                        }
                    }
                }
                MessageRole::Tool => {
                    let tool_result =
                        m.tool_result
                            .as_ref()
                            .map(|result| agendao_provider::ToolResult {
                                tool_use_id: result.tool_call_id.clone(),
                                content: result.content.clone(),
                                is_error: Some(result.is_error),
                            });

                    agendao_provider::Message {
                        role: agendao_provider::Role::Tool,
                        content: if let Some(result) = tool_result {
                            agendao_provider::Content::Parts(vec![agendao_provider::ContentPart {
                                content_type: "tool_result".to_string(),
                                text: None,
                                image_url: None,
                                tool_use: None,
                                tool_result: Some(result),
                                cache_control: None,
                                filename: None,
                                media_type: None,
                                provider_options: None,
                            }])
                        } else {
                            agendao_provider::Content::Text(m.content.clone())
                        },
                        cache_control: None,
                        provider_options: None,
                    }
                }
            })
            .collect()
    }
}

impl Default for Conversation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_with_tool_calls_serializes_tool_use_parts() {
        let mut conversation = Conversation::new();
        conversation.add_assistant_message_with_tools(
            "",
            vec![ToolCall {
                id: "tool-call-0".to_string(),
                name: "ls".to_string(),
                arguments: serde_json::json!({"path":"."}),
            }],
        );

        let provider_messages = conversation.to_provider_messages();
        assert_eq!(provider_messages.len(), 1);
        let message = &provider_messages[0];
        match &message.content {
            agendao_provider::Content::Parts(parts) => {
                assert!(parts.iter().any(|part| {
                    part.content_type == "tool_use"
                        && part
                            .tool_use
                            .as_ref()
                            .map(|tool| {
                                tool.name == "ls"
                                    && tool.id == "tool-call-0"
                                    && tool.input == serde_json::json!({"path":"."})
                            })
                            .unwrap_or(false)
                }));
            }
            agendao_provider::Content::Text(_) => {
                panic!("assistant message with tool calls must serialize as parts");
            }
        }
    }

    #[test]
    fn assistant_reasoning_serializes_as_protocol_part() {
        let mut conversation = Conversation::new();
        conversation.add_assistant_message_with_reasoning_and_tools(
            "internal trace",
            "visible answer",
            vec![ToolCall {
                id: "tool-call-0".to_string(),
                name: "ls".to_string(),
                arguments: serde_json::json!({"path":"."}),
            }],
        );

        let provider_messages = conversation.to_provider_messages();
        assert_eq!(provider_messages.len(), 1);
        match &provider_messages[0].content {
            agendao_provider::Content::Parts(parts) => {
                assert_eq!(parts[0].content_type, "reasoning");
                assert_eq!(parts[0].text.as_deref(), Some("internal trace"));
                assert_eq!(parts[1].content_type, "text");
                assert_eq!(parts[1].text.as_deref(), Some("visible answer"));
                assert_eq!(parts[2].content_type, "tool_use");
            }
            agendao_provider::Content::Text(_) => {
                panic!("assistant reasoning must serialize as typed parts");
            }
        }
    }

    #[test]
    fn user_with_image_attachment_serializes_to_image_url_part() {
        let mut conversation = Conversation::new();
        conversation.add_user_message_with_attachments(
            "What's in this image?",
            vec![Attachment {
                url: "data:image/png;base64,iVBOR...".to_string(),
                mime: "image/png".to_string(),
                filename: Some("screenshot.png".to_string()),
            }],
        );

        let provider_messages = conversation.to_provider_messages();
        assert_eq!(provider_messages.len(), 1);
        match &provider_messages[0].content {
            agendao_provider::Content::Parts(parts) => {
                assert_eq!(parts.len(), 2);
                assert_eq!(parts[0].content_type, "text");
                assert_eq!(parts[0].text.as_deref(), Some("What's in this image?"));
                assert_eq!(parts[1].content_type, "image_url");
                assert!(parts[1].image_url.is_some());
            }
            _ => panic!("expected Parts content"),
        }
    }
}
