use super::audit::{
    capability_group_activated_audit_event, capability_group_member_removed_audit_event,
    capability_group_member_role_updated_audit_event, composition_relationship_audit_event,
};
use super::relationships::{
    build_skill_complementary_relationship_candidate, build_skill_redundant_relationship_candidate,
    build_skill_specialization_relationship_candidate, format_runtime_complementary_bundle_hint,
    format_runtime_prefer_canonical_hint, format_skill_relationship_kind,
    merge_relationship_inspection_entry, normalize_runtime_selected_skill_names,
    ordered_skill_names, relationship_edge_identity_key, relationship_identity_key,
    relationship_other_skill_name, relationship_pair_key, skill_names_includes_pair,
    sort_skill_relationship_edges, validate_relationship_preferred_skill,
};
use super::semantic::collect_skill_semantic_conflicts;
use super::{
    dedupe_string_reasons, normalize_name, required_nonempty_text,
    SkillCompositionConsumptionContext, SkillGovernanceAuthority,
};
use crate::util::now_unix_timestamp;
use crate::SkillError;
use agendao_types::{
    SkillCapabilityGroup, SkillCapabilityGroupKind, SkillCapabilityMember,
    SkillCapabilityMemberRole, SkillRelationshipEdge, SkillRelationshipKind,
    SkillRelationshipState, SkillRuntimeCompositionHint, SkillRuntimeCompositionHintKind,
};
use std::collections::{BTreeMap, BTreeSet};

impl SkillGovernanceAuthority {
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

    pub(super) fn skill_composition_consumption_context(
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

fn canonical_member_skill_name(group: &SkillCapabilityGroup) -> Option<String> {
    group
        .members
        .iter()
        .find(|member| member.role == SkillCapabilityMemberRole::Canonical)
        .map(|member| member.skill_name.clone())
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

pub(super) fn format_capability_group_kind(
    kind: agendao_types::SkillCapabilityGroupKind,
) -> &'static str {
    match kind {
        agendao_types::SkillCapabilityGroupKind::CanonicalFamily => "canonical_family",
        agendao_types::SkillCapabilityGroupKind::ComplementaryBundle => "complementary_bundle",
    }
}

pub(super) fn format_capability_group_state(
    state: agendao_types::SkillCapabilityGroupState,
) -> &'static str {
    match state {
        agendao_types::SkillCapabilityGroupState::Candidate => "candidate",
        agendao_types::SkillCapabilityGroupState::Active => "active",
        agendao_types::SkillCapabilityGroupState::Dismissed => "dismissed",
    }
}

pub(super) fn format_capability_member_role(
    role: agendao_types::SkillCapabilityMemberRole,
) -> &'static str {
    match role {
        agendao_types::SkillCapabilityMemberRole::Canonical => "canonical",
        agendao_types::SkillCapabilityMemberRole::Specialization => "specialization",
        agendao_types::SkillCapabilityMemberRole::Complementary => "complementary",
        agendao_types::SkillCapabilityMemberRole::MergeCandidate => "merge_candidate",
    }
}
