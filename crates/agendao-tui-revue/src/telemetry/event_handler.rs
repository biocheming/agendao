//! 水 — FrontendEvent → SessionStore Signal mapping.

use crate::store::session_store::SessionStore;
use crate::store::types::*;
use agendao_client::SessionRunStatusKind;
use agendao_server_core::frontend_events::FrontendEvent;

pub fn apply_frontend_event(event: &FrontendEvent, session: &SessionStore) -> Option<String> {
    match event {
        FrontendEvent::SessionRuntimeReplaced {
            session_id,
            runtime,
        } => {
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
                // U9：Compacting 拆出独立态（展示层可辨"压缩中"），行为口径
                // 仍与 Running 一致。
                SessionRunStatusKind::Compacting => RunStatus::Compacting,
                SessionRunStatusKind::WaitingOnTool | SessionRunStatusKind::Cancelling => {
                    RunStatus::Running
                }
                // Blocked/Sleeping 是 session 级静置，不算运行。
                _ => RunStatus::Idle,
            };
            session.set_run_status(status);
            // runtime 快照是活跃 sandbox 集合的权威替换——重连/换会话后
            // 事件流残迹不得存活（§8：没有条目就没有"sandboxed"展示）。
            session.set_active_sandboxes(runtime.active_sandbox.clone());
            Some(session_id.clone())
        }

        FrontendEvent::SandboxExecutionUpsert {
            session_id,
            execution,
        } => {
            session.sandbox_execution_upsert(execution.clone());
            Some(session_id.clone())
        }

        FrontendEvent::SandboxExecutionRemoved {
            session_id,
            execution_id,
            ..
        } => {
            session.sandbox_execution_removed(execution_id);
            Some(session_id.clone())
        }

        FrontendEvent::OutputBlockAppended {
            session_id,
            block,
            id,
            ..
        } => {
            let kind = block.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let phase = block.get("phase").and_then(|v| v.as_str());
            let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let role = block.get("role").and_then(|v| v.as_str()).unwrap_or("");
            // Tool blocks carry `name` (web schema), not `tool_name`. Older
            // event-handler matches read `tool_name` and got an empty
            // string, which made every tool render as "?".
            let tool_name = block
                .get("name")
                .and_then(|v| v.as_str())
                .or_else(|| block.get("tool_name").and_then(|v| v.as_str()));
            // Tool detail (e.g. result text) lives under `detail` per the
            // web schema; previously we only read `text` and missed
            // results entirely.
            let detail = block.get("detail").and_then(|v| v.as_str()).unwrap_or("");
            // Unified diff preview (edit/write/apply_patch) rides in
            // `display.preview = {kind:"diff", text, truncated}`
            // The server block projection serializes this field as JSON. Parse once here; only the tool
            // `done` branch consumes it. Missing/non-diff preview → None,
            // the block falls back to plain `detail` text as before.
            let diff_preview = parse_diff_preview(block);
            let bid = id.as_deref().unwrap_or("");

            // Server emits canonical block phases:
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
                            session.set_run_status(RunStatus::Idle);
                            session.finalize_stream_block("m", bid);
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
                            if !text.is_empty() {
                                session.push_thinking(bid, text);
                            }
                        }
                        // full — 同 message 分支：start 后追加新段，否则 merge。
                        Some("full") => {
                            if !text.is_empty() {
                                session.apply_thinking_snapshot(bid, text);
                            }
                        }
                        Some("start") => {
                            session.mark_stream_segment_start("r", bid);
                        }
                        Some("end") => session.finalize_stream_block("r", bid),
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
                            // tool produced. A diff preview (edit/write/
                            // apply_patch) also justifies a ToolResult even
                            // when `detail` is empty — the diff IS the result.
                            if !detail.is_empty() || diff_preview.is_some() {
                                session.push_tool_result(bid, name, detail, false, diff_preview);
                            }
                        }
                        Some("error") => {
                            session.upsert_tool_call(bid, name, "", ToolPhase::Done);
                            session.push_tool_result(bid, name, detail, true, None);
                        }
                        _ => {}
                    }
                }
                "status" => {
                    // Plain notice line — matches web's StatusBlock.
                    if !text.is_empty() {
                        session.push_notice(bid, text);
                    }
                }
                "session_event" => {
                    apply_session_event_block(block, bid, session);
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

        FrontendEvent::ToolCallUpsert {
            session_id,
            tool_call_id,
            tool_name,
            phase,
        } => {
            let tp = match phase {
                agendao_server_core::runtime_events::ToolCallPhase::Start => ToolPhase::Starting,
                agendao_server_core::runtime_events::ToolCallPhase::Complete => ToolPhase::Done,
            };
            // Tool lifecycle is itself authoritative evidence that a tool ran.
            // Keep it in the transcript as well as the sidebar active-tool set;
            // some provider paths do not emit a parallel OutputBlock::Tool start,
            // which previously made the entire tool invocation invisible.
            session.upsert_tool_call(tool_call_id, tool_name, "", tp);
            session.set_active_tool(tool_call_id, tool_name, tp);
            Some(session_id.clone())
        }

        FrontendEvent::QuestionUpsert { session_id, .. } => {
            session.set_run_status(RunStatus::WaitingUser);
            Some(session_id.clone())
        }
        FrontendEvent::QuestionRemoved { session_id, .. } => {
            session.set_run_status(RunStatus::Running);
            Some(session_id.clone())
        }
        FrontendEvent::PermissionUpsert { session_id, .. } => {
            session.set_run_status(RunStatus::WaitingUser);
            Some(session_id.clone())
        }
        FrontendEvent::PermissionRemoved { session_id, .. } => {
            session.set_run_status(RunStatus::Running);
            Some(session_id.clone())
        }

        FrontendEvent::SessionProjectionReplaced {
            session_id,
            usage,
            topology,
            context_compaction_summary,
            ..
        } => {
            if session.session_id.get().as_deref() != Some(session_id.as_str()) {
                return None;
            }
            if let Some(ref u) = usage {
                session.set_token_usage(TokenUsage {
                    input: u.input_tokens,
                    output: u.output_tokens,
                    reasoning: u.reasoning_tokens,
                    total: u.input_tokens + u.output_tokens + u.reasoning_tokens,
                    cache_read: u.cache_read_tokens,
                    cache_miss: u.cache_miss_tokens,
                    cache_write: u.cache_write_tokens,
                    context_tokens: u.context_tokens,
                    total_cost: u.total_cost,
                });
            }
            // Build execution topology for future telemetry display.
            // **Do not** write into `session_nodes` — that field is the
            // session *navigation* tree (parent_id fork tree from
            // `reload_session_list`), not runtime agent/stage topology.
            // Overwriting it here was the root cause of sidebar clicks
            // having no NavigateSession intent (土律·第十条·可观测性).
            if let Some(topology) = topology {
                let topology_matches_event = topology.session_id.as_str() == session_id.as_str();
                let has_explicit_subagent = topology.roots.iter().any(|node| {
                    fn walk(node: &agendao_api::SessionExecutionNode) -> bool {
                        let explicit = node.metadata.as_ref().is_some_and(|m| {
                            m.get("subagent").and_then(|v| v.as_bool()) == Some(true)
                                || m.get("role").and_then(|v| v.as_str()) == Some("subagent")
                        });
                        explicit || node.children.iter().any(walk)
                    }
                    walk(node)
                });
                if !topology_matches_event {
                    // A stale, conflicting, or cross-session topology cannot
                    // alter either M7 summary or M9 panel facts.
                    return Some(session_id.clone());
                } else if has_explicit_subagent && !session.replace_subagent_projection(topology) {
                    return Some(session_id.clone());
                } else if !has_explicit_subagent {
                    session.subagent_projection.set(None);
                }
                fn walk(
                    nodes: &[agendao_api::SessionExecutionNode],
                    tools: &mut usize,
                    subagents: &mut usize,
                ) {
                    for node in nodes {
                        if matches!(node.kind, agendao_api::ExecutionKind::ToolCall)
                            && matches!(node.status, agendao_api::ExecutionStatus::Running)
                        {
                            *tools += 1;
                        }
                        if matches!(node.kind, agendao_api::ExecutionKind::SchedulerNode)
                            && node.metadata.as_ref().is_some_and(|m| {
                                m.get("subagent").and_then(|v| v.as_bool()).unwrap_or(false)
                                    || m.get("role").and_then(|v| v.as_str()) == Some("subagent")
                            })
                            && !matches!(node.status, agendao_api::ExecutionStatus::Done)
                        {
                            *subagents += 1;
                        }
                        walk(&node.children, tools, subagents);
                    }
                }
                let mut running_tools = 0;
                let mut subagents = 0;
                walk(&topology.roots, &mut running_tools, &mut subagents);
                session.set_topology_summary(TopologySummary {
                    running_tools,
                    subagents: (subagents > 0).then_some(subagents),
                });
            } else {
                session.clear_topology_summary();
                session.subagent_projection.set(None);
            }
            // Compute context meter % from compaction summary
            if let Some(ref cs) = context_compaction_summary {
                if let (Some(live), Some(limit)) = (cs.live_context_tokens, cs.limit_tokens) {
                    if limit > 0 {
                        let pct = ((live as f64 / limit as f64) * 100.0) as u8;
                        session.set_context_pct(pct);
                    }
                }
                // 分母不再算完 pct 即弃（水律：回流数据是 /compact 决策依据）。
                session.set_context_limit(cs.limit_tokens);
            }
            Some(session_id.clone())
        }

        FrontendEvent::DiffReplaced { session_id, diffs } => {
            // Replace 语义：每轮结束下发全量集合，直接替换（不累加）。
            session.set_diff_summary(
                diffs
                    .iter()
                    .map(|d| DiffStat {
                        path: d.path.clone(),
                        additions: d.additions,
                        deletions: d.deletions,
                    })
                    .collect(),
            );
            Some(session_id.clone())
        }

        FrontendEvent::TodoReplaced { session_id, todos } => {
            // Replace 语义：todowrite 全量下发。内容无变化时 push_todo_list
            // 返回 false —— 不触 Signal、不标脏、不重绘。
            let items: Vec<TodoItem> = todos
                .iter()
                .map(|t| TodoItem {
                    content: t.content.clone(),
                    status: todo_status_from_str(&t.status),
                })
                .collect();
            if session.push_todo_list("todos", items, None) {
                Some(session_id.clone())
            } else {
                None
            }
        }
        FrontendEvent::TaskLedgerReplaced {
            session_id, ledger, ..
        } => {
            if session.apply_task_ledger_snapshot(ledger.clone()) {
                Some(session_id.clone())
            } else {
                None
            }
        }
        // F6：运行期错误（如中途 provider 失败）即时上屏——此前 ServerEvent::Error
        // 在投影层被投影为空，错误要到下一次 runtime 快照才以 RunStatus 出现。
        FrontendEvent::SessionError {
            session_id, error, ..
        } => {
            session.push_notice(&format!("err-{}", session_id), &format!("⚠ {error}"));
            Some(session_id.clone())
        }
        // Global config change: the TUI reads config through its own
        // authorities on demand; no transcript region to invalidate here.
        FrontendEvent::ConfigUpdated => None,
    }
}

/// Project a typed session event into its transcript presentation.
///
/// Live events and history restoration both call this authority. Scheduler
/// progress receives a stable, upserted stage card; other events retain their
/// compact notice presentation.
pub(crate) fn apply_session_event_block(
    block: &serde_json::Value,
    fallback_id: &str,
    session: &SessionStore,
) {
    let title = block
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let event = block
        .get("event")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if event == "scheduler_step" {
        let id = block
            .get("id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .unwrap_or(fallback_id);
        let status = block
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("running");
        let message = block
            .get("body")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let fields = block
            .get("fields")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|field| {
                let label = field.get("label")?.as_str()?.trim();
                let value = field.get("value")?.as_str()?.trim();
                (!label.is_empty() && !value.is_empty()).then(|| StageField {
                    label: label.to_string(),
                    value: value.to_string(),
                })
            })
            .collect();
        session.push_stage(id, title, status, message, fields);
        return;
    }

    let summary = block
        .get("summary")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let line = if summary.is_empty() {
        title.to_string()
    } else {
        format!("{title}: {summary}")
    };
    if !line.is_empty() {
        session.push_notice(fallback_id, &line);
    }
}

/// Map a wire status string to the store TodoStatus (same mapping as the
/// one-shot REST fetch in keymap::eager_load_session_messages).
pub fn todo_status_from_str(status: &str) -> TodoStatus {
    match status {
        "completed" | "done" => TodoStatus::Completed,
        "in_progress" => TodoStatus::InProgress,
        "cancelled" | "canceled" => TodoStatus::Cancelled,
        _ => TodoStatus::Pending,
    }
}

/// 从 output block 的 `display.preview` 提取 unified diff 预览。
/// 仅 kind=="diff" 且 text 非空时返回 Some；其余（缺 display/缺 preview/
/// 其它 kind/空文本）一律 None —— 调用方回退 `detail` 纯文本旧路径。
fn parse_diff_preview(block: &serde_json::Value) -> Option<DiffPreview> {
    let preview = block.get("display")?.get("preview")?;
    if preview.get("kind").and_then(|k| k.as_str()) != Some("diff") {
        return None;
    }
    let text = preview.get("text").and_then(|t| t.as_str()).unwrap_or("");
    if text.is_empty() {
        return None;
    }
    Some(DiffPreview {
        text: text.to_string(),
        truncated: preview
            .get("truncated")
            .and_then(|t| t.as_bool())
            .unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection_event(
        session_id: &str,
        topology: Option<agendao_api::SessionExecutionTopology>,
    ) -> FrontendEvent {
        FrontendEvent::SessionProjectionReplaced {
            session_id: session_id.into(),
            topology,
            usage: None,
            usage_books: None,
            context_compaction_summary: None,
            context_compaction_lifecycle_summary: None,
            cache_semantics: None,
            context_closure_contract: None,
        }
    }

    fn execution_node(
        id: &str,
        kind: agendao_api::ExecutionKind,
        status: agendao_api::ExecutionStatus,
        metadata: Option<serde_json::Value>,
    ) -> agendao_api::SessionExecutionNode {
        agendao_api::SessionExecutionNode {
            id: id.into(),
            kind,
            status,
            label: None,
            parent_id: None,
            waiting_on: None,
            recent_event: None,
            started_at: 0,
            updated_at: 0,
            metadata,
            children: vec![],
        }
    }

    #[test]
    fn projection_topology_is_session_scoped_and_requires_explicit_subagent_metadata() {
        let session = SessionStore::new();
        session.set_session_id("s1");
        let topology = agendao_api::SessionExecutionTopology {
            session_id: "s1".into(),
            active_count: 2,
            done_count: 0,
            running_count: 2,
            waiting_count: 0,
            cancelling_count: 0,
            retry_count: 0,
            updated_at: None,
            roots: vec![
                execution_node(
                    "tool",
                    agendao_api::ExecutionKind::ToolCall,
                    agendao_api::ExecutionStatus::Running,
                    None,
                ),
                execution_node(
                    "scheduler",
                    agendao_api::ExecutionKind::SchedulerNode,
                    agendao_api::ExecutionStatus::Running,
                    None,
                ),
            ],
        };
        apply_frontend_event(&projection_event("s1", Some(topology)), &session);
        assert_eq!(session.running_tool_count(), 1);
        assert_eq!(
            session.subagent_count(),
            None,
            "ordinary scheduler node is not a subagent"
        );

        let explicit = agendao_api::SessionExecutionTopology {
            session_id: "s1".into(),
            active_count: 1,
            done_count: 0,
            running_count: 1,
            waiting_count: 0,
            cancelling_count: 0,
            retry_count: 0,
            updated_at: None,
            roots: vec![execution_node(
                "agent",
                agendao_api::ExecutionKind::SchedulerNode,
                agendao_api::ExecutionStatus::Running,
                Some(serde_json::json!({"subagent": true})),
            )],
        };
        apply_frontend_event(&projection_event("s1", Some(explicit.clone())), &session);
        assert_eq!(session.subagent_count(), Some(1));
        apply_frontend_event(&projection_event("s1", Some(explicit)), &session);
        assert_eq!(
            session.subagent_count(),
            Some(1),
            "duplicate replacement is idempotent"
        );
        apply_frontend_event(&projection_event("other", None), &session);
        assert_eq!(
            session.subagent_count(),
            Some(1),
            "cross-session snapshot must be ignored"
        );
    }

    fn output_block(id: &str, kind: &str, phase: &str, text: &str) -> FrontendEvent {
        FrontendEvent::OutputBlockAppended {
            session_id: "s1".into(),
            block: serde_json::json!({"kind": kind, "phase": phase, "text": text}),
            id: Some(id.into()),
            live_identity: None,
        }
    }

    fn scheduler_step_block(
        step: u32,
        status: &str,
        message: &str,
        tools: Option<&str>,
    ) -> FrontendEvent {
        let mut fields = vec![serde_json::json!({"label": "Agent", "value": "worker"})];
        if let Some(tools) = tools {
            fields.push(serde_json::json!({"label": "Tools", "value": tools}));
        }
        FrontendEvent::OutputBlockAppended {
            session_id: "s1".into(),
            block: serde_json::json!({
                "kind": "session_event",
                "event": "scheduler_step",
                "title": format!("Scheduler step {step}/16"),
                "status": status,
                "body": message,
                "fields": fields,
            }),
            id: Some("scheduler-progress:execute".into()),
            live_identity: None,
        }
    }

    #[test]
    fn scheduler_steps_replace_one_typed_card_in_place() {
        let session = SessionStore::new();
        apply_frontend_event(
            &scheduler_step_block(1, "running", "first\nline two\nline three\nline four", None),
            &session,
        );
        apply_frontend_event(
            &scheduler_step_block(1, "complete", "first complete", Some("2")),
            &session,
        );
        apply_frontend_event(
            &scheduler_step_block(2, "running", "second step", None),
            &session,
        );

        let messages = session.messages.get();
        assert_eq!(messages.len(), 1, "all scheduler steps share one card");
        match &messages[0] {
            TranscriptBlock::StageUpdate {
                id,
                name,
                status,
                message,
                fields,
                fold,
            } => {
                assert_eq!(id, "scheduler-progress:execute");
                assert_eq!(name, "Scheduler step 2/16");
                assert_eq!(status, "running");
                assert_eq!(message, "second step");
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].label, "Agent");
                assert_eq!(*fold, FoldState::Truncated);
            }
            other => panic!("expected scheduler stage card, got {other:?}"),
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
                assert_eq!(
                    content, "The answer to 1+1 is 2",
                    "累积快照必须替换而非拼接"
                );
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
                assert_eq!(
                    content, "thinking longer",
                    "累积快照不得拼接成 thinkthinking longer"
                );
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

    fn tool_done_block(
        id: &str,
        name: &str,
        detail: &str,
        preview: serde_json::Value,
    ) -> FrontendEvent {
        FrontendEvent::OutputBlockAppended {
            session_id: "s1".into(),
            block: serde_json::json!({
                "kind": "tool", "name": name, "phase": "done",
                "detail": detail, "display": { "preview": preview },
            }),
            id: Some(id.into()),
            live_identity: None,
        }
    }

    /// kind=="diff" 的 display.preview 落地为 ToolResult.diff 载荷（diff 即本体，
    /// detail 为空也必须出块）；diff 块默认 Truncated（3 行预览可审阅）。
    #[test]
    fn tool_done_diff_preview_lands_on_tool_result() {
        let session = SessionStore::new();
        let diff_text = "@@ -1,2 +1,2 @@\n-old\n+new\n ctx";
        apply_frontend_event(
            &tool_done_block(
                "t1",
                "edit",
                "",
                serde_json::json!({
                    "kind": "diff", "text": diff_text, "truncated": true,
                }),
            ),
            &session,
        );
        let msgs = session.messages.get();
        let result = msgs
            .iter()
            .find(|b| matches!(b, TranscriptBlock::ToolResult { .. }))
            .expect("detail 为空但 diff 存在，必须出 ToolResult");
        match result {
            TranscriptBlock::ToolResult { diff, fold, .. } => {
                let d = diff.as_ref().expect("diff 载荷必须落地");
                assert_eq!(d.text, diff_text);
                assert!(d.truncated, "truncated 标记必须透传");
                assert_eq!(*fold, FoldState::Truncated, "diff 块默认 3 行预览");
            }
            _ => unreachable!(),
        }
    }

    /// 非 diff kind 的 preview（如 text/image）不消费，回退 detail 纯文本旧路径。
    #[test]
    fn tool_done_non_diff_preview_falls_back_to_detail() {
        let session = SessionStore::new();
        apply_frontend_event(
            &tool_done_block(
                "t1",
                "read",
                "file body",
                serde_json::json!({
                    "kind": "text", "text": "whatever", "truncated": false,
                }),
            ),
            &session,
        );
        let msgs = session.messages.get();
        match msgs
            .iter()
            .find(|b| matches!(b, TranscriptBlock::ToolResult { .. }))
        {
            Some(TranscriptBlock::ToolResult {
                result, diff, fold, ..
            }) => {
                assert_eq!(result, "file body");
                assert!(diff.is_none(), "非 diff preview 不得落地");
                assert_eq!(*fold, FoldState::Folded, "无 diff 保持默认 Folded");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    /// 缺 display/preview 的 done 块：行为与旧版一致（detail → ToolResult）。
    #[test]
    fn tool_done_without_preview_unchanged() {
        let session = SessionStore::new();
        apply_frontend_event(
            &FrontendEvent::OutputBlockAppended {
                session_id: "s1".into(),
                block: serde_json::json!({
                    "kind": "tool", "name": "bash", "phase": "done", "detail": "ok",
                }),
                id: Some("t1".into()),
                live_identity: None,
            },
            &session,
        );
        let msgs = session.messages.get();
        match msgs
            .iter()
            .find(|b| matches!(b, TranscriptBlock::ToolResult { .. }))
        {
            Some(TranscriptBlock::ToolResult { result, diff, .. }) => {
                assert_eq!(result, "ok");
                assert!(diff.is_none());
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn tool_lifecycle_event_creates_and_completes_transcript_block() {
        use agendao_server_core::runtime_events::ToolCallPhase;

        let session = SessionStore::new();
        let event = |phase| FrontendEvent::ToolCallUpsert {
            session_id: "s1".into(),
            tool_call_id: "call-1".into(),
            tool_name: "bash".into(),
            phase,
        };

        apply_frontend_event(&event(ToolCallPhase::Start), &session);
        apply_frontend_event(&event(ToolCallPhase::Complete), &session);

        let messages = session.messages.get();
        assert_eq!(messages.len(), 1, "one lifecycle must produce one block");
        match &messages[0] {
            TranscriptBlock::ToolCall {
                id, name, phase, ..
            } => {
                assert_eq!(id, "call-1");
                assert_eq!(name, "bash");
                assert_eq!(*phase, ToolPhase::Done);
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    /// DiffReplaced 存取 + replace 语义：第二次下发全量替换（不累加）；
    /// 空集合表示无未决改动。
    #[test]
    fn diff_replaced_replaces_summary() {
        use agendao_server_core::runtime_events::DiffEntry;
        let session = SessionStore::new();
        let ev = |diffs: Vec<DiffEntry>| FrontendEvent::DiffReplaced {
            session_id: "s1".into(),
            diffs,
        };
        let entry = |path: &str, a: u64, d: u64| DiffEntry {
            path: path.into(),
            additions: a,
            deletions: d,
        };
        apply_frontend_event(
            &ev(vec![entry("a.rs", 3, 1), entry("b.rs", 2, 0)]),
            &session,
        );
        assert_eq!(session.diff_summary.get().len(), 2);
        // replace：新一轮只剩 1 个文件，不得残留 a/b。
        apply_frontend_event(&ev(vec![entry("c.rs", 5, 2)]), &session);
        let summary = session.diff_summary.get();
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].path, "c.rs");
        assert_eq!(summary[0].additions, 5);
        assert_eq!(summary[0].deletions, 2);
        // 空集合：清空 + 收起明细。
        session.diff_detail_open.set(true);
        apply_frontend_event(&ev(vec![]), &session);
        assert!(session.diff_summary.get().is_empty());
        assert!(!session.diff_detail_open.get(), "无 diff 时明细必须收起");
    }

    /// TodoReplaced：落地 TodoList 块；内容无变化时返回 None（不触发重绘），
    /// 内容变化时返回 Some 并替换块内容。
    #[test]
    fn todo_replaced_applies_and_dedups() {
        let session = SessionStore::new();
        let ev = |todos: Vec<agendao_types::TodoInfo>| FrontendEvent::TodoReplaced {
            session_id: "s1".into(),
            todos,
        };
        let todo = |content: &str, status: &str| agendao_types::TodoInfo {
            content: content.into(),
            status: status.into(),
            priority: "medium".into(),
        };

        // 首次下发：落地 + Some。
        let applied = apply_frontend_event(&ev(vec![todo("task a", "in_progress")]), &session);
        assert!(applied.is_some(), "首次下发必须标脏");
        let msgs = session.messages.get();
        match msgs.last() {
            Some(TranscriptBlock::TodoList { items, .. }) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].content, "task a");
                assert_eq!(items[0].status, TodoStatus::InProgress);
            }
            other => panic!("expected TodoList, got {:?}", other),
        }

        // 相同内容重发：None（不触发重绘）。
        let applied = apply_frontend_event(&ev(vec![todo("task a", "in_progress")]), &session);
        assert!(applied.is_none(), "内容无变化不得标脏");

        // 内容变化：Some + 替换。
        let applied = apply_frontend_event(&ev(vec![todo("task a", "completed")]), &session);
        assert!(applied.is_some(), "内容变化必须标脏");
        let msgs = session.messages.get();
        match msgs.last() {
            Some(TranscriptBlock::TodoList { items, .. }) => {
                assert_eq!(items[0].status, TodoStatus::Completed);
            }
            other => panic!("expected TodoList, got {:?}", other),
        }
    }

    // ── U9：Compacting 拆出独立 RunStatus ──

    fn runtime_evt(kind: SessionRunStatusKind) -> FrontendEvent {
        FrontendEvent::SessionRuntimeReplaced {
            session_id: "s1".into(),
            runtime: agendao_client::SessionRuntimeState {
                session_id: "s1".into(),
                run_status: kind,
                current_message_id: None,
                usage: None,
                active_stage_id: None,
                active_stage_count: 0,
                active_tools: vec![],
                pending_question: None,
                pending_permission: None,
                pending_followup_count: 0,
                active_sandbox: vec![],
            },
        }
    }

    #[test]
    fn compacting_maps_to_distinct_run_status() {
        let session = SessionStore::new();
        apply_frontend_event(&runtime_evt(SessionRunStatusKind::Compacting), &session);
        assert_eq!(
            session.run_status.get(),
            RunStatus::Compacting,
            "压缩相位独立成态"
        );
    }

    #[test]
    fn waiting_on_tool_and_cancelling_stay_running() {
        // 拆分不得误伤其余相位的既有口径（回归闸门）。
        let session = SessionStore::new();
        apply_frontend_event(&runtime_evt(SessionRunStatusKind::WaitingOnTool), &session);
        assert_eq!(session.run_status.get(), RunStatus::Running);
        apply_frontend_event(&runtime_evt(SessionRunStatusKind::Cancelling), &session);
        assert_eq!(session.run_status.get(), RunStatus::Running);
        apply_frontend_event(&runtime_evt(SessionRunStatusKind::Idle), &session);
        assert_eq!(session.run_status.get(), RunStatus::Idle);
    }
}
