//! Shared projection authority for session-scoped frontend telemetry data.
//!
//! This module lives in `session_runtime` (orchestration layer) so both the
//! frontend projector and the HTTP telemetry route can consume the same
//! computation without layer inversion.
//!
//! Constitution Article 4 (土): single orchestration authority — the
//! functions here are the sole source of truth for the five projection
//! fields carried by `SessionProjectionReplaced`.

use agendao_session::prompt::{explain_session_cache_semantics, explain_session_context};
use agendao_session::{Session, SessionUsage};
use agendao_types::{
    ContextCompactionLifecycleSummary, ContextCompactionSummary,
    ContextPressureGovernanceSummary, SessionCacheSemanticsSummary,
    SessionContextClosureContract, SessionContextExplain, SessionDiagnosticsSidecar,
    SessionUsageBooks, WorkflowUsageSummary,
};

// ── Tree observation ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub(crate) struct SessionTreeObservation {
    pub(crate) workflow_cumulative: WorkflowUsageSummary,
    pub(crate) attached_subtree_session_count: usize,
}

pub(crate) fn session_tree_observation_for_session(
    sessions: &agendao_session::SessionManager,
    root_session_id: &str,
) -> SessionTreeObservation {
    let mut observation = SessionTreeObservation::default();
    let mut pending = vec![(root_session_id.to_string(), true)];

    while let Some((session_id, is_root)) = pending.pop() {
        let Some(session) = sessions.get(&session_id) else {
            continue;
        };
        observation
            .workflow_cumulative
            .accumulate_session_usage(&session.get_usage());
        let attached_children = sessions
            .attached_sessions(&session_id)
            .into_iter()
            .map(|child| child.record().id.clone())
            .collect::<Vec<_>>();
        if !is_root {
            observation.attached_subtree_session_count += 1;
        }
        pending.extend(
            attached_children
                .into_iter()
                .map(|child_id| (child_id, false)),
        );
    }

    observation
}

// ── Pressure percent ───────────────────────────────────────────────────────

pub(crate) fn pressure_percent(tokens: Option<u64>, limit_tokens: Option<u64>) -> Option<u64> {
    let tokens = tokens?;
    let limit = limit_tokens?;
    (limit > 0).then_some(tokens.saturating_mul(100) / limit)
}

// ── Context closure contract ────────────────────────────────────────────────

pub(crate) fn build_context_closure_contract(
    context_explain: Option<&SessionContextExplain>,
    usage_books: &SessionUsageBooks,
    cache_semantics: Option<&SessionCacheSemanticsSummary>,
    context_compaction_summary: Option<&ContextCompactionSummary>,
    context_compaction_lifecycle_summary: Option<&ContextCompactionLifecycleSummary>,
    context_pressure_governance_summary: Option<&ContextPressureGovernanceSummary>,
    attached_subtree_session_count: usize,
) -> Option<SessionContextClosureContract> {
    let context_explain = context_explain?;
    let cache_semantics = cache_semantics
        .cloned()
        .unwrap_or(SessionCacheSemanticsSummary {
            basis: agendao_types::SessionCacheSemanticsBasis::ApiView,
            api_view_messages: context_explain.api_view_messages,
            trimmed_model_visible_messages: context_explain
                .raw_model_visible_messages
                .saturating_sub(context_explain.api_view_messages),
            boundary: None,
            cache_evidence: None,
            prompt_surface_evidence: None,
            label: None,
        });

    let prefix_change_detected = cache_semantics
        .boundary
        .as_ref()
        .is_some_and(|boundary| boundary.likely_changed_prefix)
        || cache_semantics
            .cache_evidence
            .as_ref()
            .is_some_and(|summary| summary.severity > agendao_types::SessionCacheSeverity::Stable)
        || cache_semantics
            .prompt_surface_evidence
            .as_ref()
            .is_some_and(|summary| summary.severity > agendao_types::SessionCacheSeverity::Stable);

    let installed_boundary =
        context_compaction_lifecycle_summary.and_then(|summary| summary.installed.as_ref());
    let (request_pressure_percent, live_pressure_percent) =
        if let Some(summary) = context_pressure_governance_summary {
            (
                summary.request_pressure_percent.or_else(|| {
                    pressure_percent(summary.request_context_tokens, summary.limit_tokens)
                }),
                summary.live_pressure_percent.or_else(|| {
                    pressure_percent(summary.live_context_tokens, summary.limit_tokens)
                }),
            )
        } else if let Some(installed) = installed_boundary {
            let limit_tokens =
                context_compaction_lifecycle_summary.and_then(|summary| summary.limit_tokens);
            (
                pressure_percent(installed.request_context_tokens, limit_tokens),
                pressure_percent(installed.live_context_tokens, limit_tokens),
            )
        } else if let Some(summary) = context_compaction_summary {
            (
                pressure_percent(summary.request_context_tokens, summary.limit_tokens),
                pressure_percent(summary.live_context_tokens, summary.limit_tokens),
            )
        } else {
            (None, None)
        };

    let (cache_explainability_source, cache_explainability_severity, cache_explainability_text) =
        if let Some(summary) = cache_semantics.cache_evidence.as_ref().filter(|summary| {
            summary.severity > agendao_types::SessionCacheSeverity::Stable
                && !matches!(summary.status.as_str(), "stable" | "cold_start")
        }) {
            (
                agendao_types::SessionCacheExplainabilitySource::CacheEvidence,
                Some(summary.severity),
                cache_semantics
                    .label
                    .clone()
                    .or_else(|| summary.primary_cause.clone()),
            )
        } else if let Some(summary) = cache_semantics
            .prompt_surface_evidence
            .as_ref()
            .filter(|summary| summary.severity > agendao_types::SessionCacheSeverity::Stable)
        {
            (
                agendao_types::SessionCacheExplainabilitySource::SurfaceEvidence,
                Some(summary.severity),
                cache_semantics
                    .label
                    .clone()
                    .or_else(|| Some(summary.reason.clone())),
            )
        } else if cache_semantics
            .boundary
            .as_ref()
            .is_some_and(|boundary| boundary.possible_cache_evidence)
        {
            (
                agendao_types::SessionCacheExplainabilitySource::BoundaryEvidence,
                Some(agendao_types::SessionCacheSeverity::MediumChange),
                cache_semantics.label.clone().or_else(|| {
                    cache_semantics
                        .boundary
                        .as_ref()
                        .and_then(|boundary| boundary.reason.clone())
                }),
            )
        } else {
            (
                agendao_types::SessionCacheExplainabilitySource::None,
                None,
                cache_semantics.label.clone(),
            )
        };
    let cache_issue_present = !matches!(
        cache_explainability_source,
        agendao_types::SessionCacheExplainabilitySource::None
    );
    let owner_session_cumulative_tokens = context_explain.owner_session_cumulative_tokens;
    let workflow_cumulative_tokens = usage_books.workflow_cumulative.total_tokens();
    let attached_subtree_cumulative_tokens =
        workflow_cumulative_tokens.saturating_sub(owner_session_cumulative_tokens);

    Some(SessionContextClosureContract {
        prefix_stability: agendao_types::SessionPrefixStabilityContract {
            basis: cache_semantics.basis,
            tracked_on_api_view: matches!(
                cache_semantics.basis,
                agendao_types::SessionCacheSemanticsBasis::ApiView
            ),
            api_view_messages: cache_semantics.api_view_messages,
            trimmed_model_visible_messages: cache_semantics.trimmed_model_visible_messages,
            prefix_change_detected,
            explanation: cache_semantics.label.clone(),
        },
        compaction_boundary: agendao_types::SessionCompactionBoundaryContract {
            boundary_recorded: context_compaction_summary.is_some()
                || context_compaction_lifecycle_summary.is_some()
                || context_pressure_governance_summary.is_some(),
            phase: context_pressure_governance_summary
                .map(|summary| summary.phase.clone())
                .or_else(|| {
                    context_compaction_lifecycle_summary.and_then(|summary| summary.phase.clone())
                })
                .or_else(|| context_compaction_summary.and_then(|summary| summary.phase.clone())),
            trigger: context_pressure_governance_summary
                .map(|summary| summary.trigger.clone())
                .or_else(|| {
                    context_compaction_lifecycle_summary.map(|summary| summary.trigger.clone())
                })
                .or_else(|| context_compaction_summary.map(|summary| summary.trigger.clone())),
            reason: context_pressure_governance_summary
                .and_then(|summary| summary.reason.clone())
                .or_else(|| {
                    context_compaction_lifecycle_summary.and_then(|summary| summary.reason.clone())
                })
                .or_else(|| context_compaction_summary.and_then(|summary| summary.reason.clone())),
            lifecycle_status: context_compaction_lifecycle_summary.map(|summary| summary.status),
            governance_status: context_pressure_governance_summary.map(|summary| summary.status),
            request_pressure_percent,
            live_pressure_percent,
            compaction_attempted: context_pressure_governance_summary
                .map(|summary| summary.compaction_attempted)
                .unwrap_or_else(|| {
                    context_compaction_lifecycle_summary.is_some()
                        || context_compaction_summary.is_some()
                }),
            compaction_succeeded: context_pressure_governance_summary
                .map(|summary| summary.compaction_succeeded)
                .unwrap_or_else(|| {
                    context_compaction_lifecycle_summary.is_some_and(|summary| {
                        matches!(
                            summary.status,
                            agendao_types::ContextCompactionLifecycleStatus::Installed
                        )
                    }) || context_compaction_summary.is_some()
                }),
            blocking: context_pressure_governance_summary
                .map(|summary| summary.blocking)
                .unwrap_or(false),
            installed: installed_boundary.cloned(),
        },
        cache_explainability: agendao_types::SessionCacheExplainabilityContract {
            issue_present: cache_issue_present,
            explained: !cache_issue_present || cache_explainability_text.is_some(),
            source: cache_explainability_source,
            severity: cache_explainability_severity,
            explanation: cache_explainability_text,
        },
        child_history_isolation: agendao_types::SessionChildHistoryIsolationContract {
            attached_subtree_session_count,
            owner_session_cumulative_tokens,
            workflow_cumulative_tokens,
            attached_subtree_cumulative_tokens,
            owner_live_context_tokens: usage_books.live_context_tokens,
            owner_local_live_prefix: true,
            child_history_in_live_prefix_detected: false,
            explanation: if attached_subtree_session_count > 0 {
                "Attached subtree usage contributes to workflow cumulative only; API view and live prefix remain owner-local."
                    .to_string()
            } else {
                "No attached subtree sessions were observed; the live prefix remains owner-local."
                    .to_string()
            },
        },
    })
}

// ── Projection fields ──────────────────────────────────────────────────────

/// The five projection fields carried by `SessionProjectionReplaced`.
///
/// Computed from session-scoped authority data (session record, message
/// metadata, session tree). The caller must hold a reference to the
/// `Session` and the `SessionManager` (for tree traversal).
pub(crate) struct SessionProjectionFields {
    pub usage_books: SessionUsageBooks,
    pub context_compaction_summary: Option<ContextCompactionSummary>,
    pub context_compaction_lifecycle_summary: Option<ContextCompactionLifecycleSummary>,
    pub cache_semantics: Option<SessionCacheSemanticsSummary>,
    pub context_closure_contract: Option<SessionContextClosureContract>,
}

/// Build the five projection fields from session-scoped authority data.
///
/// This is a pure synchronous computation over the session record and its
/// message metadata. It is safe to call while holding the sessions lock.
pub(crate) fn build_session_projection_fields(
    session: &Session,
    session_id: &str,
    runtime_usage: Option<&SessionUsage>,
    sessions: &agendao_session::SessionManager,
) -> SessionProjectionFields {
    // ── usage_books ────────────────────────────────────────────────────
    let tree_observation = session_tree_observation_for_session(sessions, session_id);
    let mut usage_books = SessionUsageBooks {
        request_context_tokens: session.latest_request_context_tokens(),
        live_context_tokens: runtime_usage.and_then(|u| u.live_context_tokens()),
        workflow_cumulative: tree_observation.workflow_cumulative.clone(),
    };

    // ── diagnostics sidecar (shared for multiple fields) ───────────────
    let diagnostics = SessionDiagnosticsSidecar::derive_from_session(session);

    // ── cache evidence (needed for cache_semantics) ────────────────────
    let cache_evidence = diagnostics
        .as_ref()
        .and_then(SessionDiagnosticsSidecar::latest_cache_evidence_value);
    let typed_cache_evidence = cache_evidence.clone().and_then(|value| {
        serde_json::from_value::<agendao_provider::cache::CacheEvidenceSummary>(value).ok()
    });

    // ── context compaction summary ─────────────────────────────────────
    let context_compaction_summary = diagnostics
        .as_ref()
        .and_then(SessionDiagnosticsSidecar::latest_context_compaction_record_value)
        .and_then(|value| serde_json::from_value(value).ok());

    // ── context compaction lifecycle summary ───────────────────────────
    let context_compaction_lifecycle_summary = diagnostics
        .as_ref()
        .and_then(SessionDiagnosticsSidecar::context_compaction_lifecycle_summary_value)
        .and_then(|value| serde_json::from_value(value).ok());

    // ── context pressure governance (needed for closure contract) ──────
    let context_pressure_governance_summary = diagnostics
        .as_ref()
        .and_then(SessionDiagnosticsSidecar::context_pressure_governance_summary_value)
        .and_then(|value| serde_json::from_value(value).ok());

    // ── context explain ────────────────────────────────────────────────
    let context_explain = Some(explain_session_context(
        session,
        Some(usage_books.workflow_cumulative.total_tokens()),
    ));
    if let Some(live_context_tokens) = context_explain
        .as_ref()
        .and_then(|explain| explain.live_context_tokens)
    {
        usage_books.live_context_tokens = Some(live_context_tokens);
    }

    // ── prompt surface evidence (needed for cache_semantics) ───────────
    let prompt_surface_evidence = diagnostics
        .as_ref()
        .and_then(SessionDiagnosticsSidecar::latest_prompt_surface_evidence_value)
        .and_then(|value| serde_json::from_value(value).ok());

    // ── cache semantics ────────────────────────────────────────────────
    let cache_semantics = context_explain.as_ref().map(|context_explain| {
        explain_session_cache_semantics(
            context_explain,
            context_compaction_summary.as_ref(),
            typed_cache_evidence.as_ref(),
            prompt_surface_evidence.as_ref(),
        )
    });

    // ── context closure contract ───────────────────────────────────────
    let context_closure_contract = build_context_closure_contract(
        context_explain.as_ref(),
        &usage_books,
        cache_semantics.as_ref(),
        context_compaction_summary.as_ref(),
        context_compaction_lifecycle_summary.as_ref(),
        context_pressure_governance_summary.as_ref(),
        tree_observation.attached_subtree_session_count,
    );

    SessionProjectionFields {
        usage_books,
        context_compaction_summary,
        context_compaction_lifecycle_summary,
        cache_semantics,
        context_closure_contract,
    }
}
