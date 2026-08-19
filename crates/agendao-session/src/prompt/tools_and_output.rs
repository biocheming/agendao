use std::sync::Arc;

use agendao_execution_types::session_title_request;
use agendao_provider::{Content, Message, Provider, Role};

use crate::{sanitize_display_text, MessageRole, Session, SessionMessage};

// --- Structured Output ---

const LOADED_SYSTEM_REMINDER_PREFIX: &str = "System Reminder Sent:";
const LOADED_INSTRUCTION_FILES_PREFIX: &str = "Loaded instruction files:";

// --- Tool Resolution ---

fn is_system_reminder_open_tag(line: &str) -> bool {
    line.starts_with("<system-reminder") || line.starts_with("<system_reminder")
}

fn is_system_reminder_close_tag(line: &str) -> bool {
    line.starts_with("</system-reminder") || line.starts_with("</system_reminder")
}

pub fn sanitize_session_title_source(text: &str) -> String {
    let mut lines = Vec::new();
    let mut in_system_reminder = false;
    let mut previous_blank = false;

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();

        if is_system_reminder_open_tag(trimmed) {
            in_system_reminder = true;
            if trimmed.contains("</system-reminder>") || trimmed.contains("</system_reminder>") {
                in_system_reminder = false;
            }
            continue;
        }

        if in_system_reminder {
            if is_system_reminder_close_tag(trimmed) {
                in_system_reminder = false;
            }
            continue;
        }

        if is_system_reminder_close_tag(trimmed)
            || trimmed.starts_with(LOADED_SYSTEM_REMINDER_PREFIX)
            || trimmed.starts_with(LOADED_INSTRUCTION_FILES_PREFIX)
            || trimmed.starts_with("Instructions from:")
        {
            continue;
        }

        if trimmed.is_empty() {
            if previous_blank {
                continue;
            }
            previous_blank = true;
            lines.push(String::new());
            continue;
        }

        previous_blank = false;
        lines.push(raw_line.to_string());
    }

    sanitize_display_text(&lines.join("\n")).trim().to_string()
}

pub fn generate_session_title(first_user_message: &str) -> String {
    let normalized = sanitize_session_title_source(first_user_message);
    let first_line = normalized.lines().next().unwrap_or("").trim();

    if first_line.chars().count() > 100 {
        format!("{}...", first_line.chars().take(97).collect::<String>())
    } else if first_line.is_empty() {
        "New Session".to_string()
    } else {
        first_line.to_string()
    }
}

fn trim_title_source(text: &str, max_chars: usize) -> String {
    let normalized = sanitize_session_title_source(text);
    if normalized.chars().count() <= max_chars {
        normalized
    } else {
        normalized.chars().take(max_chars).collect::<String>()
    }
}

pub fn compose_session_title_source(session: &Session) -> Option<(String, String)> {
    let first_user = session
        .messages
        .iter()
        .find(|message| matches!(message.role, MessageRole::User))
        .map(SessionMessage::get_text)
        .map(|text| sanitize_session_title_source(&text))
        .filter(|text| !text.is_empty())?;

    let fallback = generate_session_title(&first_user);
    let mut sections = vec![format!(
        "User request:\n{}",
        trim_title_source(&first_user, 400)
    )];

    if let Some(assistant_text) = session
        .messages
        .iter()
        .rev()
        .filter(|message| matches!(message.role, MessageRole::Assistant))
        .map(SessionMessage::get_text)
        .map(|text| trim_title_source(&text, 600))
        .find(|text| !text.trim().is_empty())
    {
        sections.push(format!("Assistant outcome:\n{}", assistant_text));
    }

    Some((sections.join("\n\n"), fallback))
}

/// Generate a refined session title from the session's first-turn context.
/// Uses the first user request and, when available, the latest assistant
/// outcome already persisted in the session.
pub async fn generate_session_title_for_session(
    session: &Session,
    provider: Arc<dyn Provider>,
    model_id: &str,
) -> String {
    let Some((title_source, fallback)) = compose_session_title_source(session) else {
        return "New Session".to_string();
    };

    let request = session_title_request(model_id).to_chat_request_with_system(
        vec![Message {
            role: Role::User,
            content: Content::Text(format!(
                "Generate a short session title (under 80 chars) for this conversation.\n\
                 Base it on the actual task and outcome, not the user's raw wording.\n\
                 Do not mention system reminders, instruction files, or metadata wrappers.\n\
                 Reply with ONLY the title, no quotes or explanation.\n\n{}",
                title_source
            )),
            cache_control: None,
            provider_options: None,
        }],
        vec![],
        None,
        Some(
            "You generate concise conversation titles. Prefer compact task-focused summaries. Never mention system reminders or instruction-file wrappers. Reply with only the title."
                .to_string(),
        ),
    );

    match provider.chat(request).await {
        Ok(response) => {
            let text = response
                .choices
                .first()
                .map(|c| match &c.message.content {
                    Content::Text(t) => t.clone(),
                    Content::Parts(parts) => parts
                        .iter()
                        .filter_map(|p| p.text.clone())
                        .collect::<Vec<_>>()
                        .join(""),
                })
                .unwrap_or_default();

            let cleaned = text
                .replace(['"', '\''], "")
                .lines()
                .map(|l| l.trim())
                .find(|l| !l.is_empty() && !l.starts_with("<think>"))
                .unwrap_or("")
                .to_string();

            if cleaned.is_empty() {
                fallback
            } else if cleaned.chars().count() > 100 {
                format!("{}...", cleaned.chars().take(97).collect::<String>())
            } else {
                cleaned
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to generate title via LLM, using fallback");
            fallback
        }
    }
}

/// Generate a session title using an LLM (matching TS `ensureTitle`).
/// Falls back to `generate_session_title` on any failure.
pub async fn generate_session_title_llm(
    first_user_message: &str,
    provider: Arc<dyn Provider>,
    model_id: &str,
) -> String {
    let normalized_first_user_message = sanitize_session_title_source(first_user_message);
    let fallback = generate_session_title(&normalized_first_user_message);

    let request = session_title_request(model_id).to_chat_request_with_system(
        vec![Message {
            role: Role::User,
            content: Content::Text(format!(
                "Generate a short title (under 80 chars) for this conversation. \
                     Do not mention system reminders, instruction files, or metadata wrappers. \
                     Reply with ONLY the title, no quotes or explanation.\n\n{}",
                normalized_first_user_message
            )),
            cache_control: None,
            provider_options: None,
        }],
        vec![],
        None,
        Some(
            "You generate concise conversation titles. Never mention system reminders or instruction-file wrappers. Reply with only the title."
                .to_string(),
        ),
    );

    match provider.chat(request).await {
        Ok(response) => {
            // Extract text from the first choice
            let text = response
                .choices
                .first()
                .map(|c| match &c.message.content {
                    Content::Text(t) => t.clone(),
                    Content::Parts(parts) => parts
                        .iter()
                        .filter_map(|p| p.text.clone())
                        .collect::<Vec<_>>()
                        .join(""),
                })
                .unwrap_or_default();

            // Clean up: remove thinking tags, take first non-empty line
            let cleaned = text
                .replace(['"', '\''], "")
                .lines()
                .map(|l| l.trim())
                .find(|l| !l.is_empty() && !l.starts_with("<think>"))
                .unwrap_or("")
                .to_string();

            if cleaned.is_empty() {
                fallback
            } else if cleaned.chars().count() > 100 {
                format!("{}...", cleaned.chars().take(97).collect::<String>())
            } else {
                cleaned
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to generate title via LLM, using fallback");
            fallback
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_provider::{
        ChatRequest, ChatResponse, Choice, Message as ProviderMessage, ModelInfo, ProviderError,
        StreamResult,
    };
    use async_trait::async_trait;
    use futures::stream;
    use std::sync::{Arc, Mutex};

    #[derive(Debug)]
    struct CaptureProvider {
        title: String,
        last_prompt: Arc<Mutex<Option<String>>>,
    }

    #[async_trait]
    impl Provider for CaptureProvider {
        fn id(&self) -> &str {
            "capture"
        }

        fn name(&self) -> &str {
            "Capture"
        }

        fn models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }

        fn get_model(&self, _id: &str) -> Option<&ModelInfo> {
            None
        }

        async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            let text = request
                .messages
                .first()
                .map(|message| match &message.content {
                    Content::Text(text) => text.clone(),
                    Content::Parts(parts) => parts
                        .iter()
                        .filter_map(|part| part.text.clone())
                        .collect::<Vec<_>>()
                        .join(" "),
                })
                .unwrap_or_default();
            *self.last_prompt.lock().expect("capture prompt") = Some(text);
            Ok(ChatResponse {
                id: "capture-response".to_string(),
                model: "capture-model".to_string(),
                choices: vec![Choice {
                    index: 0,
                    message: ProviderMessage {
                        role: Role::Assistant,
                        content: Content::Text(self.title.clone()),
                        cache_control: None,
                        provider_options: None,
                    },
                    finish_reason: Some("stop".to_string()),
                }],
                usage: None,
            })
        }

        async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
            Ok(Box::pin(stream::iter(Vec::<
                Result<agendao_provider::StreamEvent, ProviderError>,
            >::new())))
        }
    }

    #[test]
    fn compose_session_title_source_includes_assistant_outcome() {
        let mut session = Session::new("project", ".");
        session.add_user_message("根据 ./t.html 文件，设计一个科技感更加浓重的网页");
        session
            .add_assistant_message()
            .add_text("已完成首页重构，强化了深色科技风、发光边框和分层卡片布局。");

        let (source, fallback) =
            compose_session_title_source(&session).expect("title source should exist");
        assert!(source.contains("User request:"));
        assert!(source.contains("Assistant outcome:"));
        assert!(source.contains("已完成首页重构"));
        assert_eq!(fallback, "根据 ./t.html 文件，设计一个科技感更加浓重的网页");
    }

    #[tokio::test]
    async fn generate_session_title_for_session_uses_assistant_context() {
        let mut session = Session::new("project", ".");
        session.add_user_message("Fix the scheduler session title flow after first reply");
        session
            .add_assistant_message()
            .add_text("Implemented refined title regeneration based on the first completed turn.");

        let last_prompt = Arc::new(Mutex::new(None));
        let provider = Arc::new(CaptureProvider {
            title: "Refine Session Titles After First Reply".to_string(),
            last_prompt: last_prompt.clone(),
        });

        let title = generate_session_title_for_session(&session, provider, "mock-model").await;
        assert_eq!(title, "Refine Session Titles After First Reply");

        let captured = last_prompt
            .lock()
            .expect("capture prompt")
            .clone()
            .unwrap_or_default();
        assert!(captured.contains("User request:"));
        assert!(captured.contains("Assistant outcome:"));
        assert!(captured.contains("Implemented refined title regeneration"));
    }

    #[test]
    fn sanitize_session_title_source_strips_system_reminder_wrappers() {
        let cleaned = sanitize_session_title_source(
            "帮我重构 TUI\n\n<system-reminder>\nInstructions from: /tmp/project/AGENTS.md\nBe strict.\n</system-reminder>\n\nLoaded instruction files: /tmp/project/AGENTS.md",
        );

        assert_eq!(cleaned, "帮我重构 TUI");
    }

    #[test]
    fn generate_session_title_ignores_system_reminder_text() {
        let title = generate_session_title(
            "Fix the session renderer migration flow\n<system-reminder>\nInstructions from: /tmp/project/AGENTS.md\nUse latest renderer.\n</system-reminder>",
        );

        assert_eq!(title, "Fix the session renderer migration flow");
    }
}
