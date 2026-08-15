use agendao_command_render::output_blocks::{
    BlockTone, MessageBlock, MessagePhase, MessageRole, OutputBlock, QueueItemBlock,
    ReasoningBlock, SessionEventBlock, SessionEventField, StatusBlock, ToolBlock, ToolPhase,
};

pub(super) fn parse_output_block(payload: &serde_json::Value) -> Option<OutputBlock> {
    let kind = payload.get("kind")?.as_str()?;
    match kind {
        "status" => {
            let tone = match payload
                .get("tone")
                .and_then(|v| v.as_str())
                .unwrap_or("normal")
            {
                "title" => BlockTone::Title,
                "muted" => BlockTone::Muted,
                "success" => BlockTone::Success,
                "warning" => BlockTone::Warning,
                "error" => BlockTone::Error,
                _ => BlockTone::Normal,
            };
            let text = payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            Some(OutputBlock::Status(StatusBlock { tone, text }))
        }
        "message" => {
            let role = match payload
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("assistant")
            {
                "user" => MessageRole::User,
                "system" => MessageRole::System,
                _ => MessageRole::Assistant,
            };
            let phase = match payload
                .get("phase")
                .and_then(|v| v.as_str())
                .unwrap_or("delta")
            {
                "start" => MessagePhase::Start,
                "end" => MessagePhase::End,
                "full" => MessagePhase::Full,
                _ => MessagePhase::Delta,
            };
            let text = payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            Some(OutputBlock::Message(MessageBlock { role, phase, text }))
        }
        "tool" => {
            let name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("tool")
                .to_string();
            let phase = match payload
                .get("phase")
                .and_then(|v| v.as_str())
                .unwrap_or("running")
            {
                "start" => ToolPhase::Start,
                "done" | "result" => ToolPhase::Done,
                "error" => ToolPhase::Error,
                _ => ToolPhase::Running,
            };
            let detail = payload
                .get("detail")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Some(OutputBlock::Tool(ToolBlock {
                name,
                phase,
                detail,
                structured: None,
            }))
        }
        "reasoning" => {
            let phase = match payload
                .get("phase")
                .and_then(|v| v.as_str())
                .unwrap_or("delta")
            {
                "start" => MessagePhase::Start,
                "end" => MessagePhase::End,
                "full" => MessagePhase::Full,
                _ => MessagePhase::Delta,
            };
            let text = payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            Some(OutputBlock::Reasoning(ReasoningBlock { phase, text }))
        }
        "session_event" => Some(OutputBlock::SessionEvent(SessionEventBlock {
            event: payload
                .get("event")
                .and_then(|v| v.as_str())
                .unwrap_or("event")
                .to_string(),
            title: payload
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Session Event")
                .to_string(),
            status: payload
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            summary: payload
                .get("summary")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            fields: payload
                .get("fields")
                .and_then(|v| v.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|field| {
                            Some(SessionEventField {
                                label: field.get("label")?.as_str()?.to_string(),
                                value: field.get("value")?.as_str()?.to_string(),
                                tone: field
                                    .get("tone")
                                    .and_then(|value| value.as_str())
                                    .map(str::to_string),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            body: payload
                .get("body")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })),
        "queue_item" => Some(OutputBlock::QueueItem(QueueItemBlock {
            position: payload
                .get("position")
                .and_then(|v| v.as_u64())
                .unwrap_or(1) as usize,
            text: payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        })),
        _ => None,
    }
}
