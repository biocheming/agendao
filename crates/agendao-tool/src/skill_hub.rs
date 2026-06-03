use async_trait::async_trait;
use agendao_types::{
    SkillHubArtifactCacheResponse, SkillHubAuditResponse, SkillHubDistributionResponse,
    SkillHubGuardRunResponse, SkillHubIndexRefreshResponse, SkillHubIndexResponse,
    SkillHubLifecycleResponse, SkillHubManagedDetachResponse, SkillHubManagedRemoveResponse,
    SkillHubManagedResponse, SkillRemoteInstallResponse, SkillSourceKind, SkillSourceRef,
};
use serde::Deserialize;
use std::path::Path;

use crate::skill_support::{governance_authority_for, map_skill_error};
use crate::{PermissionRequest, Tool, ToolContext, ToolError, ToolResult};

pub struct SkillHubTool;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SkillHubAction {
    Managed,
    Search,
    Index,
    DistributionList,
    ArtifactCache,
    Lifecycle,
    IndexRefresh,
    InstallPlan,
    InstallApply,
    UpdatePlan,
    UpdateApply,
    Detach,
    Remove,
    Audit,
    GuardRun,
    SyncPlan,
    SyncApply,
}

#[derive(Debug, Clone, Deserialize)]
struct SkillHubInput {
    action: SkillHubAction,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    source_kind: Option<SkillSourceKind>,
    #[serde(default)]
    locator: Option<String>,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    skill_name: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for SkillHubTool {
    fn id(&self) -> &str {
        "skill_hub"
    }

    fn description(&self) -> &str {
        "Search indexed skill sources by name, description, category, version, and revision; inspect managed skill governance state; refresh source indices; and create/apply hub sync plans for supported sources."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["managed", "search", "index", "distribution_list", "artifact_cache", "lifecycle", "index_refresh", "install_plan", "install_apply", "update_plan", "update_apply", "detach", "remove", "audit", "guard_run", "sync_plan", "sync_apply"]
                },
                "query": {
                    "type": "string",
                    "description": "Optional free-text query matched against skill name, description, category, version, and revision across indexed sources."
                },
                "skill_name": {
                    "type": "string",
                    "description": "Existing resolved skill name for a workspace guard scan."
                },
                "source_id": {
                    "type": "string",
                    "description": "Stable source identifier such as bundled:core or local:/skills."
                },
                "source_kind": {
                    "type": "string",
                    "enum": ["bundled", "local_path", "git", "archive", "registry"]
                },
                "locator": {
                    "type": "string",
                    "description": "Source root path for bundled/local_path sync."
                },
                "revision": {
                    "type": "string",
                    "description": "Optional revision label recorded in managed records and audit."
                },
                "limit": {
                    "type": "integer",
                    "description": "Optional max number of search results to return. Defaults to 20 and caps at 100."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: SkillHubInput = serde_json::from_value(args)
            .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
        let authority =
            governance_authority_for(Path::new(&ctx.directory), ctx.config_store.clone());

        match input.action {
            SkillHubAction::Managed => {
                let managed_skills = authority
                    .refresh_managed_workspace_state()
                    .map_err(map_skill_error)?;
                let response = SkillHubManagedResponse { managed_skills };
                Ok(ToolResult::simple(
                    "Skill hub managed",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "managed_skills",
                    serde_json::to_value(&response.managed_skills).unwrap_or_default(),
                ))
            }
            SkillHubAction::Search => {
                let response =
                    authority.search_source_indices(&agendao_types::SkillHubSearchRequest {
                        query: optional_trimmed(input.query.clone()),
                        source_id: optional_trimmed(input.source_id.clone()),
                        source_kind: input.source_kind.clone(),
                        limit: input.limit,
                    });
                Ok(ToolResult::simple(
                    "Skill hub search",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "matches",
                    serde_json::to_value(&response.matches).unwrap_or_default(),
                ))
            }
            SkillHubAction::Index => {
                let response = SkillHubIndexResponse {
                    source_indices: authority.governance_snapshot().source_indices,
                };
                Ok(ToolResult::simple(
                    "Skill hub index",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "source_indices",
                    serde_json::to_value(&response.source_indices).unwrap_or_default(),
                ))
            }
            SkillHubAction::DistributionList => {
                let response = SkillHubDistributionResponse {
                    distributions: authority.distributions(),
                };
                Ok(ToolResult::simple(
                    "Skill hub distributions",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "distributions",
                    serde_json::to_value(&response.distributions).unwrap_or_default(),
                ))
            }
            SkillHubAction::ArtifactCache => {
                let response = SkillHubArtifactCacheResponse {
                    artifact_cache: authority.artifact_cache(),
                };
                Ok(ToolResult::simple(
                    "Skill hub artifact cache",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "artifact_cache",
                    serde_json::to_value(&response.artifact_cache).unwrap_or_default(),
                ))
            }
            SkillHubAction::Lifecycle => {
                let response = SkillHubLifecycleResponse {
                    lifecycle: authority.lifecycle_records(),
                };
                Ok(ToolResult::simple(
                    "Skill hub lifecycle",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "lifecycle",
                    serde_json::to_value(&response.lifecycle).unwrap_or_default(),
                ))
            }
            SkillHubAction::IndexRefresh => {
                let source = resolve_source_ref(&input)?;
                let response = SkillHubIndexRefreshResponse {
                    snapshot: authority
                        .refresh_source_index(&source, "tool:skill_hub")
                        .map_err(map_skill_error)?,
                };
                Ok(ToolResult::simple(
                    "Skill hub index refresh",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "snapshot",
                    serde_json::to_value(&response.snapshot).unwrap_or_default(),
                ))
            }
            SkillHubAction::InstallPlan => {
                let source = resolve_source_ref(&input)?;
                let skill_name = required_string(input.skill_name.clone(), "skill_name")?;
                let response = authority
                    .plan_remote_install(&source, &skill_name, "tool:skill_hub")
                    .map_err(map_skill_error)?;
                Ok(ToolResult::simple(
                    "Skill hub install plan",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata("plan", serde_json::to_value(&response).unwrap_or_default()))
            }
            SkillHubAction::InstallApply => {
                let source = resolve_source_ref(&input)?;
                let skill_name = required_string(input.skill_name.clone(), "skill_name")?;
                ctx.ask_permission(build_install_permission_request(&source, &skill_name))
                    .await?;
                let response: SkillRemoteInstallResponse = authority
                    .apply_remote_install(&source, &skill_name, "tool:skill_hub")
                    .map_err(map_skill_error)?;
                ctx.do_publish_bus(
                    "skill.hub.remote_install_applied",
                    serde_json::json!({
                        "source_id": source.source_id,
                        "skill_name": skill_name,
                        "distribution_id": response.plan.distribution.distribution_id,
                        "action": response.plan.entry.action,
                    }),
                )
                .await;
                Ok(ToolResult::simple(
                    "Skill hub remote install",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "response",
                    serde_json::to_value(&response).unwrap_or_default(),
                ))
            }
            SkillHubAction::UpdatePlan => {
                let source = resolve_source_ref(&input)?;
                let skill_name = required_string(input.skill_name.clone(), "skill_name")?;
                let response = authority
                    .plan_remote_update(&source, &skill_name, "tool:skill_hub")
                    .map_err(map_skill_error)?;
                Ok(ToolResult::simple(
                    "Skill hub update plan",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata("plan", serde_json::to_value(&response).unwrap_or_default()))
            }
            SkillHubAction::UpdateApply => {
                let source = resolve_source_ref(&input)?;
                let skill_name = required_string(input.skill_name.clone(), "skill_name")?;
                ctx.ask_permission(build_skill_permission_request(
                    &source,
                    &skill_name,
                    "update_apply",
                ))
                .await?;
                let response: SkillRemoteInstallResponse = authority
                    .apply_remote_update(&source, &skill_name, "tool:skill_hub")
                    .map_err(map_skill_error)?;
                ctx.do_publish_bus(
                    "skill.hub.remote_update_applied",
                    serde_json::json!({
                        "source_id": source.source_id,
                        "skill_name": skill_name,
                        "distribution_id": response.plan.distribution.distribution_id,
                        "action": response.plan.entry.action,
                    }),
                )
                .await;
                Ok(ToolResult::simple(
                    "Skill hub remote update",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "response",
                    serde_json::to_value(&response).unwrap_or_default(),
                ))
            }
            SkillHubAction::Detach => {
                let source = resolve_source_ref(&input)?;
                let skill_name = required_string(input.skill_name.clone(), "skill_name")?;
                ctx.ask_permission(build_skill_permission_request(
                    &source,
                    &skill_name,
                    "detach",
                ))
                .await?;
                let response: SkillHubManagedDetachResponse = authority
                    .detach_managed_skill(&source, &skill_name, "tool:skill_hub")
                    .map_err(map_skill_error)?;
                ctx.do_publish_bus(
                    "skill.hub.managed_detached",
                    serde_json::json!({
                        "source_id": source.source_id,
                        "skill_name": skill_name,
                        "distribution_id": response.lifecycle.distribution_id,
                        "state": response.lifecycle.state,
                    }),
                )
                .await;
                Ok(ToolResult::simple(
                    "Skill hub managed detach",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "response",
                    serde_json::to_value(&response).unwrap_or_default(),
                ))
            }
            SkillHubAction::Remove => {
                let source = resolve_source_ref(&input)?;
                let skill_name = required_string(input.skill_name.clone(), "skill_name")?;
                ctx.ask_permission(build_skill_permission_request(
                    &source,
                    &skill_name,
                    "remove",
                ))
                .await?;
                let response: SkillHubManagedRemoveResponse = authority
                    .remove_managed_skill(&source, &skill_name, "tool:skill_hub")
                    .map_err(map_skill_error)?;
                ctx.do_publish_bus(
                    "skill.hub.managed_removed",
                    serde_json::json!({
                        "source_id": source.source_id,
                        "skill_name": skill_name,
                        "distribution_id": response.lifecycle.distribution_id,
                        "state": response.lifecycle.state,
                        "deleted_from_workspace": response.deleted_from_workspace,
                    }),
                )
                .await;
                Ok(ToolResult::simple(
                    "Skill hub managed remove",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "response",
                    serde_json::to_value(&response).unwrap_or_default(),
                ))
            }
            SkillHubAction::Audit => {
                let response = SkillHubAuditResponse {
                    audit_events: authority.audit_tail(),
                };
                Ok(ToolResult::simple(
                    "Skill hub audit",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "audit_events",
                    serde_json::to_value(&response.audit_events).unwrap_or_default(),
                ))
            }
            SkillHubAction::GuardRun => {
                let response = if let Some(skill_name) = optional_trimmed(input.skill_name.clone())
                {
                    SkillHubGuardRunResponse {
                        reports: authority
                            .run_guard_for_skill(&skill_name, "tool:skill_hub")
                            .map_err(map_skill_error)?,
                    }
                } else {
                    let source = resolve_source_ref(&input)?;
                    SkillHubGuardRunResponse {
                        reports: authority
                            .run_guard_for_source(&source, "tool:skill_hub")
                            .map_err(map_skill_error)?,
                    }
                };
                Ok(ToolResult::simple(
                    "Skill hub guard run",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "reports",
                    serde_json::to_value(&response.reports).unwrap_or_default(),
                ))
            }
            SkillHubAction::SyncPlan => {
                let source = resolve_source_ref(&input)?;
                let response = authority
                    .plan_sync(&source)
                    .map(|plan| agendao_types::SkillHubSyncPlanResponse {
                        plan,
                        guard_reports: Vec::new(),
                    })
                    .map_err(map_skill_error)?;
                Ok(ToolResult::simple(
                    "Skill hub sync plan",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "plan",
                    serde_json::to_value(&response.plan).unwrap_or_default(),
                ))
            }
            SkillHubAction::SyncApply => {
                let source = resolve_source_ref(&input)?;
                ctx.ask_permission(build_permission_request(&source))
                    .await?;
                let response = authority
                    .apply_sync(&source, "tool:skill_hub")
                    .map(|result| agendao_types::SkillHubSyncPlanResponse {
                        plan: result.plan,
                        guard_reports: result.guard_reports,
                    })
                    .map_err(map_skill_error)?;
                ctx.do_publish_bus(
                    "skill.hub.sync_applied",
                    serde_json::json!({
                        "source_id": source.source_id,
                        "source_kind": format!("{:?}", source.source_kind).to_ascii_lowercase(),
                        "locator": source.locator,
                        "entry_count": response.plan.entries.len(),
                        "guard_reports": response.guard_reports,
                    }),
                )
                .await;
                Ok(ToolResult::simple(
                    "Skill hub sync apply",
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )
                .with_metadata(
                    "plan",
                    serde_json::to_value(&response.plan).unwrap_or_default(),
                ))
            }
        }
    }
}

impl Default for SkillHubTool {
    fn default() -> Self {
        Self
    }
}

fn resolve_source_ref(input: &SkillHubInput) -> Result<SkillSourceRef, ToolError> {
    let source_id = required_string(input.source_id.clone(), "source_id")?;
    let source_kind = input
        .source_kind
        .clone()
        .ok_or_else(|| ToolError::InvalidArguments("source_kind is required".to_string()))?;
    let locator = required_string(input.locator.clone(), "locator")?;
    Ok(SkillSourceRef {
        source_id,
        source_kind,
        locator,
        revision: optional_trimmed(input.revision.clone()),
    })
}

fn build_permission_request(source: &SkillSourceRef) -> PermissionRequest {
    let mut request = PermissionRequest::new("skill_hub")
        .with_pattern(source.source_id.clone())
        .with_metadata("action", serde_json::json!("sync_apply"))
        .with_metadata("source_id", serde_json::json!(source.source_id))
        .with_metadata(
            "source_kind",
            serde_json::json!(format!("{:?}", source.source_kind).to_ascii_lowercase()),
        )
        .with_metadata("locator", serde_json::json!(source.locator));
    if let Some(revision) = source.revision.as_deref() {
        request = request.with_metadata("revision", serde_json::json!(revision));
    }
    request
}

fn build_install_permission_request(
    source: &SkillSourceRef,
    skill_name: &str,
) -> PermissionRequest {
    build_skill_permission_request(source, skill_name, "install_apply")
}

fn build_skill_permission_request(
    source: &SkillSourceRef,
    skill_name: &str,
    action: &str,
) -> PermissionRequest {
    let mut request = PermissionRequest::new("skill_hub")
        .with_pattern(source.source_id.clone())
        .with_pattern(skill_name.to_string())
        .with_metadata("action", serde_json::json!(action))
        .with_metadata("source_id", serde_json::json!(source.source_id))
        .with_metadata("skill_name", serde_json::json!(skill_name))
        .with_metadata(
            "source_kind",
            serde_json::json!(format!("{:?}", source.source_kind).to_ascii_lowercase()),
        )
        .with_metadata("locator", serde_json::json!(source.locator));
    if let Some(revision) = source.revision.as_deref() {
        request = request.with_metadata("revision", serde_json::json!(revision));
    }
    request
}

fn required_string(value: Option<String>, field: &str) -> Result<String, ToolError> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments(format!("{field} is required")))
}

fn optional_trimmed(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
