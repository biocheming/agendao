//! 水 — FrontendEvent → SessionStore Signal mapping.

use agendao_server_core::frontend_events::FrontendEvent;
use agendao_client::SessionRunStatusKind;
use crate::store::session_store::SessionStore;
use crate::store::types::*;

pub fn apply_frontend_event(event: &FrontendEvent, session: &SessionStore) -> Option<String> {
    match event {
        FrontendEvent::SessionRuntimeReplaced { session_id, runtime } => {
            let status = match runtime.run_status {
                SessionRunStatusKind::Idle => RunStatus::Idle,
                SessionRunStatusKind::Running => RunStatus::Running,
                SessionRunStatusKind::WaitingOnUser => RunStatus::WaitingUser,
                // 轮次内的活跃相位仍算"运行中"：工具一调用，
                // runtime_state.tool_started 就把 run_status 置 WaitingOnTool，
                // 随后的 TopologyChanged/SessionStatus 快照重投影会把它广播出来；
                // 此前 `_ => Idle` 把这些相位映射成 Idle，spinner 在每次工具
                // 调用/压缩期间被冻结（"墨韵"恒静止的主因）。Cancelling 同理：
                // 取消落定前这一轮仍在跑。
                SessionRunStatusKind::Compacting
                | SessionRunStatusKind::WaitingOnTool
                | SessionRunStatusKind::Cancelling => RunStatus::Running,
                // Blocked/Sleeping 是 session 级静置，不算运行。
                _ => RunStatus::Idle,
            };
            session.run_status.set(status);
            Some(session_id.clone())
        }

        FrontendEvent::OutputBlockAppended { session_id, block, id, .. } => {
            let kind = block.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let phase = block.get("phase").and_then(|v| v.as_str());
            let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let role = block.get("role").and_then(|v| v.as_str()).unwrap_or("");
            // Tool blocks carry `name` (web schema), not `tool_name`. Older
            // event-handler matches read `tool_name` and got an empty
            // string, which made every tool render as "?".
            let tool_name = block.get("name").and_then(|v| v.as_str())
                .or_else(|| block.get("tool_name").and_then(|v| v.as_str()));
            // Tool detail (e.g. result text) lives under `detail` per the
            // web schema; previously we only read `text` and missed
            // results entirely.
            let detail = block.get("detail").and_then(|v| v.as_str()).unwrap_or("");
            let bid = id.as_deref().unwrap_or("");

            // Server emits phases per agendao_command::agent_presenter::phase_to_web:
            //   message:   start | delta | end | full
            //   reasoning: start | delta | end | full
            //   tool:      start | running | done | error
            // `complete` was a stale label that never appeared on the wire,
            // which is why the transcript stayed silent for assistant text.
            match kind {
                "message" => {
                    // User messages echo back from history rebuild on session
                    // load — push as a UserPrompt block, not an assistant
                    // delta. dispatch() already pushes locally on submit so
                    // the live event is mostly a duplicate, but session
                    // restore relies on this branch.
                    if role == "user" {
                        if matches!(phase, Some("full") | Some("end")) && !text.is_empty() {
                            session.push_user_message(bid, text);
                        }
                        return Some(session_id.clone());
                    }
                    match phase {
                        // delta — stream-extend the running assistant block
                        Some("delta") => session.push_assistant_delta(bid, text),
                        // full — 快照语义见 SessionStore::apply_assistant_snapshot：
                        // start 后首个 full 追加新段（逐 chunk 片段流），段内/
                        // 无 start 的 full 按 merge 合并（累积快照前缀替换）。
                        Some("full") => {
                            if !text.is_empty() {
                                session.apply_assistant_snapshot(bid, text);
                            }
                        }
                        Some("end") => {
                            // `end` carries no new text; just mark the loop
                            // as idle so the prompt bar reactivates.
                            session.run_status.set(RunStatus::Idle);
                        }
                        Some("start") => {
                            // 新段开始：下一条同 id 的 full 追加为新片段
                            // （逐 chunk 生命周期流）。单生命周期流里 start
                            // 只出现一次，随后的累积 full 走段内 merge。
                            session.mark_stream_segment_start("m", bid);
                        }
                        _ => {}
                    }
                }
                "reasoning" => {
                    match phase {
                        Some("delta") => {
                            if !text.is_empty() { session.push_thinking(bid, text); }
                        }
                        // full — 同 message 分支：start 后追加新段，否则 merge。
                        Some("full") => {
                            if !text.is_empty() { session.apply_thinking_snapshot(bid, text); }
                        }
                        Some("start") => {
                            session.mark_stream_segment_start("r", bid);
                        }
                        _ => {}
                    }
                }
                "tool" => {
                    let name = tool_name.unwrap_or("?");
                    match phase {
                        Some("start") => {
                            // start may carry a `detail` preview already
                            // (e.g. argument summary); record it so the
                            // transcript shows context before the result.
                            session.upsert_tool_call(bid, name, detail, ToolPhase::Starting);
                        }
                        Some("running") => {
                            session.upsert_tool_call(bid, name, detail, ToolPhase::Running);
                        }
                        Some("done") => {
                            session.upsert_tool_call(bid, name, "", ToolPhase::Done);
                            // Server emits the tool result as a separate
                            // `done`-phase block carrying detail; preserve
                            // it as a ToolResult so users can read what the
                            // tool produced.
                            if !detail.is_empty() {
                                session.push_tool_result(bid, name, detail, false);
                            }
                        }
                        Some("error") => {
                            session.upsert_tool_call(bid, name, "", ToolPhase::Done);
                            session.push_tool_result(bid, name, detail, true);
                        }
                        _ => {}
                    }
                }
                "scheduler_stage" => {
                    // SchedulerStage block carries: stage_id, stage, status,
                    // focus, last_event, waiting_on, activity, plus token
                    // counts. Use `stage` (or `title`) as the display name
                    // and `status` as the state label.
                    let name = block.get("stage").and_then(|v| v.as_str())
                        .or_else(|| block.get("title").and_then(|v| v.as_str()))
                        .unwrap_or("stage");
                    let status = block.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    // Build a one-line metadata summary out of the most
                    // useful surface fields without overwhelming the row.
                    let mut bits: Vec<String> = Vec::new();
                    if let Some(focus) = block.get("focus").and_then(|v| v.as_str()) {
                        if !focus.is_empty() { bits.push(format!("focus: {focus}")); }
                    }
                    if let Some(activity) = block.get("activity").and_then(|v| v.as_str()) {
                        if !activity.is_empty() { bits.push(format!("activity: {activity}")); }
                    }
                    if let Some(waiting) = block.get("waiting_on").and_then(|v| v.as_str()) {
                        if !waiting.is_empty() { bits.push(format!("waiting on: {waiting}")); }
                    }
                    let metadata = (!bits.is_empty()).then(|| bits.join("\n"));
                    session.push_stage(bid, name, status, metadata);
                }
                "status" => {
                    // Plain notice line — matches web's StatusBlock.
                    if !text.is_empty() {
                        session.push_notice(bid, text);
                    }
                }
                "session_event" => {
                    let title = block.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let summary = block.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                    let line = if summary.is_empty() { title.to_string() } else { format!("{title}: {summary}") };
                    if !line.is_empty() {
                        session.push_notice(bid, &line);
                    }
                }
                "skill" => {
                    session.push_skill(bid, tool_name.unwrap_or(text));
                }
                "compaction" => {
                    let before = block.get("before").and_then(|v| v.as_u64()).unwrap_or(0);
                    let after = block.get("after").and_then(|v| v.as_u64()).unwrap_or(0);
                    session.push_compaction(bid, before, after);
                }
                _ => {}
            }
            Some(session_id.clone())
        }

        FrontendEvent::ToolCallUpsert { session_id, tool_call_id, tool_name, phase } => {
            let tp = match phase {
                agendao_server_core::runtime_events::ToolCallPhase::Start => ToolPhase::Starting,
                agendao_server_core::runtime_events::ToolCallPhase::Complete => ToolPhase::Done,
            };
            session.set_active_tool(tool_call_id, tool_name, tp);
            Some(session_id.clone())
        }

        FrontendEvent::QuestionUpsert { session_id, .. } => {
            session.run_status.set(RunStatus::WaitingUser);
            Some(session_id.clone())
        }
        FrontendEvent::QuestionRemoved { session_id, .. } => {
            session.run_status.set(RunStatus::Running);
            Some(session_id.clone())
        }
        FrontendEvent::PermissionUpsert { session_id, .. } => {
            session.run_status.set(RunStatus::WaitingUser);
            Some(session_id.clone())
        }
        FrontendEvent::PermissionRemoved { session_id, .. } => {
            session.run_status.set(RunStatus::Running);
            Some(session_id.clone())
        }

        FrontendEvent::SessionProjectionReplaced { session_id, usage, topology, stages, context_compaction_summary, .. } => {
            if let Some(ref u) = usage {
                session.set_token_usage(
                    u.input_tokens, u.output_tokens, u.reasoning_tokens,
                    u.cache_read_tokens, u.cache_miss_tokens, u.cache_write_tokens,
                    u.context_tokens, u.total_cost,
                );
            }
            // Build execution topology for future telemetry display.
            // **Do not** write into `session_nodes` — that field is the
            // session *navigation* tree (parent_id fork tree from
            // `reload_session_list`), not runtime agent/stage topology.
            // Overwriting it here was the root cause of sidebar clicks
            // having no NavigateSession intent (土律·第十条·可观测性).
            let _topology = topology;
            // Compute context meter % from compaction summary
            if let Some(ref cs) = context_compaction_summary {
                if let (Some(live), Some(limit)) = (cs.live_context_tokens, cs.limit_tokens) {
                    if limit > 0 {
                        let pct = ((live as f64 / limit as f64) * 100.0) as u8;
                        session.set_context_pct(pct);
                    }
                }
            }
            // Process stage summaries (bulk update)
            for stage in stages {
                let status = format!("{:?}", stage.status);
                let stage_id = &stage.stage_id;
                // Build a formatted detail block for the stage card
                let mut detail_lines: Vec<String> = Vec::new();

                // Step progress (if stage has sub-steps)
                if let (Some(s), Some(st)) = (stage.step, stage.step_total) {
                    detail_lines.push(format!(" step {}/{}", s, st));
                }

                // Activity + focus
                if let Some(ref act) = stage.activity {
                    detail_lines.push(format!(" ▶ {}", act));
                }
                if let Some(ref f) = stage.focus {
                    detail_lines.push(format!(" ▸ focus: {}", f));
                }

                // Retry info
                if let Some(r) = stage.retry_attempt {
                    if r > 0 { detail_lines.push(format!(" ↻ retry #{}", r)); }
                }

                // Token usage
                let mut token_parts = Vec::new();
                if let Some(t) = stage.prompt_tokens { token_parts.push(format!("prompt:{}", t)); }
                if let Some(t) = stage.completion_tokens { token_parts.push(format!("comp:{}", t)); }
                if let Some(t) = stage.reasoning_tokens { token_parts.push(format!("reason:{}", t)); }
                if !token_parts.is_empty() {
                    detail_lines.push(format!("📊 tokens: {}", token_parts.join(" ")));
                }

                // Cache efficiency
                let mut cache_parts = Vec::new();
                if let Some(t) = stage.cache_read_tokens { cache_parts.push(format!("read:{}", t)); }
                if let Some(t) = stage.cache_miss_tokens { cache_parts.push(format!("miss:{}", t)); }
                if !cache_parts.is_empty() {
                    detail_lines.push(format!("💾 cache: {}", cache_parts.join(" ")));
                }

                // Context pressure
                if let Some(t) = stage.estimated_context_tokens {
                    detail_lines.push(format!("📐 ctx: {}K", t / 1000));
                }

                // Agent/tool/attached count
                let mut counts = Vec::new();
                if stage.active_agent_count > 0 { counts.push(format!("agents:{}", stage.active_agent_count)); }
                if stage.active_tool_count > 0 { counts.push(format!("tools:{}", stage.active_tool_count)); }
                if stage.attached_session_count > 0 { counts.push(format!("subs:{}", stage.attached_session_count)); }
                if !counts.is_empty() {
                    detail_lines.push(format!("👤 {}", counts.join(" ")));
                }

                // Waiting on
                if let Some(ref w) = stage.waiting_on {
                    detail_lines.push(format!("⏳ waiting: {}", w));
                }

                let meta_str = if detail_lines.is_empty() { None } else { Some(detail_lines.join("\n")) };
                // Only push if status indicates progress
                if stage.step.is_some() || stage.prompt_tokens.is_some() {
                    let label = format!("{} [{}/{}] {}",
                        stage.stage_name,
                        stage.step.unwrap_or(0),
                        stage.step_total.unwrap_or(0),
                        if !stage.focus.as_deref().unwrap_or("").is_empty() {
                            format!("({})", stage.focus.as_deref().unwrap_or(""))
                        } else { String::new() },
                    );
                    session.push_stage(stage_id, &label, &status, meta_str);
                } else {
                    session.push_stage(stage_id, &stage.stage_name, &status, meta_str);
                }
            }
            Some(session_id.clone())
        }

        FrontendEvent::DiffReplaced { session_id, .. } => Some(session_id.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output_block(id: &str, kind: &str, phase: &str, text: &str) -> FrontendEvent {
        FrontendEvent::OutputBlockAppended {
            session_id: "s1".into(),
            block: serde_json::json!({"kind": kind, "phase": phase, "text": text}),
            id: Some(id.into()),
            live_identity: None,
        }
    }

    /// 回归（Bug B）：单生命周期内服务端 coalesce 把 delta 归并成**累积全文**
    /// 快照逐帧下发（phase="full"），段内必须合并为最新快照，而非逐帧拼接——
    /// 否则渲染出 "TheThe answer toThe answer to 1..."。
    #[test]
    fn cumulative_full_snapshots_replace_within_segment() {
        let session = SessionStore::new();
        apply_frontend_event(&output_block("m1", "message", "start", ""), &session);
        for snap in ["The", "The answer to", "The answer to 1+1 is 2"] {
            apply_frontend_event(&output_block("m1", "message", "full", snap), &session);
        }
        let msgs = session.messages.get();
        assert_eq!(msgs.len(), 1, "同一 id 的快照应合并为一个块");
        match &msgs[0] {
            TranscriptBlock::AssistantMsg { content, .. } => {
                assert_eq!(content, "The answer to 1+1 is 2", "累积快照必须替换而非拼接");
            }
            _ => panic!("expected AssistantMsg"),
        }
    }

    /// 回归（逐 chunk 生命周期形态）：每个 chunk 都是独立 start/full/end，
    /// full 只携带该 chunk 的片段（实测 deepseek-v4-flash / qwen 流式），
    /// start 后必须追加拼接，否则只剩最后一截。
    #[test]
    fn fragment_full_snapshots_append_after_each_start() {
        let session = SessionStore::new();
        for frag in ["1", "+", "1", " =", " **", "2", "**"] {
            apply_frontend_event(&output_block("m1", "message", "start", ""), &session);
            apply_frontend_event(&output_block("m1", "message", "full", frag), &session);
            apply_frontend_event(&output_block("m1", "message", "end", ""), &session);
        }
        let msgs = session.messages.get();
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            TranscriptBlock::AssistantMsg { content, .. } => {
                assert_eq!(content, "1+1 = **2**", "逐 chunk 片段必须按段拼接");
            }
            _ => panic!("expected AssistantMsg"),
        }
    }

    /// reasoning 的逐 chunk 片段流同样按段拼接；段内累积快照按前缀替换。
    #[test]
    fn reasoning_full_snapshots_segmented_merge() {
        let session = SessionStore::new();
        // 逐 chunk 片段
        for frag in ["think", "ing longer"] {
            apply_frontend_event(&output_block("r1", "reasoning", "start", ""), &session);
            apply_frontend_event(&output_block("r1", "reasoning", "full", frag), &session);
        }
        let msgs = session.messages.get();
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            TranscriptBlock::Thinking { content, .. } => {
                assert_eq!(content, "thinking longer");
            }
            _ => panic!("expected Thinking"),
        }

        // 同 id 单生命周期累积流
        let session2 = SessionStore::new();
        apply_frontend_event(&output_block("r2", "reasoning", "start", ""), &session2);
        for snap in ["think", "thinking longer"] {
            apply_frontend_event(&output_block("r2", "reasoning", "full", snap), &session2);
        }
        let msgs2 = session2.messages.get();
        match &msgs2[0] {
            TranscriptBlock::Thinking { content, .. } => {
                assert_eq!(content, "thinking longer", "累积快照不得拼接成 thinkthinking longer");
            }
            _ => panic!("expected Thinking"),
        }
    }

    /// delta 分支不变：legacy（无 live_identity）透传的增量仍逐段追加。
    #[test]
    fn deltas_still_append() {
        let session = SessionStore::new();
        apply_frontend_event(&output_block("m1", "message", "delta", "Hello"), &session);
        apply_frontend_event(&output_block("m1", "message", "delta", " World"), &session);
        let msgs = session.messages.get();
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            TranscriptBlock::AssistantMsg { content, .. } => {
                assert_eq!(content, "Hello World");
            }
            _ => panic!("expected AssistantMsg"),
        }
    }

    /// reasoning 的 full 快照同样是累积全文：替换而非追加。
    #[test]
    fn reasoning_full_snapshots_replace() {
        let session = SessionStore::new();
        for snap in ["think", "thinking longer"] {
            apply_frontend_event(&output_block("r1", "reasoning", "full", snap), &session);
        }
        let msgs = session.messages.get();
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            TranscriptBlock::Thinking { content, .. } => {
                assert_eq!(content, "thinking longer");
            }
            _ => panic!("expected Thinking"),
        }
    }
}
