use super::{cli_optional_generated_at, cli_yes_no};
use crate::run::session_projection_usage::format_token_count;
use crate::util::truncate_text;

fn cli_tool_trajectory_band_label(band: rocode_types::ToolTrajectoryQualityBand) -> &'static str {
    match band {
        rocode_types::ToolTrajectoryQualityBand::Clean => "clean",
        rocode_types::ToolTrajectoryQualityBand::Recoverable => "recoverable",
        rocode_types::ToolTrajectoryQualityBand::Degraded => "degraded",
        rocode_types::ToolTrajectoryQualityBand::Risky => "risky",
    }
}

pub(super) fn cli_session_insights_lines(
    session_id: &str,
    insights: &crate::api_client::SessionInsightsResponse,
) -> Vec<String> {
    let mut lines = vec![
        format!("Session: {}", session_id),
        format!("Title: {}", insights.title),
        format!("Directory: {}", insights.directory),
        format!(
            "Updated: {}",
            chrono::DateTime::<chrono::Utc>::from_timestamp_millis(insights.updated)
                .map(|value| value.with_timezone(&chrono::Local))
                .map(|value| value.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| insights.updated.to_string())
        ),
    ];

    if let Some(telemetry) = insights.telemetry.as_ref() {
        lines.push(String::new());
        lines.push(format!(
            "Persisted telemetry: version {:?} · status {} · updated {}",
            telemetry.version, telemetry.last_run_status, telemetry.updated_at
        ));
        lines.push(format!(
            "  Session cumulative: total {} · input {} · output {} · reasoning {}",
            format_token_count(
                telemetry.usage.input_tokens
                    + telemetry.usage.output_tokens
                    + telemetry.usage.reasoning_tokens
            ),
            format_token_count(telemetry.usage.input_tokens),
            format_token_count(telemetry.usage.output_tokens),
            format_token_count(telemetry.usage.reasoning_tokens),
        ));
        lines.push(format!(
            "  Cache read {} · cache miss {} · cache write {} · cost ${:.4}",
            format_token_count(telemetry.usage.cache_read_tokens),
            format_token_count(telemetry.usage.cache_miss_tokens),
            format_token_count(telemetry.usage.cache_write_tokens),
            telemetry.usage.total_cost
        ));
        lines.push(format!(
            "  Persisted stages: {}",
            telemetry.stage_summaries.len()
        ));
        if let Some(quality) = telemetry.tool_trajectory_quality.as_ref() {
            lines.push(format!(
                "  Trajectory quality: {} · {} · repaired {}/{} · errors {}",
                quality.score,
                cli_tool_trajectory_band_label(quality.band),
                quality.repaired_tool_call_count,
                quality.total_tool_calls,
                quality.error_tool_call_count
            ));
        }
    }

    if let Some(memory) = insights.memory.as_ref() {
        lines.push(String::new());
        lines.push(format!(
            "Memory explain: {} · {}",
            memory.summary.workspace_mode,
            truncate_text(&memory.summary.workspace_key, 88)
        ));
        lines.push(format!(
            "  Frozen snapshot packet: {} items{}",
            memory.summary.frozen_snapshot_items,
            cli_optional_generated_at(memory.summary.frozen_snapshot_generated_at)
        ));
        lines.push(format!(
            "  Last prefetch packet: {} items{}",
            memory.summary.last_prefetch_items,
            cli_optional_generated_at(memory.summary.last_prefetch_generated_at)
        ));
        if let Some(query) = memory.summary.last_prefetch_query.as_deref() {
            lines.push(format!("  Prefetch query: {}", truncate_text(query, 120)));
        }
        lines.push(format!(
            "  Validation pressure: warnings {} · methodology {} · skill targets {}",
            memory.summary.warning_count,
            memory.summary.methodology_candidate_count,
            memory.summary.derived_skill_candidate_count
        ));
        if let Some(run) = memory.summary.latest_consolidation_run.as_ref() {
            lines.push(format!(
                "  Latest consolidation: {} · merged {} · promoted {} · conflicts {}",
                run.run_id, run.merged_count, run.promoted_count, run.conflict_count
            ));
        }
        if !memory.summary.recent_rule_hits.is_empty() {
            lines.push(format!(
                "  Recent rule hits ({})",
                memory.summary.recent_rule_hits.len()
            ));
            for hit in &memory.summary.recent_rule_hits {
                let detail = hit.detail.as_deref().unwrap_or("no detail");
                lines.push(format!(
                    "    {} · {}",
                    hit.hit_kind,
                    truncate_text(detail, 96)
                ));
            }
        }
        if let Some(packet) = memory.frozen_snapshot.as_ref() {
            lines.push(format!(
                "  Frozen snapshot scopes: {}",
                packet
                    .scopes
                    .iter()
                    .map(|scope| format!("{scope:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(packet) = memory.last_prefetch_packet.as_ref() {
            lines.push(format!(
                "  Last prefetch scopes: {}",
                packet
                    .scopes
                    .iter()
                    .map(|scope| format!("{scope:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let skill_linked = memory
            .recent_session_records
            .iter()
            .filter(|item| item.linked_skill_name.is_some() || item.derived_skill_name.is_some())
            .take(3)
            .collect::<Vec<_>>();
        if !skill_linked.is_empty() {
            lines.push("  Skill-linked recent records:".to_string());
            for item in skill_linked {
                lines.push(format!(
                    "    {} · linked={} · target={}",
                    truncate_text(&item.title, 72),
                    item.linked_skill_name.as_deref().unwrap_or("--"),
                    item.derived_skill_name.as_deref().unwrap_or("--")
                ));
            }
        }
        let suggested_ids = memory
            .summary
            .recent_rule_hits
            .iter()
            .filter_map(|hit| hit.memory_id.as_ref().map(|id| id.0.as_str()))
            .chain(
                memory
                    .last_prefetch_packet
                    .iter()
                    .flat_map(|packet| packet.items.iter().map(|item| item.card.id.0.as_str())),
            )
            .take(3)
            .collect::<Vec<_>>();
        if !suggested_ids.is_empty() {
            lines.push("  Follow-up commands:".to_string());
            for record_id in suggested_ids {
                lines.push(format!("    /memory show {}", record_id));
                lines.push(format!("    /memory hits record={}", record_id));
            }
        }
        if let Some(run) = memory.summary.latest_consolidation_run.as_ref() {
            lines.push(format!("    /memory hits run={}", run.run_id));
        }
    }

    if let Some(multimodal) = insights.multimodal.as_ref() {
        lines.push(String::new());
        lines.push(format!(
            "Multimodal explain: {}",
            multimodal.display_label()
        ));
        lines.push(format!(
            "  Message: {} · attachments {} · hard block {}",
            multimodal.user_message_id,
            multimodal.attachment_count,
            if multimodal.hard_block { "yes" } else { "no" }
        ));
        lines.push(format!(
            "  Resolved model: {}",
            multimodal.resolved_model.as_deref().unwrap_or("--")
        ));
        lines.push(format!(
            "  Kinds: {}",
            if multimodal.kinds.is_empty() {
                "--".to_string()
            } else {
                multimodal.kinds.join(", ")
            }
        ));
        lines.push(format!(
            "  Badges: {}",
            if multimodal.badges.is_empty() {
                "--".to_string()
            } else {
                multimodal.badges.join(", ")
            }
        ));
        lines.push(format!(
            "  Unsupported parts: {}",
            if multimodal.unsupported_parts.is_empty() {
                "none".to_string()
            } else {
                multimodal.unsupported_parts.join(", ")
            }
        ));
        lines.push(format!(
            "  Recommended downgrade: {}",
            multimodal
                .recommended_downgrade
                .as_deref()
                .unwrap_or("none")
        ));
        lines.push(format!(
            "  Transport replaced parts: {}",
            if multimodal.transport_replaced_parts.is_empty() {
                "none".to_string()
            } else {
                multimodal.transport_replaced_parts.join(", ")
            }
        ));
        if !multimodal.attachments.is_empty() {
            lines.push("  Attachments:".to_string());
            for attachment in &multimodal.attachments {
                lines.push(format!(
                    "    {} ({})",
                    truncate_text(&attachment.filename, 72),
                    attachment.mime
                ));
            }
        }
        let combined_warnings = multimodal.combined_warnings();
        if !combined_warnings.is_empty() {
            lines.push("  Warnings:".to_string());
            for warning in &combined_warnings {
                lines.push(format!("    {}", truncate_text(warning, 108)));
            }
        }
    }

    if let Some(policy) = insights.effective_policy.as_ref() {
        lines.push(String::new());
        lines.extend(cli_effective_policy_lines(policy));
    }

    lines
}

fn cli_effective_policy_lines(policy: &rocode_types::SessionEffectivePolicyView) -> Vec<String> {
    let mut lines = vec![format!("Effective policy: session {}", policy.session_id)];

    if let Some(scheduler) = policy.scheduler.as_ref() {
        let requested = scheduler.requested_profile.as_deref().unwrap_or("--");
        let effective = scheduler.effective_profile.as_deref().unwrap_or("--");
        lines.push(format!(
            "  Scheduler: requested {} · effective {} · source {} · applied {}",
            requested,
            effective,
            scheduler.source,
            cli_yes_no(scheduler.applied)
        ));
        if scheduler.mode_kind.is_some()
            || scheduler.root_agent.is_some()
            || scheduler.resolved_agent.is_some()
        {
            lines.push(format!(
                "    Mode {} · root agent {} · resolved agent {}",
                scheduler.mode_kind.as_deref().unwrap_or("--"),
                scheduler.root_agent.as_deref().unwrap_or("--"),
                scheduler.resolved_agent.as_deref().unwrap_or("--")
            ));
        }
        if !scheduler.selection_trace.is_empty() {
            lines.push(format!(
                "    Trace {}",
                scheduler
                    .selection_trace
                    .iter()
                    .map(|step| {
                        let mut parts =
                            vec![cli_scheduler_trace_step_kind_label(&step.kind).to_string()];
                        if let Some(profile) = step.profile.as_deref() {
                            parts.push(profile.to_string());
                        }
                        if let Some(detail) = step.detail.as_deref() {
                            parts.push(truncate_text(detail, 72));
                        }
                        parts.push(format!("applied {}", cli_yes_no(step.applied)));
                        parts.join(" · ")
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
        if let Some(warning) = scheduler.warning.as_deref() {
            lines.push(format!("    Warning {}", truncate_text(warning, 108)));
        }
    }

    if let Some(provider) = policy.provider.as_ref() {
        lines.push(format!(
            "  Provider: {} · variant {}",
            provider.resolved_model,
            provider.variant.as_deref().unwrap_or("--")
        ));
        if let Some(descriptor) = provider.configured_descriptor.as_ref() {
            lines.push(format!(
                "    Configured descriptor: base {} · env {}",
                descriptor.base_url.as_deref().unwrap_or("--"),
                cli_join_or_placeholder(&descriptor.env)
            ));
            if let Some(profile) = descriptor.profile.as_ref() {
                lines.push(format!(
                    "    Configured profile: {}",
                    cli_provider_profile_summary(profile)
                ));
            }
        }
        if let Some(error) = provider.configured_descriptor_error.as_deref() {
            lines.push(format!(
                "    Descriptor projection error: {}",
                truncate_text(error, 108)
            ));
        }
        if let Some(runtime) = provider.runtime_profile.as_ref() {
            lines.push(format!(
                "    Runtime profile: {} · hash {}",
                cli_provider_profile_summary(&runtime.profile),
                truncate_text(&runtime.profile_hash, 40)
            ));
        }
    }

    if let Some(skill_tree) = policy.skill_tree.as_ref() {
        lines.push(format!(
            "  Skill tree: configured {} · enabled {} · applied {} · source {}",
            cli_yes_no(skill_tree.configured),
            cli_yes_no(skill_tree.enabled),
            cli_yes_no(skill_tree.applied),
            skill_tree.source
        ));
        if skill_tree.estimated_tokens.is_some()
            || skill_tree.token_budget.is_some()
            || skill_tree.truncation_strategy.is_some()
            || skill_tree.truncated.is_some()
        {
            lines.push(format!(
                "    Estimated {} · budget {} · truncation {} · truncated {}",
                skill_tree
                    .estimated_tokens
                    .map(format_token_count)
                    .unwrap_or_else(|| "--".to_string()),
                skill_tree
                    .token_budget
                    .map(format_token_count)
                    .unwrap_or_else(|| "--".to_string()),
                skill_tree.truncation_strategy.as_deref().unwrap_or("--"),
                skill_tree.truncated.map(cli_yes_no).unwrap_or("--")
            ));
        }
    }

    if let Some(memory) = policy.memory.as_ref() {
        lines.push(format!(
            "  Memory: {} · {} · scopes {}",
            memory.workspace_mode,
            truncate_text(&memory.workspace_key, 88),
            if memory.allowed_scopes.is_empty() {
                "--".to_string()
            } else {
                memory
                    .allowed_scopes
                    .iter()
                    .map(|scope| format!("{scope:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ));
        lines.push(format!(
            "    Frozen snapshot {} · last prefetch {}",
            memory.frozen_snapshot_items, memory.last_prefetch_items
        ));
    }

    lines.push(format!(
        "  Compaction: auto {} · prune {} · reserved {}",
        cli_yes_no(policy.compaction.auto),
        cli_yes_no(policy.compaction.prune),
        policy
            .compaction
            .reserved
            .map(format_token_count)
            .unwrap_or_else(|| "--".to_string())
    ));

    if let Some(external) = policy.external_adapter.as_ref() {
        lines.push(format!(
            "  External adapter: source {} · policy {} · batch {}",
            external.last_ingress_source,
            external.last_ingress_policy.as_deref().unwrap_or("--"),
            external
                .last_ingress_batch_count
                .map(|value: u64| value.to_string())
                .unwrap_or_else(|| "--".to_string())
        ));
    }

    if !policy.warnings.is_empty() {
        lines.push("  Warnings:".to_string());
        for warning in &policy.warnings {
            lines.push(format!("    {}", truncate_text(warning, 108)));
        }
    }

    lines
}

fn cli_provider_profile_summary(profile: &rocode_types::ProviderProfileDescriptorView) -> String {
    let mut parts = vec![
        format!("source {}", profile.source),
        format!("family {}", profile.api_family),
        format!("shape {}", profile.api_shape),
        format!("transport {}", profile.transport),
        format!("usage {}", profile.usage_shape),
        format!("cache {}", profile.cache_family),
    ];
    if !profile.quirks.is_empty() {
        parts.push(format!("quirks {}", profile.quirks.join(", ")));
    }
    parts.join(" · ")
}

fn cli_scheduler_trace_step_kind_label(
    kind: &rocode_types::SessionEffectiveSchedulerTraceStepKind,
) -> &'static str {
    match kind {
        rocode_types::SessionEffectiveSchedulerTraceStepKind::RequestedProfile => {
            "requested_profile"
        }
        rocode_types::SessionEffectiveSchedulerTraceStepKind::CommandWorkflowOverride => {
            "command_workflow_override"
        }
        rocode_types::SessionEffectiveSchedulerTraceStepKind::SessionPinnedProfile => {
            "session_pinned_profile"
        }
        rocode_types::SessionEffectiveSchedulerTraceStepKind::LegacySessionPinnedProfile => {
            "legacy_session_pinned_profile"
        }
        rocode_types::SessionEffectiveSchedulerTraceStepKind::ConfigDefaultProfile => {
            "config_default_profile"
        }
        rocode_types::SessionEffectiveSchedulerTraceStepKind::AutoRoute => "auto_route",
        rocode_types::SessionEffectiveSchedulerTraceStepKind::SoftFallback => "soft_fallback",
    }
}

fn cli_join_or_placeholder(values: &[String]) -> String {
    if values.is_empty() {
        "--".to_string()
    } else {
        values.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::cli_session_insights_lines;
    use crate::api_client::SessionInsightsResponse;
    use rocode_types::{
        MemoryScope, ProviderConnectionDescriptorCandidate, ProviderProfileDescriptorView,
        SessionEffectiveCompactionPolicy, SessionEffectiveExternalAdapterPolicy,
        SessionEffectiveMemoryPolicy, SessionEffectivePolicyView, SessionEffectiveProviderPolicy,
        SessionEffectiveProviderRuntimeProfile, SessionEffectiveSchedulerPolicy,
        SessionEffectiveSchedulerTraceStep, SessionEffectiveSchedulerTraceStepKind,
        SessionEffectiveSkillTreePolicy,
    };

    #[test]
    fn session_insights_surface_effective_policy_sections() {
        let insights = SessionInsightsResponse {
            id: "sess_123".to_string(),
            title: "Session title".to_string(),
            directory: "/workspace/project".to_string(),
            updated: 1_714_560_000_000,
            telemetry: None,
            memory: None,
            multimodal: None,
            effective_policy: Some(SessionEffectivePolicyView {
                session_id: "sess_123".to_string(),
                scheduler: Some(SessionEffectiveSchedulerPolicy {
                    requested_profile: Some("prometheus".to_string()),
                    effective_profile: Some("prometheus".to_string()),
                    source: "session_pinned_profile".to_string(),
                    applied: true,
                    mode_kind: Some("orchestrator".to_string()),
                    root_agent: Some("planner".to_string()),
                    resolved_agent: Some("planner".to_string()),
                    selection_trace: vec![SessionEffectiveSchedulerTraceStep {
                        kind: SessionEffectiveSchedulerTraceStepKind::SessionPinnedProfile,
                        profile: Some("prometheus".to_string()),
                        detail: Some(
                            "session metadata pinned this scheduler profile".to_string(),
                        ),
                        applied: true,
                    }],
                    warning: Some(
                        "configured scheduler defaults could not be resolved; continuing without scheduler profile"
                            .to_string(),
                    ),
                }),
                provider: Some(SessionEffectiveProviderPolicy {
                    provider_id: "openai".to_string(),
                    model_id: "gpt-4o".to_string(),
                    resolved_model: "openai/gpt-4o".to_string(),
                    variant: Some("fast".to_string()),
                    configured_descriptor: Some(ProviderConnectionDescriptorCandidate {
                        provider_id: "openai".to_string(),
                        name: Some("OpenAI".to_string()),
                        base_url: Some("https://api.openai.com/v1".to_string()),
                        env: vec!["OPENAI_API_KEY".to_string()],
                        profile: Some(ProviderProfileDescriptorView {
                            provider_id: "openai".to_string(),
                            npm: "@ai-sdk/openai".to_string(),
                            source: "bundled_default".to_string(),
                            api_family: "closeai-compatible".to_string(),
                            api_shape: "chat-completions".to_string(),
                            transport: "bearer".to_string(),
                            usage_shape: "closeai-cached-tokens".to_string(),
                            cache_family: "closeai-compatible".to_string(),
                            quirks: vec!["responses-fallback-to-chat".to_string()],
                        }),
                    }),
                    configured_descriptor_error: None,
                    runtime_profile: Some(SessionEffectiveProviderRuntimeProfile {
                        profile: ProviderProfileDescriptorView {
                            provider_id: "openai".to_string(),
                            npm: "@ai-sdk/openai".to_string(),
                            source: "runtime_fingerprint".to_string(),
                            api_family: "closeai-compatible".to_string(),
                            api_shape: "responses".to_string(),
                            transport: "bearer".to_string(),
                            usage_shape: "closeai-cached-tokens".to_string(),
                            cache_family: "closeai-compatible".to_string(),
                            quirks: Vec::new(),
                        },
                        profile_hash: "1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
                    }),
                }),
                skill_tree: Some(SessionEffectiveSkillTreePolicy {
                    configured: true,
                    enabled: true,
                    applied: true,
                    source: "config_composition".to_string(),
                    estimated_tokens: Some(256),
                    token_budget: Some(512),
                    truncation_strategy: Some("tail".to_string()),
                    truncated: Some(false),
                }),
                memory: Some(SessionEffectiveMemoryPolicy {
                    workspace_key: "/workspace/project".to_string(),
                    workspace_mode: "workspace_shared".to_string(),
                    allowed_scopes: vec![
                        MemoryScope::WorkspaceShared,
                        MemoryScope::SessionEphemeral,
                    ],
                    frozen_snapshot_items: 2,
                    last_prefetch_items: 5,
                }),
                compaction: SessionEffectiveCompactionPolicy {
                    auto: false,
                    prune: true,
                    reserved: Some(512),
                },
                external_adapter: Some(SessionEffectiveExternalAdapterPolicy {
                    last_ingress_source: "external:generic-webhook:generic".to_string(),
                    last_ingress_policy: Some("external_adapter_metadata_only".to_string()),
                    last_ingress_batch_count: Some(1),
                }),
                warnings: vec![
                    "provider descriptor projection failed for `openai`: invalid profile".to_string(),
                ],
            }),
        };

        let lines = cli_session_insights_lines("sess_123", &insights);

        assert!(lines
            .iter()
            .any(|line| line == "Effective policy: session sess_123"));
        assert!(lines.iter().any(|line| {
            line.contains("Scheduler: requested prometheus")
                && line.contains("source session_pinned_profile")
                && line.contains("applied yes")
        }));
        assert!(lines
            .iter()
            .any(|line| line.contains("Trace session_pinned_profile")));
        assert!(lines.iter().any(
            |line| line.contains("Warning configured scheduler defaults could not be resolved")
        ));
        assert!(lines
            .iter()
            .any(|line| line.contains("Provider: openai/gpt-4o · variant fast")));
        assert!(lines.iter().any(|line| {
            line.contains("Configured profile:")
                && line.contains("family closeai-compatible")
                && line.contains("quirks responses-fallback-to-chat")
        }));
        assert!(lines
            .iter()
            .any(|line| line.contains("Runtime profile:") && line.contains("shape responses")));
        assert!(lines.iter().any(|line| {
            line.contains("Skill tree: configured yes")
                && line.contains("source config_composition")
        }));
        assert!(lines.iter().any(|line| {
            line.contains("Memory: workspace_shared")
                && line.contains("WorkspaceShared, SessionEphemeral")
        }));
        assert!(lines.iter().any(|line| {
            line.contains("Compaction: auto no")
                && line.contains("prune yes")
                && line.contains("reserved 512")
        }));
        assert!(lines.iter().any(|line| {
            line.contains("External adapter: source external:generic-webhook:generic")
                && line.contains("policy external_adapter_metadata_only")
        }));
        assert!(lines
            .iter()
            .any(|line| line.contains("provider descriptor projection failed for `openai`")));
    }
}
