//! 水 — FrontendEvent → SessionStore Signal mapping.

use agendao_server_core::frontend_events::FrontendEvent;
use agendao_client::SessionRunStatusKind;
use agendao_client::{ExecutionStatus, SessionExecutionNode};
use crate::store::session_store::SessionStore;
use crate::store::types::*;

/// Convert a SessionExecutionNode hierarchy into our TreeNode.
fn node_from_topology(node: &SessionExecutionNode) -> TreeNode {
    let label = node.label.as_deref().unwrap_or(&node.id);
    let status_str = match node.status {
        ExecutionStatus::Running => " ▶",
        ExecutionStatus::Done => " ✓",
        ExecutionStatus::Waiting => " ⏳",
        ExecutionStatus::Cancelling => " ✕",
        ExecutionStatus::Retry => " ↻",
    };
    let children: Vec<TreeNode> = node.children.iter().map(node_from_topology).collect();
    TreeNode {
        label: format!("{}{}", label, status_str),
        depth: 0,
        expanded: true,
        children,
        intent: None,
    }
}

pub fn apply_frontend_event(event: &FrontendEvent, session: &SessionStore) -> Option<String> {
    match event {
        FrontendEvent::SessionRuntimeReplaced { session_id, runtime } => {
            let status = match runtime.run_status {
                SessionRunStatusKind::Idle => RunStatus::Idle,
                SessionRunStatusKind::Running => RunStatus::Running,
                SessionRunStatusKind::WaitingOnUser => RunStatus::WaitingUser,
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
                        // full / end — append (or replace) with the final text
                        Some("full") => {
                            if !text.is_empty() {
                                session.push_assistant_delta(bid, text);
                            }
                        }
                        Some("end") => {
                            // `end` carries no new text; just mark the loop
                            // as idle so the prompt bar reactivates.
                            session.run_status.set(RunStatus::Idle);
                        }
                        Some("start") => { /* start is silent — wait for delta */ }
                        _ => {}
                    }
                }
                "reasoning" => {
                    match phase {
                        Some("delta") | Some("full") => {
                            if !text.is_empty() { session.push_thinking(bid, text); }
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
            // Build session tree from execution topology
            if let Some(ref topo) = topology {
                let nodes: Vec<crate::store::types::TreeNode> = topo.roots.iter().map(|root| {
                    node_from_topology(root)
                }).collect();
                session.sidebar_trees.update(|t| t.session_nodes = nodes);
            }
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
                    detail_lines.push(format!(" 📎 focus: {}", f));
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
                        if stage.focus.as_deref().unwrap_or("") != "" {
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
