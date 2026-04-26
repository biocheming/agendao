use crate::tool_runner::ToolCallInput;

#[derive(Debug, Clone, Default)]
pub struct OrchestratorConversation {
    messages: Vec<rocode_provider::Message>,
}

impl OrchestratorConversation {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    pub fn with_system_prompt(prompt: &str) -> Self {
        Self {
            messages: vec![rocode_provider::Message::system(prompt.to_string())],
        }
    }

    pub fn from_messages(messages: Vec<rocode_provider::Message>) -> Self {
        Self { messages }
    }

    pub fn load_messages(&mut self, messages: Vec<rocode_provider::Message>) {
        self.messages = messages;
    }

    pub fn add_user_message(&mut self, content: &str) {
        self.messages
            .push(rocode_provider::Message::user(content.to_string()));
    }

    pub fn add_assistant_message(&mut self, content: &str) {
        self.messages
            .push(rocode_provider::Message::assistant(content.to_string()));
    }

    pub fn add_assistant_with_tools(&mut self, content: &str, tool_calls: Vec<ToolCallInput>) {
        let provider_tool_calls: Vec<rocode_provider::ToolUse> = tool_calls
            .into_iter()
            .map(|call| rocode_provider::ToolUse {
                id: call.id,
                name: call.name,
                input: call.arguments,
            })
            .collect();

        if let Some(message) =
            rocode_provider::Message::assistant_turn(None, Some(content), &provider_tool_calls)
        {
            self.messages.push(message);
        }
    }

    pub fn add_tool_result(&mut self, call_id: &str, _name: &str, content: String, is_error: bool) {
        self.messages
            .push(rocode_provider::Message::tool_parts(vec![
                rocode_provider::ContentPart::tool_result(
                    call_id.to_string(),
                    content,
                    Some(is_error),
                ),
            ]));
    }

    pub fn messages(&self) -> &[rocode_provider::Message] {
        &self.messages
    }

    pub fn extend_messages(&mut self, messages: Vec<rocode_provider::Message>) {
        self.messages.extend(messages);
    }

    pub fn into_messages(self) -> Vec<rocode_provider::Message> {
        self.messages
    }
}
