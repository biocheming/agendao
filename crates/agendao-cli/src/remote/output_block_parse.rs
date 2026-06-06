use agendao_command_render::output_blocks::{
    BlockTone, MessageBlock, MessagePhase, MessageRole, OutputBlock, QueueItemBlock,
    ReasoningBlock, SchedulerDecisionBlock, SchedulerDecisionField, SchedulerDecisionRenderSpec,
    SchedulerDecisionSection, SchedulerStageBlock, SessionEventBlock, SessionEventField,
    StatusBlock, ToolBlock, ToolPhase,
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
        "scheduler_stage" => Some(OutputBlock::SchedulerStage(Box::new(SchedulerStageBlock {
            stage_id: payload
                .get("stage_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            profile: payload
                .get("profile")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            stage: payload
                .get("stage")
                .and_then(|v| v.as_str())
                .unwrap_or("stage")
                .to_string(),
            title: payload
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Scheduler Stage")
                .to_string(),
            text: payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            stage_index: payload.get("stage_index").and_then(|v| v.as_u64()),
            stage_total: payload.get("stage_total").and_then(|v| v.as_u64()),
            step: payload.get("step").and_then(|v| v.as_u64()),
            status: payload
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            focus: payload
                .get("focus")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            last_event: payload
                .get("last_event")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            waiting_on: payload
                .get("waiting_on")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            estimated_context_tokens: payload
                .get("estimated_context_tokens")
                .and_then(|v| v.as_u64()),
            skill_tree_budget: payload.get("skill_tree_budget").and_then(|v| v.as_u64()),
            skill_tree_truncation_strategy: payload
                .get("skill_tree_truncation_strategy")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            skill_tree_truncated: payload
                .get("skill_tree_truncated")
                .and_then(|v| v.as_bool()),
            retry_attempt: payload.get("retry_attempt").and_then(|v| v.as_u64()),
            activity: payload
                .get("activity")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            loop_budget: payload
                .get("loop_budget")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            available_skill_count: payload
                .get("available_skill_count")
                .and_then(|v| v.as_u64()),
            available_agent_count: payload
                .get("available_agent_count")
                .and_then(|v| v.as_u64()),
            available_category_count: payload
                .get("available_category_count")
                .and_then(|v| v.as_u64()),
            active_skills: payload
                .get("active_skills")
                .and_then(|v| v.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            active_agents: payload
                .get("active_agents")
                .and_then(|v| v.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            active_categories: payload
                .get("active_categories")
                .and_then(|v| v.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            done_agent_count: payload
                .get("done_agent_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            total_agent_count: payload
                .get("total_agent_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            prompt_tokens: payload.get("prompt_tokens").and_then(|v| v.as_u64()),
            context_tokens: payload
                .get("context_tokens")
                .and_then(|v| v.as_u64())
                .or_else(|| payload.get("prompt_tokens").and_then(|v| v.as_u64())),
            completion_tokens: payload.get("completion_tokens").and_then(|v| v.as_u64()),
            reasoning_tokens: payload.get("reasoning_tokens").and_then(|v| v.as_u64()),
            cache_read_tokens: payload.get("cache_read_tokens").and_then(|v| v.as_u64()),
            cache_miss_tokens: payload.get("cache_miss_tokens").and_then(|v| v.as_u64()),
            cache_write_tokens: payload.get("cache_write_tokens").and_then(|v| v.as_u64()),
            decision: parse_scheduler_decision(payload.get("decision")),
            attached_session_id: payload
                .get("attached_session_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        }))),
        _ => None,
    }
}

fn parse_scheduler_decision(payload: Option<&serde_json::Value>) -> Option<SchedulerDecisionBlock> {
    let payload = payload?;
    Some(SchedulerDecisionBlock {
        kind: payload.get("kind")?.as_str()?.to_string(),
        title: payload.get("title")?.as_str()?.to_string(),
        spec: parse_scheduler_decision_spec(payload.get("spec"))?,
        fields: payload
            .get("fields")
            .and_then(|value| value.as_array())
            .map(|fields| {
                fields
                    .iter()
                    .filter_map(|field| {
                        Some(SchedulerDecisionField {
                            label: field.get("label")?.as_str()?.to_string(),
                            value: field.get("value")?.as_str()?.to_string(),
                            tone: field
                                .get("tone")
                                .and_then(|value| value.as_str())
                                .map(|value| value.to_string()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        sections: payload
            .get("sections")
            .and_then(|value| value.as_array())
            .map(|sections| {
                sections
                    .iter()
                    .filter_map(|section| {
                        Some(SchedulerDecisionSection {
                            title: section.get("title")?.as_str()?.to_string(),
                            body: section.get("body")?.as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn parse_scheduler_decision_spec(
    payload: Option<&serde_json::Value>,
) -> Option<SchedulerDecisionRenderSpec> {
    let payload = payload?;
    Some(SchedulerDecisionRenderSpec {
        version: payload.get("version")?.as_str()?.to_string(),
        show_header_divider: payload.get("show_header_divider")?.as_bool()?,
        field_order: payload.get("field_order")?.as_str()?.to_string(),
        field_label_emphasis: payload.get("field_label_emphasis")?.as_str()?.to_string(),
        status_palette: payload.get("status_palette")?.as_str()?.to_string(),
        section_spacing: payload.get("section_spacing")?.as_str()?.to_string(),
        update_policy: payload.get("update_policy")?.as_str()?.to_string(),
    })
}
