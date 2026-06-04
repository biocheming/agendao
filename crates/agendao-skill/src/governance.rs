use crate::{
    CreateSkillRequest, DeleteSkillRequest, EditSkillRequest, PatchSkillRequest,
    RemoveSkillFileRequest, RuntimeInstructionSource, RuntimeSkillBootstrapReport,
    RuntimeSkillMaterialization, RuntimeSkillMaterializationAction, RuntimeSkillSourceKind,
    SkillArtifactStore, SkillAuthority, SkillConditions, SkillDetailView,
    SkillDistributionResolver, SkillError, SkillGuardEngine, SkillHubSnapshot, SkillHubStore,
    SkillLifecycleCoordinator, SkillSyncPlanner, SkillWriteAction, SkillWriteResult,
    WriteSkillFileRequest,
};
use agendao_config::ConfigStore;
use agendao_types::{
    BundledSkillManifest, ManagedSkillRecord, SkillArtifactCacheEntry, SkillArtifactCacheStatus,
    SkillAuditEvent, SkillAuditKind, SkillCapabilityGroup, SkillCapabilityGroupKind,
    SkillCapabilityMember, SkillCapabilityMemberRole, SkillDistributionRecord,
    SkillEvolutionEvidenceSummary, SkillGovernanceDiagnosticSeverity, SkillGovernanceTimelineEntry,
    SkillGovernanceTimelineKind, SkillGovernanceTimelineStatus, SkillGovernanceWriteResult,
    SkillGuardReport, SkillGuardSeverity, SkillGuardStatus, SkillGuardViolation,
    SkillHubManagedDetachResponse, SkillHubManagedRemoveResponse, SkillHubPolicy,
    SkillHubSearchMatch, SkillHubSearchRequest, SkillHubSearchResponse, SkillHubTimelineQuery,
    SkillManagedLifecycleRecord, SkillManagedLifecycleState, SkillNegativeEntropyDiagnostic,
    SkillNegativeEntropySignal, SkillOperationalSnapshot, SkillOperationalSourceScope,
    SkillRelationshipEdge, SkillRelationshipKind, SkillRelationshipState, SkillRemoteInstallAction,
    SkillRemoteInstallEntry, SkillRemoteInstallPlan, SkillRemoteInstallResponse,
    SkillRetirementReason, SkillRetirementReasonKind, SkillRuntimeCompositionHint,
    SkillRuntimeCompositionHintKind, SkillSemanticConflictDiagnostic, SkillSemanticConflictKind,
    SkillSourceIndexEntry, SkillSourceIndexSnapshot, SkillSourceRef, SkillSyncAction,
    SkillSyncPlan, SkillUsageLedgerEntry, SkillVitalityRecord, SkillVitalityState,
    SkillWriteLedgerAction, SkillWriteLedgerEntry,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillGovernedWriteResult {
    #[serde(flatten)]
    pub result: SkillWriteResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_report: Option<SkillGuardReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillGovernedSyncResult {
    pub plan: SkillSyncPlan,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guard_reports: Vec<SkillGuardReport>,
}

const DEFAULT_INDEX_FRESHNESS_MAX_AGE_SECONDS: u64 = 604_800; // 7 days

#[derive(Clone)]
pub struct SkillGovernanceAuthority {
    skill_authority: SkillAuthority,
    hub_store: Arc<SkillHubStore>,
    sync_planner: Arc<SkillSyncPlanner>,
    guard_engine: Arc<SkillGuardEngine>,
    distribution_resolver: Arc<SkillDistributionResolver>,
    artifact_store: Arc<SkillArtifactStore>,
    lifecycle: Arc<SkillLifecycleCoordinator>,
    config_store: Option<Arc<ConfigStore>>,
}

#[derive(Debug, Clone, Default)]
struct SkillCompositionConsumptionContext {
    canonical_skill_name: Option<String>,
    canonical_family_id: Option<String>,
    family_member_role: Option<SkillCapabilityMemberRole>,
    complementary_group_ids: Vec<String>,
    complementary_peer_skill_names: Vec<String>,
}

impl SkillCompositionConsumptionContext {
    fn complementary_protected(&self) -> bool {
        !self.complementary_group_ids.is_empty() || !self.complementary_peer_skill_names.is_empty()
    }

    fn related_skill_name_for_review(&self, skill_name: &str) -> Option<String> {
        let canonical = self.canonical_skill_name.as_deref()?;
        (!canonical.eq_ignore_ascii_case(skill_name)).then(|| canonical.to_string())
    }
}

impl SkillGovernanceAuthority {
    pub fn new(base_dir: impl Into<PathBuf>, config_store: Option<Arc<ConfigStore>>) -> Self {
        let base_dir = base_dir.into();
        Self {
            skill_authority: SkillAuthority::new(base_dir.clone(), config_store.clone()),
            hub_store: Arc::new(SkillHubStore::new(base_dir.clone())),
            sync_planner: Arc::new(SkillSyncPlanner::new()),
            guard_engine: Arc::new(SkillGuardEngine::new()),
            distribution_resolver: Arc::new(SkillDistributionResolver::new()),
            artifact_store: Arc::new(SkillArtifactStore::new(
                base_dir.clone(),
                config_store.clone(),
            )),
            lifecycle: Arc::new(SkillLifecycleCoordinator::new()),
            config_store,
        }
    }

    pub fn skill_authority(&self) -> &SkillAuthority {
        &self.skill_authority
    }

    pub fn hub_store(&self) -> Arc<SkillHubStore> {
        Arc::clone(&self.hub_store)
    }

    pub fn sync_planner(&self) -> Arc<SkillSyncPlanner> {
        Arc::clone(&self.sync_planner)
    }

    pub fn guard_engine(&self) -> Arc<SkillGuardEngine> {
        Arc::clone(&self.guard_engine)
    }

    pub fn distribution_resolver(&self) -> Arc<SkillDistributionResolver> {
        Arc::clone(&self.distribution_resolver)
    }

    pub fn artifact_store(&self) -> Arc<SkillArtifactStore> {
        Arc::clone(&self.artifact_store)
    }

    pub fn lifecycle(&self) -> Arc<SkillLifecycleCoordinator> {
        Arc::clone(&self.lifecycle)
    }

    pub fn governance_snapshot(&self) -> SkillHubSnapshot {
        self.hub_store.snapshot()
    }

    pub fn managed_skills(&self) -> Vec<ManagedSkillRecord> {
        self.hub_store.managed_skills()
    }

    pub fn skill_operational_snapshots(&self) -> Vec<SkillOperationalSnapshot> {
        self.hub_store.skill_operational_snapshots()
    }

    pub fn skill_composition_relationships(&self) -> Vec<SkillRelationshipEdge> {
        self.hub_store.composition_relationships()
    }

    pub fn skill_capability_groups(&self) -> Vec<SkillCapabilityGroup> {
        self.hub_store.capability_groups()
    }

    pub fn skill_composition_proposal_target(&self, skill_name: &str) -> Option<String> {
        self.skill_composition_consumption_context(skill_name)
            .related_skill_name_for_review(skill_name)
    }

    pub fn runtime_skill_composition_hints(
        &self,
        selected_skill_names: &[String],
    ) -> Vec<SkillRuntimeCompositionHint> {
        let selected = normalize_runtime_selected_skill_names(selected_skill_names);
        if selected.is_empty() {
            return Vec::new();
        }

        let selected_keys = selected
            .iter()
            .map(|skill_name| normalize_name(skill_name))
            .collect::<BTreeSet<_>>();
        let mut hints = Vec::new();
        let mut seen_prefer = BTreeSet::new();
        for skill_name in &selected {
            let context = self.skill_composition_consumption_context(skill_name);
            let Some(preferred_skill_name) = context.related_skill_name_for_review(skill_name)
            else {
                continue;
            };
            let identity = (
                normalize_name(skill_name),
                normalize_name(&preferred_skill_name),
                context.canonical_family_id.clone().unwrap_or_default(),
            );
            if !seen_prefer.insert(identity) {
                continue;
            }
            hints.push(SkillRuntimeCompositionHint {
                kind: SkillRuntimeCompositionHintKind::PreferCanonicalSkill,
                skill_names: vec![skill_name.clone()],
                preferred_skill_name: Some(preferred_skill_name.clone()),
                capability_id: context.canonical_family_id.clone(),
                summary: format_runtime_prefer_canonical_hint(
                    skill_name,
                    &preferred_skill_name,
                    context.family_member_role,
                    context.canonical_family_id.as_deref(),
                ),
            });
        }

        let mut seen_bundle = BTreeSet::new();
        for group in self.skill_capability_groups().into_iter().filter(|group| {
            group.state == agendao_types::SkillCapabilityGroupState::Active
                && group.group_kind == SkillCapabilityGroupKind::ComplementaryBundle
        }) {
            let selected_members = group
                .members
                .iter()
                .filter(|member| selected_keys.contains(&normalize_name(&member.skill_name)))
                .map(|member| member.skill_name.clone())
                .collect::<Vec<_>>();
            if selected_members.len() < 2 {
                continue;
            }
            let identity = normalize_name(&group.capability_id);
            if !seen_bundle.insert(identity) {
                continue;
            }
            hints.push(SkillRuntimeCompositionHint {
                kind: SkillRuntimeCompositionHintKind::ComplementaryBundle,
                skill_names: selected_members.clone(),
                preferred_skill_name: None,
                capability_id: Some(group.capability_id.clone()),
                summary: format_runtime_complementary_bundle_hint(
                    &selected_members,
                    Some(group.capability_id.as_str()),
                ),
            });
        }

        let mut seen_pair = BTreeSet::new();
        for relationship in
            self.skill_composition_relationships()
                .into_iter()
                .filter(|relationship| {
                    relationship.state == SkillRelationshipState::Accepted
                        && relationship.relation_kind
                            == SkillRelationshipKind::ComplementaryComponent
                })
        {
            let left_key = normalize_name(&relationship.left_skill_name);
            let right_key = normalize_name(&relationship.right_skill_name);
            if !selected_keys.contains(&left_key) || !selected_keys.contains(&right_key) {
                continue;
            }
            let identity = relationship_pair_key(
                &relationship.left_skill_name,
                &relationship.right_skill_name,
            );
            if !seen_pair.insert(identity) {
                continue;
            }
            let skill_names = ordered_skill_names(
                &relationship.left_skill_name,
                &relationship.right_skill_name,
            );
            if hints.iter().any(|hint| {
                hint.kind == SkillRuntimeCompositionHintKind::ComplementaryBundle
                    && skill_names_includes_pair(&hint.skill_names, &skill_names.0, &skill_names.1)
            }) {
                continue;
            }
            hints.push(SkillRuntimeCompositionHint {
                kind: SkillRuntimeCompositionHintKind::ComplementaryBundle,
                skill_names: vec![skill_names.0.clone(), skill_names.1.clone()],
                preferred_skill_name: None,
                capability_id: None,
                summary: format_runtime_complementary_bundle_hint(
                    &[skill_names.0, skill_names.1],
                    None,
                ),
            });
        }

        sort_runtime_composition_hints(&mut hints);
        hints
    }

    pub fn skill_composition_relationship_inspection(
        &self,
    ) -> Result<Vec<SkillRelationshipEdge>, SkillError> {
        let mut candidate_by_key = self
            .skill_composition_relationship_candidates()?
            .into_iter()
            .map(|relationship| (relationship_edge_identity_key(&relationship), relationship))
            .collect::<BTreeMap<_, _>>();
        let mut merged = Vec::new();

        for stored in self.skill_composition_relationships() {
            let key = relationship_edge_identity_key(&stored);
            if let Some(candidate) = candidate_by_key.remove(&key) {
                merged.push(merge_relationship_inspection_entry(&stored, &candidate));
            } else {
                merged.push(stored);
            }
        }

        merged.extend(candidate_by_key.into_values());
        sort_skill_relationship_edges(&mut merged);
        Ok(merged)
    }

    pub fn skill_capability_group_inspection(
        &self,
    ) -> Result<Vec<SkillCapabilityGroup>, SkillError> {
        let mut candidate_by_id = self
            .skill_capability_group_candidates()?
            .into_iter()
            .map(|group| (normalize_name(&group.capability_id), group))
            .collect::<BTreeMap<_, _>>();
        let mut merged = Vec::new();

        for stored in self.skill_capability_groups() {
            let key = normalize_name(&stored.capability_id);
            if let Some(candidate) = candidate_by_id.remove(&key) {
                merged.push(merge_capability_group_inspection_entry(&stored, &candidate));
            } else {
                merged.push(stored);
            }
        }

        merged.extend(candidate_by_id.into_values());
        sort_skill_capability_groups(&mut merged);
        Ok(merged)
    }

    pub fn skill_composition_relationship_candidates(
        &self,
    ) -> Result<Vec<SkillRelationshipEdge>, SkillError> {
        let (descriptors, snapshot_by_name) = self.skill_semantic_analysis_inputs()?;
        let descriptor_by_name = descriptors
            .iter()
            .cloned()
            .map(|descriptor| (descriptor.normalized_name.clone(), descriptor))
            .collect::<BTreeMap<_, _>>();
        let conflicts = collect_skill_semantic_conflicts(&descriptors, &snapshot_by_name);
        let conflict_by_pair = conflicts
            .iter()
            .cloned()
            .map(|conflict| {
                (
                    relationship_pair_key(&conflict.left_skill_name, &conflict.right_skill_name),
                    conflict,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut edges = BTreeMap::new();

        for conflict in &conflicts {
            let pair_key =
                relationship_pair_key(&conflict.left_skill_name, &conflict.right_skill_name);
            let Some(left) = descriptor_by_name.get(&normalize_name(&conflict.left_skill_name))
            else {
                continue;
            };
            let Some(right) = descriptor_by_name.get(&normalize_name(&conflict.right_skill_name))
            else {
                continue;
            };

            if let Some(edge) =
                build_skill_specialization_relationship_candidate(left, right, conflict)
            {
                edges.insert(pair_key, edge);
                continue;
            }
            if let Some(edge) = build_skill_redundant_relationship_candidate(conflict) {
                edges.insert(pair_key, edge);
            }
        }

        for left_index in 0..descriptors.len() {
            for right_index in (left_index + 1)..descriptors.len() {
                let left = &descriptors[left_index];
                let right = &descriptors[right_index];
                let pair_key = relationship_pair_key(&left.skill_name, &right.skill_name);
                if edges.contains_key(&pair_key) {
                    continue;
                }
                let Some(edge) = build_skill_complementary_relationship_candidate(
                    left,
                    right,
                    conflict_by_pair.get(&pair_key),
                    snapshot_by_name.get(&normalize_name(&left.skill_name)),
                    snapshot_by_name.get(&normalize_name(&right.skill_name)),
                ) else {
                    continue;
                };
                edges.insert(pair_key, edge);
            }
        }

        let mut edges = edges.into_values().collect::<Vec<_>>();
        sort_skill_relationship_edges(&mut edges);
        Ok(edges)
    }

    pub fn skill_capability_group_candidates(
        &self,
    ) -> Result<Vec<SkillCapabilityGroup>, SkillError> {
        let relationships = self.skill_composition_relationship_candidates()?;
        Ok(build_skill_capability_group_candidates(&relationships))
    }

    pub fn accept_skill_composition_relationship(
        &self,
        left_skill_name: &str,
        right_skill_name: &str,
        relation_kind: agendao_types::SkillRelationshipKind,
        preferred_skill_name: Option<&str>,
        actor: &str,
    ) -> Result<SkillRelationshipEdge, SkillError> {
        self.set_skill_composition_relationship_state(
            left_skill_name,
            right_skill_name,
            relation_kind,
            agendao_types::SkillRelationshipState::Accepted,
            preferred_skill_name,
            actor,
        )
    }

    pub fn dismiss_skill_composition_relationship(
        &self,
        left_skill_name: &str,
        right_skill_name: &str,
        relation_kind: agendao_types::SkillRelationshipKind,
        actor: &str,
    ) -> Result<SkillRelationshipEdge, SkillError> {
        self.set_skill_composition_relationship_state(
            left_skill_name,
            right_skill_name,
            relation_kind,
            agendao_types::SkillRelationshipState::Dismissed,
            None,
            actor,
        )
    }

    pub fn activate_skill_capability_group(
        &self,
        capability_id: Option<&str>,
        group_kind: agendao_types::SkillCapabilityGroupKind,
        canonical_skill_name: Option<&str>,
        members: Vec<SkillCapabilityMember>,
        reasons: Vec<String>,
        actor: &str,
    ) -> Result<SkillCapabilityGroup, SkillError> {
        let candidate_lookup_id = capability_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let candidate_group = if let Some(capability_id) = candidate_lookup_id.as_deref() {
            self.skill_capability_group_candidates()?
                .into_iter()
                .find(|group| normalize_name(&group.capability_id) == normalize_name(capability_id))
        } else {
            None
        };

        let mut group = validate_capability_group_input(
            capability_id,
            group_kind,
            canonical_skill_name,
            members,
            reasons,
            candidate_group.as_ref(),
            |skill_name| self.resolve_composition_skill_name(skill_name),
        )?;
        group.state = agendao_types::SkillCapabilityGroupState::Active;
        group.updated_at = Some(now_unix_timestamp());

        self.upsert_capability_group(group.clone())?;
        self.append_audit_event(capability_group_activated_audit_event(&group, actor))?;
        Ok(group)
    }

    pub fn set_skill_capability_group_member_role(
        &self,
        capability_id: &str,
        skill_name: &str,
        role: agendao_types::SkillCapabilityMemberRole,
        actor: &str,
    ) -> Result<SkillCapabilityGroup, SkillError> {
        let capability_id = required_nonempty_text(capability_id, "capability_id")?;
        let resolved_skill_name = self.resolve_composition_skill_name(skill_name)?;
        let mut group = self.existing_capability_group(&capability_id)?;
        validate_capability_group_member_role_update(&group, role)?;

        if let Some(existing) = group
            .members
            .iter()
            .find(|member| member.skill_name.eq_ignore_ascii_case(&resolved_skill_name))
        {
            if existing.role == role {
                return Ok(group);
            }
        }

        let previous_role = group
            .members
            .iter()
            .find(|member| member.skill_name.eq_ignore_ascii_case(&resolved_skill_name))
            .map(|member| member.role);
        if let Some(existing) = group
            .members
            .iter_mut()
            .find(|member| member.skill_name.eq_ignore_ascii_case(&resolved_skill_name))
        {
            existing.role = role;
        } else {
            group.members.push(SkillCapabilityMember {
                skill_name: resolved_skill_name.clone(),
                role,
            });
        }

        sort_skill_capability_members(&mut group.members);
        group.updated_at = Some(now_unix_timestamp());
        self.upsert_capability_group(group.clone())?;
        self.append_audit_event(capability_group_member_role_updated_audit_event(
            &group,
            &resolved_skill_name,
            previous_role,
            role,
            actor,
        ))?;
        Ok(group)
    }

    pub fn remove_skill_capability_group_member(
        &self,
        capability_id: &str,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillCapabilityGroup, SkillError> {
        let capability_id = required_nonempty_text(capability_id, "capability_id")?;
        let resolved_skill_name = self.resolve_composition_skill_name(skill_name)?;
        let mut group = self.existing_capability_group(&capability_id)?;
        let remove_index = group
            .members
            .iter()
            .position(|member| member.skill_name.eq_ignore_ascii_case(&resolved_skill_name))
            .ok_or_else(|| SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is not a member of capability group `{}`",
                    resolved_skill_name, capability_id
                ),
            })?;
        if group.members[remove_index].role == agendao_types::SkillCapabilityMemberRole::Canonical
            || group
                .canonical_skill_name
                .as_deref()
                .map(|value| value.eq_ignore_ascii_case(&resolved_skill_name))
                .unwrap_or(false)
        {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "cannot remove canonical member `{}` from capability group `{}`",
                    resolved_skill_name, capability_id
                ),
            });
        }

        group.members.remove(remove_index);
        if group.members.len() < 2 {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "removing `{}` would collapse capability group `{}` below 2 members",
                    resolved_skill_name, capability_id
                ),
            });
        }

        sort_skill_capability_members(&mut group.members);
        group.updated_at = Some(now_unix_timestamp());
        self.upsert_capability_group(group.clone())?;
        self.append_audit_event(capability_group_member_removed_audit_event(
            &group,
            &resolved_skill_name,
            actor,
        ))?;
        Ok(group)
    }

    pub fn effective_skill_vitality_state(&self, skill_name: &str) -> SkillVitalityState {
        self.hub_store
            .skill_operational_snapshot(skill_name)
            .as_ref()
            .and_then(|snapshot| snapshot.vitality.as_ref())
            .map(|record| record.state)
            .unwrap_or(SkillVitalityState::Active)
    }

    pub fn ensure_skill_runtime_available(&self, skill_name: &str) -> Result<(), SkillError> {
        let state = self.effective_skill_vitality_state(skill_name);
        match state {
            SkillVitalityState::Retired | SkillVitalityState::Archived => {
                Err(SkillError::SkillRuntimeUnavailable {
                    name: skill_name.trim().to_string(),
                    state,
                })
            }
            SkillVitalityState::Active | SkillVitalityState::ReviewCandidate => Ok(()),
        }
    }

    pub fn sync_negative_entropy_review_candidates(
        &self,
        actor: &str,
    ) -> Result<Vec<SkillOperationalSnapshot>, SkillError> {
        let diagnostics = self.skill_negative_entropy_diagnostics()?;
        let mut updated = Vec::new();
        for diagnostic in diagnostics.into_iter().filter(|entry| {
            entry.source_scope == SkillOperationalSourceScope::WorkspaceLocal
                && entry.severity == SkillGovernanceDiagnosticSeverity::Warn
        }) {
            let current = self
                .hub_store
                .skill_operational_snapshot(&diagnostic.skill_name);
            if !should_sync_negative_entropy_review_candidate(current.as_ref()) {
                continue;
            }

            let composition_context =
                self.skill_composition_consumption_context(&diagnostic.skill_name);
            let reason = SkillRetirementReason {
                kind: SkillRetirementReasonKind::NegativeEntropy,
                summary: negative_entropy_review_candidate_summary(
                    &diagnostic,
                    &composition_context,
                ),
                noted_at: now_unix_timestamp(),
                related_skill_name: composition_context
                    .related_skill_name_for_review(&diagnostic.skill_name),
            };
            updated.push(self.set_skill_vitality_state(
                &diagnostic.skill_name,
                SkillVitalityState::ReviewCandidate,
                reason,
                actor,
            )?);
        }
        Ok(updated)
    }

    pub fn sync_semantic_conflict_review_candidates(
        &self,
        actor: &str,
    ) -> Result<Vec<SkillOperationalSnapshot>, SkillError> {
        let diagnostics = self.skill_semantic_conflict_diagnostics()?;
        let mut updated = Vec::new();
        let mut seen_redundant = BTreeSet::new();
        for conflict in diagnostics
            .into_iter()
            .filter(semantic_conflict_is_review_candidate)
        {
            let Some(preferred_skill_name) = conflict.preferred_skill_name.clone() else {
                continue;
            };
            let Some(redundant_skill_name) =
                semantic_conflict_redundant_skill_name(&conflict, &preferred_skill_name)
            else {
                continue;
            };
            if !seen_redundant.insert(normalize_name(&redundant_skill_name)) {
                continue;
            }

            let current = self.prepare_operational_snapshot(
                &redundant_skill_name,
                None,
                SkillOperationalSourceScope::Unknown,
            )?;
            if current.source_scope != SkillOperationalSourceScope::WorkspaceLocal {
                continue;
            }
            if !should_sync_semantic_conflict_review_candidate(
                Some(&current),
                &preferred_skill_name,
            ) {
                continue;
            }

            let reason = SkillRetirementReason {
                kind: SkillRetirementReasonKind::SemanticConflict,
                summary: semantic_conflict_review_candidate_summary(
                    &conflict,
                    &preferred_skill_name,
                ),
                noted_at: now_unix_timestamp(),
                related_skill_name: Some(preferred_skill_name.clone()),
            };
            updated.push(self.set_skill_vitality_state(
                &redundant_skill_name,
                SkillVitalityState::ReviewCandidate,
                reason,
                actor,
            )?);
        }
        Ok(updated)
    }

    pub fn set_skill_vitality_state(
        &self,
        skill_name: &str,
        state: SkillVitalityState,
        reason: SkillRetirementReason,
        actor: &str,
    ) -> Result<SkillOperationalSnapshot, SkillError> {
        let mut snapshot = self.prepare_operational_snapshot(
            skill_name,
            None,
            SkillOperationalSourceScope::Unknown,
        )?;
        if snapshot.source_scope != SkillOperationalSourceScope::WorkspaceLocal {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is not a workspace-local mutable skill and cannot change vitality state",
                    skill_name.trim()
                ),
            });
        }

        let previous_state = snapshot
            .vitality
            .as_ref()
            .map(|record| record.state)
            .unwrap_or(SkillVitalityState::Active);
        let vitality = SkillVitalityRecord {
            state,
            updated_at: reason.noted_at,
            reason: reason.clone(),
        };
        snapshot.vitality = Some(vitality.clone());
        self.hub_store
            .upsert_skill_operational_snapshot(snapshot.clone())?;
        self.append_audit_event(vitality_transition_audit_event(
            &snapshot,
            previous_state,
            &vitality,
            actor,
        ))?;
        Ok(snapshot)
    }

    pub fn skill_negative_entropy_diagnostics(
        &self,
    ) -> Result<Vec<SkillNegativeEntropyDiagnostic>, SkillError> {
        let snapshots = self.skill_operational_snapshots();
        let conflicts = self.skill_semantic_conflict_diagnostics()?;
        let mut overlap_counts = BTreeMap::<String, u64>::new();
        for conflict in &conflicts {
            *overlap_counts
                .entry(normalize_name(&conflict.left_skill_name))
                .or_default() += 1;
            *overlap_counts
                .entry(normalize_name(&conflict.right_skill_name))
                .or_default() += 1;
        }

        let now = now_unix_timestamp();
        let mut diagnostics = Vec::new();
        for snapshot in snapshots {
            let runtime_use_count = snapshot
                .usage
                .as_ref()
                .map(|entry| entry.runtime_use_count)
                .unwrap_or(0);
            let runtime_error_count = snapshot
                .usage
                .as_ref()
                .map(|entry| entry.runtime_error_count)
                .unwrap_or(0);
            let last_used_at = snapshot.usage.as_ref().and_then(|entry| entry.last_used_at);
            let write_count = snapshot
                .writes
                .as_ref()
                .map(total_skill_write_count)
                .unwrap_or(0);
            let last_write_at = snapshot
                .writes
                .as_ref()
                .and_then(|entry| entry.last_write_at);
            let semantic_overlap_count = overlap_counts
                .get(&normalize_name(&snapshot.skill_name))
                .copied()
                .unwrap_or(0);
            let recent_positive_evolution = snapshot
                .evolution
                .as_ref()
                .and_then(|summary| summary.last_positive_signal_at)
                .is_some_and(|timestamp| {
                    now.saturating_sub(timestamp) < SKILL_POSITIVE_EVOLUTION_GRACE_SECONDS
                });
            let composition_context =
                self.skill_composition_consumption_context(&snapshot.skill_name);

            let mut signals = Vec::new();
            let mut reasons = Vec::new();

            if write_count > 0 && runtime_use_count == 0 {
                signals.push(SkillNegativeEntropySignal::NeverReused);
                reasons.push(format!(
                    "write history exists ({write_count} write actions) but runtime reuse has never been recorded"
                ));
            }

            if write_count >= 3 && runtime_use_count <= 1 {
                signals.push(SkillNegativeEntropySignal::WriteHeavyLowReuse);
                reasons.push(format!(
                    "write churn is high ({write_count} write actions) while runtime reuse remains low ({runtime_use_count})"
                ));
            }

            if is_skill_timestamp_stale(last_used_at, now, SKILL_NEGATIVE_ENTROPY_STALE_SECONDS)
                && is_skill_timestamp_stale(
                    last_write_at.or(last_used_at),
                    now,
                    SKILL_NEGATIVE_ENTROPY_STALE_SECONDS,
                )
            {
                signals.push(SkillNegativeEntropySignal::StaleUnused);
                reasons.push(format!(
                    "no recent use or write activity in the last {} days",
                    SKILL_NEGATIVE_ENTROPY_STALE_SECONDS / 86_400
                ));
            }

            if matches!(snapshot.source_scope, SkillOperationalSourceScope::Managed)
                && runtime_use_count == 0
                && last_used_at.is_none()
            {
                signals.push(SkillNegativeEntropySignal::DormantManaged);
                reasons.push(
                    "managed skill has been installed or tracked, but runtime usage has not been observed yet"
                        .to_string(),
                );
            }

            if signals.is_empty() {
                continue;
            }

            if semantic_overlap_count > 0 {
                reasons.push(format!(
                    "semantic conflict diagnostics report {semantic_overlap_count} overlap candidate(s)"
                ));
            }
            if runtime_error_count > 0 {
                reasons.push(format!(
                    "runtime ledger recorded {runtime_error_count} error event(s)"
                ));
            }
            if recent_positive_evolution {
                if let Some(evolution) = snapshot.evolution.as_ref() {
                    reasons.push(format_skill_positive_evolution_reason(evolution, now));
                }
            }
            if let Some(reason) = format_negative_entropy_composition_reason(
                &snapshot.skill_name,
                &composition_context,
            ) {
                reasons.push(reason);
            }

            let severity = skill_negative_entropy_severity(
                snapshot.source_scope,
                signals.as_slice(),
                recent_positive_evolution,
                &composition_context,
            );
            diagnostics.push(SkillNegativeEntropyDiagnostic {
                skill_name: snapshot.skill_name,
                source_scope: snapshot.source_scope,
                source_id: snapshot.source_id,
                signals,
                severity,
                runtime_use_count,
                runtime_error_count,
                write_count,
                last_used_at,
                last_write_at,
                semantic_overlap_count,
                reasons,
            });
        }

        diagnostics.sort_by(|left, right| {
            skill_diagnostic_sort_key(left.severity)
                .cmp(&skill_diagnostic_sort_key(right.severity))
                .then_with(|| {
                    right
                        .semantic_overlap_count
                        .cmp(&left.semantic_overlap_count)
                })
                .then_with(|| right.write_count.cmp(&left.write_count))
                .then_with(|| left.runtime_use_count.cmp(&right.runtime_use_count))
                .then_with(|| left.skill_name.cmp(&right.skill_name))
        });
        Ok(diagnostics)
    }

    pub fn skill_semantic_conflict_diagnostics(
        &self,
    ) -> Result<Vec<SkillSemanticConflictDiagnostic>, SkillError> {
        let (descriptors, snapshot_by_name) = self.skill_semantic_analysis_inputs()?;
        Ok(collect_skill_semantic_conflicts(
            &descriptors,
            &snapshot_by_name,
        ))
    }

    pub fn distributions(&self) -> Vec<SkillDistributionRecord> {
        self.hub_store.distributions()
    }

    pub fn artifact_cache(&self) -> Vec<SkillArtifactCacheEntry> {
        self.hub_store.artifact_cache()
    }

    pub fn artifact_policy(&self) -> SkillHubPolicy {
        self.artifact_store.policy()
    }

    pub fn reconcile_artifact_cache_policy(
        &self,
    ) -> Result<Vec<SkillArtifactCacheEntry>, SkillError> {
        let existing = self.hub_store.artifact_cache();
        let retained = self.artifact_store.evict_expired_entries(&existing)?;
        self.hub_store.replace_artifact_cache(retained.clone())?;
        let retained_ids = retained
            .iter()
            .map(|entry| entry.artifact.artifact_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let distributions = self.distributions();
        let policy = self.artifact_policy();
        for entry in existing
            .into_iter()
            .filter(|entry| !retained_ids.contains(entry.artifact.artifact_id.as_str()))
        {
            self.append_audit_event(artifact_evicted_audit_event(
                &entry,
                distributions.iter().find(|record| {
                    record.resolution.artifact.artifact_id == entry.artifact.artifact_id
                }),
                &policy,
            ))?;
        }
        Ok(retained)
    }

    pub fn lifecycle_records(&self) -> Vec<SkillManagedLifecycleRecord> {
        self.hub_store.lifecycle()
    }

    pub fn audit_tail(&self) -> Vec<SkillAuditEvent> {
        self.hub_store.audit_tail()
    }

    pub fn refresh_source_index(
        &self,
        source: &SkillSourceRef,
        actor: &str,
    ) -> Result<SkillSourceIndexSnapshot, SkillError> {
        let snapshot = match source.source_kind {
            agendao_types::SkillSourceKind::Bundled => {
                let manifest =
                    self.hub_store
                        .bundled_manifest()
                        .ok_or_else(|| SkillError::ReadFailed {
                            path: self.hub_store.bundled_manifest_path(),
                            message: "missing bundled manifest for bundled sync source".to_string(),
                        })?;
                let root = self.resolve_source_root(&source.locator);
                let source_snapshot = self
                    .sync_planner
                    .build_bundled_source_snapshot(source, &root, &manifest)?;
                self.sync_planner.source_index_snapshot(&source_snapshot)
            }
            agendao_types::SkillSourceKind::LocalPath => {
                let root = self.resolve_source_root(&source.locator);
                let source_snapshot = self
                    .sync_planner
                    .build_local_source_snapshot(source, &root)?;
                self.sync_planner.source_index_snapshot(&source_snapshot)
            }
            agendao_types::SkillSourceKind::Git
            | agendao_types::SkillSourceKind::Archive
            | agendao_types::SkillSourceKind::Registry => self
                .hub_store
                .upsert_remote_source_index(crate::hub::refresh_remote_source_index(
                    self.hub_store.base_dir(),
                    source,
                    self.artifact_policy().fetch_timeout_ms,
                )?)?,
        };
        if !matches!(
            source.source_kind,
            agendao_types::SkillSourceKind::Git
                | agendao_types::SkillSourceKind::Archive
                | agendao_types::SkillSourceKind::Registry
        ) {
            self.hub_store.upsert_source_index(snapshot.clone())?;
        }
        self.append_audit_event(source_index_refresh_audit_event(source, actor, &snapshot))?;
        Ok(snapshot)
    }

    fn index_freshness_max_age_seconds(&self) -> u64 {
        self.config_store
            .as_deref()
            .map(|store| store.config())
            .and_then(|config| {
                config
                    .skills
                    .as_ref()?
                    .hub
                    .as_ref()?
                    .index_freshness_max_age_seconds
            })
            .unwrap_or(DEFAULT_INDEX_FRESHNESS_MAX_AGE_SECONDS)
    }

    fn default_registry_sources(&self) -> Vec<SkillSourceRef> {
        let Some(config) = self.config_store.as_deref().map(|store| store.config()) else {
            return Vec::new();
        };
        let Some(registries) = config
            .skills
            .as_ref()
            .and_then(|skills| skills.hub.as_ref())
            .and_then(|hub| hub.default_registries.as_deref())
        else {
            return Vec::new();
        };
        registries
            .iter()
            .map(|entry| SkillSourceRef {
                source_id: entry.source_id.clone(),
                source_kind: entry.source_kind.clone(),
                locator: entry.locator.clone(),
                revision: None,
            })
            .collect()
    }

    fn compute_stale(&self, source_updated_at: i64) -> bool {
        let threshold = self.index_freshness_max_age_seconds() as i64;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        now.saturating_sub(source_updated_at) > threshold
    }

    fn trust_level_for_source(source: &SkillSourceRef) -> agendao_types::SkillTrustLevel {
        // Trust is derived from source_kind, not source_id.
        // source_id is user-configurable and trivially spoofable;
        // source_kind is a code-level enum that reflects how the source
        // was registered (bundled at build time, configured as a registry,
        // or resolved from a git/archive locator).
        match source.source_kind {
            agendao_types::SkillSourceKind::Bundled => agendao_types::SkillTrustLevel::Official,
            agendao_types::SkillSourceKind::Registry | agendao_types::SkillSourceKind::Git => {
                agendao_types::SkillTrustLevel::Community
            }
            _ => agendao_types::SkillTrustLevel::Unknown,
        }
    }

    fn trust_score(trust_level: agendao_types::SkillTrustLevel) -> i64 {
        match trust_level {
            agendao_types::SkillTrustLevel::Official => 200,
            agendao_types::SkillTrustLevel::Community => 100,
            agendao_types::SkillTrustLevel::Unknown => 0,
        }
    }

    fn maintenance_status_label(stale: bool, source_updated_at: i64) -> Option<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let age_days = (now.saturating_sub(source_updated_at)).max(1) / 86_400;
        if stale {
            Some(format!("stale ({} days)", age_days.max(1)))
        } else if age_days < 30 {
            Some("active".to_string())
        } else {
            None
        }
    }

    pub fn search_source_indices(&self, request: &SkillHubSearchRequest) -> SkillHubSearchResponse {
        let normalized_query = trimmed_option(request.query.as_deref());
        let query_terms = search_query_terms(normalized_query.as_deref());
        let normalized_source_id = trimmed_option(request.source_id.as_deref());
        let source_kind = request.source_kind.clone();
        let limit = request.limit.unwrap_or(20).clamp(1, 100);
        let managed_by_name = self
            .managed_skills()
            .into_iter()
            .map(|record| (normalize_name(&record.skill_name), record))
            .collect::<BTreeMap<_, _>>();
        let governance_snapshot = self.governance_snapshot();
        let has_indexed_sources = !governance_snapshot.source_indices.is_empty();
        let mut matches = Vec::new();

        for snapshot in governance_snapshot
            .source_indices
            .into_iter()
            .filter(|snapshot| {
                search_snapshot_matches_filters(
                    snapshot,
                    normalized_source_id.as_deref(),
                    source_kind.clone(),
                )
            })
        {
            let stale = self.compute_stale(snapshot.updated_at);
            let trust_level = Self::trust_level_for_source(&snapshot.source);
            for entry in snapshot.entries {
                let Some((base_score, match_reasons)) =
                    score_source_index_entry(&entry, normalized_query.as_deref(), &query_terms)
                else {
                    continue;
                };
                let score = base_score + Self::trust_score(trust_level);
                let managed_record = managed_by_name.get(&normalize_name(&entry.skill_name));
                let managed_for_source = managed_record
                    .and_then(|record| record.source.as_ref())
                    .is_some_and(|source| source == &snapshot.source);
                let maintenance_status = Self::maintenance_status_label(stale, snapshot.updated_at);
                matches.push(SkillHubSearchMatch {
                    source: snapshot.source.clone(),
                    entry,
                    source_updated_at: snapshot.updated_at,
                    score,
                    match_reasons,
                    managed: managed_for_source,
                    locally_modified: managed_record
                        .filter(|_| managed_for_source)
                        .map(|record| record.locally_modified)
                        .unwrap_or(false),
                    deleted_locally: managed_record
                        .filter(|_| managed_for_source)
                        .map(|record| record.deleted_locally)
                        .unwrap_or(false),
                    installed_revision: managed_record
                        .filter(|_| managed_for_source)
                        .and_then(|record| record.installed_revision.clone()),
                    stale,
                    trust_level,
                    maintenance_status,
                });
            }
        }

        matches.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| right.source_updated_at.cmp(&left.source_updated_at))
                .then_with(|| left.entry.skill_name.cmp(&right.entry.skill_name))
                .then_with(|| left.source.source_id.cmp(&right.source.source_id))
        });
        matches.truncate(limit);

        let suggested_refresh_sources: Vec<SkillSourceRef> =
            if matches.is_empty() || !has_indexed_sources {
                self.default_registry_sources()
                    .into_iter()
                    .filter(|source| {
                        search_source_matches_filters(
                            source,
                            normalized_source_id.as_deref(),
                            source_kind.clone(),
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };

        let web_fallback_query =
            if matches.is_empty() && !has_indexed_sources && normalized_query.is_some() {
                normalized_query.clone()
            } else {
                None
            };

        SkillHubSearchResponse {
            query: normalized_query,
            matches,
            suggested_refresh_sources,
            web_fallback_query,
        }
    }

    pub fn governance_timeline(
        &self,
        query: &SkillHubTimelineQuery,
    ) -> Vec<SkillGovernanceTimelineEntry> {
        let normalized_skill_filter = query.skill_name.as_deref().map(normalize_name);
        let source_filter = trimmed_option(query.source_id.as_deref());
        let limit = query.limit.unwrap_or(120).clamp(1, 500);

        let managed_records = self.managed_skills();
        let managed_by_name = managed_records
            .iter()
            .map(|record| (normalize_name(&record.skill_name), record.clone()))
            .collect::<BTreeMap<_, _>>();

        let mut entries = managed_records
            .into_iter()
            .filter(|record| {
                timeline_matches_filters(
                    Some(record.skill_name.as_str()),
                    record
                        .source
                        .as_ref()
                        .map(|source| source.source_id.as_str()),
                    normalized_skill_filter.as_deref(),
                    source_filter.as_deref(),
                )
            })
            .map(managed_record_timeline_entry)
            .collect::<Vec<_>>();

        entries.extend(self.audit_tail().into_iter().filter_map(|event| {
            if !audit_event_matches_filters(
                &event,
                normalized_skill_filter.as_deref(),
                source_filter.as_deref(),
            ) {
                return None;
            }
            Some(audit_event_timeline_entry(
                &event,
                event
                    .skill_name
                    .as_deref()
                    .and_then(|name| managed_by_name.get(&normalize_name(name)).cloned()),
            ))
        }));

        entries.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.entry_id.cmp(&right.entry_id))
        });
        entries.truncate(limit);
        entries
    }

    pub fn upsert_managed_skill(
        &self,
        record: ManagedSkillRecord,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.upsert_managed_skill(record)
    }

    pub fn replace_source_indices(
        &self,
        source_indices: Vec<SkillSourceIndexSnapshot>,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.replace_source_indices(source_indices)
    }

    pub fn replace_distributions(
        &self,
        distributions: Vec<SkillDistributionRecord>,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.replace_distributions(distributions)
    }

    pub fn replace_artifact_cache(
        &self,
        artifact_cache: Vec<SkillArtifactCacheEntry>,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.replace_artifact_cache(artifact_cache)
    }

    pub fn upsert_distribution(
        &self,
        distribution: SkillDistributionRecord,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.upsert_distribution(distribution)
    }

    pub fn upsert_artifact_cache_entry(
        &self,
        entry: SkillArtifactCacheEntry,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.upsert_artifact_cache_entry(entry)
    }

    pub fn replace_lifecycle_records(
        &self,
        lifecycle: Vec<SkillManagedLifecycleRecord>,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.replace_lifecycle(lifecycle)
    }

    pub fn upsert_lifecycle_record(
        &self,
        record: SkillManagedLifecycleRecord,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.upsert_lifecycle_record(record)
    }

    pub fn replace_bundled_manifest(
        &self,
        bundled_manifest: Option<BundledSkillManifest>,
    ) -> Result<(), crate::SkillError> {
        self.hub_store.replace_bundled_manifest(bundled_manifest)
    }

    pub fn append_audit_event(&self, event: SkillAuditEvent) -> Result<(), crate::SkillError> {
        self.hub_store.append_audit_event(event)
    }

    pub fn resolve_distribution(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillDistributionRecord, SkillError> {
        let source_index = self
            .hub_store
            .source_index(&source.source_id)
            .unwrap_or(self.refresh_source_index(source, actor)?);
        match self.distribution_resolver.resolve_distribution(
            self.hub_store.base_dir(),
            source,
            &source_index,
            skill_name,
            self.artifact_policy().fetch_timeout_ms,
        ) {
            Ok(resolved) => {
                let record = resolved.record.clone();
                self.upsert_distribution(record.clone())?;
                self.record_lifecycle(
                    Some(actor),
                    SkillManagedLifecycleRecord {
                        distribution_id: record.distribution_id.clone(),
                        source_id: source.source_id.clone(),
                        skill_name: record.skill_name.clone(),
                        state: SkillManagedLifecycleState::Resolved,
                        updated_at: record.resolution.resolved_at,
                        error: None,
                    },
                )?;
                self.append_audit_event(distribution_audit_event(
                    SkillAuditKind::SourceResolved,
                    actor,
                    &record,
                    None,
                ))?;
                Ok(record)
            }
            Err(error) => {
                self.record_lifecycle(
                    Some(actor),
                    SkillManagedLifecycleRecord {
                        distribution_id: unresolved_distribution_id(source, skill_name),
                        source_id: source.source_id.clone(),
                        skill_name: skill_name.trim().to_string(),
                        state: SkillManagedLifecycleState::ResolutionFailed,
                        updated_at: now_unix_timestamp(),
                        error: Some(error.to_string()),
                    },
                )?;
                Err(error)
            }
        }
    }

    pub fn fetch_distribution_artifact(
        &self,
        distribution_id: &str,
        actor: &str,
    ) -> Result<SkillArtifactCacheEntry, SkillError> {
        let _ = self.reconcile_artifact_cache_policy()?;
        let distribution = self
            .distributions()
            .into_iter()
            .find(|record| record.distribution_id == distribution_id)
            .ok_or_else(|| SkillError::InvalidSkillContent {
                message: format!("unknown distribution `{distribution_id}`"),
            })?;

        match self
            .artifact_store
            .fetch_artifact(&distribution.resolution.artifact)
        {
            Ok(entry) => {
                self.upsert_artifact_cache_entry(entry.clone())?;
                self.record_lifecycle(
                    Some(actor),
                    SkillManagedLifecycleRecord {
                        distribution_id: distribution.distribution_id.clone(),
                        source_id: distribution.source.source_id.clone(),
                        skill_name: distribution.skill_name.clone(),
                        state: SkillManagedLifecycleState::Fetched,
                        updated_at: entry.cached_at,
                        error: None,
                    },
                )?;
                self.append_audit_event(distribution_audit_event(
                    SkillAuditKind::ArtifactFetched,
                    actor,
                    &distribution,
                    None,
                ))?;
                Ok(entry)
            }
            Err(error) => {
                self.upsert_artifact_cache_entry(SkillArtifactCacheEntry {
                    artifact: distribution.resolution.artifact.clone(),
                    cached_at: now_unix_timestamp(),
                    local_path: self
                        .artifact_store
                        .artifact_cache_dir()
                        .to_string_lossy()
                        .to_string(),
                    extracted_path: None,
                    status: SkillArtifactCacheStatus::Failed,
                    error: Some(error.to_string()),
                })?;
                self.record_lifecycle(
                    Some(actor),
                    SkillManagedLifecycleRecord {
                        distribution_id: distribution.distribution_id.clone(),
                        source_id: distribution.source.source_id.clone(),
                        skill_name: distribution.skill_name.clone(),
                        state: SkillManagedLifecycleState::FetchFailed,
                        updated_at: now_unix_timestamp(),
                        error: Some(error.to_string()),
                    },
                )?;
                self.append_audit_event(distribution_audit_event(
                    SkillAuditKind::ArtifactFetchFailed,
                    actor,
                    &distribution,
                    Some(error.to_string()),
                ))?;
                Err(error)
            }
        }
    }

    pub fn plan_remote_install(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillRemoteInstallPlan, SkillError> {
        let distribution = self.resolve_distribution(source, skill_name, actor)?;
        let action = self.remote_install_action(&distribution)?;
        let plan = SkillRemoteInstallPlan {
            source_id: source.source_id.clone(),
            distribution: distribution.clone(),
            entry: SkillRemoteInstallEntry {
                distribution_id: distribution.distribution_id.clone(),
                source_id: source.source_id.clone(),
                skill_name: distribution.skill_name.clone(),
                action,
                reason: remote_install_reason(&distribution),
            },
        };
        self.record_lifecycle(
            Some(actor),
            SkillManagedLifecycleRecord {
                distribution_id: distribution.distribution_id.clone(),
                source_id: source.source_id.clone(),
                skill_name: distribution.skill_name.clone(),
                state: SkillManagedLifecycleState::PlannedInstall,
                updated_at: now_unix_timestamp(),
                error: None,
            },
        )?;
        self.append_audit_event(remote_plan_audit_event(
            match plan.entry.action {
                SkillRemoteInstallAction::Install => SkillAuditKind::RemoteInstallPlanned,
                SkillRemoteInstallAction::Update => SkillAuditKind::RemoteUpdatePlanned,
            },
            actor,
            &plan,
        ))?;
        Ok(plan)
    }

    pub fn apply_remote_install(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillRemoteInstallResponse, SkillError> {
        let plan = self.plan_remote_install(source, skill_name, actor)?;
        self.apply_remote_plan(source, actor, plan)
    }

    pub fn plan_remote_update(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillRemoteInstallPlan, SkillError> {
        let managed = self.refresh_managed_record_for_source_skill(source, skill_name)?;
        let mut distribution = self.resolve_distribution(source, skill_name, actor)?;
        let installed_distribution = self
            .current_distribution_for_managed_record(&managed.record)
            .and_then(|record| record.installed);
        if distribution.installed.is_none() {
            distribution.installed = installed_distribution;
        }

        let lifecycle_state = self
            .lifecycle
            .managed_runtime_state(&managed.record, release_identity(&distribution.release));
        let update_available = self.lifecycle.update_available(
            managed.record.installed_revision.as_deref(),
            release_identity(&distribution.release),
        );
        if !update_available && lifecycle_state != SkillManagedLifecycleState::Diverged {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is already current for source `{}`",
                    managed.record.skill_name, source.source_id
                ),
            });
        }

        distribution.lifecycle = lifecycle_state.clone();
        self.upsert_distribution(distribution.clone())?;
        self.record_lifecycle(
            Some(actor),
            self.lifecycle.build_record(
                distribution.distribution_id.clone(),
                source.source_id.clone(),
                distribution.skill_name.clone(),
                lifecycle_state.clone(),
                now_unix_timestamp(),
                None,
            ),
        )?;
        self.append_audit_event(remote_plan_audit_event(
            SkillAuditKind::RemoteUpdatePlanned,
            actor,
            &SkillRemoteInstallPlan {
                source_id: source.source_id.clone(),
                distribution: distribution.clone(),
                entry: SkillRemoteInstallEntry {
                    distribution_id: distribution.distribution_id.clone(),
                    source_id: source.source_id.clone(),
                    skill_name: distribution.skill_name.clone(),
                    action: SkillRemoteInstallAction::Update,
                    reason: remote_update_reason(
                        &distribution,
                        &managed.record,
                        lifecycle_state.clone(),
                    ),
                },
            },
        ))?;

        Ok(SkillRemoteInstallPlan {
            source_id: source.source_id.clone(),
            distribution: distribution.clone(),
            entry: SkillRemoteInstallEntry {
                distribution_id: distribution.distribution_id.clone(),
                source_id: source.source_id.clone(),
                skill_name: distribution.skill_name.clone(),
                action: SkillRemoteInstallAction::Update,
                reason: remote_update_reason(&distribution, &managed.record, lifecycle_state),
            },
        })
    }

    pub fn apply_remote_update(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillRemoteInstallResponse, SkillError> {
        let plan = self.plan_remote_update(source, skill_name, actor)?;
        self.apply_remote_plan(source, actor, plan)
    }

    pub fn detach_managed_skill(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillHubManagedDetachResponse, SkillError> {
        let managed = self.refresh_managed_record_for_source_skill(source, skill_name)?;
        let removed = self
            .hub_store
            .remove_managed_skill(&managed.record.skill_name)?
            .ok_or_else(|| SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is not managed by source `{}`",
                    skill_name.trim(),
                    source.source_id
                ),
            })?;
        let timestamp = now_unix_timestamp();
        let distribution_id = self
            .current_distribution_for_managed_record(&removed)
            .map(|distribution| distribution.distribution_id)
            .unwrap_or_else(|| unresolved_distribution_id(source, &removed.skill_name));
        if let Some(mut distribution) = self.current_distribution_for_managed_record(&removed) {
            distribution.lifecycle = SkillManagedLifecycleState::Detached;
            self.upsert_distribution(distribution)?;
        }
        let lifecycle = self.lifecycle.build_record(
            distribution_id,
            source.source_id.clone(),
            removed.skill_name.clone(),
            SkillManagedLifecycleState::Detached,
            timestamp,
            None,
        );
        self.record_lifecycle(Some(actor), lifecycle.clone())?;
        self.append_audit_event(hub_detach_audit_event(source, actor, &removed))?;
        self.record_skill_write_action(
            &removed.skill_name,
            None,
            SkillWriteLedgerAction::Detach,
            SkillOperationalSourceScope::WorkspaceLocal,
            None,
            None,
        )?;
        Ok(SkillHubManagedDetachResponse { lifecycle })
    }

    pub fn remove_managed_skill(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
        actor: &str,
    ) -> Result<SkillHubManagedRemoveResponse, SkillError> {
        let managed = self.refresh_managed_record_for_source_skill(source, skill_name)?;
        let mut deleted_from_workspace = false;
        let mut result = None;
        if let Some(current_hash) = managed.current_hash.as_deref() {
            if managed.record.local_hash.as_deref() == Some(current_hash) {
                let write_result = self.skill_authority.delete_skill(DeleteSkillRequest {
                    name: managed.record.skill_name.clone(),
                })?;
                deleted_from_workspace = true;
                result = Some(governance_write_result(&write_result));
            }
        }

        let removed = self
            .hub_store
            .remove_managed_skill(&managed.record.skill_name)?
            .ok_or_else(|| SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is not managed by source `{}`",
                    skill_name.trim(),
                    source.source_id
                ),
            })?;
        let timestamp = now_unix_timestamp();
        let distribution_id = self
            .current_distribution_for_managed_record(&removed)
            .map(|distribution| distribution.distribution_id)
            .unwrap_or_else(|| unresolved_distribution_id(source, &removed.skill_name));
        if let Some(mut distribution) = self.current_distribution_for_managed_record(&removed) {
            distribution.lifecycle = SkillManagedLifecycleState::Removed;
            if deleted_from_workspace {
                distribution.installed = None;
            }
            self.upsert_distribution(distribution)?;
        }
        let lifecycle = self.lifecycle.build_record(
            distribution_id,
            source.source_id.clone(),
            removed.skill_name.clone(),
            SkillManagedLifecycleState::Removed,
            timestamp,
            None,
        );
        self.record_lifecycle(Some(actor), lifecycle.clone())?;
        self.append_audit_event(hub_remove_audit_event(
            source,
            actor,
            &removed,
            deleted_from_workspace,
        ))?;
        self.record_skill_write_action(
            &removed.skill_name,
            None,
            SkillWriteLedgerAction::Remove,
            SkillOperationalSourceScope::Managed,
            None,
            None,
        )?;
        Ok(SkillHubManagedRemoveResponse {
            lifecycle,
            deleted_from_workspace,
            result,
        })
    }

    pub fn refresh_managed_workspace_state(&self) -> Result<Vec<ManagedSkillRecord>, SkillError> {
        let catalog = self.skill_authority.list_skill_catalog(None)?;
        let resolved = self.sync_planner.refresh_managed_records(
            &self.hub_store.managed_skills(),
            &catalog,
            None,
        )?;
        let records = resolved
            .into_iter()
            .map(|record| record.record)
            .collect::<Vec<_>>();
        self.hub_store.replace_managed_skills(records.clone())?;
        self.update_distribution_runtime_state(
            &self
                .sync_planner
                .refresh_managed_records(&records, &catalog, None)?,
        )?;
        Ok(records)
    }

    pub fn record_runtime_skill_usage(
        &self,
        skill_name: &str,
        tool_name: &str,
        stage_id: Option<&str>,
        category: Option<&str>,
        is_error: bool,
    ) -> Result<SkillOperationalSnapshot, SkillError> {
        let mut snapshot = self.prepare_operational_snapshot(
            skill_name,
            None,
            SkillOperationalSourceScope::Unknown,
        )?;
        let now = now_unix_timestamp();
        let usage = snapshot
            .usage
            .get_or_insert_with(SkillUsageLedgerEntry::default);
        usage.first_seen_at.get_or_insert(now);
        usage.last_used_at = Some(now);
        usage.runtime_use_count += 1;
        if is_error {
            usage.runtime_error_count += 1;
        } else {
            usage.runtime_success_count += 1;
        }
        usage.last_stage_id = stage_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        usage.last_tool_name = Some(tool_name.trim().to_string()).filter(|value| !value.is_empty());
        usage.last_category = category
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        self.hub_store
            .upsert_skill_operational_snapshot(snapshot.clone())?;
        Ok(snapshot)
    }

    pub fn record_skill_memory_promotion_signal(
        &self,
        skill_name: &str,
        promoted_record_count: u64,
    ) -> Result<SkillOperationalSnapshot, SkillError> {
        let mut snapshot = self.prepare_operational_snapshot(
            skill_name,
            None,
            SkillOperationalSourceScope::Unknown,
        )?;
        if matches!(snapshot.source_scope, SkillOperationalSourceScope::Unknown) {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is unresolved and cannot record memory promotion evidence",
                    skill_name.trim()
                ),
            });
        }
        if promoted_record_count == 0 {
            return Ok(snapshot);
        }

        let now = now_unix_timestamp();
        let evolution = snapshot
            .evolution
            .get_or_insert_with(SkillEvolutionEvidenceSummary::default);
        evolution.memory_promotion_count += promoted_record_count;
        evolution.last_memory_promotion_at = Some(now);
        evolution.last_positive_signal_at = Some(
            evolution
                .last_positive_signal_at
                .map(|current| current.max(now))
                .unwrap_or(now),
        );

        self.hub_store
            .upsert_skill_operational_snapshot(snapshot.clone())?;
        Ok(snapshot)
    }

    pub fn record_skill_proposal_signal(
        &self,
        skill_name: &str,
        draft_proposal_count: u64,
    ) -> Result<SkillOperationalSnapshot, SkillError> {
        let mut snapshot = self.prepare_operational_snapshot(
            skill_name,
            None,
            SkillOperationalSourceScope::Unknown,
        )?;
        if matches!(snapshot.source_scope, SkillOperationalSourceScope::Unknown) {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is unresolved and cannot record proposal evidence",
                    skill_name.trim()
                ),
            });
        }

        let now = now_unix_timestamp();
        let evolution = snapshot
            .evolution
            .get_or_insert_with(SkillEvolutionEvidenceSummary::default);
        evolution.last_observed_draft_proposal_count = draft_proposal_count;
        if draft_proposal_count > 0 {
            evolution.proposal_signal_count += 1;
            evolution.last_proposal_at = Some(now);
            evolution.last_positive_signal_at = Some(
                evolution
                    .last_positive_signal_at
                    .map(|current| current.max(now))
                    .unwrap_or(now),
            );
        }

        self.hub_store
            .upsert_skill_operational_snapshot(snapshot.clone())?;
        Ok(snapshot)
    }

    pub fn create_skill(
        &self,
        req: CreateSkillRequest,
        actor: &str,
    ) -> Result<SkillGovernedWriteResult, SkillError> {
        let duplicate_conflict = self
            .skill_authority
            .discover_skills()
            .iter()
            .any(|skill| skill.name.eq_ignore_ascii_case(req.name.trim()));
        let guard_report = self.apply_guard_report(
            actor,
            None,
            self.evaluate_create_guard_report(&req, duplicate_conflict),
        )?;
        let result = self.skill_authority.create_skill(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        self.record_skill_write_action(
            &result.skill_name,
            None,
            SkillWriteLedgerAction::Create,
            SkillOperationalSourceScope::WorkspaceLocal,
            Some(result.location.as_path()),
            result.supporting_file.as_deref(),
        )?;
        Ok(SkillGovernedWriteResult {
            result,
            guard_report,
        })
    }

    pub fn patch_skill(
        &self,
        req: PatchSkillRequest,
        actor: &str,
    ) -> Result<SkillGovernedWriteResult, SkillError> {
        let current = self
            .skill_authority
            .resolve_skill_for_inspection(&req.name, None)?;
        let next_name = req.new_name.as_deref().unwrap_or(current.name.as_str());
        let duplicate_conflict = !next_name.eq_ignore_ascii_case(&current.name)
            && self
                .skill_authority
                .discover_skills()
                .iter()
                .any(|skill| skill.name.eq_ignore_ascii_case(next_name));
        let guard_report = self.apply_guard_report(
            actor,
            None,
            self.evaluate_patch_guard_report(&current, &req, next_name, duplicate_conflict),
        )?;
        let result = self.skill_authority.patch_skill(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        self.record_skill_write_action(
            &result.skill_name,
            Some(&current.name),
            SkillWriteLedgerAction::Patch,
            SkillOperationalSourceScope::WorkspaceLocal,
            Some(result.location.as_path()),
            result.supporting_file.as_deref(),
        )?;
        Ok(SkillGovernedWriteResult {
            result,
            guard_report,
        })
    }

    pub fn materialize_runtime_skills(
        &self,
        instructions: &[RuntimeInstructionSource],
        actor: &str,
    ) -> Result<RuntimeSkillBootstrapReport, SkillError> {
        let (specs, warnings) = crate::runtime::collect_runtime_skill_specs(
            self.skill_authority.base_dir(),
            instructions,
        );
        self.materialize_runtime_specs(specs, warnings, actor)
    }

    pub fn materialize_runtime_skill_by_name(
        &self,
        skill_name: &str,
        instructions: &[RuntimeInstructionSource],
        actor: &str,
    ) -> Result<RuntimeSkillBootstrapReport, SkillError> {
        let (specs, warnings) = crate::runtime::collect_runtime_skill_specs(
            self.skill_authority.base_dir(),
            instructions,
        );
        let filtered = specs
            .into_iter()
            .filter(|spec| spec.name.eq_ignore_ascii_case(skill_name))
            .collect::<Vec<_>>();
        self.materialize_runtime_specs(filtered, warnings, actor)
    }

    fn materialize_runtime_specs(
        &self,
        specs: Vec<crate::runtime::RuntimeSkillSpec>,
        warnings: Vec<String>,
        actor: &str,
    ) -> Result<RuntimeSkillBootstrapReport, SkillError> {
        let mut report = RuntimeSkillBootstrapReport {
            materializations: Vec::new(),
            imported_legacy_sources: specs
                .iter()
                .filter(|spec| matches!(spec.source_kind, RuntimeSkillSourceKind::LegacyMarkdown))
                .filter_map(|spec| spec.source_path.clone())
                .collect(),
            warnings,
        };

        for spec in specs {
            let existing = self
                .skill_authority
                .resolve_skill_for_inspection(&spec.name, None);
            match existing {
                Ok(meta) => {
                    if !self.skill_authority.is_skill_meta_writable(&meta) {
                        report.materializations.push(RuntimeSkillMaterialization {
                            skill_name: spec.name.clone(),
                            action: RuntimeSkillMaterializationAction::Skipped,
                            source_kind: spec.source_kind,
                            source_path: spec.source_path.clone(),
                            detail: Some(format!(
                                "existing skill is outside the workspace sandbox: {}",
                                meta.location.display()
                            )),
                        });
                        continue;
                    }

                    let loaded = self
                        .skill_authority
                        .load_skill_for_inspection(&spec.name, None)?;
                    let description_matches = meta.description.trim() == spec.description.trim();
                    let body_matches = loaded.content.trim() == spec.body.trim();
                    if description_matches && body_matches {
                        report.materializations.push(RuntimeSkillMaterialization {
                            skill_name: spec.name.clone(),
                            action: RuntimeSkillMaterializationAction::Unchanged,
                            source_kind: spec.source_kind,
                            source_path: spec.source_path.clone(),
                            detail: None,
                        });
                        continue;
                    }

                    let content = crate::write::build_skill_document(
                        &crate::write::build_create_frontmatter(
                            &spec.name,
                            &spec.description,
                            None,
                        )?,
                        &spec.body,
                    )?;
                    let _ = self.edit_skill(
                        EditSkillRequest {
                            name: spec.name.clone(),
                            content,
                        },
                        actor,
                    )?;
                    report.materializations.push(RuntimeSkillMaterialization {
                        skill_name: spec.name.clone(),
                        action: RuntimeSkillMaterializationAction::Refreshed,
                        source_kind: spec.source_kind,
                        source_path: spec.source_path.clone(),
                        detail: None,
                    });
                }
                Err(SkillError::UnknownSkill { .. }) => {
                    let _ = self.create_skill(
                        CreateSkillRequest {
                            name: spec.name.clone(),
                            description: spec.description.clone(),
                            body: spec.body.clone(),
                            frontmatter: None,
                            category: None,
                            directory_name: None,
                        },
                        actor,
                    )?;
                    report.materializations.push(RuntimeSkillMaterialization {
                        skill_name: spec.name.clone(),
                        action: RuntimeSkillMaterializationAction::Created,
                        source_kind: spec.source_kind,
                        source_path: spec.source_path.clone(),
                        detail: None,
                    });
                }
                Err(error) => return Err(error),
            }
        }

        Ok(report)
    }

    pub fn edit_skill(
        &self,
        req: EditSkillRequest,
        actor: &str,
    ) -> Result<SkillGovernedWriteResult, SkillError> {
        let current = self
            .skill_authority
            .resolve_skill_for_inspection(&req.name, None)?;
        let next_name = crate::write::parse_skill_document(&req.content)
            .ok()
            .and_then(|document| {
                crate::write::read_frontmatter_value(&document.frontmatter_lines, "name")
            })
            .unwrap_or_else(|| current.name.clone());
        let duplicate_conflict = !next_name.eq_ignore_ascii_case(&current.name)
            && self
                .skill_authority
                .discover_skills()
                .iter()
                .any(|skill| skill.name.eq_ignore_ascii_case(&next_name));
        let guard_report = self.apply_guard_report(
            actor,
            None,
            self.evaluate_edit_guard_report(&current, &req, &next_name, duplicate_conflict),
        )?;
        let result = self.skill_authority.edit_skill(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        self.record_skill_write_action(
            &result.skill_name,
            Some(&current.name),
            SkillWriteLedgerAction::Edit,
            SkillOperationalSourceScope::WorkspaceLocal,
            Some(result.location.as_path()),
            result.supporting_file.as_deref(),
        )?;
        Ok(SkillGovernedWriteResult {
            result,
            guard_report,
        })
    }

    pub fn write_supporting_file(
        &self,
        req: WriteSkillFileRequest,
        actor: &str,
    ) -> Result<SkillGovernedWriteResult, SkillError> {
        let guard_report = self.apply_guard_report(
            actor,
            None,
            self.guard_engine.evaluate_supporting_file(
                &req.name,
                &req.file_path,
                &req.content,
                now_unix_timestamp(),
            ),
        )?;
        let result = self.skill_authority.write_supporting_file(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        self.record_skill_write_action(
            &result.skill_name,
            None,
            SkillWriteLedgerAction::WriteFile,
            SkillOperationalSourceScope::WorkspaceLocal,
            Some(result.location.as_path()),
            result.supporting_file.as_deref(),
        )?;
        Ok(SkillGovernedWriteResult {
            result,
            guard_report,
        })
    }

    pub fn remove_supporting_file(
        &self,
        req: RemoveSkillFileRequest,
        actor: &str,
    ) -> Result<SkillGovernedWriteResult, SkillError> {
        let result = self.skill_authority.remove_supporting_file(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        self.record_skill_write_action(
            &result.skill_name,
            None,
            SkillWriteLedgerAction::RemoveFile,
            SkillOperationalSourceScope::WorkspaceLocal,
            Some(result.location.as_path()),
            result.supporting_file.as_deref(),
        )?;
        Ok(SkillGovernedWriteResult {
            result,
            guard_report: None,
        })
    }

    pub fn delete_skill(
        &self,
        req: DeleteSkillRequest,
        actor: &str,
    ) -> Result<SkillGovernedWriteResult, SkillError> {
        let result = self.skill_authority.delete_skill(req)?;
        self.append_audit_event(write_audit_event(
            audit_kind_for_write_action(&result.action),
            actor,
            &result,
            None,
        ))?;
        self.record_skill_write_action(
            &result.skill_name,
            None,
            SkillWriteLedgerAction::Delete,
            SkillOperationalSourceScope::WorkspaceLocal,
            Some(result.location.as_path()),
            result.supporting_file.as_deref(),
        )?;
        Ok(SkillGovernedWriteResult {
            result,
            guard_report: None,
        })
    }

    pub fn plan_sync(&self, source: &SkillSourceRef) -> Result<SkillSyncPlan, SkillError> {
        let source_snapshot = self.build_source_snapshot(source)?;
        self.hub_store
            .upsert_source_index(self.sync_planner.source_index_snapshot(&source_snapshot))?;

        let catalog = self.skill_authority.list_skill_catalog(None)?;
        let resolved = self.sync_planner.refresh_managed_records(
            &self.hub_store.managed_skills(),
            &catalog,
            Some(&source_snapshot),
        )?;
        self.hub_store.replace_managed_skills(
            resolved
                .iter()
                .map(|record| record.record.clone())
                .collect::<Vec<_>>(),
        )?;

        let plan = self
            .sync_planner
            .plan_sync(&source_snapshot, &resolved, &catalog);
        self.append_audit_event(sync_audit_event(
            SkillAuditKind::SyncPlanCreated,
            source,
            "authority:skill_sync_plan",
            &plan,
        ))?;
        Ok(plan)
    }

    pub fn run_guard_for_skill(
        &self,
        skill_name: &str,
        actor: &str,
    ) -> Result<Vec<SkillGuardReport>, SkillError> {
        let meta = self
            .skill_authority
            .resolve_skill_for_inspection(skill_name, None)?;
        let markdown_content = self
            .skill_authority
            .load_skill_source_for_inspection(skill_name, None)?;
        let supporting_files = meta
            .supporting_files
            .iter()
            .map(|file| {
                let content = std::fs::read_to_string(&file.location).map_err(|error| {
                    SkillError::ReadFailed {
                        path: file.location.clone(),
                        message: error.to_string(),
                    }
                })?;
                Ok((file.relative_path.clone(), content))
            })
            .collect::<Result<Vec<_>, SkillError>>()?;

        let report = self.guard_engine.evaluate_imported_skill(
            &meta.name,
            &markdown_content,
            &supporting_files,
            false,
            now_unix_timestamp(),
        );
        let report = self.with_semantic_overlap_guard_warnings(
            report,
            Some(markdown_content.as_str()),
            meta.category.as_deref(),
            &[],
            Some(meta.name.as_str()),
        );
        self.audit_guard_observation(actor, None, &report)?;
        Ok(vec![report])
    }

    pub fn run_guard_for_source(
        &self,
        source: &SkillSourceRef,
        actor: &str,
    ) -> Result<Vec<SkillGuardReport>, SkillError> {
        let source_snapshot = self.build_source_snapshot(source)?;
        self.hub_store
            .upsert_source_index(self.sync_planner.source_index_snapshot(&source_snapshot))?;

        let catalog = self.skill_authority.list_skill_catalog(None)?;
        let resolved = self.sync_planner.refresh_managed_records(
            &self.hub_store.managed_skills(),
            &catalog,
            Some(&source_snapshot),
        )?;
        let catalog_by_name = catalog
            .iter()
            .map(|meta| (normalize_name(&meta.name), meta))
            .collect::<BTreeMap<_, _>>();
        let managed_by_name = resolved
            .iter()
            .filter(|record| {
                record
                    .record
                    .source
                    .as_ref()
                    .map(|managed_source| managed_source.source_id == source.source_id)
                    .unwrap_or(false)
            })
            .map(|record| (normalize_name(&record.record.skill_name), record))
            .collect::<BTreeMap<_, _>>();

        let mut reports = Vec::new();
        for entry in &source_snapshot.entries {
            let normalized_name = normalize_name(&entry.skill_name);
            let duplicate_conflict = catalog_by_name.contains_key(&normalized_name)
                && !managed_by_name.contains_key(&normalized_name);
            let report = self.guard_engine.evaluate_imported_skill(
                &entry.skill_name,
                &entry.markdown_content,
                &entry
                    .supporting_files
                    .iter()
                    .map(|file| (file.relative_path.clone(), file.content.clone()))
                    .collect::<Vec<_>>(),
                duplicate_conflict,
                now_unix_timestamp(),
            );
            let report = self.with_semantic_overlap_guard_warnings(
                report,
                Some(entry.markdown_content.as_str()),
                entry.category.as_deref(),
                &[],
                None,
            );
            self.audit_guard_observation(actor, Some(source), &report)?;
            reports.push(report);
        }
        Ok(reports)
    }

    pub fn apply_sync(
        &self,
        source: &SkillSourceRef,
        actor: &str,
    ) -> Result<SkillGovernedSyncResult, SkillError> {
        let source_snapshot = self.build_source_snapshot(source)?;
        self.hub_store
            .upsert_source_index(self.sync_planner.source_index_snapshot(&source_snapshot))?;

        let catalog = self.skill_authority.list_skill_catalog(None)?;
        let resolved = self.sync_planner.refresh_managed_records(
            &self.hub_store.managed_skills(),
            &catalog,
            Some(&source_snapshot),
        )?;
        let plan = self
            .sync_planner
            .plan_sync(&source_snapshot, &resolved, &catalog);

        let source_entries = source_snapshot
            .entries
            .iter()
            .map(|entry| (normalize_name(&entry.skill_name), entry))
            .collect::<BTreeMap<_, _>>();
        let resolved_managed = resolved
            .iter()
            .map(|record| (normalize_name(&record.record.skill_name), record))
            .collect::<BTreeMap<_, _>>();
        let catalog_by_name = catalog
            .iter()
            .map(|meta| (normalize_name(&meta.name), meta))
            .collect::<BTreeMap<_, _>>();
        let mut guard_reports = Vec::new();

        for plan_entry in &plan.entries {
            let normalized_name = normalize_name(&plan_entry.skill_name);
            let source_entry = source_entries.get(&normalized_name).copied();
            let managed_record = resolved_managed.get(&normalized_name).copied();
            let catalog_entry = catalog_by_name.get(&normalized_name).copied();

            match plan_entry.action {
                SkillSyncAction::Install => {
                    let source_entry =
                        source_entry.ok_or_else(|| SkillError::InvalidSkillContent {
                            message: format!(
                                "sync plan for `{}` was missing source content",
                                plan_entry.skill_name
                            ),
                        })?;
                    if let Some(report) =
                        self.apply_import_guard(actor, source, source_entry, false, None)?
                    {
                        guard_reports.push(report);
                    }
                    let result = self.install_skill_from_source(source_entry)?;
                    self.append_audit_event(write_audit_event(
                        SkillAuditKind::HubInstall,
                        actor,
                        &result,
                        Some(source),
                    ))?;
                    self.hub_store
                        .upsert_managed_skill(self.synced_managed_record(source, source_entry)?)?;
                    self.record_skill_write_action(
                        &result.skill_name,
                        None,
                        SkillWriteLedgerAction::Install,
                        SkillOperationalSourceScope::Managed,
                        Some(result.location.as_path()),
                        result.supporting_file.as_deref(),
                    )?;
                }
                SkillSyncAction::Update => {
                    let source_entry =
                        source_entry.ok_or_else(|| SkillError::InvalidSkillContent {
                            message: format!(
                                "sync plan for `{}` was missing source content",
                                plan_entry.skill_name
                            ),
                        })?;
                    if let Some(report) = self.apply_import_guard(
                        actor,
                        source,
                        source_entry,
                        false,
                        Some(plan_entry.skill_name.as_str()),
                    )? {
                        guard_reports.push(report);
                    }
                    let result = self.update_skill_from_source(source_entry, catalog_entry)?;
                    self.append_audit_event(write_audit_event(
                        SkillAuditKind::HubUpdate,
                        actor,
                        &result,
                        Some(source),
                    ))?;
                    self.hub_store
                        .upsert_managed_skill(self.synced_managed_record(source, source_entry)?)?;
                    self.record_skill_write_action(
                        &result.skill_name,
                        Some(&plan_entry.skill_name),
                        SkillWriteLedgerAction::Update,
                        SkillOperationalSourceScope::Managed,
                        Some(result.location.as_path()),
                        result.supporting_file.as_deref(),
                    )?;
                }
                SkillSyncAction::SkipLocalModification => {
                    if let Some(managed_record) = managed_record {
                        let mut next_record = managed_record.record.clone();
                        next_record.locally_modified = true;
                        next_record.deleted_locally = false;
                        self.hub_store.upsert_managed_skill(next_record)?;
                    }
                }
                SkillSyncAction::SkipDeletedLocally => {
                    if let Some(managed_record) = managed_record {
                        let mut next_record = managed_record.record.clone();
                        next_record.deleted_locally = true;
                        next_record.locally_modified = false;
                        self.hub_store.upsert_managed_skill(next_record)?;
                    }
                }
                SkillSyncAction::RemoveManaged => {
                    if let Some(managed_record) = managed_record {
                        let mut deleted_from_workspace = false;
                        if let Some(current_hash) = managed_record.current_hash.as_deref() {
                            if managed_record.record.local_hash.as_deref() == Some(current_hash) {
                                self.skill_authority.delete_skill(DeleteSkillRequest {
                                    name: managed_record.record.skill_name.clone(),
                                })?;
                                deleted_from_workspace = true;
                            }
                        }
                        self.hub_store
                            .remove_managed_skill(&managed_record.record.skill_name)?;
                        self.append_audit_event(hub_remove_audit_event(
                            source,
                            actor,
                            &managed_record.record,
                            deleted_from_workspace,
                        ))?;
                        self.record_skill_write_action(
                            &managed_record.record.skill_name,
                            None,
                            SkillWriteLedgerAction::Remove,
                            SkillOperationalSourceScope::Managed,
                            None,
                            None,
                        )?;
                    }
                }
                SkillSyncAction::Noop => {
                    if let (Some(_managed_record), Some(source_entry)) =
                        (managed_record, source_entry)
                    {
                        self.hub_store.upsert_managed_skill(
                            self.synced_managed_record(source, source_entry)?,
                        )?;
                    }
                }
            }
        }

        self.refresh_managed_workspace_state()?;
        self.append_audit_event(sync_audit_event(
            SkillAuditKind::SyncApplyCompleted,
            source,
            actor,
            &plan,
        ))?;
        Ok(SkillGovernedSyncResult {
            plan,
            guard_reports,
        })
    }

    fn build_source_snapshot(
        &self,
        source: &SkillSourceRef,
    ) -> Result<crate::sync::SkillSyncSourceSnapshot, SkillError> {
        if !crate::sync::source_root_kind_supported(source) {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "unsupported skill source kind for sync: {:?}",
                    source.source_kind
                ),
            });
        }

        let root = self.resolve_source_root(&source.locator);
        if !root.exists() {
            return Err(SkillError::ReadFailed {
                path: root,
                message: "sync source root does not exist".to_string(),
            });
        }

        match source.source_kind {
            agendao_types::SkillSourceKind::Bundled => {
                let manifest =
                    self.hub_store
                        .bundled_manifest()
                        .ok_or_else(|| SkillError::ReadFailed {
                            path: self.hub_store.bundled_manifest_path(),
                            message: "missing bundled manifest for bundled sync source".to_string(),
                        })?;
                self.sync_planner
                    .build_bundled_source_snapshot(source, &root, &manifest)
            }
            agendao_types::SkillSourceKind::LocalPath => {
                self.sync_planner.build_local_source_snapshot(source, &root)
            }
            _ => Err(SkillError::InvalidSkillContent {
                message: format!(
                    "unsupported skill source kind for sync: {:?}",
                    source.source_kind
                ),
            }),
        }
    }

    fn resolve_source_root(&self, locator: &str) -> PathBuf {
        let path = PathBuf::from(locator);
        if path.is_absolute() {
            path
        } else {
            self.hub_store.base_dir().join(path)
        }
    }

    fn evaluate_create_guard_report(
        &self,
        req: &CreateSkillRequest,
        duplicate_conflict: bool,
    ) -> SkillGuardReport {
        let report = self.guard_engine.evaluate_create(
            &req.name,
            &req.description,
            &req.body,
            duplicate_conflict,
            now_unix_timestamp(),
        );
        let preview_markdown = crate::write::build_create_frontmatter(
            &req.name,
            &req.description,
            req.frontmatter.as_ref(),
        )
        .and_then(|frontmatter| crate::write::build_skill_document(&frontmatter, &req.body))
        .ok();
        self.with_semantic_overlap_guard_warnings(
            report,
            preview_markdown.as_deref(),
            req.category.as_deref(),
            &[],
            None,
        )
    }

    fn evaluate_patch_guard_report(
        &self,
        current: &crate::SkillMeta,
        req: &PatchSkillRequest,
        next_name: &str,
        duplicate_conflict: bool,
    ) -> SkillGuardReport {
        let report = self.guard_engine.evaluate_patch(
            &current.name,
            next_name,
            req.body.as_deref(),
            duplicate_conflict,
            now_unix_timestamp(),
        );
        let preview_markdown = self.build_patch_preview_markdown(current, req).ok();
        self.with_semantic_overlap_guard_warnings(
            report,
            preview_markdown.as_deref(),
            current.category.as_deref(),
            &[current.name.as_str()],
            Some(current.name.as_str()),
        )
    }

    fn evaluate_edit_guard_report(
        &self,
        current: &crate::SkillMeta,
        req: &EditSkillRequest,
        next_name: &str,
        duplicate_conflict: bool,
    ) -> SkillGuardReport {
        let report = self.guard_engine.evaluate_edit(
            next_name,
            &req.content,
            duplicate_conflict,
            now_unix_timestamp(),
        );
        self.with_semantic_overlap_guard_warnings(
            report,
            Some(req.content.as_str()),
            current.category.as_deref(),
            &[current.name.as_str()],
            Some(current.name.as_str()),
        )
    }

    fn evaluate_imported_skill_guard_report(
        &self,
        skill_name: &str,
        markdown_content: &str,
        supporting_files: &[(String, String)],
        duplicate_conflict: bool,
        category: Option<&str>,
        current_skill_name: Option<&str>,
    ) -> SkillGuardReport {
        let report = self.guard_engine.evaluate_imported_skill(
            skill_name,
            markdown_content,
            supporting_files,
            duplicate_conflict,
            now_unix_timestamp(),
        );
        let exclude_names = current_skill_name
            .map(|value| vec![value])
            .unwrap_or_default();
        self.with_semantic_overlap_guard_warnings(
            report,
            Some(markdown_content),
            category,
            exclude_names.as_slice(),
            current_skill_name,
        )
    }

    fn build_patch_preview_markdown(
        &self,
        current: &crate::SkillMeta,
        req: &PatchSkillRequest,
    ) -> Result<String, SkillError> {
        let mut document = crate::write::load_skill_document(&current.location)?;
        let mut frontmatter = crate::write::parse_skill_frontmatter(&document)?;
        let next_name = match req.new_name.as_deref() {
            Some(value) => crate::write::validate_skill_name(value)?,
            None => current.name.clone(),
        };
        let next_description = match req.description.as_deref() {
            Some(value) => crate::write::validate_skill_description(&next_name, value)?,
            None => current.description.clone(),
        };
        let next_body = match req.body.as_deref() {
            Some(value) => crate::write::validate_skill_body(value)?,
            None => document.body.clone(),
        };

        frontmatter.name = next_name;
        frontmatter.description = next_description;
        if let Some(patch) = req.frontmatter.as_ref() {
            crate::write::apply_frontmatter_patch(&mut frontmatter, patch);
        }
        document.frontmatter_lines = crate::write::render_skill_frontmatter_lines(&frontmatter)?;
        document.body = next_body;
        Ok(crate::write::render_skill_document(&document))
    }

    fn with_semantic_overlap_guard_warnings(
        &self,
        mut report: SkillGuardReport,
        markdown_content: Option<&str>,
        category: Option<&str>,
        exclude_names: &[&str],
        current_skill_name: Option<&str>,
    ) -> SkillGuardReport {
        let Some(markdown_content) = markdown_content else {
            return report;
        };
        let Some(candidate) = self.semantic_descriptor_from_markdown(markdown_content, category)
        else {
            return report;
        };
        let violations =
            self.semantic_overlap_guard_violations(&candidate, exclude_names, current_skill_name);
        if violations.is_empty() {
            return report;
        }
        if report.status == SkillGuardStatus::Passed {
            report.status = SkillGuardStatus::Warn;
        }
        report.violations.extend(violations);
        report
    }

    fn semantic_descriptor_from_markdown(
        &self,
        markdown_content: &str,
        category: Option<&str>,
    ) -> Option<SkillSemanticDescriptor> {
        let document = crate::write::parse_skill_document(markdown_content).ok()?;
        let frontmatter = crate::write::parse_skill_frontmatter(&document).ok()?;
        let agendao = frontmatter
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.agendao.as_ref());
        let detail = SkillDetailView {
            tags: semantic_detail_tags(&frontmatter),
            related_skills: semantic_detail_related_skills(&frontmatter),
            ..SkillDetailView::default()
        };
        let conditions = SkillConditions {
            requires_tools: agendao
                .map(|metadata| metadata.requires_tools.clone())
                .unwrap_or_default(),
            fallback_for_tools: agendao
                .map(|metadata| metadata.fallback_for_tools.clone())
                .unwrap_or_default(),
            requires_toolsets: agendao
                .map(|metadata| metadata.requires_toolsets.clone())
                .unwrap_or_default(),
            fallback_for_toolsets: agendao
                .map(|metadata| metadata.fallback_for_toolsets.clone())
                .unwrap_or_default(),
            stage_filter: agendao
                .map(|metadata| metadata.stage_filter.clone())
                .unwrap_or_default(),
        };
        Some(build_skill_semantic_descriptor_from_parts(
            &frontmatter.name,
            &frontmatter.description,
            category,
            &conditions,
            &detail,
        ))
    }

    fn semantic_overlap_guard_violations(
        &self,
        candidate: &SkillSemanticDescriptor,
        exclude_names: &[&str],
        current_skill_name: Option<&str>,
    ) -> Vec<SkillGuardViolation> {
        let snapshot_by_name = self
            .skill_operational_snapshots()
            .into_iter()
            .map(|snapshot| (normalize_name(&snapshot.skill_name), snapshot))
            .collect::<BTreeMap<_, _>>();
        let exclude = exclude_names
            .iter()
            .map(|name| normalize_name(name))
            .collect::<BTreeSet<_>>();
        let candidate_snapshot = current_skill_name.and_then(|name| {
            let normalized = normalize_name(name);
            snapshot_by_name.get(&normalized)
        });
        let mut conflicts = self
            .skill_authority
            .list_skill_catalog(None)
            .unwrap_or_default()
            .into_iter()
            .filter(|meta| !exclude.contains(&normalize_name(&meta.name)))
            .filter_map(|meta| {
                let existing = self
                    .build_skill_semantic_descriptor(&meta, &snapshot_by_name)
                    .ok()?;
                build_skill_semantic_conflict(
                    candidate,
                    &existing,
                    candidate_snapshot,
                    snapshot_by_name.get(&normalize_name(&existing.skill_name)),
                )
            })
            .collect::<Vec<_>>();
        conflicts.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.left_skill_name.cmp(&right.left_skill_name))
                .then_with(|| left.right_skill_name.cmp(&right.right_skill_name))
        });
        conflicts
            .into_iter()
            .take(3)
            .map(|conflict| semantic_conflict_guard_violation(candidate, &conflict))
            .collect()
    }

    fn apply_guard_report(
        &self,
        actor: &str,
        source: Option<&SkillSourceRef>,
        report: SkillGuardReport,
    ) -> Result<Option<SkillGuardReport>, SkillError> {
        if report.violations.is_empty() {
            return Ok(None);
        }

        self.audit_guard_observation(actor, source, &report)?;
        let blocked = report.status == SkillGuardStatus::Blocked;
        if blocked {
            return Err(SkillError::GuardBlocked { report });
        }
        Ok(Some(report))
    }

    fn apply_import_guard(
        &self,
        actor: &str,
        source: &SkillSourceRef,
        entry: &crate::sync::SkillSyncSourceEntry,
        duplicate_conflict: bool,
        current_skill_name: Option<&str>,
    ) -> Result<Option<SkillGuardReport>, SkillError> {
        let report = self.evaluate_imported_skill_guard_report(
            &entry.skill_name,
            &entry.markdown_content,
            &entry
                .supporting_files
                .iter()
                .map(|file| (file.relative_path.clone(), file.content.clone()))
                .collect::<Vec<_>>(),
            duplicate_conflict,
            entry.category.as_deref(),
            current_skill_name,
        );
        self.apply_guard_report(actor, Some(source), report)
    }

    fn audit_guard_observation(
        &self,
        actor: &str,
        source: Option<&SkillSourceRef>,
        report: &SkillGuardReport,
    ) -> Result<(), SkillError> {
        if report.violations.is_empty() {
            return Ok(());
        }
        self.append_audit_event(guard_audit_event(
            if report.status == SkillGuardStatus::Blocked {
                SkillAuditKind::GuardBlocked
            } else {
                SkillAuditKind::GuardWarned
            },
            source,
            actor,
            report,
        ))
    }

    fn install_skill_from_source(
        &self,
        entry: &crate::sync::SkillSyncSourceEntry,
    ) -> Result<SkillWriteResult, SkillError> {
        let (category, directory_name) = create_target_from_relative_path(&entry.relative_path)?;
        let result = self.skill_authority.create_skill(CreateSkillRequest {
            name: entry.skill_name.clone(),
            description: entry.description.clone(),
            body: entry.body.clone(),
            frontmatter: None,
            category,
            directory_name,
        })?;
        self.sync_supporting_files(entry, None)?;
        Ok(result)
    }

    fn update_skill_from_source(
        &self,
        entry: &crate::sync::SkillSyncSourceEntry,
        existing: Option<&crate::SkillMeta>,
    ) -> Result<SkillWriteResult, SkillError> {
        let result = self.skill_authority.edit_skill(EditSkillRequest {
            name: entry.skill_name.clone(),
            content: entry.markdown_content.clone(),
        })?;
        self.sync_supporting_files(entry, existing)?;
        Ok(result)
    }

    fn sync_supporting_files(
        &self,
        entry: &crate::sync::SkillSyncSourceEntry,
        existing: Option<&crate::SkillMeta>,
    ) -> Result<(), SkillError> {
        let source_files = entry
            .supporting_files
            .iter()
            .map(|file| (file.relative_path.as_str(), file))
            .collect::<BTreeMap<_, _>>();

        if let Some(existing) = existing {
            for file in &existing.supporting_files {
                if !source_files.contains_key(file.relative_path.as_str()) {
                    self.skill_authority
                        .remove_supporting_file(RemoveSkillFileRequest {
                            name: entry.skill_name.clone(),
                            file_path: file.relative_path.clone(),
                        })?;
                }
            }
        }

        for source_file in &entry.supporting_files {
            self.skill_authority
                .write_supporting_file(WriteSkillFileRequest {
                    name: entry.skill_name.clone(),
                    file_path: source_file.relative_path.clone(),
                    content: source_file.content.clone(),
                })?;
        }
        Ok(())
    }

    fn sync_remote_supporting_files(
        &self,
        package: &crate::artifact::SkillArtifactPackage,
    ) -> Result<(), SkillError> {
        let existing = self
            .skill_authority
            .resolve_skill_for_inspection(&package.skill_name, None)
            .ok();
        let source_files = package
            .supporting_files
            .iter()
            .map(|file| (file.relative_path.as_str(), file))
            .collect::<BTreeMap<_, _>>();

        if let Some(existing) = existing.as_ref() {
            for file in &existing.supporting_files {
                if !source_files.contains_key(file.relative_path.as_str()) {
                    self.skill_authority
                        .remove_supporting_file(RemoveSkillFileRequest {
                            name: package.skill_name.clone(),
                            file_path: file.relative_path.clone(),
                        })?;
                }
            }
        }

        for file in &package.supporting_files {
            self.skill_authority
                .write_supporting_file(WriteSkillFileRequest {
                    name: package.skill_name.clone(),
                    file_path: file.relative_path.clone(),
                    content: file.content.clone(),
                })?;
        }
        Ok(())
    }

    fn remote_install_action(
        &self,
        distribution: &SkillDistributionRecord,
    ) -> Result<SkillRemoteInstallAction, SkillError> {
        match self
            .hub_store
            .managed_skill(&distribution.skill_name)
            .filter(|record| {
                record
                    .source
                    .as_ref()
                    .map(|source| source.source_id == distribution.source.source_id)
                    .unwrap_or(false)
            }) {
            Some(_) => Ok(SkillRemoteInstallAction::Update),
            None => {
                if self
                    .skill_authority
                    .discover_skills()
                    .iter()
                    .any(|skill| skill.name.eq_ignore_ascii_case(&distribution.skill_name))
                {
                    return Err(SkillError::InvalidSkillContent {
                        message: format!(
                            "skill `{}` already exists in workspace and is not managed by source `{}`",
                            distribution.skill_name, distribution.source.source_id
                        ),
                    });
                }
                Ok(SkillRemoteInstallAction::Install)
            }
        }
    }

    fn apply_remote_plan(
        &self,
        source: &SkillSourceRef,
        actor: &str,
        plan: SkillRemoteInstallPlan,
    ) -> Result<SkillRemoteInstallResponse, SkillError> {
        let plan_for_apply = plan.clone();
        let artifact_cache =
            self.fetch_distribution_artifact(&plan.distribution.distribution_id, actor)?;
        let apply = (|| -> Result<SkillRemoteInstallResponse, SkillError> {
            let package = self.artifact_store.load_package(&artifact_cache)?;
            if !package
                .skill_name
                .eq_ignore_ascii_case(&plan_for_apply.distribution.skill_name)
            {
                return Err(SkillError::InvalidSkillContent {
                    message: format!(
                        "artifact package resolved `{}` but distribution expected `{}`",
                        package.skill_name, plan_for_apply.distribution.skill_name
                    ),
                });
            }

            let duplicate_conflict =
                matches!(
                    plan_for_apply.entry.action,
                    SkillRemoteInstallAction::Install
                ) && self.skill_authority.discover_skills().iter().any(|skill| {
                    skill
                        .name
                        .eq_ignore_ascii_case(&plan_for_apply.distribution.skill_name)
                });
            let current_meta = if matches!(
                plan_for_apply.entry.action,
                SkillRemoteInstallAction::Update
            ) {
                self.skill_authority
                    .resolve_skill_for_inspection(&package.skill_name, None)
                    .ok()
            } else {
                None
            };
            let guard_report = self.apply_guard_report(
                actor,
                Some(source),
                self.evaluate_imported_skill_guard_report(
                    &package.skill_name,
                    &package.markdown_content(),
                    &package
                        .supporting_files
                        .iter()
                        .map(|file| (file.relative_path.clone(), file.content.clone()))
                        .collect::<Vec<_>>(),
                    duplicate_conflict,
                    package.category.as_deref().or(current_meta
                        .as_ref()
                        .and_then(|meta| meta.category.as_deref())),
                    current_meta.as_ref().map(|meta| meta.name.as_str()),
                ),
            )?;

            let result = match plan_for_apply.entry.action {
                SkillRemoteInstallAction::Install => {
                    self.skill_authority.create_skill(CreateSkillRequest {
                        name: package.skill_name.clone(),
                        description: package.description.clone(),
                        body: package.body.clone().unwrap_or_else(|| {
                            extract_body_from_markdown(&package.markdown_content())
                        }),
                        frontmatter: None,
                        category: package.category.clone(),
                        directory_name: package.directory_name.clone(),
                    })?
                }
                SkillRemoteInstallAction::Update => {
                    self.skill_authority.edit_skill(EditSkillRequest {
                        name: package.skill_name.clone(),
                        content: package.markdown_content(),
                    })?
                }
            };
            self.sync_remote_supporting_files(&package)?;

            let resolved_meta = self
                .skill_authority
                .resolve_skill_for_inspection(&package.skill_name, None)?;
            let local_hash = crate::sync::hash_skill_meta(&resolved_meta)?;
            let installed_at = now_unix_timestamp();
            let mut distribution = plan_for_apply.distribution.clone();
            distribution.installed = Some(agendao_types::SkillInstalledDistribution {
                installed_at,
                workspace_skill_path: resolved_meta.location.to_string_lossy().to_string(),
                installed_revision: distribution.release.revision.clone(),
                local_hash: Some(local_hash.clone()),
            });
            distribution.lifecycle = SkillManagedLifecycleState::Installed;
            self.upsert_distribution(distribution.clone())?;
            self.upsert_managed_skill(ManagedSkillRecord {
                skill_name: package.skill_name.clone(),
                source: Some(source.clone()),
                installed_revision: release_identity(&distribution.release).map(ToOwned::to_owned),
                local_hash: Some(local_hash),
                last_synced_at: Some(installed_at),
                locally_modified: false,
                deleted_locally: false,
            })?;
            self.record_lifecycle(
                Some(actor),
                self.lifecycle.build_record(
                    distribution.distribution_id.clone(),
                    source.source_id.clone(),
                    distribution.skill_name.clone(),
                    SkillManagedLifecycleState::Installed,
                    installed_at,
                    None,
                ),
            )?;
            self.append_audit_event(write_audit_event(
                match plan_for_apply.entry.action {
                    SkillRemoteInstallAction::Install => SkillAuditKind::HubInstall,
                    SkillRemoteInstallAction::Update => SkillAuditKind::HubUpdate,
                },
                actor,
                &result,
                Some(source),
            ))?;
            self.record_skill_write_action(
                &result.skill_name,
                None,
                match plan_for_apply.entry.action {
                    SkillRemoteInstallAction::Install => SkillWriteLedgerAction::Install,
                    SkillRemoteInstallAction::Update => SkillWriteLedgerAction::Update,
                },
                SkillOperationalSourceScope::Managed,
                Some(result.location.as_path()),
                result.supporting_file.as_deref(),
            )?;

            Ok(SkillRemoteInstallResponse {
                plan: plan_for_apply.clone(),
                artifact_cache,
                guard_report,
                result: governance_write_result(&result),
            })
        })();

        match apply {
            Ok(response) => Ok(response),
            Err(error) => {
                self.record_lifecycle(
                    Some(actor),
                    self.lifecycle.build_record(
                        plan.distribution.distribution_id.clone(),
                        source.source_id.clone(),
                        plan.distribution.skill_name.clone(),
                        SkillManagedLifecycleState::ApplyFailed,
                        now_unix_timestamp(),
                        Some(error.to_string()),
                    ),
                )?;
                Err(error)
            }
        }
    }

    fn record_lifecycle(
        &self,
        actor: Option<&str>,
        record: SkillManagedLifecycleRecord,
    ) -> Result<(), SkillError> {
        let previous = self
            .lifecycle_records()
            .into_iter()
            .find(|entry| entry.distribution_id == record.distribution_id);
        let changed = previous
            .as_ref()
            .map(|entry| entry.state != record.state || entry.error != record.error)
            .unwrap_or(true);
        self.upsert_lifecycle_record(record.clone())?;
        if changed {
            if let Some(actor) = actor {
                self.append_audit_event(lifecycle_transition_audit_event(
                    actor,
                    previous.as_ref(),
                    &record,
                ))?;
            }
        }
        Ok(())
    }

    fn refresh_managed_record_for_source_skill(
        &self,
        source: &SkillSourceRef,
        skill_name: &str,
    ) -> Result<crate::sync::ResolvedManagedSkillRecord, SkillError> {
        let catalog = self.skill_authority.list_skill_catalog(None)?;
        let resolved = self.sync_planner.refresh_managed_records(
            &self.hub_store.managed_skills(),
            &catalog,
            None,
        )?;
        let records = resolved
            .iter()
            .map(|record| record.record.clone())
            .collect::<Vec<_>>();
        self.hub_store.replace_managed_skills(records)?;
        self.update_distribution_runtime_state(&resolved)?;
        resolved
            .into_iter()
            .find(|record| {
                record.record.skill_name.eq_ignore_ascii_case(skill_name)
                    && record
                        .record
                        .source
                        .as_ref()
                        .map(|managed_source| managed_source.source_id == source.source_id)
                        .unwrap_or(false)
            })
            .ok_or_else(|| SkillError::InvalidSkillContent {
                message: format!(
                    "skill `{}` is not managed by source `{}`",
                    skill_name.trim(),
                    source.source_id
                ),
            })
    }

    fn update_distribution_runtime_state(
        &self,
        managed_records: &[crate::sync::ResolvedManagedSkillRecord],
    ) -> Result<(), SkillError> {
        let mut distributions = self.distributions();
        let mut touched = BTreeMap::<String, SkillManagedLifecycleRecord>::new();
        for distribution in &mut distributions {
            let Some(managed_record) = managed_records.iter().find(|record| {
                record
                    .record
                    .skill_name
                    .eq_ignore_ascii_case(&distribution.skill_name)
                    && record
                        .record
                        .source
                        .as_ref()
                        .map(|source| source.source_id == distribution.source.source_id)
                        .unwrap_or(false)
            }) else {
                continue;
            };

            let next_state = self.lifecycle.managed_runtime_state(
                &managed_record.record,
                release_identity(&distribution.release),
            );
            distribution.lifecycle = next_state.clone();
            touched.insert(
                distribution.distribution_id.clone(),
                self.lifecycle.build_record(
                    distribution.distribution_id.clone(),
                    distribution.source.source_id.clone(),
                    distribution.skill_name.clone(),
                    next_state,
                    now_unix_timestamp(),
                    None,
                ),
            );
        }

        for distribution in distributions {
            self.upsert_distribution(distribution)?;
        }
        for record in managed_records {
            let distribution_id = self
                .current_distribution_for_managed_record(&record.record)
                .map(|distribution| distribution.distribution_id)
                .unwrap_or_else(|| {
                    unresolved_distribution_id(
                        record
                            .record
                            .source
                            .as_ref()
                            .expect("managed record source must exist"),
                        &record.record.skill_name,
                    )
                });
            touched.entry(distribution_id.clone()).or_insert_with(|| {
                self.lifecycle.build_record(
                    distribution_id,
                    record
                        .record
                        .source
                        .as_ref()
                        .expect("managed record source must exist")
                        .source_id
                        .clone(),
                    record.record.skill_name.clone(),
                    self.lifecycle.managed_runtime_state(&record.record, None),
                    now_unix_timestamp(),
                    None,
                )
            });
        }
        for lifecycle in touched.into_values() {
            self.upsert_lifecycle_record(lifecycle)?;
        }
        Ok(())
    }

    fn current_distribution_for_managed_record(
        &self,
        record: &ManagedSkillRecord,
    ) -> Option<SkillDistributionRecord> {
        let source_id = record.source.as_ref()?.source_id.as_str();
        let installed_revision = record.installed_revision.as_deref();
        let mut candidates = self
            .distributions()
            .into_iter()
            .filter(|distribution| {
                distribution.source.source_id == source_id
                    && distribution
                        .skill_name
                        .eq_ignore_ascii_case(&record.skill_name)
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            left.resolution
                .resolved_at
                .cmp(&right.resolution.resolved_at)
                .then_with(|| left.distribution_id.cmp(&right.distribution_id))
        });
        candidates
            .iter()
            .find(|distribution| {
                release_identity(&distribution.release) == installed_revision
                    || distribution
                        .installed
                        .as_ref()
                        .and_then(|installed| installed.installed_revision.as_deref())
                        == installed_revision
            })
            .cloned()
            .or_else(|| candidates.pop())
    }

    fn record_skill_write_action(
        &self,
        skill_name: &str,
        previous_skill_name: Option<&str>,
        action: SkillWriteLedgerAction,
        fallback_scope: SkillOperationalSourceScope,
        last_location: Option<&Path>,
        last_supporting_file: Option<&str>,
    ) -> Result<SkillOperationalSnapshot, SkillError> {
        let mut snapshot =
            self.prepare_operational_snapshot(skill_name, previous_skill_name, fallback_scope)?;
        let now = now_unix_timestamp();
        let writes = snapshot
            .writes
            .get_or_insert_with(SkillWriteLedgerEntry::default);
        writes.first_written_at.get_or_insert(now);
        writes.last_write_at = Some(now);
        writes.last_action = Some(action);
        writes.last_location = last_location.map(|path| path.to_string_lossy().to_string());
        writes.last_supporting_file = last_supporting_file.map(ToOwned::to_owned);

        match action {
            SkillWriteLedgerAction::Create => writes.create_count += 1,
            SkillWriteLedgerAction::Patch => writes.patch_count += 1,
            SkillWriteLedgerAction::Edit => writes.edit_count += 1,
            SkillWriteLedgerAction::WriteFile => writes.supporting_file_write_count += 1,
            SkillWriteLedgerAction::RemoveFile => writes.supporting_file_remove_count += 1,
            SkillWriteLedgerAction::Install => writes.install_count += 1,
            SkillWriteLedgerAction::Update => writes.update_count += 1,
            SkillWriteLedgerAction::Detach => writes.detach_count += 1,
            SkillWriteLedgerAction::Remove => writes.remove_count += 1,
            SkillWriteLedgerAction::Delete => writes.delete_count += 1,
        }

        self.hub_store
            .upsert_skill_operational_snapshot(snapshot.clone())?;
        Ok(snapshot)
    }

    fn prepare_operational_snapshot(
        &self,
        skill_name: &str,
        previous_skill_name: Option<&str>,
        fallback_scope: SkillOperationalSourceScope,
    ) -> Result<SkillOperationalSnapshot, SkillError> {
        if let Some(previous_skill_name) =
            previous_skill_name.filter(|previous| !previous.eq_ignore_ascii_case(skill_name))
        {
            self.hub_store
                .rename_skill_operational_snapshot(previous_skill_name, skill_name)?;
        }

        let mut snapshot = self
            .hub_store
            .skill_operational_snapshot(skill_name)
            .unwrap_or_else(|| SkillOperationalSnapshot {
                skill_name: skill_name.to_string(),
                ..SkillOperationalSnapshot::default()
            });
        snapshot.skill_name = skill_name.to_string();

        let (source_scope, source_id) = self.resolve_operational_identity(skill_name);
        if !matches!(source_scope, SkillOperationalSourceScope::Unknown) {
            snapshot.source_scope = source_scope;
            snapshot.source_id = source_id;
        } else if !matches!(fallback_scope, SkillOperationalSourceScope::Unknown) {
            snapshot.source_scope = fallback_scope;
            if !matches!(fallback_scope, SkillOperationalSourceScope::Managed) {
                snapshot.source_id = None;
            }
        }

        Ok(snapshot)
    }

    fn resolve_operational_identity(
        &self,
        skill_name: &str,
    ) -> (SkillOperationalSourceScope, Option<String>) {
        if let Some(managed) = self.hub_store.managed_skill(skill_name) {
            return (
                SkillOperationalSourceScope::Managed,
                managed.source.map(|source| source.source_id),
            );
        }

        match self
            .skill_authority
            .resolve_skill_for_inspection(skill_name, None)
        {
            Ok(meta) => {
                if meta
                    .location
                    .starts_with(self.skill_authority.workspace_skill_root())
                {
                    (SkillOperationalSourceScope::WorkspaceLocal, None)
                } else {
                    (SkillOperationalSourceScope::DiscoveredReadOnly, None)
                }
            }
            Err(_) => (SkillOperationalSourceScope::Unknown, None),
        }
    }

    fn build_skill_semantic_descriptor(
        &self,
        meta: &crate::SkillMeta,
        _snapshot_by_name: &BTreeMap<String, SkillOperationalSnapshot>,
    ) -> Result<SkillSemanticDescriptor, SkillError> {
        let detail = self
            .skill_authority
            .load_skill_detail_for_meta_for_inspection(meta)
            .unwrap_or_default();
        Ok(build_skill_semantic_descriptor_from_parts(
            &meta.name,
            &meta.description,
            meta.category.as_deref(),
            &meta.conditions,
            &detail,
        ))
    }

    fn skill_semantic_analysis_inputs(
        &self,
    ) -> Result<
        (
            Vec<SkillSemanticDescriptor>,
            BTreeMap<String, SkillOperationalSnapshot>,
        ),
        SkillError,
    > {
        let snapshots = self.skill_operational_snapshots();
        let snapshot_by_name = snapshots
            .iter()
            .cloned()
            .map(|snapshot| (normalize_name(&snapshot.skill_name), snapshot))
            .collect::<BTreeMap<_, _>>();
        let catalog = self.skill_authority.list_skill_catalog(None)?;
        let mut descriptors = Vec::with_capacity(catalog.len());
        for meta in &catalog {
            descriptors.push(self.build_skill_semantic_descriptor(meta, &snapshot_by_name)?);
        }
        Ok((descriptors, snapshot_by_name))
    }

    fn resolve_composition_skill_name(&self, skill_name: &str) -> Result<String, SkillError> {
        let requested = required_nonempty_text(skill_name, "skill_name")?;
        Ok(self
            .skill_authority
            .resolve_skill_for_inspection(&requested, None)?
            .name)
    }

    fn set_skill_composition_relationship_state(
        &self,
        left_skill_name: &str,
        right_skill_name: &str,
        relation_kind: agendao_types::SkillRelationshipKind,
        state: agendao_types::SkillRelationshipState,
        preferred_skill_name: Option<&str>,
        actor: &str,
    ) -> Result<SkillRelationshipEdge, SkillError> {
        let left_skill_name = self.resolve_composition_skill_name(left_skill_name)?;
        let right_skill_name = self.resolve_composition_skill_name(right_skill_name)?;
        if left_skill_name.eq_ignore_ascii_case(&right_skill_name) {
            return Err(SkillError::InvalidSkillContent {
                message: "composition relationship requires two distinct skills".to_string(),
            });
        }

        let inspection = self.skill_composition_relationship_inspection()?;
        let mut relationship = inspection
            .into_iter()
            .find(|entry| {
                relationship_edge_identity_key(entry)
                    == relationship_identity_key(&left_skill_name, &right_skill_name, relation_kind)
            })
            .ok_or_else(|| SkillError::InvalidSkillContent {
                message: format!(
                    "no composition relationship candidate exists for `{}` <-> `{}` with kind `{}`",
                    left_skill_name,
                    right_skill_name,
                    format_skill_relationship_kind(relation_kind)
                ),
            })?;
        relationship.state = state;
        relationship.preferred_skill_name = validate_relationship_preferred_skill(
            &left_skill_name,
            &right_skill_name,
            relation_kind,
            relationship.preferred_skill_name.as_deref(),
            preferred_skill_name,
        )?;
        relationship.updated_at = Some(now_unix_timestamp());

        self.upsert_composition_relationship(relationship.clone())?;
        self.append_audit_event(composition_relationship_audit_event(
            &relationship,
            actor,
            state,
        ))?;
        Ok(relationship)
    }

    fn upsert_composition_relationship(
        &self,
        relationship: SkillRelationshipEdge,
    ) -> Result<(), SkillError> {
        let mut relationships = self.skill_composition_relationships();
        if let Some(existing) = relationships.iter_mut().find(|entry| {
            relationship_edge_identity_key(entry) == relationship_edge_identity_key(&relationship)
        }) {
            *existing = relationship;
        } else {
            relationships.push(relationship);
        }
        self.hub_store
            .replace_composition_relationships(relationships)
    }

    fn upsert_capability_group(&self, group: SkillCapabilityGroup) -> Result<(), SkillError> {
        let mut groups = self.skill_capability_groups();
        if let Some(existing) = groups.iter_mut().find(|entry| {
            normalize_name(&entry.capability_id) == normalize_name(&group.capability_id)
        }) {
            *existing = group;
        } else {
            groups.push(group);
        }
        self.hub_store.replace_capability_groups(groups)
    }

    fn existing_capability_group(
        &self,
        capability_id: &str,
    ) -> Result<SkillCapabilityGroup, SkillError> {
        self.skill_capability_groups()
            .into_iter()
            .find(|group| normalize_name(&group.capability_id) == normalize_name(capability_id))
            .ok_or_else(|| SkillError::InvalidSkillContent {
                message: format!("unknown capability group `{}`", capability_id.trim()),
            })
    }

    fn synced_managed_record(
        &self,
        source: &SkillSourceRef,
        entry: &crate::sync::SkillSyncSourceEntry,
    ) -> Result<ManagedSkillRecord, SkillError> {
        let meta = self
            .skill_authority
            .resolve_skill_for_inspection(&entry.skill_name, None)?;
        let local_hash = crate::sync::hash_skill_meta(&meta)?;
        Ok(ManagedSkillRecord {
            skill_name: entry.skill_name.clone(),
            source: Some(source.clone()),
            installed_revision: entry
                .revision
                .clone()
                .or_else(|| Some(entry.content_hash.clone())),
            local_hash: Some(local_hash),
            last_synced_at: Some(now_unix_timestamp()),
            locally_modified: false,
            deleted_locally: false,
        })
    }

    fn skill_composition_consumption_context(
        &self,
        skill_name: &str,
    ) -> SkillCompositionConsumptionContext {
        let key = normalize_name(skill_name);
        let mut context = SkillCompositionConsumptionContext::default();

        for group in self
            .skill_capability_groups()
            .into_iter()
            .filter(|group| group.state == agendao_types::SkillCapabilityGroupState::Active)
        {
            let Some(member) = group
                .members
                .iter()
                .find(|member| normalize_name(&member.skill_name) == key)
            else {
                continue;
            };

            match group.group_kind {
                SkillCapabilityGroupKind::CanonicalFamily => {
                    if context.canonical_family_id.is_none() {
                        context.canonical_family_id = Some(group.capability_id.clone());
                    }
                    if context.family_member_role.is_none() {
                        context.family_member_role = Some(member.role);
                    }
                    if context.canonical_skill_name.is_none() {
                        context.canonical_skill_name = group
                            .canonical_skill_name
                            .clone()
                            .or_else(|| canonical_member_skill_name(&group));
                    }
                }
                SkillCapabilityGroupKind::ComplementaryBundle => {
                    context
                        .complementary_group_ids
                        .push(group.capability_id.clone());
                    context.complementary_peer_skill_names.extend(
                        group
                            .members
                            .iter()
                            .filter(|entry| normalize_name(&entry.skill_name) != key)
                            .map(|entry| entry.skill_name.clone()),
                    );
                }
            }
        }

        for relationship in self
            .skill_composition_relationships()
            .into_iter()
            .filter(|relationship| relationship.state == SkillRelationshipState::Accepted)
        {
            if normalize_name(&relationship.left_skill_name) != key
                && normalize_name(&relationship.right_skill_name) != key
            {
                continue;
            }

            match relationship.relation_kind {
                SkillRelationshipKind::ComplementaryComponent => {
                    if let Some(peer_skill_name) =
                        relationship_other_skill_name(&relationship, skill_name)
                    {
                        context.complementary_peer_skill_names.push(peer_skill_name);
                    }
                }
                SkillRelationshipKind::SpecializationVariant => {
                    if let Some(preferred_skill_name) = relationship
                        .preferred_skill_name
                        .as_deref()
                        .filter(|preferred| !preferred.eq_ignore_ascii_case(skill_name))
                    {
                        context
                            .canonical_skill_name
                            .get_or_insert_with(|| preferred_skill_name.to_string());
                        context
                            .family_member_role
                            .get_or_insert(SkillCapabilityMemberRole::Specialization);
                    }
                }
                SkillRelationshipKind::RedundantOverlap => {
                    if let Some(preferred_skill_name) = relationship
                        .preferred_skill_name
                        .as_deref()
                        .filter(|preferred| !preferred.eq_ignore_ascii_case(skill_name))
                    {
                        context
                            .canonical_skill_name
                            .get_or_insert_with(|| preferred_skill_name.to_string());
                        context
                            .family_member_role
                            .get_or_insert(SkillCapabilityMemberRole::MergeCandidate);
                    }
                }
            }
        }

        context.complementary_group_ids.sort();
        context.complementary_group_ids.dedup();
        context.complementary_peer_skill_names.sort();
        context.complementary_peer_skill_names.dedup();
        context
    }
}

fn audit_kind_for_write_action(action: &SkillWriteAction) -> SkillAuditKind {
    match action {
        SkillWriteAction::Created => SkillAuditKind::Create,
        SkillWriteAction::Patched => SkillAuditKind::Patch,
        SkillWriteAction::Edited => SkillAuditKind::Edit,
        SkillWriteAction::SupportingFileWritten => SkillAuditKind::WriteFile,
        SkillWriteAction::SupportingFileRemoved => SkillAuditKind::RemoveFile,
        SkillWriteAction::Deleted => SkillAuditKind::Delete,
    }
}

fn governance_write_result(result: &SkillWriteResult) -> SkillGovernanceWriteResult {
    SkillGovernanceWriteResult {
        action: format!("{:?}", result.action).to_ascii_lowercase(),
        skill_name: result.skill_name.clone(),
        location: result.location.to_string_lossy().to_string(),
        supporting_file: result.supporting_file.clone(),
    }
}

fn remote_install_reason(distribution: &SkillDistributionRecord) -> String {
    let release_hint = release_identity(&distribution.release).unwrap_or("unversioned");
    format!("{} via {}", release_hint, distribution.source.source_id)
}

fn remote_update_reason(
    distribution: &SkillDistributionRecord,
    record: &ManagedSkillRecord,
    lifecycle_state: SkillManagedLifecycleState,
) -> String {
    let release_hint = release_identity(&distribution.release).unwrap_or("unversioned");
    match lifecycle_state {
        SkillManagedLifecycleState::Diverged => format!(
            "repair local divergence{} via {}",
            if record.installed_revision.as_deref() != Some(release_hint) {
                format!(
                    " and move {} -> {}",
                    record.installed_revision.as_deref().unwrap_or("unknown"),
                    release_hint
                )
            } else {
                String::new()
            },
            distribution.source.source_id
        ),
        _ => format!(
            "{} -> {} via {}",
            record.installed_revision.as_deref().unwrap_or("unknown"),
            release_hint,
            distribution.source.source_id
        ),
    }
}

fn release_identity(release: &agendao_types::SkillDistributionRelease) -> Option<&str> {
    release
        .revision
        .as_deref()
        .or(release.version.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn extract_body_from_markdown(markdown_content: &str) -> String {
    if let Ok(document) = crate::write::parse_skill_document(markdown_content) {
        return document.body;
    }
    markdown_content.trim().to_string()
}

fn source_index_refresh_audit_event(
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

fn remote_plan_audit_event(
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

fn artifact_evicted_audit_event(
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

fn lifecycle_transition_audit_event(
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

fn vitality_transition_audit_event(
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

fn composition_relationship_audit_event(
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

fn capability_group_activated_audit_event(
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

fn capability_group_member_role_updated_audit_event(
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

fn capability_group_member_removed_audit_event(
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

fn write_audit_event(
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

fn guard_audit_event(
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

fn hub_remove_audit_event(
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

fn hub_detach_audit_event(
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

fn sync_audit_event(
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

fn distribution_audit_event(
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

fn unresolved_distribution_id(source: &SkillSourceRef, skill_name: &str) -> String {
    format!(
        "dist:{}:{}:unresolved",
        source
            .source_id
            .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"),
        skill_name
            .trim()
            .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
    )
}

fn create_target_from_relative_path(
    relative_path: &str,
) -> Result<(Option<String>, Option<String>), SkillError> {
    let parent =
        Path::new(relative_path)
            .parent()
            .ok_or_else(|| SkillError::InvalidSkillContent {
                message: format!("invalid source skill path `{relative_path}`"),
            })?;
    let components = parent
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let directory_name = components.last().cloned();
    let category = if components.len() > 1 {
        Some(components[..components.len() - 1].join("/"))
    } else {
        None
    };
    Ok((category, directory_name))
}

fn normalize_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn trimmed_option(value: Option<&str>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_search_text(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn search_query_terms(query: Option<&str>) -> Vec<String> {
    query
        .into_iter()
        .flat_map(|query| {
            query
                .split(|ch: char| ch.is_whitespace() || ch == '/' || ch == '-' || ch == '_')
                .map(str::trim)
                .filter(|term| !term.is_empty())
                .map(|term| term.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn search_snapshot_matches_filters(
    snapshot: &SkillSourceIndexSnapshot,
    source_id_filter: Option<&str>,
    source_kind_filter: Option<agendao_types::SkillSourceKind>,
) -> bool {
    if let Some(source_id_filter) = source_id_filter {
        if snapshot.source.source_id.trim() != source_id_filter {
            return false;
        }
    }
    if let Some(source_kind_filter) = source_kind_filter {
        if snapshot.source.source_kind != source_kind_filter {
            return false;
        }
    }
    true
}

fn search_source_matches_filters(
    source: &SkillSourceRef,
    source_id_filter: Option<&str>,
    source_kind_filter: Option<agendao_types::SkillSourceKind>,
) -> bool {
    if let Some(source_id_filter) = source_id_filter {
        if source.source_id.trim() != source_id_filter {
            return false;
        }
    }
    if let Some(source_kind_filter) = source_kind_filter {
        if source.source_kind != source_kind_filter {
            return false;
        }
    }
    true
}

fn score_source_index_entry(
    entry: &SkillSourceIndexEntry,
    normalized_query: Option<&str>,
    query_terms: &[String],
) -> Option<(i64, Vec<String>)> {
    if normalized_query.is_none() {
        return Some((0, Vec::new()));
    }

    let name = normalize_search_text(&entry.skill_name);
    let description = entry
        .description
        .as_deref()
        .map(normalize_search_text)
        .unwrap_or_default();
    let category = entry
        .category
        .as_deref()
        .map(normalize_search_text)
        .unwrap_or_default();
    let version = entry
        .version
        .as_deref()
        .map(normalize_search_text)
        .unwrap_or_default();
    let revision = entry
        .revision
        .as_deref()
        .map(normalize_search_text)
        .unwrap_or_default();
    let query = normalized_query.unwrap_or_default();

    let mut score = 0_i64;
    let mut reasons = Vec::new();

    if name == query {
        score += 1_000;
        reasons.push("exact_skill_name".to_string());
    } else if name.starts_with(query) {
        score += 700;
        reasons.push("prefix_skill_name".to_string());
    } else if name.contains(query) {
        score += 500;
        reasons.push("skill_name".to_string());
    }

    if !description.is_empty() && description.contains(query) {
        score += 250;
        reasons.push("description".to_string());
    }
    if !category.is_empty() && category.contains(query) {
        score += 200;
        reasons.push("category".to_string());
    }
    if (!version.is_empty() && version.contains(query))
        || (!revision.is_empty() && revision.contains(query))
    {
        score += 80;
        reasons.push("release".to_string());
    }

    if !query_terms.is_empty() {
        let minimum_term_matches = std::cmp::max(1, query_terms.len().div_ceil(2));
        let mut matched_terms = 0_usize;
        for term in query_terms {
            if name.contains(term) {
                score += 120;
                matched_terms += 1;
            } else if description.contains(term) {
                score += 60;
                matched_terms += 1;
            } else if category.contains(term) {
                score += 50;
                matched_terms += 1;
            } else if version.contains(term) || revision.contains(term) {
                score += 20;
                matched_terms += 1;
            }
        }
        if matched_terms < minimum_term_matches {
            return None;
        }
    }

    (score > 0).then_some((score, reasons))
}

fn timeline_matches_filters(
    skill_name: Option<&str>,
    source_id: Option<&str>,
    skill_filter: Option<&str>,
    source_filter: Option<&str>,
) -> bool {
    if let Some(skill_filter) = skill_filter {
        if skill_name.map(normalize_name).as_deref() != Some(skill_filter) {
            return false;
        }
    }
    if let Some(source_filter) = source_filter {
        if source_id.map(str::trim) != Some(source_filter) {
            return false;
        }
    }
    true
}

fn audit_event_matches_filters(
    event: &SkillAuditEvent,
    skill_filter: Option<&str>,
    source_filter: Option<&str>,
) -> bool {
    if let Some(source_filter) = source_filter {
        if event.source_id.as_deref().map(str::trim) != Some(source_filter) {
            return false;
        }
    }
    if let Some(skill_filter) = skill_filter {
        if event.skill_name.as_deref().map(normalize_name).as_deref() == Some(skill_filter) {
            return true;
        }
        return payload_skill_names(&event.payload)
            .iter()
            .any(|skill_name| normalize_name(skill_name) == skill_filter);
    }
    true
}

const SKILL_NEGATIVE_ENTROPY_STALE_SECONDS: i64 = 30 * 24 * 60 * 60;
const SKILL_POSITIVE_EVOLUTION_GRACE_SECONDS: i64 = SKILL_NEGATIVE_ENTROPY_STALE_SECONDS;
const SKILL_GUARD_RULE_SEMANTIC_OVERLAP: &str = "semantic.skill_overlap";
const SKILL_GUARD_RULE_TRIGGER_OVERLAP: &str = "semantic.trigger_overlap";

#[derive(Debug, Clone)]
struct SkillSemanticDescriptor {
    skill_name: String,
    normalized_name: String,
    category: Option<String>,
    tokens: BTreeSet<String>,
    trigger_terms: BTreeSet<String>,
    related_skills: BTreeSet<String>,
    requires_tools: BTreeSet<String>,
    requires_toolsets: BTreeSet<String>,
    stage_filter: BTreeSet<String>,
}

fn build_skill_semantic_descriptor_from_parts(
    skill_name: &str,
    description: &str,
    category: Option<&str>,
    conditions: &SkillConditions,
    detail: &SkillDetailView,
) -> SkillSemanticDescriptor {
    let normalized_name = normalize_name(skill_name);

    let mut tokens = BTreeSet::new();
    for token in skill_descriptor_tokens(skill_name) {
        tokens.insert(token);
    }
    for token in skill_descriptor_tokens(description) {
        tokens.insert(token);
    }
    if let Some(category) = category {
        for token in skill_descriptor_tokens(category) {
            tokens.insert(token);
        }
    }
    for token in &detail.tags {
        for normalized in skill_descriptor_tokens(token) {
            tokens.insert(normalized);
        }
    }

    let mut trigger_terms = BTreeSet::new();
    for value in conditions
        .requires_tools
        .iter()
        .chain(conditions.requires_toolsets.iter())
        .chain(conditions.stage_filter.iter())
        .chain(conditions.fallback_for_tools.iter())
        .chain(conditions.fallback_for_toolsets.iter())
    {
        let normalized = normalize_name(value);
        if !normalized.is_empty() {
            trigger_terms.insert(normalized);
        }
    }

    let related_skills = detail
        .related_skills
        .iter()
        .map(|value| normalize_name(value))
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();
    let requires_tools = conditions
        .requires_tools
        .iter()
        .map(|value| normalize_name(value))
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();
    let requires_toolsets = conditions
        .requires_toolsets
        .iter()
        .map(|value| normalize_name(value))
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();
    let stage_filter = conditions
        .stage_filter
        .iter()
        .map(|value| normalize_name(value))
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();

    SkillSemanticDescriptor {
        skill_name: skill_name.to_string(),
        normalized_name,
        category: category.map(normalize_name),
        tokens,
        trigger_terms,
        related_skills,
        requires_tools,
        requires_toolsets,
        stage_filter,
    }
}

fn semantic_detail_tags(frontmatter: &crate::SkillFrontmatter) -> Vec<String> {
    frontmatter
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.hermes.as_ref())
        .map(|metadata| metadata.tags.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| frontmatter.tags.clone())
}

fn semantic_detail_related_skills(frontmatter: &crate::SkillFrontmatter) -> Vec<String> {
    frontmatter
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.hermes.as_ref())
        .map(|metadata| metadata.related_skills.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| frontmatter.related_skills.clone())
}

fn semantic_conflict_guard_violation(
    candidate: &SkillSemanticDescriptor,
    conflict: &SkillSemanticConflictDiagnostic,
) -> SkillGuardViolation {
    let counterpart = semantic_conflict_counterpart_skill_name(candidate, conflict)
        .unwrap_or_else(|| conflict.right_skill_name.as_str());
    let overlap_reason = conflict
        .reasons
        .iter()
        .filter(|reason| !reason.contains("usage ledger currently favors"))
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    let mut message = format!(
        "skill `{}` overlaps existing `{}` ({}, score {}): {}.",
        candidate.skill_name,
        counterpart,
        semantic_conflict_guard_kind_label(conflict.kind),
        conflict.score,
        if overlap_reason.is_empty() {
            "semantic overlap was detected".to_string()
        } else {
            overlap_reason
        }
    );
    if let Some(preferred_skill_name) = conflict.preferred_skill_name.as_deref() {
        message.push_str(&format!(
            " usage ledger currently favors `{preferred_skill_name}` in this overlap pair."
        ));
    }
    SkillGuardViolation {
        rule_id: semantic_conflict_guard_rule_id(conflict.kind).to_string(),
        severity: SkillGuardSeverity::Warn,
        message,
        file_path: Some("SKILL.md".to_string()),
    }
}

fn semantic_conflict_counterpart_skill_name<'a>(
    candidate: &SkillSemanticDescriptor,
    conflict: &'a SkillSemanticConflictDiagnostic,
) -> Option<&'a str> {
    if conflict
        .left_skill_name
        .eq_ignore_ascii_case(&candidate.skill_name)
    {
        return Some(conflict.right_skill_name.as_str());
    }
    if conflict
        .right_skill_name
        .eq_ignore_ascii_case(&candidate.skill_name)
    {
        return Some(conflict.left_skill_name.as_str());
    }
    None
}

fn semantic_conflict_guard_rule_id(kind: SkillSemanticConflictKind) -> &'static str {
    match kind {
        SkillSemanticConflictKind::TriggerOverlap => SKILL_GUARD_RULE_TRIGGER_OVERLAP,
        SkillSemanticConflictKind::NearDuplicate | SkillSemanticConflictKind::ReplacementHint => {
            SKILL_GUARD_RULE_SEMANTIC_OVERLAP
        }
    }
}

fn semantic_conflict_guard_kind_label(kind: SkillSemanticConflictKind) -> &'static str {
    match kind {
        SkillSemanticConflictKind::NearDuplicate => "near duplicate",
        SkillSemanticConflictKind::TriggerOverlap => "trigger overlap",
        SkillSemanticConflictKind::ReplacementHint => "replacement hint",
    }
}

fn build_skill_semantic_conflict(
    left: &SkillSemanticDescriptor,
    right: &SkillSemanticDescriptor,
    left_snapshot: Option<&SkillOperationalSnapshot>,
    right_snapshot: Option<&SkillOperationalSnapshot>,
) -> Option<SkillSemanticConflictDiagnostic> {
    if left.normalized_name == right.normalized_name {
        return None;
    }

    let shared_tokens = set_intersection_count(&left.tokens, &right.tokens);
    let token_jaccard = set_jaccard_ratio(&left.tokens, &right.tokens);
    let shared_triggers = set_intersection_count(&left.trigger_terms, &right.trigger_terms);
    let trigger_jaccard = set_jaccard_ratio(&left.trigger_terms, &right.trigger_terms);
    let same_category = left.category.is_some() && left.category == right.category;
    let related_overlap = left.related_skills.contains(&right.normalized_name)
        || right.related_skills.contains(&left.normalized_name)
        || set_intersection_count(&left.related_skills, &right.related_skills) > 0;

    let mut score = 0u16;
    let mut reasons = Vec::new();

    if same_category {
        score += 15;
        if let Some(category) = left.category.as_deref() {
            reasons.push(format!("shared category `{category}`"));
        }
    }

    if shared_triggers > 0 && trigger_jaccard >= 0.6 {
        score += 35;
        reasons.push(format!(
            "runtime trigger conditions heavily overlap ({shared_triggers} shared trigger terms)"
        ));
    } else if shared_triggers >= 2 {
        score += 20;
        reasons.push(format!(
            "runtime trigger conditions overlap ({shared_triggers} shared trigger terms)"
        ));
    }

    if shared_tokens >= 3 && token_jaccard >= 0.45 {
        score += 30;
        reasons.push(format!(
            "name/description tokens strongly overlap ({shared_tokens} shared descriptor terms)"
        ));
    } else if shared_tokens >= 4 {
        score += 20;
        reasons.push(format!(
            "descriptor vocabulary overlaps ({shared_tokens} shared terms)"
        ));
    }

    if related_overlap {
        score += 10;
        reasons.push(
            "frontmatter related-skills metadata points at the same capability cluster".to_string(),
        );
    }

    if score < 45 {
        return None;
    }

    let left_runtime_use_count = left_snapshot
        .and_then(|snapshot| snapshot.usage.as_ref())
        .map(|usage| usage.runtime_use_count)
        .unwrap_or(0);
    let right_runtime_use_count = right_snapshot
        .and_then(|snapshot| snapshot.usage.as_ref())
        .map(|usage| usage.runtime_use_count)
        .unwrap_or(0);
    let left_last_used_at =
        left_snapshot.and_then(|snapshot| snapshot.usage.as_ref()?.last_used_at);
    let right_last_used_at =
        right_snapshot.and_then(|snapshot| snapshot.usage.as_ref()?.last_used_at);

    let preferred_skill_name = preferred_skill_name(
        left,
        right,
        left_runtime_use_count,
        right_runtime_use_count,
        left_last_used_at,
        right_last_used_at,
    );
    if let Some(preferred) = preferred_skill_name.as_deref() {
        reasons.push(format!(
            "usage ledger currently favors `{preferred}` as the more active skill in this overlap pair"
        ));
    }

    let kind = if score >= 70 && preferred_skill_name.is_some() {
        SkillSemanticConflictKind::ReplacementHint
    } else if score >= 70 {
        SkillSemanticConflictKind::NearDuplicate
    } else {
        SkillSemanticConflictKind::TriggerOverlap
    };
    let severity = if matches!(
        kind,
        SkillSemanticConflictKind::ReplacementHint | SkillSemanticConflictKind::NearDuplicate
    ) {
        SkillGovernanceDiagnosticSeverity::Warn
    } else {
        SkillGovernanceDiagnosticSeverity::Info
    };

    let (
        left_skill_name,
        right_skill_name,
        left_runtime_use_count,
        right_runtime_use_count,
        left_last_used_at,
        right_last_used_at,
        preferred_skill_name,
    ) = if left.skill_name <= right.skill_name {
        (
            left.skill_name.clone(),
            right.skill_name.clone(),
            left_runtime_use_count,
            right_runtime_use_count,
            left_last_used_at,
            right_last_used_at,
            preferred_skill_name,
        )
    } else {
        (
            right.skill_name.clone(),
            left.skill_name.clone(),
            right_runtime_use_count,
            left_runtime_use_count,
            right_last_used_at,
            left_last_used_at,
            preferred_skill_name,
        )
    };

    Some(SkillSemanticConflictDiagnostic {
        left_skill_name,
        right_skill_name,
        kind,
        severity,
        score,
        reasons,
        preferred_skill_name,
        left_runtime_use_count,
        right_runtime_use_count,
        left_last_used_at,
        right_last_used_at,
    })
}

fn collect_skill_semantic_conflicts(
    descriptors: &[SkillSemanticDescriptor],
    snapshot_by_name: &BTreeMap<String, SkillOperationalSnapshot>,
) -> Vec<SkillSemanticConflictDiagnostic> {
    let mut diagnostics = Vec::new();
    for left_index in 0..descriptors.len() {
        for right_index in (left_index + 1)..descriptors.len() {
            if let Some(conflict) = build_skill_semantic_conflict(
                &descriptors[left_index],
                &descriptors[right_index],
                snapshot_by_name.get(&normalize_name(&descriptors[left_index].skill_name)),
                snapshot_by_name.get(&normalize_name(&descriptors[right_index].skill_name)),
            ) {
                diagnostics.push(conflict);
            }
        }
    }

    diagnostics.sort_by(|left, right| {
        skill_diagnostic_sort_key(left.severity)
            .cmp(&skill_diagnostic_sort_key(right.severity))
            .then_with(|| right.score.cmp(&left.score))
            .then_with(|| left.left_skill_name.cmp(&right.left_skill_name))
            .then_with(|| left.right_skill_name.cmp(&right.right_skill_name))
    });
    diagnostics
}

fn preferred_skill_name(
    left: &SkillSemanticDescriptor,
    right: &SkillSemanticDescriptor,
    left_runtime_use_count: u64,
    right_runtime_use_count: u64,
    left_last_used_at: Option<i64>,
    right_last_used_at: Option<i64>,
) -> Option<String> {
    if left_runtime_use_count > right_runtime_use_count {
        return Some(left.skill_name.clone());
    }
    if right_runtime_use_count > left_runtime_use_count {
        return Some(right.skill_name.clone());
    }
    match (left_last_used_at, right_last_used_at) {
        (Some(left_ts), Some(right_ts)) if left_ts > right_ts => Some(left.skill_name.clone()),
        (Some(left_ts), Some(right_ts)) if right_ts > left_ts => Some(right.skill_name.clone()),
        (Some(_), None) => Some(left.skill_name.clone()),
        (None, Some(_)) => Some(right.skill_name.clone()),
        _ => None,
    }
}

fn semantic_conflict_is_review_candidate(conflict: &SkillSemanticConflictDiagnostic) -> bool {
    conflict.severity == SkillGovernanceDiagnosticSeverity::Warn
        && conflict.kind == SkillSemanticConflictKind::ReplacementHint
        && conflict.preferred_skill_name.is_some()
}

fn semantic_conflict_redundant_skill_name(
    conflict: &SkillSemanticConflictDiagnostic,
    preferred_skill_name: &str,
) -> Option<String> {
    if conflict
        .left_skill_name
        .eq_ignore_ascii_case(preferred_skill_name)
    {
        return Some(conflict.right_skill_name.clone());
    }
    if conflict
        .right_skill_name
        .eq_ignore_ascii_case(preferred_skill_name)
    {
        return Some(conflict.left_skill_name.clone());
    }
    None
}

fn semantic_conflict_review_candidate_summary(
    conflict: &SkillSemanticConflictDiagnostic,
    preferred_skill_name: &str,
) -> String {
    let overlap_reason = conflict
        .reasons
        .iter()
        .find(|reason| !reason.contains("usage ledger currently favors"))
        .cloned()
        .unwrap_or_else(|| "semantic overlap was detected".to_string());
    format!(
        "{overlap_reason}; usage ledger currently favors `{preferred_skill_name}` as the more active skill in this overlap pair"
    )
}

fn should_sync_negative_entropy_review_candidate(
    snapshot: Option<&SkillOperationalSnapshot>,
) -> bool {
    !matches!(
        snapshot
            .and_then(|entry| entry.vitality.as_ref())
            .map(|record| record.state),
        Some(
            SkillVitalityState::ReviewCandidate
                | SkillVitalityState::Retired
                | SkillVitalityState::Archived
        )
    )
}

fn negative_entropy_review_candidate_summary(
    diagnostic: &SkillNegativeEntropyDiagnostic,
    context: &SkillCompositionConsumptionContext,
) -> String {
    let base = diagnostic
        .reasons
        .first()
        .cloned()
        .unwrap_or_else(|| "negative entropy review candidate".to_string());
    let Some(canonical_skill_name) = context.related_skill_name_for_review(&diagnostic.skill_name)
    else {
        return base;
    };
    let qualifier = match context.family_member_role {
        Some(SkillCapabilityMemberRole::Specialization) => format!(
            "the skill is governed as a specialization variant under canonical skill `{canonical_skill_name}`"
        ),
        Some(SkillCapabilityMemberRole::MergeCandidate) => format!(
            "the skill is governed as a merge candidate under canonical skill `{canonical_skill_name}`"
        ),
        _ => format!(
            "the skill is governed relative to canonical skill `{canonical_skill_name}`"
        ),
    };
    format!("{base}; {qualifier}")
}

fn should_sync_semantic_conflict_review_candidate(
    snapshot: Option<&SkillOperationalSnapshot>,
    preferred_skill_name: &str,
) -> bool {
    let Some(vitality) = snapshot.and_then(|entry| entry.vitality.as_ref()) else {
        return true;
    };
    match vitality.state {
        SkillVitalityState::Retired | SkillVitalityState::Archived => false,
        SkillVitalityState::Active => true,
        SkillVitalityState::ReviewCandidate => {
            !(vitality.reason.kind == SkillRetirementReasonKind::SemanticConflict
                && vitality
                    .reason
                    .related_skill_name
                    .as_deref()
                    .map(normalize_name)
                    == Some(normalize_name(preferred_skill_name)))
        }
    }
}

fn skill_negative_entropy_severity(
    source_scope: SkillOperationalSourceScope,
    signals: &[SkillNegativeEntropySignal],
    recent_positive_evolution: bool,
    composition_context: &SkillCompositionConsumptionContext,
) -> SkillGovernanceDiagnosticSeverity {
    if recent_positive_evolution {
        return SkillGovernanceDiagnosticSeverity::Info;
    }
    if composition_context.complementary_protected() {
        return SkillGovernanceDiagnosticSeverity::Info;
    }
    if matches!(source_scope, SkillOperationalSourceScope::WorkspaceLocal)
        && signals.iter().any(|signal| {
            matches!(
                signal,
                SkillNegativeEntropySignal::NeverReused
                    | SkillNegativeEntropySignal::StaleUnused
                    | SkillNegativeEntropySignal::WriteHeavyLowReuse
            )
        })
    {
        SkillGovernanceDiagnosticSeverity::Warn
    } else {
        SkillGovernanceDiagnosticSeverity::Info
    }
}

fn skill_diagnostic_sort_key(severity: SkillGovernanceDiagnosticSeverity) -> u8 {
    match severity {
        SkillGovernanceDiagnosticSeverity::Warn => 0,
        SkillGovernanceDiagnosticSeverity::Info => 1,
    }
}

fn total_skill_write_count(entry: &SkillWriteLedgerEntry) -> u64 {
    entry.create_count
        + entry.patch_count
        + entry.edit_count
        + entry.supporting_file_write_count
        + entry.supporting_file_remove_count
        + entry.install_count
        + entry.update_count
        + entry.detach_count
        + entry.remove_count
        + entry.delete_count
}

fn is_skill_timestamp_stale(timestamp: Option<i64>, now: i64, threshold_seconds: i64) -> bool {
    timestamp
        .map(|value| value > 0 && now.saturating_sub(value) >= threshold_seconds)
        .unwrap_or(false)
}

fn format_skill_positive_evolution_reason(
    evolution: &SkillEvolutionEvidenceSummary,
    now: i64,
) -> String {
    let days_ago = evolution
        .last_positive_signal_at
        .map(|timestamp| now.saturating_sub(timestamp) / 86_400)
        .unwrap_or(0);
    format!(
        "recent memory/proposal evolution signal observed {} day(s) ago ({} memory promotion(s), {} proposal signal(s), {} active draft proposal(s)); review severity is downgraded while the skill is still evolving",
        days_ago,
        evolution.memory_promotion_count,
        evolution.proposal_signal_count,
        evolution.last_observed_draft_proposal_count
    )
}

fn canonical_member_skill_name(group: &SkillCapabilityGroup) -> Option<String> {
    group
        .members
        .iter()
        .find(|member| member.role == SkillCapabilityMemberRole::Canonical)
        .map(|member| member.skill_name.clone())
}

fn relationship_other_skill_name(
    relationship: &SkillRelationshipEdge,
    skill_name: &str,
) -> Option<String> {
    if relationship
        .left_skill_name
        .eq_ignore_ascii_case(skill_name)
    {
        return Some(relationship.right_skill_name.clone());
    }
    if relationship
        .right_skill_name
        .eq_ignore_ascii_case(skill_name)
    {
        return Some(relationship.left_skill_name.clone());
    }
    None
}

fn format_negative_entropy_composition_reason(
    skill_name: &str,
    context: &SkillCompositionConsumptionContext,
) -> Option<String> {
    if context.complementary_protected() {
        if context.complementary_peer_skill_names.is_empty() {
            return Some(
                "skill is explicitly governed as a complementary component; low standalone reuse is expected and is not treated as pure redundancy"
                    .to_string(),
            );
        }
        return Some(format!(
            "skill is explicitly governed as a complementary component alongside {}; low standalone reuse is expected and is not treated as pure redundancy",
            context.complementary_peer_skill_names.join(", ")
        ));
    }

    let canonical_skill_name = context.related_skill_name_for_review(skill_name)?;
    let relation_label = match context.family_member_role {
        Some(SkillCapabilityMemberRole::Specialization) => "specialization member",
        Some(SkillCapabilityMemberRole::MergeCandidate) => "merge candidate",
        _ => "family member",
    };
    if let Some(capability_id) = context.canonical_family_id.as_deref() {
        return Some(format!(
            "skill is an explicit {relation_label} in canonical family `{capability_id}` led by `{canonical_skill_name}`; low reuse is evaluated relative to that family owner"
        ));
    }
    Some(format!(
        "skill is an explicit {relation_label} governed relative to canonical skill `{canonical_skill_name}`; low reuse is evaluated relative to that owner"
    ))
}

fn set_intersection_count(left: &BTreeSet<String>, right: &BTreeSet<String>) -> usize {
    left.intersection(right).count()
}

fn relationship_pair_key(left_skill_name: &str, right_skill_name: &str) -> (String, String) {
    let left = normalize_name(left_skill_name);
    let right = normalize_name(right_skill_name);
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn ordered_skill_names(left_skill_name: &str, right_skill_name: &str) -> (String, String) {
    if left_skill_name <= right_skill_name {
        (left_skill_name.to_string(), right_skill_name.to_string())
    } else {
        (right_skill_name.to_string(), left_skill_name.to_string())
    }
}

fn normalize_runtime_selected_skill_names(raw_names: &[String]) -> Vec<String> {
    let mut names = Vec::new();
    for raw_name in raw_names {
        let trimmed = raw_name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !names
            .iter()
            .any(|seen: &String| seen.eq_ignore_ascii_case(trimmed))
        {
            names.push(trimmed.to_string());
        }
    }
    names
}

fn skill_names_includes_pair(
    skill_names: &[String],
    left_skill_name: &str,
    right_skill_name: &str,
) -> bool {
    let left_present = skill_names
        .iter()
        .any(|skill_name| skill_name.eq_ignore_ascii_case(left_skill_name));
    let right_present = skill_names
        .iter()
        .any(|skill_name| skill_name.eq_ignore_ascii_case(right_skill_name));
    left_present && right_present
}

fn build_skill_redundant_relationship_candidate(
    conflict: &SkillSemanticConflictDiagnostic,
) -> Option<SkillRelationshipEdge> {
    if conflict.score < 70 || conflict.preferred_skill_name.is_none() {
        return None;
    }
    if !matches!(
        conflict.kind,
        SkillSemanticConflictKind::ReplacementHint | SkillSemanticConflictKind::NearDuplicate
    ) {
        return None;
    }

    Some(SkillRelationshipEdge {
        left_skill_name: conflict.left_skill_name.clone(),
        right_skill_name: conflict.right_skill_name.clone(),
        relation_kind: agendao_types::SkillRelationshipKind::RedundantOverlap,
        state: agendao_types::SkillRelationshipState::Observed,
        score: conflict.score,
        reasons: dedupe_string_reasons(conflict.reasons.clone()),
        preferred_skill_name: conflict.preferred_skill_name.clone(),
        observed_at: None,
        updated_at: None,
    })
}

fn build_skill_specialization_relationship_candidate(
    left: &SkillSemanticDescriptor,
    right: &SkillSemanticDescriptor,
    conflict: &SkillSemanticConflictDiagnostic,
) -> Option<SkillRelationshipEdge> {
    if conflict.score < 55 {
        return None;
    }

    let specialization = specialization_variant_direction(left, right)?;
    let mut reasons = conflict
        .reasons
        .iter()
        .filter(|reason| !reason.contains("usage ledger currently favors"))
        .take(2)
        .cloned()
        .collect::<Vec<_>>();
    reasons.extend(specialization.reasons);
    if conflict.preferred_skill_name.as_deref()
        == Some(specialization.canonical_skill_name.as_str())
    {
        reasons.push(format!(
            "usage ledger also currently favors `{}` within this variant family",
            specialization.canonical_skill_name
        ));
    }
    let (left_skill_name, right_skill_name) =
        ordered_skill_names(&left.skill_name, &right.skill_name);

    Some(SkillRelationshipEdge {
        left_skill_name,
        right_skill_name,
        relation_kind: agendao_types::SkillRelationshipKind::SpecializationVariant,
        state: agendao_types::SkillRelationshipState::Observed,
        score: conflict
            .score
            .saturating_add((specialization.strict_signal_count as u16).saturating_mul(5))
            .min(100),
        reasons: dedupe_string_reasons(reasons),
        preferred_skill_name: Some(specialization.canonical_skill_name),
        observed_at: None,
        updated_at: None,
    })
}

fn build_skill_complementary_relationship_candidate(
    left: &SkillSemanticDescriptor,
    right: &SkillSemanticDescriptor,
    conflict: Option<&SkillSemanticConflictDiagnostic>,
    left_snapshot: Option<&SkillOperationalSnapshot>,
    right_snapshot: Option<&SkillOperationalSnapshot>,
) -> Option<SkillRelationshipEdge> {
    if conflict.map(|entry| entry.score >= 70).unwrap_or(false) {
        return None;
    }

    let same_category = left.category.is_some() && left.category == right.category;
    let direct_related = left.related_skills.contains(&right.normalized_name)
        || right.related_skills.contains(&left.normalized_name);
    let shared_related = set_intersection_count(&left.related_skills, &right.related_skills);
    let shared_tools = set_intersection_count(&left.requires_tools, &right.requires_tools);
    let shared_toolsets = set_intersection_count(&left.requires_toolsets, &right.requires_toolsets);
    let shared_stages = if left.stage_filter.is_empty() || right.stage_filter.is_empty() {
        0
    } else {
        set_intersection_count(&left.stage_filter, &right.stage_filter)
    };
    let shared_last_category = shared_usage_value(
        left_snapshot.and_then(|snapshot| snapshot.usage.as_ref()?.last_category.as_deref()),
        right_snapshot.and_then(|snapshot| snapshot.usage.as_ref()?.last_category.as_deref()),
    );
    let shared_last_stage = shared_usage_value(
        left_snapshot.and_then(|snapshot| snapshot.usage.as_ref()?.last_stage_id.as_deref()),
        right_snapshot.and_then(|snapshot| snapshot.usage.as_ref()?.last_stage_id.as_deref()),
    );

    let has_anchor = direct_related || shared_related > 0 || same_category;
    let has_domain = shared_tools > 0
        || shared_toolsets > 0
        || shared_stages > 0
        || shared_last_category.is_some()
        || shared_last_stage.is_some()
        || conflict.is_some();
    if !has_anchor || !has_domain {
        return None;
    }

    let mut score = 0u16;
    let mut reasons = Vec::new();
    if direct_related {
        score += 30;
        reasons.push("frontmatter related_skills directly links the pair".to_string());
    } else if shared_related > 0 {
        score += 20;
        reasons.push(format!(
            "related_skills metadata points at {shared_related} shared adjacent skill(s)"
        ));
    }
    if same_category {
        score += 20;
        if let Some(category) = left.category.as_deref() {
            reasons.push(format!("shared category `{category}`"));
        }
    }
    if shared_tools > 0 {
        score += 15;
        reasons.push(format!(
            "runtime tool requirements share {shared_tools} tool(s): {}",
            join_terms(left.requires_tools.intersection(&right.requires_tools))
        ));
    }
    if shared_toolsets > 0 {
        score += 15;
        reasons.push(format!(
            "runtime toolset requirements share {shared_toolsets} toolset(s): {}",
            join_terms(
                left.requires_toolsets
                    .intersection(&right.requires_toolsets)
            )
        ));
    }
    if shared_stages > 0 {
        score += 10;
        reasons.push(format!(
            "stage filters intersect at {shared_stages} stage(s): {}",
            join_terms(left.stage_filter.intersection(&right.stage_filter))
        ));
    }
    if let Some(category) = shared_last_category {
        score += 5;
        reasons.push(format!(
            "usage ledger recently observed both skills under runtime category `{category}`"
        ));
    }
    if let Some(stage) = shared_last_stage {
        score += 5;
        reasons.push(format!(
            "usage ledger recently observed both skills in runtime stage `{stage}`"
        ));
    }
    if let Some(conflict) = conflict {
        score += 10;
        if let Some(reason) = conflict
            .reasons
            .iter()
            .find(|reason| !reason.contains("usage ledger currently favors"))
        {
            reasons.push(format!(
                "semantic overlap stays below merge threshold but still signals shared working surface: {reason}"
            ));
        }
    }
    if score < 45 {
        return None;
    }

    let (left_skill_name, right_skill_name) =
        ordered_skill_names(&left.skill_name, &right.skill_name);
    Some(SkillRelationshipEdge {
        left_skill_name,
        right_skill_name,
        relation_kind: agendao_types::SkillRelationshipKind::ComplementaryComponent,
        state: agendao_types::SkillRelationshipState::Observed,
        score: score.min(100),
        reasons: dedupe_string_reasons(reasons),
        preferred_skill_name: None,
        observed_at: None,
        updated_at: None,
    })
}

#[derive(Debug, Clone)]
struct SkillSpecializationVariantDirection {
    canonical_skill_name: String,
    reasons: Vec<String>,
    strict_signal_count: usize,
}

fn specialization_variant_direction(
    left: &SkillSemanticDescriptor,
    right: &SkillSemanticDescriptor,
) -> Option<SkillSpecializationVariantDirection> {
    let left_specializes_right = skill_narrowing_reasons(left, right);
    let right_specializes_left = skill_narrowing_reasons(right, left);
    match (left_specializes_right, right_specializes_left) {
        (Some((reasons, strict_signal_count)), None) => Some(SkillSpecializationVariantDirection {
            canonical_skill_name: right.skill_name.clone(),
            reasons,
            strict_signal_count,
        }),
        (None, Some((reasons, strict_signal_count))) => Some(SkillSpecializationVariantDirection {
            canonical_skill_name: left.skill_name.clone(),
            reasons,
            strict_signal_count,
        }),
        _ => None,
    }
}

fn skill_narrowing_reasons(
    candidate: &SkillSemanticDescriptor,
    broad: &SkillSemanticDescriptor,
) -> Option<(Vec<String>, usize)> {
    let mut reasons = Vec::new();
    let mut strict_signal_count = 0usize;

    if !broad.requires_tools.is_subset(&candidate.requires_tools) {
        return None;
    }
    let extra_tools = candidate
        .requires_tools
        .difference(&broad.requires_tools)
        .cloned()
        .collect::<Vec<_>>();
    if !extra_tools.is_empty() {
        strict_signal_count += 1;
        reasons.push(format!(
            "`{}` adds narrower runtime tool requirements beyond `{}`: {}",
            candidate.skill_name,
            broad.skill_name,
            join_terms(extra_tools.iter())
        ));
    }

    if !broad
        .requires_toolsets
        .is_subset(&candidate.requires_toolsets)
    {
        return None;
    }
    let extra_toolsets = candidate
        .requires_toolsets
        .difference(&broad.requires_toolsets)
        .cloned()
        .collect::<Vec<_>>();
    if !extra_toolsets.is_empty() {
        strict_signal_count += 1;
        reasons.push(format!(
            "`{}` adds narrower runtime toolset requirements beyond `{}`: {}",
            candidate.skill_name,
            broad.skill_name,
            join_terms(extra_toolsets.iter())
        ));
    }

    match stage_filter_specialization_state(candidate, broad) {
        StageFilterSpecializationState::Narrower(reason) => {
            strict_signal_count += 1;
            reasons.push(reason);
        }
        StageFilterSpecializationState::Equal => {}
        StageFilterSpecializationState::Incompatible => return None,
    }

    if strict_signal_count == 0 {
        return None;
    }

    Some((reasons, strict_signal_count))
}

enum StageFilterSpecializationState {
    Equal,
    Narrower(String),
    Incompatible,
}

fn stage_filter_specialization_state(
    candidate: &SkillSemanticDescriptor,
    broad: &SkillSemanticDescriptor,
) -> StageFilterSpecializationState {
    match (candidate.stage_filter.is_empty(), broad.stage_filter.is_empty()) {
        (true, true) => StageFilterSpecializationState::Equal,
        (true, false) => StageFilterSpecializationState::Incompatible,
        (false, true) => StageFilterSpecializationState::Narrower(format!(
            "`{}` narrows runtime stage scope relative to broad `{}` by restricting execution to {}",
            candidate.skill_name,
            broad.skill_name,
            join_terms(candidate.stage_filter.iter())
        )),
        (false, false) => {
            if !candidate.stage_filter.is_subset(&broad.stage_filter) {
                return StageFilterSpecializationState::Incompatible;
            }
            if candidate.stage_filter.len() == broad.stage_filter.len() {
                return StageFilterSpecializationState::Equal;
            }
            StageFilterSpecializationState::Narrower(format!(
                "`{}` narrows runtime stage scope from {} to {} relative to `{}`",
                candidate.skill_name,
                join_terms(broad.stage_filter.iter()),
                join_terms(candidate.stage_filter.iter()),
                broad.skill_name
            ))
        }
    }
}

fn shared_usage_value(left: Option<&str>, right: Option<&str>) -> Option<String> {
    let left = left.map(str::trim).filter(|value| !value.is_empty())?;
    let right = right.map(str::trim).filter(|value| !value.is_empty())?;
    if normalize_name(left) == normalize_name(right) {
        Some(left.to_string())
    } else {
        None
    }
}

fn sort_skill_relationship_edges(edges: &mut [SkillRelationshipEdge]) {
    edges.sort_by(|left, right| {
        left.left_skill_name
            .cmp(&right.left_skill_name)
            .then_with(|| left.right_skill_name.cmp(&right.right_skill_name))
            .then_with(|| {
                relationship_kind_sort_key(left.relation_kind)
                    .cmp(&relationship_kind_sort_key(right.relation_kind))
            })
            .then_with(|| right.score.cmp(&left.score))
    });
}

fn relationship_kind_sort_key(kind: agendao_types::SkillRelationshipKind) -> u8 {
    match kind {
        agendao_types::SkillRelationshipKind::RedundantOverlap => 0,
        agendao_types::SkillRelationshipKind::SpecializationVariant => 1,
        agendao_types::SkillRelationshipKind::ComplementaryComponent => 2,
    }
}

fn build_skill_capability_group_candidates(
    relationships: &[SkillRelationshipEdge],
) -> Vec<SkillCapabilityGroup> {
    let mut groups = Vec::new();

    let family_edges = relationships
        .iter()
        .filter(|edge| {
            matches!(
                edge.relation_kind,
                agendao_types::SkillRelationshipKind::RedundantOverlap
                    | agendao_types::SkillRelationshipKind::SpecializationVariant
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    for component in relationship_components(&family_edges) {
        let members = component_members(&component);
        if members.len() < 2 {
            continue;
        }
        let canonical_skill_name = canonical_family_skill_name(&component);
        let Some(canonical_skill_name) = canonical_skill_name else {
            continue;
        };
        let mut capability_members = members
            .iter()
            .map(|skill_name| SkillCapabilityMember {
                skill_name: skill_name.clone(),
                role: family_member_role(skill_name, &canonical_skill_name, &component),
            })
            .collect::<Vec<_>>();
        sort_skill_capability_members(&mut capability_members);
        groups.push(SkillCapabilityGroup {
            capability_id: build_capability_group_id(
                agendao_types::SkillCapabilityGroupKind::CanonicalFamily,
                &members,
            ),
            group_kind: agendao_types::SkillCapabilityGroupKind::CanonicalFamily,
            state: agendao_types::SkillCapabilityGroupState::Candidate,
            canonical_skill_name: Some(canonical_skill_name),
            members: capability_members,
            reasons: component_reasons(&component),
            updated_at: None,
        });
    }

    let complementary_edges = relationships
        .iter()
        .filter(|edge| {
            edge.relation_kind == agendao_types::SkillRelationshipKind::ComplementaryComponent
        })
        .cloned()
        .collect::<Vec<_>>();
    for component in relationship_components(&complementary_edges) {
        let members = component_members(&component);
        if members.len() < 2 {
            continue;
        }
        let mut capability_members = members
            .iter()
            .map(|skill_name| SkillCapabilityMember {
                skill_name: skill_name.clone(),
                role: agendao_types::SkillCapabilityMemberRole::Complementary,
            })
            .collect::<Vec<_>>();
        sort_skill_capability_members(&mut capability_members);
        groups.push(SkillCapabilityGroup {
            capability_id: build_capability_group_id(
                agendao_types::SkillCapabilityGroupKind::ComplementaryBundle,
                &members,
            ),
            group_kind: agendao_types::SkillCapabilityGroupKind::ComplementaryBundle,
            state: agendao_types::SkillCapabilityGroupState::Candidate,
            canonical_skill_name: None,
            members: capability_members,
            reasons: component_reasons(&component),
            updated_at: None,
        });
    }

    sort_skill_capability_groups(&mut groups);
    groups
}

fn relationship_components(edges: &[SkillRelationshipEdge]) -> Vec<Vec<SkillRelationshipEdge>> {
    let mut adjacency = BTreeMap::<String, BTreeSet<String>>::new();
    let mut edge_by_pair = BTreeMap::<(String, String), SkillRelationshipEdge>::new();
    for edge in edges {
        adjacency
            .entry(edge.left_skill_name.clone())
            .or_default()
            .insert(edge.right_skill_name.clone());
        adjacency
            .entry(edge.right_skill_name.clone())
            .or_default()
            .insert(edge.left_skill_name.clone());
        edge_by_pair.insert(
            relationship_pair_key(&edge.left_skill_name, &edge.right_skill_name),
            edge.clone(),
        );
    }

    let mut visited = BTreeSet::new();
    let mut components = Vec::new();
    for start in adjacency.keys() {
        if !visited.insert(start.clone()) {
            continue;
        }
        let mut stack = vec![start.clone()];
        let mut nodes = BTreeSet::new();
        nodes.insert(start.clone());
        while let Some(node) = stack.pop() {
            if let Some(neighbors) = adjacency.get(&node) {
                for neighbor in neighbors {
                    if visited.insert(neighbor.clone()) {
                        stack.push(neighbor.clone());
                    }
                    nodes.insert(neighbor.clone());
                }
            }
        }

        let mut component_edges = edge_by_pair
            .iter()
            .filter(|((left, right), _)| nodes.contains(left) && nodes.contains(right))
            .map(|(_, edge)| edge.clone())
            .collect::<Vec<_>>();
        sort_skill_relationship_edges(&mut component_edges);
        components.push(component_edges);
    }

    components
}

fn component_members(component: &[SkillRelationshipEdge]) -> Vec<String> {
    let mut members = BTreeSet::new();
    for edge in component {
        members.insert(edge.left_skill_name.clone());
        members.insert(edge.right_skill_name.clone());
    }
    members.into_iter().collect()
}

fn canonical_family_skill_name(component: &[SkillRelationshipEdge]) -> Option<String> {
    let mut votes = BTreeMap::<String, usize>::new();
    for edge in component {
        let Some(preferred_skill_name) = edge.preferred_skill_name.as_ref() else {
            continue;
        };
        *votes.entry(preferred_skill_name.clone()).or_default() += 1;
    }
    votes
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
        .map(|(skill_name, _)| skill_name)
}

fn family_member_role(
    skill_name: &str,
    canonical_skill_name: &str,
    component: &[SkillRelationshipEdge],
) -> agendao_types::SkillCapabilityMemberRole {
    if skill_name.eq_ignore_ascii_case(canonical_skill_name) {
        return agendao_types::SkillCapabilityMemberRole::Canonical;
    }
    if component.iter().any(|edge| {
        edge.relation_kind == agendao_types::SkillRelationshipKind::SpecializationVariant
            && (edge.left_skill_name.eq_ignore_ascii_case(skill_name)
                || edge.right_skill_name.eq_ignore_ascii_case(skill_name))
    }) {
        agendao_types::SkillCapabilityMemberRole::Specialization
    } else {
        agendao_types::SkillCapabilityMemberRole::MergeCandidate
    }
}

fn component_reasons(component: &[SkillRelationshipEdge]) -> Vec<String> {
    let mut reasons = Vec::new();
    for edge in component {
        reasons.extend(edge.reasons.clone());
    }
    let mut reasons = dedupe_string_reasons(reasons);
    reasons.truncate(6);
    reasons
}

fn build_capability_group_id(
    group_kind: agendao_types::SkillCapabilityGroupKind,
    members: &[String],
) -> String {
    let prefix = match group_kind {
        agendao_types::SkillCapabilityGroupKind::CanonicalFamily => "canonical_family",
        agendao_types::SkillCapabilityGroupKind::ComplementaryBundle => "complementary_bundle",
    };
    let normalized_members = members
        .iter()
        .map(|member| normalize_name(member))
        .collect::<Vec<_>>()
        .join("+");
    format!("{prefix}:{normalized_members}")
}

fn sort_skill_capability_members(members: &mut [SkillCapabilityMember]) {
    members.sort_by(|left, right| {
        capability_member_role_sort_key(left.role)
            .cmp(&capability_member_role_sort_key(right.role))
            .then_with(|| left.skill_name.cmp(&right.skill_name))
    });
}

fn capability_member_role_sort_key(role: agendao_types::SkillCapabilityMemberRole) -> u8 {
    match role {
        agendao_types::SkillCapabilityMemberRole::Canonical => 0,
        agendao_types::SkillCapabilityMemberRole::Specialization => 1,
        agendao_types::SkillCapabilityMemberRole::MergeCandidate => 2,
        agendao_types::SkillCapabilityMemberRole::Complementary => 3,
    }
}

fn sort_skill_capability_groups(groups: &mut [SkillCapabilityGroup]) {
    groups.sort_by(|left, right| {
        capability_group_kind_sort_key(left.group_kind)
            .cmp(&capability_group_kind_sort_key(right.group_kind))
            .then_with(|| right.members.len().cmp(&left.members.len()))
            .then_with(|| left.capability_id.cmp(&right.capability_id))
    });
}

fn sort_runtime_composition_hints(hints: &mut [SkillRuntimeCompositionHint]) {
    hints.sort_by(|left, right| {
        runtime_composition_hint_sort_key(left.kind)
            .cmp(&runtime_composition_hint_sort_key(right.kind))
            .then_with(|| left.skill_names.cmp(&right.skill_names))
            .then_with(|| left.preferred_skill_name.cmp(&right.preferred_skill_name))
            .then_with(|| left.capability_id.cmp(&right.capability_id))
    });
}

fn capability_group_kind_sort_key(kind: agendao_types::SkillCapabilityGroupKind) -> u8 {
    match kind {
        agendao_types::SkillCapabilityGroupKind::CanonicalFamily => 0,
        agendao_types::SkillCapabilityGroupKind::ComplementaryBundle => 1,
    }
}

fn runtime_composition_hint_sort_key(kind: SkillRuntimeCompositionHintKind) -> u8 {
    match kind {
        SkillRuntimeCompositionHintKind::PreferCanonicalSkill => 0,
        SkillRuntimeCompositionHintKind::ComplementaryBundle => 1,
    }
}

fn relationship_identity_key(
    left_skill_name: &str,
    right_skill_name: &str,
    relation_kind: agendao_types::SkillRelationshipKind,
) -> (String, String, u8) {
    let (left, right) = relationship_pair_key(left_skill_name, right_skill_name);
    (left, right, relationship_kind_sort_key(relation_kind))
}

fn relationship_edge_identity_key(edge: &SkillRelationshipEdge) -> (String, String, u8) {
    relationship_identity_key(
        &edge.left_skill_name,
        &edge.right_skill_name,
        edge.relation_kind,
    )
}

fn merge_relationship_inspection_entry(
    stored: &SkillRelationshipEdge,
    candidate: &SkillRelationshipEdge,
) -> SkillRelationshipEdge {
    SkillRelationshipEdge {
        left_skill_name: candidate.left_skill_name.clone(),
        right_skill_name: candidate.right_skill_name.clone(),
        relation_kind: stored.relation_kind,
        state: stored.state,
        score: candidate.score.max(stored.score),
        reasons: if stored.reasons.is_empty() {
            candidate.reasons.clone()
        } else {
            dedupe_string_reasons(
                stored
                    .reasons
                    .iter()
                    .cloned()
                    .chain(candidate.reasons.iter().cloned())
                    .collect(),
            )
        },
        preferred_skill_name: stored
            .preferred_skill_name
            .clone()
            .or_else(|| candidate.preferred_skill_name.clone()),
        observed_at: stored.observed_at.or(candidate.observed_at),
        updated_at: stored.updated_at,
    }
}

fn merge_capability_group_inspection_entry(
    stored: &SkillCapabilityGroup,
    candidate: &SkillCapabilityGroup,
) -> SkillCapabilityGroup {
    SkillCapabilityGroup {
        capability_id: stored.capability_id.clone(),
        group_kind: stored.group_kind,
        state: stored.state,
        canonical_skill_name: stored
            .canonical_skill_name
            .clone()
            .or_else(|| candidate.canonical_skill_name.clone()),
        members: if stored.members.is_empty() {
            candidate.members.clone()
        } else {
            stored.members.clone()
        },
        reasons: if stored.reasons.is_empty() {
            candidate.reasons.clone()
        } else {
            dedupe_string_reasons(
                stored
                    .reasons
                    .iter()
                    .cloned()
                    .chain(candidate.reasons.iter().cloned())
                    .collect(),
            )
        },
        updated_at: stored.updated_at,
    }
}

fn validate_relationship_preferred_skill(
    left_skill_name: &str,
    right_skill_name: &str,
    relation_kind: agendao_types::SkillRelationshipKind,
    existing_preferred_skill_name: Option<&str>,
    requested_preferred_skill_name: Option<&str>,
) -> Result<Option<String>, SkillError> {
    match relation_kind {
        agendao_types::SkillRelationshipKind::RedundantOverlap
        | agendao_types::SkillRelationshipKind::SpecializationVariant => {
            let preferred_skill_name = requested_preferred_skill_name
                .or(existing_preferred_skill_name)
                .ok_or_else(|| SkillError::InvalidSkillContent {
                    message: format!(
                        "relationship `{}` requires a preferred canonical skill",
                        format_skill_relationship_kind(relation_kind)
                    ),
                })?;
            canonicalize_pair_skill_name(left_skill_name, right_skill_name, preferred_skill_name)
                .map(Some)
        }
        agendao_types::SkillRelationshipKind::ComplementaryComponent => {
            if requested_preferred_skill_name.is_some() {
                return Err(SkillError::InvalidSkillContent {
                    message: "complementary_component does not allow preferred_skill_name"
                        .to_string(),
                });
            }
            Ok(None)
        }
    }
}

fn canonicalize_pair_skill_name(
    left_skill_name: &str,
    right_skill_name: &str,
    requested_skill_name: &str,
) -> Result<String, SkillError> {
    let requested = required_nonempty_text(requested_skill_name, "preferred_skill_name")?;
    if left_skill_name.eq_ignore_ascii_case(&requested) {
        return Ok(left_skill_name.to_string());
    }
    if right_skill_name.eq_ignore_ascii_case(&requested) {
        return Ok(right_skill_name.to_string());
    }
    Err(SkillError::InvalidSkillContent {
        message: format!(
            "preferred skill `{}` must match one of `{}` or `{}`",
            requested, left_skill_name, right_skill_name
        ),
    })
}

fn validate_capability_group_input<F>(
    capability_id: Option<&str>,
    group_kind: agendao_types::SkillCapabilityGroupKind,
    canonical_skill_name: Option<&str>,
    members: Vec<SkillCapabilityMember>,
    reasons: Vec<String>,
    candidate_group: Option<&SkillCapabilityGroup>,
    mut resolve_skill_name: F,
) -> Result<SkillCapabilityGroup, SkillError>
where
    F: FnMut(&str) -> Result<String, SkillError>,
{
    if members.len() < 2 {
        return Err(SkillError::InvalidSkillContent {
            message: "capability group requires at least 2 members".to_string(),
        });
    }

    let mut seen = BTreeSet::new();
    let mut resolved_members = Vec::with_capacity(members.len());
    for member in members {
        let resolved_skill_name = resolve_skill_name(&member.skill_name)?;
        let normalized = normalize_name(&resolved_skill_name);
        if !seen.insert(normalized) {
            return Err(SkillError::InvalidSkillContent {
                message: format!(
                    "capability group contains duplicate member `{}`",
                    resolved_skill_name
                ),
            });
        }
        resolved_members.push(SkillCapabilityMember {
            skill_name: resolved_skill_name,
            role: member.role,
        });
    }

    let cleaned_reasons = {
        let cleaned = dedupe_string_reasons(
            reasons
                .into_iter()
                .map(|reason| reason.trim().to_string())
                .filter(|reason| !reason.is_empty())
                .collect(),
        );
        if cleaned.is_empty() {
            candidate_group
                .map(|group| group.reasons.clone())
                .unwrap_or_default()
        } else {
            cleaned
        }
    };

    let mut canonical_skill_name = canonical_skill_name.map(resolve_skill_name).transpose()?;
    match group_kind {
        agendao_types::SkillCapabilityGroupKind::CanonicalFamily => {
            let Some(canonical_name) = canonical_skill_name.clone() else {
                return Err(SkillError::InvalidSkillContent {
                    message: "canonical_family group requires canonical_skill_name".to_string(),
                });
            };
            let canonical_count = resolved_members
                .iter()
                .filter(|member| member.role == agendao_types::SkillCapabilityMemberRole::Canonical)
                .count();
            if canonical_count > 1
                || (canonical_count == 1
                    && !resolved_members.iter().any(|member| {
                        member.role == agendao_types::SkillCapabilityMemberRole::Canonical
                            && member.skill_name.eq_ignore_ascii_case(&canonical_name)
                    }))
            {
                return Err(SkillError::InvalidSkillContent {
                    message: "canonical_family group must have exactly one canonical member matching canonical_skill_name".to_string(),
                });
            }
            if !resolved_members
                .iter()
                .any(|member| member.skill_name.eq_ignore_ascii_case(&canonical_name))
            {
                return Err(SkillError::InvalidSkillContent {
                    message: format!(
                        "canonical skill `{}` must appear in capability group members",
                        canonical_name
                    ),
                });
            }

            for member in &mut resolved_members {
                if member.skill_name.eq_ignore_ascii_case(&canonical_name) {
                    member.role = agendao_types::SkillCapabilityMemberRole::Canonical;
                } else if matches!(
                    member.role,
                    agendao_types::SkillCapabilityMemberRole::Canonical
                        | agendao_types::SkillCapabilityMemberRole::Complementary
                ) {
                    return Err(SkillError::InvalidSkillContent {
                        message: format!(
                            "canonical_family member `{}` must use specialization or merge_candidate role",
                            member.skill_name
                        ),
                    });
                }
            }
        }
        agendao_types::SkillCapabilityGroupKind::ComplementaryBundle => {
            if canonical_skill_name.is_some() {
                return Err(SkillError::InvalidSkillContent {
                    message: "complementary_bundle does not allow canonical_skill_name".to_string(),
                });
            }
            canonical_skill_name = None;
            if resolved_members.iter().any(|member| {
                member.role != agendao_types::SkillCapabilityMemberRole::Complementary
            }) {
                return Err(SkillError::InvalidSkillContent {
                    message: "complementary_bundle members must all use complementary role"
                        .to_string(),
                });
            }
        }
    }

    sort_skill_capability_members(&mut resolved_members);
    let capability_id = capability_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            build_capability_group_id(
                group_kind,
                &resolved_members
                    .iter()
                    .map(|member| member.skill_name.clone())
                    .collect::<Vec<_>>(),
            )
        });

    Ok(SkillCapabilityGroup {
        capability_id,
        group_kind,
        state: agendao_types::SkillCapabilityGroupState::Candidate,
        canonical_skill_name,
        members: resolved_members,
        reasons: cleaned_reasons,
        updated_at: None,
    })
}

fn validate_capability_group_member_role_update(
    group: &SkillCapabilityGroup,
    role: agendao_types::SkillCapabilityMemberRole,
) -> Result<(), SkillError> {
    match group.group_kind {
        agendao_types::SkillCapabilityGroupKind::CanonicalFamily => {
            if matches!(
                role,
                agendao_types::SkillCapabilityMemberRole::Canonical
                    | agendao_types::SkillCapabilityMemberRole::Complementary
            ) {
                return Err(SkillError::InvalidSkillContent {
                    message: format!(
                        "canonical_family member role update only supports specialization or merge_candidate for `{}`",
                        group.capability_id
                    ),
                });
            }
        }
        agendao_types::SkillCapabilityGroupKind::ComplementaryBundle => {
            if role != agendao_types::SkillCapabilityMemberRole::Complementary {
                return Err(SkillError::InvalidSkillContent {
                    message: format!(
                        "complementary_bundle member role update only supports complementary for `{}`",
                        group.capability_id
                    ),
                });
            }
        }
    }
    Ok(())
}

fn required_nonempty_text(value: &str, field_name: &str) -> Result<String, SkillError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(SkillError::InvalidSkillContent {
            message: format!("{field_name} cannot be empty"),
        });
    }
    Ok(trimmed.to_string())
}

fn format_skill_relationship_kind(kind: agendao_types::SkillRelationshipKind) -> &'static str {
    match kind {
        agendao_types::SkillRelationshipKind::RedundantOverlap => "redundant_overlap",
        agendao_types::SkillRelationshipKind::SpecializationVariant => "specialization_variant",
        agendao_types::SkillRelationshipKind::ComplementaryComponent => "complementary_component",
    }
}

fn format_skill_relationship_state(state: agendao_types::SkillRelationshipState) -> &'static str {
    match state {
        agendao_types::SkillRelationshipState::Observed => "observed",
        agendao_types::SkillRelationshipState::Accepted => "accepted",
        agendao_types::SkillRelationshipState::Dismissed => "dismissed",
    }
}

fn format_capability_group_kind(kind: agendao_types::SkillCapabilityGroupKind) -> &'static str {
    match kind {
        agendao_types::SkillCapabilityGroupKind::CanonicalFamily => "canonical_family",
        agendao_types::SkillCapabilityGroupKind::ComplementaryBundle => "complementary_bundle",
    }
}

fn format_capability_group_state(state: agendao_types::SkillCapabilityGroupState) -> &'static str {
    match state {
        agendao_types::SkillCapabilityGroupState::Candidate => "candidate",
        agendao_types::SkillCapabilityGroupState::Active => "active",
        agendao_types::SkillCapabilityGroupState::Dismissed => "dismissed",
    }
}

fn format_capability_member_role(role: agendao_types::SkillCapabilityMemberRole) -> &'static str {
    match role {
        agendao_types::SkillCapabilityMemberRole::Canonical => "canonical",
        agendao_types::SkillCapabilityMemberRole::Specialization => "specialization",
        agendao_types::SkillCapabilityMemberRole::Complementary => "complementary",
        agendao_types::SkillCapabilityMemberRole::MergeCandidate => "merge_candidate",
    }
}

fn format_runtime_prefer_canonical_hint(
    skill_name: &str,
    preferred_skill_name: &str,
    role: Option<SkillCapabilityMemberRole>,
    capability_id: Option<&str>,
) -> String {
    let (relation_label, closing_clause) = match role {
        Some(SkillCapabilityMemberRole::Specialization) => (
            "specialization variant",
            format!("only use `{skill_name}` for its narrower responsibility"),
        ),
        Some(SkillCapabilityMemberRole::MergeCandidate) => (
            "merge candidate",
            format!("avoid splitting duplicate instructions across both skills"),
        ),
        _ => (
            "related member",
            format!("keep `{preferred_skill_name}` as the family owner when the two overlap"),
        ),
    };
    let family_clause = capability_id
        .map(|value| format!(" within canonical family `{}`", value.trim()))
        .unwrap_or_default();
    format!(
        "Skill `{skill_name}` is governed as a {relation_label}{family_clause} under preferred skill `{preferred_skill_name}`. Prefer the canonical workflow as the family owner, and {closing_clause}."
    )
}

fn format_runtime_complementary_bundle_hint(
    skill_names: &[String],
    capability_id: Option<&str>,
) -> String {
    let listed = skill_names
        .iter()
        .map(|skill_name| format!("`{skill_name}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let bundle_clause = capability_id
        .map(|value| format!(" in complementary bundle `{}`", value.trim()))
        .unwrap_or_default();
    format!(
        "Skills {listed} are governed as complementary components{bundle_clause}. Keep their responsibilities distinct and do not collapse one skill into another."
    )
}

fn dedupe_string_reasons(reasons: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for reason in reasons {
        let normalized = normalize_name(&reason);
        if normalized.is_empty() || !seen.insert(normalized) {
            continue;
        }
        deduped.push(reason);
    }
    deduped
}

fn join_terms<'a>(terms: impl IntoIterator<Item = &'a String>) -> String {
    terms
        .into_iter()
        .map(|term| term.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn set_jaccard_ratio(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let shared = set_intersection_count(left, right) as f32;
    let union = (left.len() + right.len() - shared as usize) as f32;
    if union <= 0.0 {
        0.0
    } else {
        shared / union
    }
}

fn skill_descriptor_tokens(value: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "and", "are", "be", "for", "from", "how", "into", "of", "or", "that", "the",
        "this", "to", "use", "with",
    ];

    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| part.len() >= 3)
        .filter(|part| !STOP_WORDS.contains(&part.as_str()))
        .collect()
}

fn managed_record_timeline_entry(record: ManagedSkillRecord) -> SkillGovernanceTimelineEntry {
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

fn audit_event_timeline_entry(
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
        SkillAuditKind::ArtifactFetched => format!(
            "{}",
            payload_string(&event.payload, "artifact_locator")
                .unwrap_or_else(|| "artifact cached".to_string())
        ),
        SkillAuditKind::ArtifactEvicted => format!(
            "{} · retention {}s",
            payload_string(&event.payload, "artifact_locator")
                .unwrap_or_else(|| "artifact cache entry".to_string()),
            payload_usize(&event.payload, "retention_seconds").unwrap_or_default()
        ),
        SkillAuditKind::ArtifactFetchFailed => format!(
            "{}",
            payload_string(&event.payload, "error")
                .unwrap_or_else(|| "artifact fetch failed".to_string())
        ),
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
        | SkillAuditKind::Delete => format!(
            "{}",
            payload_string(&event.payload, "location").unwrap_or_else(|| "workspace write".into())
        ),
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

fn payload_skill_names(payload: &Value) -> Vec<String> {
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

fn format_skill_vitality_state(state: SkillVitalityState) -> &'static str {
    match state {
        SkillVitalityState::Active => "active",
        SkillVitalityState::ReviewCandidate => "review_candidate",
        SkillVitalityState::Retired => "retired",
        SkillVitalityState::Archived => "archived",
    }
}

fn format_skill_retirement_reason_kind(kind: SkillRetirementReasonKind) -> &'static str {
    match kind {
        SkillRetirementReasonKind::NegativeEntropy => "negative_entropy",
        SkillRetirementReasonKind::SemanticConflict => "semantic_conflict",
        SkillRetirementReasonKind::ManualOverride => "manual_override",
        SkillRetirementReasonKind::Restored => "restored",
    }
}

fn now_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}
