use super::{
    cli_context_closure_boundary_status_label, cli_context_closure_cache_status_label,
    cli_context_closure_evidence_detail_label, cli_context_closure_evidence_impact_label,
    cli_context_closure_evidence_source_label, cli_context_closure_isolation_status_label,
    cli_context_closure_prefix_status_label, cli_prompt_surface_evidence_label,
    cli_session_context_kind_label, cli_session_handoff_mode_label, cli_stage_usage_line,
    cli_yes_no, CliFrontendProjection, CliModelCatalogEntry,
};

#[cfg(test)]
pub(super) fn cli_current_context_tokens(projection: &CliFrontendProjection) -> Option<u64> {
    projection.current_context_tokens()
}

pub(super) fn cli_usage_snapshot_lines(
    session_id: &str,
    telemetry: &crate::api_client::SessionTelemetrySnapshot,
    projection: Option<&CliFrontendProjection>,
) -> Vec<String> {
    let usage = &telemetry.usage;
    let usage_books = &telemetry.usage_books;
    let workflow = &usage_books.workflow_cumulative;
    let mut lines = vec![format!("Session: {}", session_id)];

    if let Some(explain) = telemetry.context_explain.as_ref() {
        lines.push(String::new());
        lines.push("Surface Views".to_string());
        if let Some(model) = explain.resolved_model.as_deref() {
            lines.push(format!("  Model: {}", model));
        }
        lines.push(format!(
            "  Raw history: {} persisted messages",
            explain.raw_history_messages
        ));
        if explain.raw_history_messages > explain.raw_model_visible_messages {
            lines.push(format!(
                "  Model-visible history: {} messages ({} runtime-only hidden)",
                explain.raw_model_visible_messages,
                explain
                    .raw_history_messages
                    .saturating_sub(explain.raw_model_visible_messages)
            ));
        } else {
            lines.push(format!(
                "  Model-visible history: {} messages",
                explain.raw_model_visible_messages
            ));
        }
        let mut api_view_line = format!("  API view: {} messages", explain.api_view_messages);
        if let Some(tokens) = explain.api_view_estimated_input_tokens {
            api_view_line.push_str(&format!(" · ~{} tokens", format_token_count(tokens)));
        }
        if let Some(chars) = explain.api_view_body_chars {
            api_view_line.push_str(&format!(" · {} chars", format_token_count(chars as u64)));
        }
        lines.push(api_view_line);
        if explain.raw_model_visible_messages > explain.api_view_messages {
            lines.push(format!(
                "  Boundary: {} earlier model-visible messages trimmed before the next request",
                explain
                    .raw_model_visible_messages
                    .saturating_sub(explain.api_view_messages)
            ));
        }
    }

    if let Some(ownership) = telemetry.ownership.as_ref() {
        lines.push(String::new());
        lines.push("Ownership".to_string());
        lines.push(format!(
            "  Kind: {}",
            cli_session_context_kind_label(ownership.context_kind)
        ));
        lines.push(format!(
            "  Handoff: {}",
            cli_session_handoff_mode_label(ownership.handoff_mode)
        ));
        lines.push(format!(
            "  Prompt continuity: {}",
            if ownership.owns_prompt_continuity {
                "owned by this session"
            } else {
                "not owned by this session"
            }
        ));
        lines.push(format!(
            "  Compact owner: {}",
            if ownership.compact_owner {
                "this session"
            } else {
                "not eligible"
            }
        ));
        lines.push("  Provider/model: request shape only".to_string());
        lines.push("  Workflow cumulative: observation only".to_string());
    }

    if telemetry.context_closure_contract.is_none() {
        if let Some(cache_semantics) = telemetry.cache_semantics.as_ref() {
            lines.push(String::new());
            lines.push("Cache semantics".to_string());
            lines.push(format!(
                "  Basis: API view ({} messages)",
                cache_semantics.api_view_messages
            ));
            if cache_semantics.trimmed_model_visible_messages > 0 {
                lines.push(format!(
                    "  Boundary: {} earlier model-visible messages trimmed before the next request",
                    cache_semantics.trimmed_model_visible_messages
                ));
            }
            if let Some(boundary) = cache_semantics.boundary.as_ref() {
                let trigger = boundary.trigger.replace('_', " ");
                let reason = boundary
                    .reason
                    .as_deref()
                    .map(|value| value.replace('_', " "));
                let mut detail = format!("  Compact: {}", trigger);
                if let Some(reason) = reason.as_deref() {
                    detail.push_str(&format!(" · {}", reason));
                }
                if boundary.possible_cache_evidence {
                    detail.push_str(" · may have shifted the cache prefix");
                }
                lines.push(detail);
            }
            if let Some(label) = cache_semantics.label.as_deref() {
                lines.push(format!("  Impact: {}", label));
            }
            if let Some(evidence) = cache_semantics.prompt_surface_evidence.as_ref() {
                if !evidence.changed_fields.is_empty() {
                    lines.push(format!(
                        "  Prompt surface: {}",
                        evidence.changed_fields.join(", ")
                    ));
                }
            }
        }
    }

    if let Some(contract) = telemetry.context_closure_contract.as_ref() {
        lines.push(String::new());
        lines.push("Context Closure".to_string());
        lines.push(format!(
            "  Prefix: {}",
            cli_context_closure_prefix_status_label(&contract.prefix_stability)
        ));
        lines.push(format!(
            "  Basis: API view · {} messages · trimmed {}",
            contract.prefix_stability.api_view_messages,
            contract.prefix_stability.trimmed_model_visible_messages
        ));
        if let Some(explanation) = contract.prefix_stability.explanation.as_deref() {
            lines.push(format!(
                "  Prefix explain: {}",
                cli_context_closure_evidence_detail_label(explanation)
            ));
        }

        let boundary = &contract.compaction_boundary;
        if boundary.boundary_recorded {
            let mut summary = format!(
                "  Boundary: {}",
                cli_context_closure_boundary_status_label(boundary)
            );
            if let Some(request_pressure_percent) = boundary.request_pressure_percent {
                summary.push_str(&format!(" · request {}%", request_pressure_percent));
            }
            if let Some(live_pressure_percent) = boundary.live_pressure_percent {
                summary.push_str(&format!(" · live {}%", live_pressure_percent));
            }
            lines.push(summary);

            let mut detail = Vec::new();
            if let Some(status) = boundary.governance_status {
                detail.push(status.label().to_string());
            }
            if let Some(phase) = boundary.phase.as_deref() {
                detail.push(phase.to_string());
            }
            if let Some(trigger) = boundary.trigger.as_deref() {
                detail.push(trigger.replace('_', " "));
            }
            if let Some(reason) = boundary.reason.as_deref() {
                detail.push(reason.replace('_', " "));
            }
            if !detail.is_empty() {
                lines.push(format!("  Detail: {}", detail.join(" · ")));
            }
            if let Some(continuity) = telemetry.compaction_continuity.as_ref() {
                let mut continuity_parts = Vec::new();
                continuity_parts.push(match continuity.source {
                    rocode_types::SessionCompactionContinuityInspectionSource::ContinuityPacket => {
                        "packet installed".to_string()
                    }
                    rocode_types::SessionCompactionContinuityInspectionSource::RawSummaryFallback => {
                        "legacy summary fallback".to_string()
                    }
                });
                if let Some(exact_recent_tail_count) = continuity.exact_recent_tail_count {
                    continuity_parts.push(format!("tail {}", exact_recent_tail_count));
                }
                if let Some(omitted_older_turns) = continuity.omitted_older_turns {
                    continuity_parts.push(format!("omitted {}", omitted_older_turns));
                }
                if continuity.has_working_ledger {
                    continuity_parts.push("ledger".to_string());
                }
                if continuity.has_memory_anchors {
                    continuity_parts.push("memory anchors".to_string());
                }
                if !continuity_parts.is_empty() {
                    lines.push(format!("  Continuity: {}", continuity_parts.join(" · ")));
                }
                if let Some(recall_policy) = continuity.recall_policy.as_deref() {
                    lines.push(format!("  Recall: {}", recall_policy));
                }
                if let Some(summary_text) = continuity.summary_text.as_deref() {
                    lines.push(format!(
                        "  Summary: {}",
                        crate::util::truncate_text(summary_text, 120)
                    ));
                }
            }
            lines.push(format!(
                "  Action: attempted {} · succeeded {} · blocking {}",
                cli_yes_no(boundary.compaction_attempted),
                cli_yes_no(boundary.compaction_succeeded),
                cli_yes_no(boundary.blocking)
            ));
            if let Some(installed) = boundary.installed.as_ref() {
                let mut install_parts = Vec::new();
                if let Some(request_context_tokens) = installed.request_context_tokens {
                    install_parts.push(format!(
                        "request {}",
                        format_token_count(request_context_tokens)
                    ));
                }
                if let Some(live_context_tokens) = installed.live_context_tokens {
                    install_parts.push(format!("live {}", format_token_count(live_context_tokens)));
                }
                if let Some(body_chars) = installed.body_chars {
                    install_parts.push(format!("{} chars", format_token_count(body_chars as u64)));
                }
                if !install_parts.is_empty() {
                    lines.push(format!("  Installed: {}", install_parts.join(" · ")));
                }
                if let Some(explanation) = installed.cache_explanation.as_deref() {
                    lines.push(format!(
                        "  Install explain: {}",
                        cli_context_closure_evidence_detail_label(explanation)
                    ));
                }
            }
        } else {
            lines.push(format!(
                "  Boundary: {}",
                cli_context_closure_boundary_status_label(boundary)
            ));
        }

        let cache_explainability = &contract.cache_explainability;
        let mut cache_line = format!(
            "  Cache: {}",
            cli_context_closure_cache_status_label(cache_explainability)
        );
        if cache_explainability.issue_present && !cache_explainability.explained {
            cache_line.push_str(" · explanation missing");
        }
        lines.push(cache_line);
        if cache_explainability.issue_present {
            lines.push(format!(
                "  Source: {} · impact {}",
                cli_context_closure_evidence_source_label(cache_explainability.source),
                cache_explainability
                    .severity
                    .map(cli_context_closure_evidence_impact_label)
                    .unwrap_or("--")
            ));
        }
        if let Some(explanation) = cache_explainability.explanation.as_deref() {
            lines.push(format!(
                "  Cache explain: {}",
                cli_context_closure_evidence_detail_label(explanation)
            ));
        }
        if let Some(cache_semantics) = telemetry.cache_semantics.as_ref() {
            if let Some(evidence) = cache_semantics.prompt_surface_evidence.as_ref() {
                if !evidence.changed_fields.is_empty() {
                    lines.push(format!(
                        "  Evidence: {}",
                        cli_prompt_surface_evidence_label(&evidence.changed_fields)
                    ));
                }
            }
        }

        let child_isolation = &contract.child_history_isolation;
        lines.push(format!(
            "  Isolation: {}",
            cli_context_closure_isolation_status_label(child_isolation)
        ));
        lines.push(format!(
            "  Usage: attached subtree {} · subtree cumulative {} · owner live {}",
            child_isolation.attached_subtree_session_count,
            format_token_count(child_isolation.attached_subtree_cumulative_tokens),
            child_isolation
                .owner_live_context_tokens
                .map(format_token_count)
                .unwrap_or_else(|| "--".to_string())
        ));
        lines.push(format!(
            "  Scope: owner-local live prefix {} · workflow cumulative {}",
            cli_yes_no(child_isolation.owner_local_live_prefix),
            format_token_count(child_isolation.workflow_cumulative_tokens)
        ));
        lines.push(format!(
            "  Isolation explain: {}",
            child_isolation.explanation
        ));
        if child_isolation.child_history_in_live_prefix_detected {
            lines.push("  Leak: child history appeared in the owner live prefix".to_string());
        }
    }

    if let Some(projection) = projection {
        if let Some(current_tokens) = projection.current_context_tokens() {
            lines.push(String::new());
            lines.push("Live context".to_string());
            if let Some(model) = cli_lookup_model_catalog_entry(projection)
                .filter(|model| model.context_window.unwrap_or(0) > 0)
            {
                let limit = model.context_window.unwrap_or(0);
                let percent = cli_context_usage_percent(current_tokens, limit);
                lines.push(format!(
                    "  Pressure: {}",
                    cli_format_context_meter(current_tokens, Some(limit))
                ));
                if let Some(note) = rocode_types::context_pressure_label(percent) {
                    lines.push(format!("  State: {}", note));
                }
            } else {
                lines.push(format!(
                    "  Pressure: {}",
                    format_token_count(current_tokens)
                ));
            }
        }

        let last_turn = &projection.last_turn_tokens;
        if last_turn.input_tokens > 0
            || last_turn.output_tokens > 0
            || usage_books.request_context_tokens.is_some()
        {
            lines.push(String::new());
            lines.push("Last request".to_string());
            if let Some(request_context_tokens) = usage_books.request_context_tokens {
                lines.push(format!(
                    "  Context: {}",
                    format_token_count(request_context_tokens)
                ));
            }
            lines.push(format!(
                "  Input {} · Output {}",
                format_token_count(last_turn.input_tokens),
                format_token_count(last_turn.output_tokens)
            ));
        }
    }

    lines.push(String::new());
    lines.push("Workflow cumulative".to_string());
    lines.push(format!(
        "  Total: {}",
        format_token_count(workflow.total_tokens())
    ));
    lines.push(format!(
        "  Input: {}",
        format_token_count(workflow.input_tokens)
    ));
    lines.push(format!(
        "  Output: {}",
        format_token_count(workflow.output_tokens)
    ));
    lines.push(format!(
        "  Reasoning: {}",
        format_token_count(workflow.reasoning_tokens)
    ));
    lines.push(format!(
        "  Cache read: {}",
        format_token_count(workflow.cache_read_tokens)
    ));
    lines.push(format!(
        "  Cache miss: {}",
        format_token_count(workflow.cache_miss_tokens)
    ));
    lines.push(format!(
        "  Cache write: {}",
        format_token_count(workflow.cache_write_tokens)
    ));
    lines.push(format!("  Cost: ${:.4}", workflow.total_cost));

    if workflow.total_tokens() != usage.session_cumulative_tokens() {
        lines.push(String::new());
        lines.push("Owner session cumulative".to_string());
        lines.push(format!(
            "  Total: {}",
            format_token_count(usage.session_cumulative_tokens())
        ));
        lines.push(format!(
            "  Input: {}",
            format_token_count(usage.input_tokens)
        ));
        lines.push(format!(
            "  Output: {}",
            format_token_count(usage.output_tokens)
        ));
        lines.push(format!(
            "  Reasoning: {}",
            format_token_count(usage.reasoning_tokens)
        ));
        lines.push(format!("  Cost: ${:.4}", usage.total_cost));
    }

    if !telemetry.stages.is_empty() {
        lines.push(String::new());
        lines.push(format!("Stage totals ({})", telemetry.stages.len()));
        for stage in &telemetry.stages {
            lines.push(format!("  {}", cli_stage_usage_line(stage)));
        }
    }

    lines
}

pub(super) fn format_token_count(n: u64) -> String {
    if n >= 1_000_000 {
        let compact = n as f64 / 1_000_000.0;
        if compact.fract() == 0.0 {
            format!("{compact:.0}M")
        } else {
            format!("{compact:.1}M")
        }
    } else if n >= 1_000 {
        let compact = n as f64 / 1_000.0;
        if compact.fract() == 0.0 {
            format!("{compact:.0}K")
        } else {
            format!("{compact:.1}K")
        }
    } else {
        n.to_string()
    }
}

pub(super) fn cli_context_usage_percent(used: u64, limit: u64) -> Option<u64> {
    rocode_types::context_usage_percent(used, limit)
}

pub(super) fn cli_context_usage_bar(percent: Option<u64>, width: usize) -> String {
    rocode_types::context_usage_bar(percent, width)
}

pub(super) fn cli_format_context_meter(used: u64, limit: Option<u64>) -> String {
    let Some(limit) = limit.filter(|limit| *limit > 0) else {
        return format_token_count(used);
    };
    let percent = cli_context_usage_percent(used, limit);
    format!(
        "{}/{} {} {}%",
        format_token_count(used),
        format_token_count(limit),
        cli_context_usage_bar(percent, 10),
        percent.unwrap_or(0)
    )
}

pub(super) fn cli_lookup_model_catalog_entry(
    projection: &CliFrontendProjection,
) -> Option<&CliModelCatalogEntry> {
    let model_label = projection
        .current_model_label
        .as_deref()
        .filter(|value| !value.trim().is_empty() && *value != "auto")?;
    projection.model_catalog.get(model_label).or_else(|| {
        projection
            .model_catalog
            .iter()
            .find(|(candidate, _)| {
                candidate.as_str() == model_label
                    || candidate
                        .rsplit_once('/')
                        .map(|(_, suffix)| suffix == model_label)
                        .unwrap_or(false)
            })
            .map(|(_, model)| model)
    })
}

#[cfg(test)]
pub(super) fn cli_format_price_pair(input: f64, output: f64) -> String {
    format!(
        "${}/{} /1M",
        cli_format_price(input),
        cli_format_price(output)
    )
}

#[cfg(test)]
pub(super) fn cli_format_price(value: f64) -> String {
    if value >= 10.0 {
        format!("{value:.0}")
    } else if value >= 1.0 {
        format!("{value:.2}")
    } else if value >= 0.1 {
        format!("{value:.3}")
    } else {
        format!("{value:.4}")
    }
}

#[cfg(test)]
mod tests {
    use super::super::CliFrontendProjection;
    use super::{
        cli_context_usage_bar, cli_current_context_tokens, cli_format_context_meter,
        cli_usage_snapshot_lines,
    };
    use rocode_command::stage_protocol::{StageStatus, StageSummary};
    use rocode_types::{SessionContextExplain, SessionUsageBooks, WorkflowUsageSummary};

    #[test]
    fn formats_compact_context_meter() {
        assert_eq!(
            cli_format_context_meter(12_450, Some(200_000)),
            "12.4K/200K [█░░░░░░░░░] 6%"
        );
    }

    #[test]
    fn builds_context_bar_with_clamped_width() {
        assert_eq!(cli_context_usage_bar(Some(0), 5), "[░░░░░]");
        assert_eq!(cli_context_usage_bar(Some(50), 5), "[███░░]");
        assert_eq!(cli_context_usage_bar(Some(140), 5), "[█████]");
    }

    #[test]
    fn current_context_does_not_fall_back_to_last_turn_input() {
        let mut projection = CliFrontendProjection::default();
        projection.last_turn_tokens.input_tokens = 1_388_907;

        assert_eq!(cli_current_context_tokens(&projection), None);
    }

    #[test]
    fn current_context_prefers_root_usage_over_active_stage_estimate() {
        let mut projection = CliFrontendProjection::default();
        projection.token_stats.context_tokens = 52_830;
        projection.session_runtime = Some(crate::api_client::SessionRuntimeState {
            session_id: "sess_123".to_string(),
            run_status: crate::api_client::SessionRunStatusKind::Running,
            current_message_id: None,
            usage: None,
            active_stage_id: Some("stage-exec".to_string()),
            active_stage_count: 1,
            active_tools: Vec::new(),
            pending_question: None,
            pending_permission: None,
            attached_sessions: Vec::new(),
        });
        projection.stage_summaries = vec![StageSummary {
            stage_id: "stage-exec".to_string(),
            stage_name: "Execution".to_string(),
            index: None,
            total: None,
            step: None,
            step_total: None,
            status: StageStatus::Running,
            prompt_tokens: None,
            context_tokens: None,
            completion_tokens: None,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_miss_tokens: None,
            cache_write_tokens: None,
            focus: None,
            last_event: None,
            waiting_on: None,
            activity: None,
            estimated_context_tokens: Some(1_105_000),
            skill_tree_budget: None,
            skill_tree_truncation_strategy: None,
            skill_tree_truncated: None,
            retry_attempt: None,
            active_agent_count: 0,
            active_tool_count: 0,
            attached_session_count: 0,
            primary_attached_session_id: None,
        }];

        assert_eq!(cli_current_context_tokens(&projection), Some(52_830));
    }

    #[test]
    fn usage_surface_explains_raw_history_and_api_view() {
        let mut projection = CliFrontendProjection::default();
        projection.token_stats.context_tokens = 82_000;
        projection.last_turn_tokens.input_tokens = 88_000;
        projection.last_turn_tokens.output_tokens = 2_400;

        let telemetry = crate::api_client::SessionTelemetrySnapshot {
            runtime: crate::api_client::SessionRuntimeState {
                session_id: "sess_123".to_string(),
                run_status: crate::api_client::SessionRunStatusKind::Idle,
                current_message_id: None,
                usage: None,
                active_stage_id: None,
                active_stage_count: 0,
                active_tools: Vec::new(),
                pending_question: None,
                pending_permission: None,
                attached_sessions: Vec::new(),
            },
            stages: Vec::new(),
            topology: crate::api_client::SessionExecutionTopology {
                session_id: "sess_123".to_string(),
                active_count: 0,
                done_count: 0,
                running_count: 0,
                waiting_count: 0,
                cancelling_count: 0,
                retry_count: 0,
                updated_at: None,
                roots: Vec::new(),
            },
            usage: rocode_session::SessionUsage {
                input_tokens: 90_000,
                output_tokens: 10_000,
                reasoning_tokens: 4_000,
                cache_write_tokens: 2_000,
                cache_read_tokens: 30_000,
                cache_miss_tokens: 6_000,
                context_tokens: 82_000,
                total_cost: 1.25,
            },
            usage_books: SessionUsageBooks {
                request_context_tokens: Some(88_000),
                live_context_tokens: Some(82_000),
                workflow_cumulative: WorkflowUsageSummary {
                    input_tokens: 120_000,
                    output_tokens: 18_000,
                    reasoning_tokens: 5_000,
                    cache_write_tokens: 2_000,
                    cache_read_tokens: 34_000,
                    cache_miss_tokens: 7_000,
                    total_cost: 1.60,
                },
            },
            memory: None,
            cache_evidence: None,
            context_explain: Some(SessionContextExplain {
                resolved_model: Some("openai/gpt-4o".to_string()),
                fork: None,
                raw_history_messages: 18,
                raw_model_visible_messages: 15,
                api_view_messages: 8,
                api_view_estimated_input_tokens: Some(92_000),
                api_view_body_chars: Some(360_000),
                live_context_tokens: Some(82_000),
                last_request_context_tokens: Some(88_000),
                owner_session_cumulative_tokens: 104_000,
                workflow_cumulative_tokens: 143_000,
            }),
            ownership: Some(rocode_types::SessionOwnershipSummary {
                context_kind: rocode_types::SessionContextKind::RootSessionContinuity,
                handoff_mode: rocode_types::SessionHandoffMode::SelfContinuity,
                owns_prompt_continuity: true,
                compact_owner: true,
                provider_model_role: rocode_types::SessionProviderModelRole::RequestShapeOnly,
                workflow_usage_role: rocode_types::SessionWorkflowUsageRole::ObservationOnly,
            }),
            context_compaction_summary: Some(rocode_types::ContextCompactionSummary {
                trigger: "auto_preflight".to_string(),
                phase: Some("prompt.pre_request".to_string()),
                reason: Some("request_view_threshold".to_string()),
                forced: false,
                request_context_tokens: Some(92_000),
                live_context_tokens: Some(82_000),
                limit_tokens: Some(100_000),
                body_chars: Some(360_000),
                message_count_before: Some(15),
                compacted_message_count: Some(7),
                kept_message_count: Some(8),
                summary: Some("Compacted 7 messages.".to_string()),
            }),
            compaction_continuity: Some(
                rocode_types::SessionCompactionContinuityInspection {
                    source: rocode_types::SessionCompactionContinuityInspectionSource::ContinuityPacket,
                    summary_message_id: Some("msg_compact".to_string()),
                    summary_text: Some("Packet-owned continuity summary.".to_string()),
                    eligible_message_count: Some(15),
                    exact_recent_tail_count: Some(8),
                    omitted_older_turns: Some(7),
                    has_working_ledger: true,
                    has_memory_anchors: false,
                    recall_policy: Some("recent_tail_plus_memory".to_string()),
                },
            ),
            context_compaction_lifecycle_summary: Some(
                rocode_types::ContextCompactionLifecycleSummary {
                    trigger: "auto_preflight".to_string(),
                    phase: Some("prompt.pre_request".to_string()),
                    reason: Some("request_view_threshold".to_string()),
                    status: rocode_types::ContextCompactionLifecycleStatus::Installed,
                    forced: false,
                    request_context_tokens: Some(92_000),
                    live_context_tokens: Some(82_000),
                    limit_tokens: Some(100_000),
                    body_chars: Some(360_000),
                    installed: Some(rocode_types::ContextCompactionInstalledDiagnostics {
                        request_context_tokens: Some(70_000),
                        live_context_tokens: Some(67_000),
                        body_chars: Some(250_000),
                        cache_explanation: Some(
                            "boundary recorded · 7 earlier messages trimmed from the API view"
                                .to_string(),
                        ),
                    }),
                },
            ),
            context_pressure_governance_summary: None,
            cache_semantics: Some(rocode_types::SessionCacheSemanticsSummary {
                basis: rocode_types::SessionCacheSemanticsBasis::ApiView,
                api_view_messages: 8,
                trimmed_model_visible_messages: 7,
                boundary: Some(rocode_types::SessionCacheBoundarySummary {
                    kind: rocode_types::SessionCacheBoundaryKind::Compaction,
                    trigger: "auto_preflight".to_string(),
                    phase: Some("prompt.pre_request".to_string()),
                    reason: Some("request_view_threshold".to_string()),
                    message_count_before: Some(15),
                    compacted_message_count: Some(7),
                    kept_message_count: Some(8),
                    trimmed_model_visible_messages: 7,
                    likely_changed_prefix: true,
                    possible_cache_evidence: true,
                }),
                cache_evidence: Some(rocode_types::SessionCacheEvidenceExplain {
                    status: "degraded".to_string(),
                    severity: rocode_types::SessionCacheSeverity::MediumChange,
                    primary_cause: Some(
                        "prefix changed before the stable boundary".to_string(),
                    ),
                    change_count: 1,
                }),
                prompt_surface_evidence: Some(
                    rocode_types::PromptSurfaceEvidenceSummary {
                        severity: rocode_types::SessionCacheSeverity::LowChange,
                        reason: "surface changed: ingressPolicyHash".to_string(),
                        changed_fields: vec!["ingressPolicyHash".to_string()],
                    },
                ),
                label: Some("boundary recorded · prefix changed".to_string()),
            }),
            context_closure_contract: Some(rocode_types::SessionContextClosureContract {
                prefix_stability: rocode_types::SessionPrefixStabilityContract {
                    basis: rocode_types::SessionCacheSemanticsBasis::ApiView,
                    tracked_on_api_view: true,
                    api_view_messages: 8,
                    trimmed_model_visible_messages: 7,
                    prefix_change_detected: true,
                    explanation: Some("boundary recorded · prefix changed".to_string()),
                },
                compaction_boundary: rocode_types::SessionCompactionBoundaryContract {
                    boundary_recorded: true,
                    phase: Some("prompt.pre_request".to_string()),
                    trigger: Some("auto_preflight".to_string()),
                    reason: Some("request_view_threshold".to_string()),
                    lifecycle_status: Some(
                        rocode_types::ContextCompactionLifecycleStatus::Installed,
                    ),
                    governance_status: Some(
                        rocode_types::ContextPressureGovernanceStatus::Compacted,
                    ),
                    request_pressure_percent: Some(92),
                    live_pressure_percent: Some(82),
                    compaction_attempted: true,
                    compaction_succeeded: true,
                    blocking: false,
                    installed: Some(rocode_types::ContextCompactionInstalledDiagnostics {
                        request_context_tokens: Some(70_000),
                        live_context_tokens: Some(67_000),
                        body_chars: Some(250_000),
                        cache_explanation: Some(
                            "boundary recorded · 7 earlier messages trimmed from the API view"
                                .to_string(),
                        ),
                    }),
                },
                cache_explainability: rocode_types::SessionCacheExplainabilityContract {
                    issue_present: true,
                    explained: true,
                    source:
                        rocode_types::SessionCacheExplainabilitySource::CacheEvidence,
                    severity: Some(rocode_types::SessionCacheSeverity::MediumChange),
                    explanation: Some("boundary recorded · prefix changed".to_string()),
                },
                child_history_isolation: rocode_types::SessionChildHistoryIsolationContract {
                    attached_subtree_session_count: 0,
                    owner_session_cumulative_tokens: 104_000,
                    workflow_cumulative_tokens: 143_000,
                    attached_subtree_cumulative_tokens: 0,
                    owner_live_context_tokens: Some(82_000),
                    owner_local_live_prefix: true,
                    child_history_in_live_prefix_detected: false,
                    explanation:
                        "No attached subtree sessions were observed; the live prefix remains owner-local."
                            .to_string(),
                },
            }),
            prompt_surface_evidence: Some(
                rocode_types::PromptSurfaceEvidenceSummary {
                    severity: rocode_types::SessionCacheSeverity::LowChange,
                    reason: "surface changed: ingressPolicyHash".to_string(),
                    changed_fields: vec!["ingressPolicyHash".to_string()],
                },
            ),
            ingress_stabilization: None,
            execution_preflight_summary: None,
            provider_diagnostic_summary: None,
        };

        let lines = cli_usage_snapshot_lines("sess_123", &telemetry, Some(&projection));

        assert!(lines.iter().any(|line| line == "Surface Views"));
        assert!(lines
            .iter()
            .any(|line| line == "  Raw history: 18 persisted messages"));
        assert!(lines.iter().any(|line| {
            line == "  Model-visible history: 15 messages (3 runtime-only hidden)"
        }));
        assert!(lines
            .iter()
            .any(|line| { line == "  API view: 8 messages · ~92K tokens · 360K chars" }));
        assert!(lines.iter().any(|line| {
            line == "  Boundary: 7 earlier model-visible messages trimmed before the next request"
        }));
        assert!(lines.iter().any(|line| line == "Ownership"));
        assert!(lines.iter().any(|line| line == "  Kind: root continuity"));
        assert!(lines
            .iter()
            .any(|line| line == "  Compact owner: this session"));
        assert!(!lines.iter().any(|line| line == "Cache semantics"));
        assert!(lines.iter().any(|line| line == "Context Closure"));
        assert!(lines
            .iter()
            .any(|line| { line == "  Prefix: prefix changed" }));
        assert!(lines
            .iter()
            .any(|line| { line == "  Prefix explain: boundary recorded · prefix changed" }));
        assert!(lines
            .iter()
            .any(|line| { line == "  Cache: cache explained" }));
        assert!(lines.iter().any(|line| {
            line == "  Continuity: packet installed · tail 8 · omitted 7 · ledger"
        }));
        assert!(lines
            .iter()
            .any(|line| { line == "  Recall: recent_tail_plus_memory" }));
        assert!(lines
            .iter()
            .any(|line| { line == "  Summary: Packet-owned continuity summary." }));
        assert!(lines
            .iter()
            .any(|line| { line == "  Installed: request 70K · live 67K · 250K chars" }));
        assert!(lines.iter().any(|line| {
            line == "  Install explain: boundary recorded · 7 earlier messages trimmed from the API view"
        }));
        assert!(lines
            .iter()
            .any(|line| { line == "  Source: cache evidence · impact medium change" }));
        assert!(lines
            .iter()
            .any(|line| { line == "  Cache explain: boundary recorded · prefix changed" }));
        assert!(lines
            .iter()
            .any(|line| { line == "  Evidence: surface ingressPolicyHash" }));
        assert!(lines.iter().all(|line| !line.contains("bust")));
        assert!(lines.iter().any(|line| { line == "  Isolation: isolated" }));
        assert!(lines.iter().any(|line| line == "Live context"));
        assert!(lines.iter().any(|line| line == "Last request"));
        assert!(lines.iter().any(|line| line == "Workflow cumulative"));
    }
}
