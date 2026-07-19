use super::composition::{
    format_capability_group_kind, format_capability_group_state, format_capability_member_role,
};
use super::evolution::{format_skill_retirement_reason_kind, format_skill_vitality_state};
use super::relationships::{format_skill_relationship_kind, format_skill_relationship_state};
use crate::util::now_unix_timestamp;
use crate::SkillWriteResult;
use agendao_types::{
    ManagedSkillRecord, SkillArtifactCacheEntry, SkillAuditEvent, SkillAuditKind,
    SkillCapabilityGroup, SkillDistributionRecord, SkillGovernanceTimelineEntry,
    SkillGovernanceTimelineKind, SkillGovernanceTimelineStatus, SkillGuardReport, SkillGuardStatus,
    SkillGuardViolation, SkillHubPolicy, SkillManagedLifecycleRecord, SkillOperationalSnapshot,
    SkillRelationshipEdge, SkillRemoteInstallPlan, SkillSourceIndexSnapshot, SkillSourceRef,
    SkillSyncPlan, SkillVitalityRecord, SkillVitalityState,
};
use serde_json::{json, Value};

pub(super) fn source_index_refresh_audit_event(
    source: &SkillSourceRef,
    actor: &str,
    snapshot: &SkillSourceIndexSnapshot,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-index-refresh-{}-{}",
            created_at,
            source
                .source_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillAuditKind::SourceIndexRefreshed,
        skill_name: None,
        source_id: Some(source.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "source_kind": format!("{:?}", source.source_kind).to_ascii_lowercase(),
            "locator": source.locator.clone(),
            "revision": source.revision.clone(),
            "entry_count": snapshot.entries.len(),
            "updated_at": snapshot.updated_at,
        }),
    }
}

pub(super) fn remote_plan_audit_event(
    kind: SkillAuditKind,
    actor: &str,
    plan: &SkillRemoteInstallPlan,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-remote-plan-{}-{}",
            created_at,
            plan.distribution
                .distribution_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind,
        skill_name: Some(plan.entry.skill_name.clone()),
        source_id: Some(plan.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "distribution_id": plan.distribution.distribution_id,
            "artifact_id": plan.distribution.resolution.artifact.artifact_id,
            "artifact_locator": plan.distribution.resolution.artifact.locator,
            "revision": plan.distribution.release.revision,
            "version": plan.distribution.release.version,
            "action": format!("{:?}", plan.entry.action).to_ascii_lowercase(),
            "reason": plan.entry.reason,
        }),
    }
}

pub(super) fn artifact_evicted_audit_event(
    entry: &SkillArtifactCacheEntry,
    distribution: Option<&SkillDistributionRecord>,
    policy: &SkillHubPolicy,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-artifact-evicted-{}-{}",
            created_at,
            entry
                .artifact
                .artifact_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillAuditKind::ArtifactEvicted,
        skill_name: distribution.map(|record| record.skill_name.clone()),
        source_id: distribution.map(|record| record.source.source_id.clone()),
        actor: "authority:artifact_cache_policy".to_string(),
        created_at,
        payload: json!({
            "artifact_id": entry.artifact.artifact_id,
            "artifact_locator": entry.artifact.locator,
            "cached_at": entry.cached_at,
            "local_path": entry.local_path,
            "extracted_path": entry.extracted_path,
            "previous_status": format!("{:?}", entry.status).to_ascii_lowercase(),
            "retention_seconds": policy.artifact_cache_retention_seconds,
            "reason": "retention_expired",
        }),
    }
}

pub(super) fn lifecycle_transition_audit_event(
    actor: &str,
    previous: Option<&SkillManagedLifecycleRecord>,
    current: &SkillManagedLifecycleRecord,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-lifecycle-{}-{}",
            created_at,
            current
                .distribution_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillAuditKind::LifecycleTransitioned,
        skill_name: Some(current.skill_name.clone()),
        source_id: Some(current.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "distribution_id": current.distribution_id,
            "from_state": previous.map(|entry| format!("{:?}", entry.state).to_ascii_lowercase()),
            "to_state": format!("{:?}", current.state).to_ascii_lowercase(),
            "error": current.error,
        }),
    }
}

pub(super) fn vitality_transition_audit_event(
    snapshot: &SkillOperationalSnapshot,
    previous_state: SkillVitalityState,
    current: &SkillVitalityRecord,
    actor: &str,
) -> SkillAuditEvent {
    let created_at = current.updated_at;
    SkillAuditEvent {
        event_id: format!(
            "skill-vitality-{}-{}",
            created_at,
            snapshot
                .skill_name
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillAuditKind::VitalityTransitioned,
        skill_name: Some(snapshot.skill_name.clone()),
        source_id: snapshot.source_id.clone(),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "from_state": format_skill_vitality_state(previous_state),
            "to_state": format_skill_vitality_state(current.state),
            "reason_kind": format_skill_retirement_reason_kind(current.reason.kind),
            "reason_summary": current.reason.summary,
            "related_skill_name": current.reason.related_skill_name,
        }),
    }
}

pub(super) fn composition_relationship_audit_event(
    relationship: &SkillRelationshipEdge,
    actor: &str,
    state: agendao_types::SkillRelationshipState,
) -> SkillAuditEvent {
    let created_at = relationship.updated_at.unwrap_or_else(now_unix_timestamp);
    let primary_skill_name = relationship
        .preferred_skill_name
        .clone()
        .unwrap_or_else(|| relationship.left_skill_name.clone());
    SkillAuditEvent {
        event_id: format!(
            "skill-composition-relationship-{}-{}-{}-{}",
            created_at,
            relationship
                .left_skill_name
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"),
            relationship
                .right_skill_name
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"),
            format_skill_relationship_kind(relationship.relation_kind)
        ),
        kind: match state {
            agendao_types::SkillRelationshipState::Accepted => {
                SkillAuditKind::CompositionRelationshipAccepted
            }
            agendao_types::SkillRelationshipState::Dismissed => {
                SkillAuditKind::CompositionRelationshipDismissed
            }
            agendao_types::SkillRelationshipState::Observed => {
                SkillAuditKind::CompositionRelationshipAccepted
            }
        },
        skill_name: Some(primary_skill_name),
        source_id: None,
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "relation_kind": format_skill_relationship_kind(relationship.relation_kind),
            "state": format_skill_relationship_state(state),
            "preferred_skill_name": relationship.preferred_skill_name,
            "left_skill_name": relationship.left_skill_name,
            "right_skill_name": relationship.right_skill_name,
            "skill_names": [
                relationship.left_skill_name.clone(),
                relationship.right_skill_name.clone()
            ],
            "score": relationship.score,
            "reasons": relationship.reasons,
        }),
    }
}

pub(super) fn capability_group_activated_audit_event(
    group: &SkillCapabilityGroup,
    actor: &str,
) -> SkillAuditEvent {
    let created_at = group.updated_at.unwrap_or_else(now_unix_timestamp);
    SkillAuditEvent {
        event_id: format!(
            "skill-capability-group-activated-{}-{}",
            created_at,
            group
                .capability_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillAuditKind::CapabilityGroupActivated,
        skill_name: group.canonical_skill_name.clone().or_else(|| {
            group
                .members
                .first()
                .map(|member| member.skill_name.clone())
        }),
        source_id: None,
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "capability_id": group.capability_id,
            "group_kind": format_capability_group_kind(group.group_kind),
            "state": format_capability_group_state(group.state),
            "canonical_skill_name": group.canonical_skill_name,
            "skill_names": group.members.iter().map(|member| member.skill_name.clone()).collect::<Vec<_>>(),
            "member_roles": group.members.iter().map(|member| {
                json!({
                    "skill_name": member.skill_name,
                    "role": format_capability_member_role(member.role),
                })
            }).collect::<Vec<_>>(),
            "reasons": group.reasons,
        }),
    }
}

pub(super) fn capability_group_member_role_updated_audit_event(
    group: &SkillCapabilityGroup,
    skill_name: &str,
    previous_role: Option<agendao_types::SkillCapabilityMemberRole>,
    current_role: agendao_types::SkillCapabilityMemberRole,
    actor: &str,
) -> SkillAuditEvent {
    let created_at = group.updated_at.unwrap_or_else(now_unix_timestamp);
    SkillAuditEvent {
        event_id: format!(
            "skill-capability-group-role-{}-{}-{}",
            created_at,
            group
                .capability_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"),
            skill_name.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillAuditKind::CapabilityGroupMemberRoleUpdated,
        skill_name: Some(skill_name.to_string()),
        source_id: None,
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "capability_id": group.capability_id,
            "group_kind": format_capability_group_kind(group.group_kind),
            "skill_names": group.members.iter().map(|member| member.skill_name.clone()).collect::<Vec<_>>(),
            "target_skill_name": skill_name,
            "previous_role": previous_role.map(format_capability_member_role),
            "current_role": format_capability_member_role(current_role),
            "canonical_skill_name": group.canonical_skill_name,
        }),
    }
}

pub(super) fn capability_group_member_removed_audit_event(
    group: &SkillCapabilityGroup,
    removed_skill_name: &str,
    actor: &str,
) -> SkillAuditEvent {
    let created_at = group.updated_at.unwrap_or_else(now_unix_timestamp);
    SkillAuditEvent {
        event_id: format!(
            "skill-capability-group-remove-{}-{}-{}",
            created_at,
            group
                .capability_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"),
            removed_skill_name.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillAuditKind::CapabilityGroupMemberRemoved,
        skill_name: Some(removed_skill_name.to_string()),
        source_id: None,
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "capability_id": group.capability_id,
            "group_kind": format_capability_group_kind(group.group_kind),
            "skill_names": group.members.iter().map(|member| member.skill_name.clone()).collect::<Vec<_>>(),
            "removed_skill_name": removed_skill_name,
            "remaining_member_count": group.members.len(),
            "canonical_skill_name": group.canonical_skill_name,
        }),
    }
}

pub(super) fn write_audit_event(
    kind: SkillAuditKind,
    actor: &str,
    result: &SkillWriteResult,
    source: Option<&SkillSourceRef>,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-write-{}-{}",
            created_at,
            result
                .skill_name
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind,
        skill_name: Some(result.skill_name.clone()),
        source_id: source.map(|source| source.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "action": format!("{:?}", result.action).to_ascii_lowercase(),
            "location": result.location.to_string_lossy().to_string(),
            "supporting_file": result.supporting_file,
            "category": result.skill.as_ref().and_then(|skill| skill.category.clone()),
        }),
    }
}

pub(super) fn guard_audit_event(
    kind: SkillAuditKind,
    source: Option<&SkillSourceRef>,
    actor: &str,
    report: &SkillGuardReport,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-guard-{}-{}",
            created_at,
            report
                .skill_name
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind,
        skill_name: Some(report.skill_name.clone()),
        source_id: source.map(|source| source.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "status": format!("{:?}", report.status).to_ascii_lowercase(),
            "violation_count": report.violations.len(),
            "violations": report.violations,
        }),
    }
}

pub(super) fn hub_remove_audit_event(
    source: &SkillSourceRef,
    actor: &str,
    record: &ManagedSkillRecord,
    deleted_from_workspace: bool,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-hub-remove-{}-{}",
            created_at,
            record
                .skill_name
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillAuditKind::HubRemove,
        skill_name: Some(record.skill_name.clone()),
        source_id: Some(source.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "deleted_from_workspace": deleted_from_workspace,
            "installed_revision": record.installed_revision,
            "local_hash": record.local_hash,
        }),
    }
}

pub(super) fn hub_detach_audit_event(
    source: &SkillSourceRef,
    actor: &str,
    record: &ManagedSkillRecord,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-hub-detach-{}-{}",
            created_at,
            record
                .skill_name
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillAuditKind::HubDetach,
        skill_name: Some(record.skill_name.clone()),
        source_id: Some(source.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "installed_revision": record.installed_revision,
            "local_hash": record.local_hash,
            "locally_modified": record.locally_modified,
            "deleted_locally": record.deleted_locally,
        }),
    }
}

pub(super) fn sync_audit_event(
    kind: SkillAuditKind,
    source: &SkillSourceRef,
    actor: &str,
    plan: &SkillSyncPlan,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-sync-{}-{}",
            created_at,
            source
                .source_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind,
        skill_name: None,
        source_id: Some(source.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "source_kind": format!("{:?}", source.source_kind).to_ascii_lowercase(),
            "entry_count": plan.entries.len(),
            "entries": plan.entries.iter().map(|entry| {
                json!({
                    "skill_name": entry.skill_name,
                    "action": format!("{:?}", entry.action).to_ascii_lowercase(),
                    "reason": entry.reason,
                })
            }).collect::<Vec<_>>(),
        }),
    }
}

pub(super) fn distribution_audit_event(
    kind: SkillAuditKind,
    actor: &str,
    distribution: &SkillDistributionRecord,
    error: Option<String>,
) -> SkillAuditEvent {
    let created_at = now_unix_timestamp();
    SkillAuditEvent {
        event_id: format!(
            "skill-distribution-{}-{}",
            created_at,
            distribution
                .distribution_id
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind,
        skill_name: Some(distribution.skill_name.clone()),
        source_id: Some(distribution.source.source_id.clone()),
        actor: actor.to_string(),
        created_at,
        payload: json!({
            "distribution_id": distribution.distribution_id,
            "source_kind": format!("{:?}", distribution.source.source_kind).to_ascii_lowercase(),
            "version": distribution.release.version,
            "revision": distribution.release.revision,
            "artifact_id": distribution.resolution.artifact.artifact_id,
            "artifact_locator": distribution.resolution.artifact.locator,
            "error": error,
        }),
    }
}

pub(super) fn managed_record_timeline_entry(record: ManagedSkillRecord) -> SkillGovernanceTimelineEntry {
    let status = if record.deleted_locally || record.locally_modified {
        SkillGovernanceTimelineStatus::Warn
    } else {
        SkillGovernanceTimelineStatus::Success
    };
    let summary = if let Some(source) = record.source.as_ref() {
        format!(
            "{} · revision {} · {}",
            source.source_id,
            record.installed_revision.as_deref().unwrap_or("--"),
            managed_record_state_label(&record)
        )
    } else {
        format!("workspace-local · {}", managed_record_state_label(&record))
    };

    SkillGovernanceTimelineEntry {
        entry_id: format!(
            "managed-{}",
            record
                .skill_name
                .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
        ),
        kind: SkillGovernanceTimelineKind::ManagedSnapshot,
        created_at: record.last_synced_at.unwrap_or_default(),
        skill_name: Some(record.skill_name.clone()),
        source_id: record
            .source
            .as_ref()
            .map(|source| source.source_id.clone()),
        actor: None,
        title: format!("Managed provenance · {}", record.skill_name),
        summary,
        status,
        managed_record: Some(record.clone()),
        guard_report: None,
        payload: json!({
            "installed_revision": record.installed_revision,
            "local_hash": record.local_hash,
            "last_synced_at": record.last_synced_at,
            "locally_modified": record.locally_modified,
            "deleted_locally": record.deleted_locally,
        }),
    }
}

fn managed_record_state_label(record: &ManagedSkillRecord) -> &'static str {
    if record.deleted_locally {
        "deleted locally"
    } else if record.locally_modified {
        "locally modified"
    } else {
        "clean"
    }
}

pub(super) fn audit_event_timeline_entry(
    event: &SkillAuditEvent,
    managed_record: Option<ManagedSkillRecord>,
) -> SkillGovernanceTimelineEntry {
    let guard_report = guard_report_from_audit_event(event);
    SkillGovernanceTimelineEntry {
        entry_id: event.event_id.clone(),
        kind: event.kind.clone().into(),
        created_at: event.created_at,
        skill_name: event.skill_name.clone(),
        source_id: event.source_id.clone(),
        actor: Some(event.actor.clone()),
        title: audit_event_title(event),
        summary: audit_event_summary(event),
        status: audit_event_status(&event.kind),
        managed_record,
        guard_report,
        payload: event.payload.clone(),
    }
}

fn audit_event_title(event: &SkillAuditEvent) -> String {
    match event.kind {
        SkillAuditKind::CompositionRelationshipAccepted => format!(
            "Composition relationship accepted · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::CompositionRelationshipDismissed => format!(
            "Composition relationship dismissed · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::CapabilityGroupActivated => format!(
            "Capability group activated · {}",
            payload_string(&event.payload, "capability_id").unwrap_or_else(|| "group".to_string())
        ),
        SkillAuditKind::CapabilityGroupMemberRoleUpdated => format!(
            "Capability group member updated · {}",
            payload_string(&event.payload, "capability_id").unwrap_or_else(|| "group".to_string())
        ),
        SkillAuditKind::CapabilityGroupMemberRemoved => format!(
            "Capability group member removed · {}",
            payload_string(&event.payload, "capability_id").unwrap_or_else(|| "group".to_string())
        ),
        SkillAuditKind::VitalityTransitioned => format!(
            "Vitality transitioned · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::SourceIndexRefreshed => format!(
            "Source index refreshed · {}",
            event.source_id.as_deref().unwrap_or("source")
        ),
        SkillAuditKind::SourceResolved => format!(
            "Source resolved · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::ArtifactFetched => format!(
            "Artifact fetched · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::ArtifactEvicted => format!(
            "Artifact evicted · {}",
            event
                .skill_name
                .as_deref()
                .or(event.source_id.as_deref())
                .unwrap_or("artifact")
        ),
        SkillAuditKind::ArtifactFetchFailed => format!(
            "Artifact fetch failed · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::RemoteInstallPlanned => format!(
            "Remote install planned · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::RemoteUpdatePlanned => format!(
            "Remote update planned · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::LifecycleTransitioned => format!(
            "Lifecycle transitioned · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::Create => format!(
            "Workspace create · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::Patch => format!(
            "Workspace patch · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::Edit => format!(
            "Workspace edit · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::Delete => format!(
            "Workspace delete · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::WriteFile => format!(
            "Supporting file write · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::RemoveFile => format!(
            "Supporting file remove · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::HubInstall => format!(
            "Hub install · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::HubUpdate => format!(
            "Hub update · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::HubDetach => format!(
            "Hub detach · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::HubRemove => format!(
            "Hub remove · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::SyncPlanCreated => format!(
            "Sync plan created · {}",
            event.source_id.as_deref().unwrap_or("source")
        ),
        SkillAuditKind::SyncApplyCompleted => format!(
            "Sync apply completed · {}",
            event.source_id.as_deref().unwrap_or("source")
        ),
        SkillAuditKind::GuardBlocked => format!(
            "Guard blocked · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
        SkillAuditKind::GuardWarned => format!(
            "Guard warned · {}",
            event.skill_name.as_deref().unwrap_or("skill")
        ),
    }
}

fn audit_event_summary(event: &SkillAuditEvent) -> String {
    match event.kind {
        SkillAuditKind::CompositionRelationshipAccepted
        | SkillAuditKind::CompositionRelationshipDismissed => {
            let relation_kind = payload_string(&event.payload, "relation_kind")
                .unwrap_or_else(|| "relationship".to_string());
            let left_skill_name = payload_string(&event.payload, "left_skill_name")
                .unwrap_or_else(|| "left".to_string());
            let right_skill_name = payload_string(&event.payload, "right_skill_name")
                .unwrap_or_else(|| "right".to_string());
            let preferred_skill_name = payload_string(&event.payload, "preferred_skill_name");
            match preferred_skill_name {
                Some(preferred_skill_name) => format!(
                    "{relation_kind} · {left_skill_name} <-> {right_skill_name} · preferred {preferred_skill_name}"
                ),
                None => format!("{relation_kind} · {left_skill_name} <-> {right_skill_name}"),
            }
        }
        SkillAuditKind::CapabilityGroupActivated => format!(
            "{} · {} member(s){}",
            payload_string(&event.payload, "group_kind").unwrap_or_else(|| "group".to_string()),
            payload_skill_names(&event.payload).len(),
            payload_string(&event.payload, "canonical_skill_name")
                .map(|canonical| format!(" · canonical {canonical}"))
                .unwrap_or_default()
        ),
        SkillAuditKind::CapabilityGroupMemberRoleUpdated => format!(
            "{} -> {} · {}",
            payload_string(&event.payload, "previous_role").unwrap_or_else(|| "none".to_string()),
            payload_string(&event.payload, "current_role").unwrap_or_else(|| "role".to_string()),
            payload_string(&event.payload, "target_skill_name")
                .unwrap_or_else(|| "member".to_string())
        ),
        SkillAuditKind::CapabilityGroupMemberRemoved => format!(
            "{} removed · {} remaining",
            payload_string(&event.payload, "removed_skill_name")
                .unwrap_or_else(|| "member".to_string()),
            payload_usize(&event.payload, "remaining_member_count").unwrap_or_default()
        ),
        SkillAuditKind::VitalityTransitioned => format!(
            "{} -> {} · {}",
            payload_string(&event.payload, "from_state").unwrap_or_else(|| "active".to_string()),
            payload_string(&event.payload, "to_state").unwrap_or_else(|| "active".to_string()),
            payload_string(&event.payload, "reason_summary")
                .unwrap_or_else(|| "vitality change".to_string())
        ),
        SkillAuditKind::SourceIndexRefreshed => {
            let entry_count = payload_usize(&event.payload, "entry_count").unwrap_or_default();
            let source_kind =
                payload_string(&event.payload, "source_kind").unwrap_or_else(|| "source".into());
            let locator =
                payload_string(&event.payload, "locator").unwrap_or_else(|| "locator".into());
            format!("{entry_count} entries · {source_kind} · {locator}")
        }
        SkillAuditKind::SourceResolved => format!(
            "{} · distribution {}",
            payload_string(&event.payload, "revision")
                .or_else(|| payload_string(&event.payload, "version"))
                .unwrap_or_else(|| "unversioned".to_string()),
            payload_string(&event.payload, "distribution_id")
                .unwrap_or_else(|| "distribution".to_string())
        ),
        SkillAuditKind::ArtifactFetched => payload_string(&event.payload, "artifact_locator")
                .unwrap_or_else(|| "artifact cached".to_string()).to_string(),
        SkillAuditKind::ArtifactEvicted => format!(
            "{} · retention {}s",
            payload_string(&event.payload, "artifact_locator")
                .unwrap_or_else(|| "artifact cache entry".to_string()),
            payload_usize(&event.payload, "retention_seconds").unwrap_or_default()
        ),
        SkillAuditKind::ArtifactFetchFailed => payload_string(&event.payload, "error")
                .unwrap_or_else(|| "artifact fetch failed".to_string()).to_string(),
        SkillAuditKind::RemoteInstallPlanned | SkillAuditKind::RemoteUpdatePlanned => format!(
            "{} · {}",
            payload_string(&event.payload, "action").unwrap_or_else(|| "plan".to_string()),
            payload_string(&event.payload, "reason").unwrap_or_else(|| "remote plan".to_string())
        ),
        SkillAuditKind::LifecycleTransitioned => format!(
            "{} -> {}",
            payload_string(&event.payload, "from_state").unwrap_or_else(|| "none".to_string()),
            payload_string(&event.payload, "to_state").unwrap_or_else(|| "unknown".to_string())
        ),
        SkillAuditKind::Create
        | SkillAuditKind::Patch
        | SkillAuditKind::Edit
        | SkillAuditKind::Delete => payload_string(&event.payload, "location").unwrap_or_else(|| "workspace write".into()).to_string(),
        SkillAuditKind::WriteFile | SkillAuditKind::RemoveFile => {
            let file_path = payload_string(&event.payload, "supporting_file")
                .unwrap_or_else(|| "supporting file".to_string());
            format!(
                "{} · {}",
                event.skill_name.as_deref().unwrap_or("skill"),
                file_path
            )
        }
        SkillAuditKind::HubInstall | SkillAuditKind::HubUpdate => format!(
            "{} · {}",
            event.source_id.as_deref().unwrap_or("source"),
            payload_string(&event.payload, "location").unwrap_or_else(|| "workspace import".into())
        ),
        SkillAuditKind::HubDetach => format!(
            "{} · workspace content preserved",
            event.source_id.as_deref().unwrap_or("source")
        ),
        SkillAuditKind::HubRemove => format!(
            "{} · deleted_from_workspace={}",
            event.source_id.as_deref().unwrap_or("source"),
            payload_bool(&event.payload, "deleted_from_workspace").unwrap_or(false)
        ),
        SkillAuditKind::SyncPlanCreated | SkillAuditKind::SyncApplyCompleted => format!(
            "{} entries · {}",
            payload_usize(&event.payload, "entry_count").unwrap_or_default(),
            event.source_id.as_deref().unwrap_or("source")
        ),
        SkillAuditKind::GuardBlocked | SkillAuditKind::GuardWarned => {
            let violation_count = payload_usize(&event.payload, "violation_count").unwrap_or(0);
            let first_rule = payload_first_guard_rule(&event.payload);
            if let Some(first_rule) = first_rule {
                format!("{violation_count} violations · first rule {first_rule}")
            } else {
                format!("{violation_count} violations")
            }
        }
    }
}

fn audit_event_status(kind: &SkillAuditKind) -> SkillGovernanceTimelineStatus {
    match kind {
        SkillAuditKind::CompositionRelationshipDismissed => SkillGovernanceTimelineStatus::Info,
        SkillAuditKind::CompositionRelationshipAccepted
        | SkillAuditKind::CapabilityGroupActivated
        | SkillAuditKind::CapabilityGroupMemberRoleUpdated
        | SkillAuditKind::CapabilityGroupMemberRemoved => SkillGovernanceTimelineStatus::Success,
        SkillAuditKind::VitalityTransitioned => SkillGovernanceTimelineStatus::Warn,
        SkillAuditKind::ArtifactEvicted => SkillGovernanceTimelineStatus::Info,
        SkillAuditKind::ArtifactFetchFailed => SkillGovernanceTimelineStatus::Error,
        SkillAuditKind::GuardBlocked => SkillGovernanceTimelineStatus::Error,
        SkillAuditKind::GuardWarned => SkillGovernanceTimelineStatus::Warn,
        SkillAuditKind::SyncPlanCreated => SkillGovernanceTimelineStatus::Info,
        SkillAuditKind::RemoteInstallPlanned
        | SkillAuditKind::RemoteUpdatePlanned
        | SkillAuditKind::LifecycleTransitioned => SkillGovernanceTimelineStatus::Info,
        SkillAuditKind::HubDetach => SkillGovernanceTimelineStatus::Info,
        SkillAuditKind::HubRemove => SkillGovernanceTimelineStatus::Info,
        SkillAuditKind::SourceIndexRefreshed
        | SkillAuditKind::SourceResolved
        | SkillAuditKind::ArtifactFetched => SkillGovernanceTimelineStatus::Success,
        SkillAuditKind::Create
        | SkillAuditKind::Patch
        | SkillAuditKind::Edit
        | SkillAuditKind::Delete
        | SkillAuditKind::WriteFile
        | SkillAuditKind::RemoveFile
        | SkillAuditKind::HubInstall
        | SkillAuditKind::HubUpdate
        | SkillAuditKind::SyncApplyCompleted => SkillGovernanceTimelineStatus::Success,
    }
}

fn guard_report_from_audit_event(event: &SkillAuditEvent) -> Option<SkillGuardReport> {
    if !matches!(
        event.kind,
        SkillAuditKind::GuardBlocked | SkillAuditKind::GuardWarned
    ) {
        return None;
    }

    let skill_name = event.skill_name.clone()?;
    let status = match payload_string(&event.payload, "status").as_deref() {
        Some("passed") => SkillGuardStatus::Passed,
        Some("blocked") => SkillGuardStatus::Blocked,
        _ => SkillGuardStatus::Warn,
    };
    let violations = event
        .payload
        .get("violations")
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<SkillGuardViolation>>(value).ok())
        .unwrap_or_default();

    Some(SkillGuardReport {
        skill_name,
        status,
        violations,
        scanned_at: event.created_at,
    })
}

fn payload_string(payload: &Value, key: &str) -> Option<String> {
    payload.get(key)?.as_str().map(|value| value.to_string())
}

fn payload_bool(payload: &Value, key: &str) -> Option<bool> {
    payload.get(key)?.as_bool()
}

fn payload_usize(payload: &Value, key: &str) -> Option<usize> {
    payload.get(key)?.as_u64().map(|value| value as usize)
}

pub(super) fn payload_skill_names(payload: &Value) -> Vec<String> {
    payload
        .get("skill_names")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(|value| value.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn payload_first_guard_rule(payload: &Value) -> Option<String> {
    let violations = payload.get("violations")?.as_array()?;
    violations
        .first()?
        .get("rule_id")?
        .as_str()
        .map(|value| value.to_string())
}
