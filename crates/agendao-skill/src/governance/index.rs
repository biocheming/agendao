use super::audit::{
    audit_event_timeline_entry, managed_record_timeline_entry, payload_skill_names,
    source_index_refresh_audit_event,
};
use super::normalize_name;
use super::SkillGovernanceAuthority;
use crate::SkillError;
use agendao_types::{
    SkillAuditEvent, SkillGovernanceTimelineEntry, SkillHubSearchMatch, SkillHubSearchRequest,
    SkillHubSearchResponse, SkillHubTimelineQuery, SkillSourceIndexEntry, SkillSourceIndexSnapshot,
    SkillSourceRef,
};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_INDEX_FRESHNESS_MAX_AGE_SECONDS: u64 = 604_800; // 7 days

impl SkillGovernanceAuthority {
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
