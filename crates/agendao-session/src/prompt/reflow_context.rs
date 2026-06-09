//! Prompt Reflow Context — unified explanation surface for memory and
//! continuity reflow into the next-turn prompt.
//!
//! ## Constitution Placement
//!
//! This module provides the **single explanation surface** for 水生木
//! (water nourishes wood): how memory, compaction continuity, and
//! scheduler hydrate anchors feed back into the next turn's input.
//!
//! ## What already exists
//!
//! The typed contracts are already in place:
//!
//! | Contract                       | Location                | Role                        |
//! |--------------------------------|-------------------------|-----------------------------|
//! | `MemoryRetrievalPacket`        | `agendao-types/memory`  | Memory recall result         |
//! | `SessionContinuityPacket`      | `agendao-types/session` | Compaction continuity        |
//! | `prompt_surface_state_snapshot`| Session metadata        | Frozen snapshot for diagnostics|
//! | `memory_last_prefetch_packet`  | Session metadata        | Last prefetch for diagnostics |
//!
//! ## What this module adds
//!
//! A **single explanation surface** (`PromptReflowContext`) that
//! aggregates these existing contracts so that:
//!
//! - session prompt
//! - scheduler hydrate route
//! - diagnostics sidecar
//!
//! all share the same naming and interpretation for "how memory flows
//! back into the next turn."
//!
//! ## Non-goals
//!
//! - Does NOT create new storage.
//! - Does NOT replace `MemoryRetrievalPacket` or `SessionContinuityPacket`.
//! - Does NOT modify session metadata or production paths.
//! - Does NOT physically merge scheduler hydrate with session prompt.
//!
//! ## Migration contract
//!
//! - **Phase 4 (this commit)**: Define view structures only.
//! - **Phase 5**: Migrate session-side consumers to read from these views.
//! - **Phase 6**: Migrate scheduler / diagnostics to shared naming.

// Skeleton types are intentionally unused until Phase 5 cut-over.
#![allow(dead_code)]

use agendao_types::{
    MemoryRetrievalPacket, SessionContinuityPacket,
};

// ── Top-level reflow context ────────────────────────────────────────────

/// Aggregated reflow information for a single turn.
///
/// This is the single explanation surface consumed by session prompt,
/// scheduler hydrate, and diagnostics.  It projects the existing
/// typed contracts (`MemoryRetrievalPacket`, `SessionContinuityPacket`,
/// session metadata) without owning or duplicating them.
///
/// # Writer / reader / displayer / authority
///
/// | Role       | Module(s)                                         |
/// |------------|---------------------------------------------------|
/// | Writer     | `MemoryAuthority`, continuity packet builder, prefetch writer |
/// | Reader     | `SessionPrompt`, scheduler hydrate route, diagnostics sidecar |
/// | Displayer  | TUI / Web / API diagnostics                       |
/// | Authority  | `PromptReflowContext` (explanation, not storage)  |
#[derive(Debug, Clone)]
pub(crate) struct PromptReflowContext {
    /// Session identity.
    pub(crate) session_id: String,

    /// Memory recall projection (from `MemoryRetrievalPacket`).
    pub(crate) memory: Option<PromptReflowMemoryView>,

    /// Compaction continuity projection (from `SessionContinuityPacket`).
    pub(crate) continuity: Option<PromptReflowContinuityView>,

    /// Diagnostics-visible projection (from session metadata).
    pub(crate) diagnostics: PromptReflowDiagnosticsView,
}

// ── Memory view ─────────────────────────────────────────────────────────

/// Projection of `MemoryRetrievalPacket` for reflow explanation.
///
/// This view preserves every field that `render_memory_prefetch_reminder()`
/// currently reads from `MemoryRetrievalPacket`, so that the Phase 5
/// migration to `PromptReflowContext` is lossless.
#[derive(Debug, Clone)]
pub(crate) struct PromptReflowMemoryView {
    /// Whether this is a snapshot (frozen) or live prefetch.
    pub(crate) is_snapshot: bool,

    /// The recall query that produced this packet.
    pub(crate) query: Option<String>,

    /// Number of recalled items.
    pub(crate) item_count: usize,

    /// Per-item detail: every field that the current reminder renderer
    /// reads from `MemoryRecallView` / `MemoryCardView`.
    pub(crate) items: Vec<PromptReflowMemoryItem>,

    /// Memory record IDs that are authorized for `scheduler_memory_hydrate`.
    pub(crate) hydrate_record_ids: Vec<String>,
}

/// A single recalled memory item projected for reflow explanation.
///
/// Mirrors the fields of `MemoryRecallView` and `MemoryCardView` that
/// `render_memory_prefetch_reminder()` currently consumes.
#[derive(Debug, Clone)]
pub(crate) struct PromptReflowMemoryItem {
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) why_recalled: String,
    pub(crate) evidence_summary: Option<String>,
    pub(crate) kind: String,
    pub(crate) validation_status: String,
    pub(crate) last_validated_at: Option<i64>,
    pub(crate) record_id: String,
}

// ── Continuity view ─────────────────────────────────────────────────────

/// Projection of `SessionContinuityPacket` for reflow explanation.
///
/// This captures the compaction continuity contract: which messages
/// and memory records are authorized for hydrate calls in the next turn.
#[derive(Debug, Clone)]
pub(crate) struct PromptReflowContinuityView {
    /// Number of eligible context messages at compaction time.
    pub(crate) eligible_message_count: usize,

    /// Number of messages in the exact recent tail.
    pub(crate) exact_recent_tail_count: usize,

    /// Number of older turns omitted by compaction.
    pub(crate) omitted_older_turns: usize,

    /// Message IDs authorized for `scheduler_context_hydrate`.
    /// These are the non-projected tail messages + continuation
    /// dependency message IDs.
    pub(crate) hydrate_message_ids: Vec<String>,

    /// Memory record IDs authorized for `scheduler_memory_hydrate`.
    /// From the packet's `memory_anchors`.
    pub(crate) hydrate_memory_record_ids: Vec<String>,

    /// Whether a continuation dependency exists (tool-call turn
    /// that requires exact message chain preservation).
    pub(crate) has_continuation_dependency: bool,

    /// Compaction summary text, if present.
    pub(crate) compaction_summary: Option<String>,

    /// Recall policy guidance text (how the model should use hydrate tools).
    pub(crate) recall_policy: Option<String>,
}

// ── Diagnostics view ────────────────────────────────────────────────────

/// Diagnostics-visible projection of reflow state.
///
/// These fields mirror what `SessionDiagnosticsSidecar` exposes,
/// using the same naming as `PromptReflowContext` for consistency.
#[derive(Debug, Clone, Default)]
pub(crate) struct PromptReflowDiagnosticsView {
    /// Whether a continuity packet exists in session metadata.
    pub(crate) has_continuity_packet: bool,

    /// Whether a frozen memory snapshot exists in session metadata.
    pub(crate) has_frozen_snapshot: bool,

    /// Whether a last-prefetch packet exists in session metadata
    /// (`memory_last_prefetch_packet`).  This is NOT derived from the
    /// current turn's `memory_prefetch` — it reflects the persisted
    /// diagnostics sidecar state, which is a separate input.
    pub(crate) has_last_prefetch: bool,

    /// Cache evidence summary status, if present.
    pub(crate) cache_evidence_status: Option<String>,
}

// ── Construction ────────────────────────────────────────────────────────

impl PromptReflowContext {
    /// Build the reflow context from existing typed contracts.
    ///
    /// This aggregates `MemoryRetrievalPacket` and
    /// `SessionContinuityPacket` into a single explanation surface.
    /// It does not create new storage — it only projects what
    /// already exists.
    ///
    /// All diagnostics-level flags (`has_frozen_snapshot`,
    /// `has_last_prefetch`) are explicit inputs so the caller
    /// controls which session metadata key they come from.
    ///
    /// In Phase 5, session-side consumers will call this instead of
    /// inspecting individual metadata keys.
    pub(crate) fn build(
        session_id: impl Into<String>,
        memory_prefetch: Option<&MemoryRetrievalPacket>,
        continuity_packet: Option<&SessionContinuityPacket>,
        has_frozen_snapshot: bool,
        has_last_prefetch: bool,
        cache_evidence_status: Option<String>,
    ) -> Self {
        let memory = memory_prefetch.map(|packet| PromptReflowMemoryView {
            is_snapshot: packet.snapshot,
            query: packet.query.clone(),
            item_count: packet.items.len(),
            items: packet
                .items
                .iter()
                .map(|item| PromptReflowMemoryItem {
                    title: item.card.title.clone(),
                    summary: item.card.summary.clone(),
                    why_recalled: item.why_recalled.clone(),
                    evidence_summary: item.evidence_summary.clone(),
                    kind: format!("{:?}", item.card.kind),
                    validation_status: format!("{:?}", item.card.validation_status),
                    last_validated_at: item.card.last_validated_at,
                    record_id: item.card.id.0.clone(),
                })
                .collect(),
            hydrate_record_ids: packet.items.iter().map(|item| item.card.id.0.clone()).collect(),
        });

        let continuity = continuity_packet.map(|packet| PromptReflowContinuityView {
            eligible_message_count: packet.eligible_message_count,
            exact_recent_tail_count: packet.exact_recent_tail_count,
            omitted_older_turns: packet.omitted_older_turns,
            hydrate_message_ids: packet.allowed_message_ids(),
            hydrate_memory_record_ids: packet.allowed_memory_record_ids(),
            has_continuation_dependency: !packet.continuation_dependencies.is_empty(),
            compaction_summary: packet
                .latest_compaction_summary
                .as_ref()
                .map(|summary| summary.summary.clone()),
            recall_policy: packet.recall_policy.clone(),
        });

        let diagnostics = PromptReflowDiagnosticsView {
            has_continuity_packet: continuity_packet.is_some(),
            has_frozen_snapshot,
            has_last_prefetch,
            cache_evidence_status,
        };

        Self {
            session_id: session_id.into(),
            memory,
            continuity,
            diagnostics,
        }
    }
}

// ── Construction from existing typed contracts ─────────────────────────

impl PromptReflowMemoryView {
    /// Project a `MemoryRetrievalPacket` into the reflow memory view.
    ///
    /// This is the same field mapping that `PromptReflowContext::build()`
    /// uses internally; exposed here so that `render_memory_prefetch_reminder`
    /// can delegate without constructing the full context.
    pub(crate) fn from_packet(packet: &MemoryRetrievalPacket) -> Self {
        Self {
            is_snapshot: packet.snapshot,
            query: packet.query.clone(),
            item_count: packet.items.len(),
            items: packet
                .items
                .iter()
                .map(|item| PromptReflowMemoryItem {
                    title: item.card.title.clone(),
                    summary: item.card.summary.clone(),
                    why_recalled: item.why_recalled.clone(),
                    evidence_summary: item.evidence_summary.clone(),
                    kind: format!("{:?}", item.card.kind),
                    validation_status: format!("{:?}", item.card.validation_status),
                    last_validated_at: item.card.last_validated_at,
                    record_id: item.card.id.0.clone(),
                })
                .collect(),
            hydrate_record_ids: packet
                .items
                .iter()
                .map(|item| item.card.id.0.clone())
                .collect(),
        }
    }
}

// ── Rendering: memory reminder (lossless from render_memory_prefetch_reminder) ─

impl PromptReflowMemoryView {
    /// Render the memory prefetch reminder in the same format as
    /// `SessionPrompt::render_memory_prefetch_reminder()`.
    ///
    /// The output is byte-equivalent to the legacy path so that
    /// Commit 2's reminder regression tests hold.
    pub(crate) fn render_reminder(&self) -> Option<String> {
        if self.items.is_empty() {
            return None;
        }

        let mut lines = vec!["Turn Memory Recall:".to_string()];
        if let Some(query) = self
            .query
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            lines.push(format!("- query: {}", query.trim()));
        }
        for item in &self.items {
            // kind / validation_status are stored as their Debug representation
            // (e.g. "Lesson", "Passed") — use {} to avoid quoting.
            lines.push(format!(
                "- {} [{} / {}]",
                item.title, item.kind, item.validation_status
            ));
            lines.push(format!("  why: {}", item.why_recalled));
            lines.push(format!("  summary: {}", item.summary));
            if let Some(ref evidence) = item.evidence_summary {
                lines.push(format!("  evidence: {}", evidence));
            }
            if let Some(last_validated_at) = item.last_validated_at {
                lines.push(format!("  last_validated_at: {}", last_validated_at));
            }
        }

        Some(lines.join("\n"))
    }
}

// ── Construction from existing typed contracts ─────────────────────────

impl PromptReflowContinuityView {
    /// Project a `SessionContinuityPacket` into the reflow continuity view.
    ///
    /// This is the same field mapping that `PromptReflowContext::build()`
    /// uses internally; exposed here so that callers can project a packet
    /// without constructing the full context.
    pub(crate) fn from_packet(packet: &SessionContinuityPacket) -> Self {
        Self {
            eligible_message_count: packet.eligible_message_count,
            exact_recent_tail_count: packet.exact_recent_tail_count,
            omitted_older_turns: packet.omitted_older_turns,
            hydrate_message_ids: packet.allowed_message_ids(),
            hydrate_memory_record_ids: packet.allowed_memory_record_ids(),
            has_continuation_dependency: !packet.continuation_dependencies.is_empty(),
            compaction_summary: packet
                .latest_compaction_summary
                .as_ref()
                .map(|summary| summary.summary.clone()),
            recall_policy: packet.recall_policy.clone(),
        }
    }
}

// ── Rendering: continuity explanation ──────────────────────────────────

impl PromptReflowContinuityView {
    /// Human-readable explanation of what continuity reflow provides
    /// to the next turn.
    ///
    /// This is a diagnostics-level summary (not model-visible) that
    /// answers: "which hydrate anchors are available, and what
    /// compaction context was preserved?"
    pub(crate) fn explain(&self) -> String {
        let mut lines = Vec::new();

        lines.push(format!(
            "continuity: eligible={} tail={} omitted={} hydrate_msg_ids={} hydrate_mem_ids={}",
            self.eligible_message_count,
            self.exact_recent_tail_count,
            self.omitted_older_turns,
            self.hydrate_message_ids.len(),
            self.hydrate_memory_record_ids.len(),
        ));

        if self.has_continuation_dependency {
            lines.push("  continuation_dependency: present (tool-call turn requires exact chain)".to_string());
        }

        if let Some(ref summary) = self.compaction_summary {
            lines.push(format!("  compaction_summary: {}", summary));
        }

        if let Some(ref policy) = self.recall_policy {
            lines.push(format!("  recall_policy: {}", policy));
        }

        lines.join("\n")
    }
}

// ── Rendering: top-level summary ───────────────────────────────────────

impl PromptReflowContext {
    /// Render a human-readable summary of the reflow context.
    ///
    /// This produces a diagnostic string (not model-visible) that
    /// explains what reflow sources are active for this turn.
    /// It is consumed by TUI / Web / API diagnostics, not injected
    /// into the prompt.
    pub(crate) fn render_summary(&self) -> String {
        let mut lines: Vec<String> = Vec::new();

        if let Some(ref mem) = self.memory {
            lines.push(format!(
                "memory: {} items recalled{}",
                mem.item_count,
                if mem.is_snapshot { " (frozen snapshot)" } else { "" },
            ));
        } else {
            lines.push("memory: none".to_string());
        }

        if let Some(ref cont) = self.continuity {
            lines.push(format!(
                "continuity: tail={} omitted={} hydrate_msg_ids={} hydrate_mem_ids={}{}",
                cont.exact_recent_tail_count,
                cont.omitted_older_turns,
                cont.hydrate_message_ids.len(),
                cont.hydrate_memory_record_ids.len(),
                if cont.has_continuation_dependency {
                    " [has_continuation_dep]"
                } else {
                    ""
                },
            ));
        } else {
            lines.push("continuity: none".to_string());
        }

        lines.push(format!(
            "diagnostics: continuity_packet={} frozen_snapshot={} last_prefetch={}",
            self.diagnostics.has_continuity_packet,
            self.diagnostics.has_frozen_snapshot,
            self.diagnostics.has_last_prefetch,
        ));

        lines.join("\n")
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_types::{
        MemoryCardView, MemoryKind, MemoryRecallView, MemoryRecordId, MemoryScope, MemoryStatus,
        MemoryValidationStatus,
    };

    fn sample_retrieval_packet() -> MemoryRetrievalPacket {
        MemoryRetrievalPacket {
            generated_at: 1700000000000,
            snapshot: false,
            query: Some("relevant patterns".to_string()),
            scopes: vec![MemoryScope::GlobalWorkspace],
            items: vec![
                MemoryRecallView {
                    card: MemoryCardView {
                        id: MemoryRecordId("rec-1".to_string()),
                        kind: MemoryKind::Lesson,
                        scope: MemoryScope::GlobalWorkspace,
                        status: MemoryStatus::Validated,
                        title: "Use ArcSwap for config".to_string(),
                        summary: "Config reads should be lock-free".to_string(),
                        derived_skill_name: None,
                        linked_skill_name: None,
                        confidence: Some(0.85),
                        validation_status: MemoryValidationStatus::Passed,
                        last_validated_at: Some(1700000000000),
                    },
                    why_recalled: "matches current refactoring task".to_string(),
                    evidence_summary: Some("session-42 evidence".to_string()),
                },
                MemoryRecallView {
                    card: MemoryCardView {
                        id: MemoryRecordId("rec-2".to_string()),
                        kind: MemoryKind::Pattern,
                        scope: MemoryScope::GlobalWorkspace,
                        status: MemoryStatus::Consolidated,
                        title: "Prefer single write point".to_string(),
                        summary: "Mutations go through one authority".to_string(),
                        derived_skill_name: None,
                        linked_skill_name: None,
                        confidence: Some(0.9),
                        validation_status: MemoryValidationStatus::Passed,
                        last_validated_at: Some(1700000000000),
                    },
                    why_recalled: "reinforces current architecture work".to_string(),
                    evidence_summary: Some("session-43 evidence".to_string()),
                },
            ],
            note: None,
            budget_limit: None,
        }
    }

    fn sample_continuity_packet() -> SessionContinuityPacket {
        use agendao_types::{
            SessionContinuityCompactionSummary, SessionContinuityDependency,
            SessionContinuityDependencyKind, SessionContinuityLimits, SessionContinuityTurn,
        };
        SessionContinuityPacket {
            version: 1,
            eligible_message_count: 8,
            exact_recent_tail_count: 3,
            omitted_older_turns: 5,
            exact_recent_tail: vec![
                SessionContinuityTurn {
                    message_id: "msg-a".to_string(),
                    role: "user".to_string(),
                    text: "latest question".to_string(),
                    projected: false,
                },
                SessionContinuityTurn {
                    message_id: "msg-b".to_string(),
                    role: "assistant".to_string(),
                    text: "answer".to_string(),
                    projected: false,
                },
            ],
            memory_anchors: vec![],
            working_ledger: vec![],
            continuation_dependencies: vec![SessionContinuityDependency {
                kind: SessionContinuityDependencyKind::AssistantToolCallContinuation,
                anchor_message_id: Some("msg-a".to_string()),
                message_ids: vec!["msg-a".to_string(), "msg-b".to_string()],
            }],
            latest_compaction_summary: Some(SessionContinuityCompactionSummary {
                message_id: "msg-compact".to_string(),
                summary: "compacted 5 older turns".to_string(),
            }),
            limits: Some(SessionContinuityLimits {
                recent_tail_messages: 6,
                context_text_chars: 6000,
                turn_text_chars: 1200,
            }),
            recall_policy: Some(
                "use scheduler_context_hydrate for Source Anchors; use scheduler_memory_hydrate for Memory Anchors"
                    .to_string(),
            ),
        }
    }

    #[test]
    fn reflow_context_builds_from_memory_prefetch_only() {
        let packet = sample_retrieval_packet();
        // has_last_prefetch = true independently of memory_prefetch:
        // the diagnostics sidecar can have a persisted packet even
        // when the current turn has a live prefetch too.
        let ctx = PromptReflowContext::build(
            "ses-1",
            Some(&packet),
            None,
            false,
            true,
            None,
        );

        let mem = ctx.memory.expect("memory view should exist");
        assert!(!mem.is_snapshot);
        assert_eq!(mem.query.as_deref(), Some("relevant patterns"));
        assert_eq!(mem.item_count, 2);
        assert_eq!(mem.items.len(), 2);
        // Per-item detail preserved for lossless reminder migration.
        assert_eq!(mem.items[0].title, "Use ArcSwap for config");
        assert_eq!(mem.items[0].summary, "Config reads should be lock-free");
        assert_eq!(mem.items[0].why_recalled, "matches current refactoring task");
        assert_eq!(mem.items[0].evidence_summary.as_deref(), Some("session-42 evidence"));
        assert!(mem.items[0].last_validated_at.is_some());
        assert_eq!(mem.hydrate_record_ids, vec!["rec-1", "rec-2"]);

        assert!(ctx.continuity.is_none());
        assert!(ctx.diagnostics.has_last_prefetch);
        assert!(!ctx.diagnostics.has_continuity_packet);
        assert!(!ctx.diagnostics.has_frozen_snapshot);
    }

    #[test]
    fn reflow_context_builds_from_continuity_packet_only() {
        let packet = sample_continuity_packet();
        let ctx = PromptReflowContext::build(
            "ses-2",
            None,
            Some(&packet),
            false,
            false,
            None,
        );

        assert!(ctx.memory.is_none());

        let cont = ctx.continuity.expect("continuity view should exist");
        assert_eq!(cont.eligible_message_count, 8);
        assert_eq!(cont.exact_recent_tail_count, 3);
        assert_eq!(cont.omitted_older_turns, 5);
        assert!(!cont.hydrate_message_ids.is_empty());
        assert!(cont.has_continuation_dependency);
        assert!(cont.compaction_summary.is_some());
        assert!(cont
            .recall_policy
            .as_deref()
            .is_some_and(|policy| policy.contains("scheduler_context_hydrate")));

        assert!(ctx.diagnostics.has_continuity_packet);
        assert!(!ctx.diagnostics.has_last_prefetch);
    }

    #[test]
    fn reflow_context_handles_memory_and_continuity_together() {
        let mem_packet = sample_retrieval_packet();
        let cont_packet = sample_continuity_packet();
        let ctx = PromptReflowContext::build(
            "ses-3",
            Some(&mem_packet),
            Some(&cont_packet),
            true,
            true,
            Some("degraded".to_string()),
        );

        assert!(ctx.memory.is_some());
        assert!(ctx.continuity.is_some());
        assert!(ctx.diagnostics.has_continuity_packet);
        assert!(ctx.diagnostics.has_frozen_snapshot);
        assert!(ctx.diagnostics.has_last_prefetch);
        assert_eq!(
            ctx.diagnostics.cache_evidence_status.as_deref(),
            Some("degraded")
        );
    }

    #[test]
    fn reflow_context_handles_empty_state() {
        let ctx = PromptReflowContext::build(
            "ses-4",
            None,
            None,
            false,
            false,
            None,
        );

        assert!(ctx.memory.is_none());
        assert!(ctx.continuity.is_none());
        assert!(!ctx.diagnostics.has_continuity_packet);
        assert!(!ctx.diagnostics.has_frozen_snapshot);
        assert!(!ctx.diagnostics.has_last_prefetch);
    }

    #[test]
    fn render_summary_includes_memory_and_continuity_counts() {
        let mem_packet = sample_retrieval_packet();
        let cont_packet = sample_continuity_packet();
        let ctx = PromptReflowContext::build(
            "ses-5",
            Some(&mem_packet),
            Some(&cont_packet),
            false,
            false,
            None,
        );

        let summary = ctx.render_summary();

        assert!(summary.contains("memory: 2 items recalled"));
        assert!(summary.contains("continuity: tail=3 omitted=5"));
        assert!(summary.contains("hydrate_msg_ids="));
        assert!(summary.contains("has_continuation_dep"));
        assert!(summary.contains("continuity_packet=true"));
        // has_last_prefetch was passed as false → sidecar state is absent.
        assert!(summary.contains("last_prefetch=false"));
    }
}
