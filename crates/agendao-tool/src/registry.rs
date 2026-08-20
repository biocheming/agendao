use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::tool_access;
use crate::{Tool, ToolContext, ToolError, ToolRegistryAccess, ToolResult, ToolSchema};
use agendao_plugin::{HookContext, HookEvent};
use agendao_types::{
    ExternalContentProvenance, ExternalContentSourceKind, ToolCatalogMetadata,
    EXTERNAL_CONTENT_PROVENANCE_METADATA_KEY,
};

/// Tools that should not appear in suggestion lists when a tool is not found.
const FILTERED_FROM_SUGGESTIONS: &[&str] = &["invalid", "patch", "batch"];

pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
}

struct CatalogedTool<T> {
    inner: T,
    catalog: ToolCatalogMetadata,
}

impl<T> CatalogedTool<T> {
    fn new(inner: T, catalog: ToolCatalogMetadata) -> Self {
        Self { inner, catalog }
    }
}

#[async_trait::async_trait]
impl<T> Tool for CatalogedTool<T>
where
    T: Tool + Send + Sync,
{
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters(&self) -> serde_json::Value {
        self.inner.parameters()
    }

    fn source_kind(&self) -> crate::ToolSchemaSourceKind {
        self.inner.source_kind()
    }

    fn catalog_metadata(&self) -> Option<ToolCatalogMetadata> {
        Some(self.catalog.clone())
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        self.inner.execute(args, ctx).await
    }

    fn validate(&self, args: &serde_json::Value) -> Result<(), ToolError> {
        self.inner.validate(args)
    }
}

// First-pass built-in catalog authority lives at registry assembly time so the
// family map stays centralized instead of being duplicated across individual tools.
fn builtin_catalog(family: &str, subfamily: Option<&str>, tags: &[&str]) -> ToolCatalogMetadata {
    ToolCatalogMetadata {
        domain: Some("agendao_builtin".to_string()),
        family: Some(family.to_string()),
        subfamily: subfamily.map(ToOwned::to_owned),
        tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
        provenance: Some("builtin".to_string()),
    }
}

async fn register_builtin_tool<T>(
    registry: &ToolRegistry,
    tool: T,
    family: &str,
    subfamily: Option<&str>,
    tags: &[&str],
) where
    T: Tool + Send + Sync + 'static,
{
    registry
        .register(CatalogedTool::new(
            tool,
            builtin_catalog(family, subfamily, tags),
        ))
        .await;
}

fn rewrite_invalid_arguments(tool_id: &str, err: ToolError) -> ToolError {
    match err {
        ToolError::InvalidArguments(msg) => {
            if msg.contains("Please rewrite the input so it satisfies the expected schema.") {
                ToolError::InvalidArguments(msg)
            } else {
                ToolError::InvalidArguments(format!(
                    "The {} tool was called with invalid arguments: {}.\nPlease rewrite the input so it satisfies the expected schema.",
                    tool_id, msg
                ))
            }
        }
        other => other,
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register<T: Tool + 'static>(&self, tool: T) {
        let mut tools = self.tools.write().await;
        tools.insert(tool.id().to_string(), Arc::new(tool));
    }

    pub async fn get(&self, id: &str) -> Option<Arc<dyn Tool>> {
        let tools = self.tools.read().await;
        tools.get(id).cloned()
    }

    /// Removes a tool from the registry. Returns true when a tool was present.
    pub async fn unregister(&self, id: &str) -> bool {
        let mut tools = self.tools.write().await;
        tools.remove(id).is_some()
    }

    /// Atomically replaces the entire tool set with another registry's
    /// contents. Used by the server's config-write path to rebuild the live
    /// registry after `disabled_tools` changes — holders of the same
    /// `Arc<ToolRegistry>` observe the new set on their next lookup.
    pub async fn replace_with(&self, other: ToolRegistry) {
        let new_tools = other.tools.into_inner();
        *self.tools.write().await = new_tools;
    }

    pub async fn list(&self) -> Vec<Arc<dyn Tool>> {
        let tools = self.tools.read().await;
        tools.values().cloned().collect()
    }

    /// Returns all registered tool IDs.
    pub async fn list_ids(&self) -> Vec<String> {
        let tools = self.tools.read().await;
        tools.keys().cloned().collect()
    }

    /// Given a tool name that was not found, returns a list of available tool names
    /// filtered to exclude tools in `FILTERED_FROM_SUGGESTIONS`.
    pub async fn suggest_tools(&self, _requested: &str) -> Vec<String> {
        let tools = self.tools.read().await;
        let mut names: Vec<String> = tools
            .keys()
            .filter(|name| !FILTERED_FROM_SUGGESTIONS.contains(&name.as_str()))
            .cloned()
            .collect();
        names.sort();
        names
    }

    pub async fn list_schemas(&self) -> Vec<ToolSchema> {
        let tools = self.tools.read().await;
        let mut schemas: Vec<ToolSchema> = tools
            .values()
            .map(|t| ToolSchema {
                name: t.id().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters(),
                source_kind: t.source_kind(),
                catalog: t.catalog_metadata(),
            })
            .collect();

        // Trigger tool.definition hook for each schema so plugins can transform them
        for schema in &mut schemas {
            let hook_outputs = agendao_plugin::trigger_collect(
                HookContext::new(HookEvent::ToolDefinition)
                    .with_data("tool_id", serde_json::json!(&schema.name))
                    .with_data("description", serde_json::json!(&schema.description))
                    .with_data("parameters", schema.parameters.clone())
                    .with_data("catalog", serde_json::json!(&schema.catalog)),
            )
            .await;
            for output in hook_outputs {
                if let Some(payload) = output.payload.as_ref() {
                    apply_tool_definition_payload(schema, payload);
                }
            }
        }

        schemas
    }

    pub async fn execute(
        &self,
        tool_id: &str,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let tool = match self.get(tool_id).await {
            Some(t) => t,
            None => {
                let suggestions = self.suggest_tools(tool_id).await;
                return Err(ToolError::InvalidArguments(format!(
                    "Tool '{}' not found in registry. Available tools: {}",
                    tool_id,
                    suggestions.join(", ")
                )));
            }
        };

        let mut args = args;
        let source_kind = tool.source_kind();

        // If args is still an empty object, log a warning for diagnostics.
        if args.is_object() && args.as_object().is_some_and(|o| o.is_empty()) {
            tracing::warn!(
                tool = %tool_id,
                "tool called with empty arguments object"
            );
        }

        // Plugin hook: tool.execute.before
        tracing::debug!(
            tool = %tool_id,
            "[plugin-seq] tool.execute.before"
        );
        if agendao_plugin::should_trigger_agent_hooks(
            HookEvent::ToolExecuteBefore,
            Some(ctx.agent.as_str()),
        )
        .await
        {
            let mut before_hook_ctx = HookContext::new(HookEvent::ToolExecuteBefore)
                .with_session(&ctx.session_id)
                .with_data("tool", serde_json::json!(tool_id))
                .with_data("args", args.clone());
            if let Some(call_id) = &ctx.call_id {
                before_hook_ctx = before_hook_ctx.with_data("callID", serde_json::json!(call_id));
            }
            let before_outputs = agendao_plugin::trigger_collect(before_hook_ctx).await;
            for output in before_outputs {
                if let Some(payload) = output.payload.as_ref() {
                    apply_tool_before_payload(&mut args, payload);
                }
            }
        }

        tool.validate(&args)
            .map_err(|e| rewrite_invalid_arguments(tool_id, e))?;
        if !matches!(tool_id, "read" | "grep") {
            tool_access::notify_other_tool_call(&ctx.session_id);
        }
        let mut result = tool.execute(args.clone(), ctx.clone()).await;
        if let Err(e) = &result {
            // Log the exact args when a tool fails, to diagnose argument parsing issues.
            tracing::error!(
                tool = %tool_id,
                error = %e,
                args_type = %match &args {
                    serde_json::Value::Object(o) => format!("object(keys={})", o.keys().cloned().collect::<Vec<_>>().join(",")),
                    serde_json::Value::String(s) => format!(
                        "string(len={},preview={})",
                        s.len(),
                        s.chars().take(200).collect::<String>()
                    ),
                    serde_json::Value::Null => "null".to_string(),
                    serde_json::Value::Array(_) => "array".to_string(),
                    serde_json::Value::Bool(_) => "bool".to_string(),
                    serde_json::Value::Number(_) => "number".to_string(),
                },
                args_json = %serde_json::to_string(&args).unwrap_or_else(|_| "??".to_string()),
                "tool execution failed"
            );
        }
        if let Err(e) = result {
            result = Err(rewrite_invalid_arguments(tool_id, e));
        }

        // Plugin hook: tool.execute.after
        tracing::debug!(
            tool = %tool_id,
            "[plugin-seq] tool.execute.after"
        );
        if agendao_plugin::should_trigger_agent_hooks(
            HookEvent::ToolExecuteAfter,
            Some(ctx.agent.as_str()),
        )
        .await
        {
            let mut hook_ctx = HookContext::new(HookEvent::ToolExecuteAfter)
                .with_session(&ctx.session_id)
                .with_data("tool", serde_json::json!(tool_id))
                .with_data("args", args.clone());
            if let Some(call_id) = &ctx.call_id {
                hook_ctx = hook_ctx.with_data("callID", serde_json::json!(call_id));
            }

            hook_ctx = match &result {
                Ok(r) => hook_ctx
                    .with_data("title", serde_json::json!(&r.title))
                    .with_data("output", serde_json::json!(&r.output))
                    .with_data("metadata", serde_json::json!(&r.metadata))
                    .with_data("error", serde_json::json!(false)),
                Err(e) => hook_ctx
                    .with_data("output", serde_json::json!(e.to_string()))
                    .with_data("error", serde_json::json!(true)),
            };

            let after_outputs = agendao_plugin::trigger_collect(hook_ctx).await;
            if let Ok(tool_result) = &mut result {
                for output in after_outputs {
                    if let Some(payload) = output.payload.as_ref() {
                        apply_tool_after_payload(tool_result, payload);
                    }
                }
            }
        }

        if let Ok(tool_result) = &mut result {
            if let Some(provenance) = external_content_provenance(
                tool_id,
                source_kind,
                &args,
                chrono::Utc::now().timestamp_millis(),
            ) {
                tool_result.metadata.insert(
                    EXTERNAL_CONTENT_PROVENANCE_METADATA_KEY.to_string(),
                    serde_json::to_value(provenance).unwrap_or_default(),
                );
            }
        }

        result
    }
}

fn external_content_provenance(
    tool_id: &str,
    source_kind: crate::ToolSchemaSourceKind,
    args: &serde_json::Value,
    fetched_at: i64,
) -> Option<ExternalContentProvenance> {
    let kind = match source_kind {
        crate::ToolSchemaSourceKind::Mcp => ExternalContentSourceKind::Mcp,
        crate::ToolSchemaSourceKind::Plugin => ExternalContentSourceKind::Plugin,
        crate::ToolSchemaSourceKind::Dynamic => ExternalContentSourceKind::DynamicTool,
        crate::ToolSchemaSourceKind::BuiltIn => match tool_id {
            "webfetch" | "websearch" | "codesearch" | "github_research" | "browser_session" => {
                ExternalContentSourceKind::Web
            }
            "skill_hub"
                if matches!(
                    args.get("action").and_then(serde_json::Value::as_str),
                    Some(
                        "search"
                            | "index"
                            | "artifact_cache"
                            | "index_refresh"
                            | "install_plan"
                            | "update_plan"
                            | "sync_plan"
                    )
                ) =>
            {
                ExternalContentSourceKind::RemoteSkill
            }
            _ => return None,
        },
    };
    let resource_id = ["url", "uri", "query", "source_id", "locator", "path"]
        .iter()
        .find_map(|key| args.get(key).and_then(serde_json::Value::as_str))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(tool_id);
    Some(ExternalContentProvenance::untrusted(
        kind,
        resource_id,
        fetched_at,
    ))
}

fn apply_tool_definition_payload(schema: &mut ToolSchema, payload: &serde_json::Value) {
    let Some(object) = agendao_plugin::hook_payload_object(payload) else {
        return;
    };
    if let Some(description) = object.get("description").and_then(|value| value.as_str()) {
        schema.description = description.to_string();
    }
    if let Some(parameters) = object.get("parameters") {
        schema.parameters = parameters.clone();
    }
    if let Some(catalog) = object.get("catalog") {
        schema.catalog = serde_json::from_value(catalog.clone()).ok();
    }
}

fn apply_tool_before_payload(args: &mut serde_json::Value, payload: &serde_json::Value) {
    let Some(object) = agendao_plugin::hook_payload_object(payload) else {
        return;
    };
    if let Some(next_args) = object.get("args") {
        *args = next_args.clone();
    }
}

fn apply_tool_after_payload(result: &mut ToolResult, payload: &serde_json::Value) {
    let Some(object) = agendao_plugin::hook_payload_object(payload) else {
        return;
    };
    if let Some(title) = object.get("title").and_then(|value| value.as_str()) {
        result.title = title.to_string();
    }
    if let Some(output) = object.get("output") {
        if let Some(output_str) = output.as_str() {
            result.output = output_str.to_string();
        } else if !output.is_null() {
            result.output = output.to_string();
        }
    }
    if let Some(metadata) = object.get("metadata").and_then(|value| value.as_object()) {
        result.metadata = metadata
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ToolRegistryAccess for ToolRegistry {
    async fn get(&self, id: &str) -> Option<Arc<dyn Tool>> {
        Self::get(self, id).await
    }

    async fn list_ids(&self) -> Vec<String> {
        Self::list_ids(self).await
    }

    async fn suggest_tools(&self, requested: &str) -> Vec<String> {
        Self::suggest_tools(self, requested).await
    }

    async fn execute(
        &self,
        tool_id: &str,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        Self::execute(self, tool_id, args, ctx).await
    }
}

pub async fn create_default_registry() -> ToolRegistry {
    create_default_registry_with_config(None).await
}

/// Create the default tool registry, optionally using web search config from the
/// application config. When `config` is `None` or `config.web_search` is unset,
/// the web search tool falls back to the default Exa MCP endpoint.
pub async fn create_default_registry_with_config(
    config: Option<&agendao_config::Config>,
) -> ToolRegistry {
    let registry = ToolRegistry::new();
    #[cfg(not(feature = "web-tools"))]
    let _ = config;

    register_builtin_tool(
        &registry,
        crate::read::ReadTool::new(),
        "filesystem_edit",
        Some("read"),
        &["file", "read"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::artifact_read::ArtifactReadTool::new(),
        "filesystem_edit",
        Some("artifact_read"),
        &["file", "artifact", "read"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::write::WriteTool::new(),
        "filesystem_edit",
        Some("write"),
        &["file", "write"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::edit::EditTool::new(),
        "filesystem_edit",
        Some("patch_edit"),
        &["file", "edit"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::bash::BashTool::new(),
        "shell_execution",
        Some("one_shot"),
        &["shell", "command"],
    )
    .await;
    #[cfg(feature = "terminal-tools")]
    register_builtin_tool(
        &registry,
        crate::shell_session::ShellSessionTool::new(),
        "shell_execution",
        Some("interactive_session"),
        &["shell", "session"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::glob_tool::GlobTool::new(),
        "filesystem_discovery",
        Some("glob"),
        &["file", "glob"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::grep_tool::GrepTool::new(),
        "filesystem_discovery",
        Some("content_search"),
        &["file", "search", "text"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::ls::LsTool::new(),
        "filesystem_discovery",
        Some("listing"),
        &["file", "list", "directory"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::question::QuestionTool::new(),
        "task_governance",
        Some("user_input"),
        &["question", "approval"],
    )
    .await;
    #[cfg(feature = "web-tools")]
    register_builtin_tool(
        &registry,
        agendao_tool_web::WebFetchTool::new(),
        "web_research",
        Some("fetch"),
        &["web", "http", "fetch"],
    )
    .await;
    #[cfg(feature = "web-tools")]
    register_builtin_tool(
        &registry,
        agendao_tool_web::WebSearchTool::from_config(config.and_then(|c| c.web_search.as_ref())),
        "web_research",
        Some("search"),
        &["web", "search"],
    )
    .await;
    #[cfg(feature = "web-tools")]
    register_builtin_tool(
        &registry,
        agendao_tool_web::CodeSearchTool::new(),
        "web_research",
        Some("code_search"),
        &["web", "code", "search"],
    )
    .await;
    #[cfg(feature = "web-tools")]
    register_builtin_tool(
        &registry,
        agendao_tool_web::GitHubResearchTool::new(),
        "web_research",
        Some("github"),
        &["web", "github", "research"],
    )
    .await;
    #[cfg(feature = "web-tools")]
    register_builtin_tool(
        &registry,
        agendao_tool_web::BrowserSessionTool::new(),
        "web_research",
        Some("browser_session"),
        &["web", "browser", "session"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::todo::TodoReadTool,
        "task_governance",
        Some("todo_read"),
        &["todo", "read"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::todo::TodoWriteTool,
        "task_governance",
        Some("todo_write"),
        &["todo", "write"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::multiedit::MultiEditTool,
        "filesystem_edit",
        Some("multi_edit"),
        &["file", "edit", "batch"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::apply_patch::ApplyPatchTool,
        "filesystem_edit",
        Some("apply_patch"),
        &["file", "patch", "diff"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::skills_categories::SkillsCategoriesTool,
        "skill_knowledge",
        Some("catalog_categories"),
        &["skill", "catalog", "category"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::skills_list::SkillsListTool,
        "skill_knowledge",
        Some("catalog_list"),
        &["skill", "catalog", "list"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::skill_search::SkillSearchTool,
        "skill_knowledge",
        Some("catalog_search"),
        &["skill", "catalog", "search"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::skill_view::SkillViewTool,
        "skill_knowledge",
        Some("catalog_view"),
        &["skill", "catalog", "view"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::skill_hub::SkillHubTool,
        "skill_knowledge",
        Some("hub_governance"),
        &["skill", "hub", "governance"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::skill_manage::SkillManageTool,
        "skill_knowledge",
        Some("catalog_mutation"),
        &["skill", "manage", "catalog"],
    )
    .await;
    #[cfg(feature = "lsp")]
    register_builtin_tool(
        &registry,
        crate::lsp_tool::LspTool,
        "code_intelligence",
        Some("lsp"),
        &["code", "lsp", "analysis"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::batch::BatchTool,
        "task_governance",
        Some("batch"),
        &["batch", "parallel", "orchestration"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::context_docs::ContextDocsTool::new(),
        "skill_knowledge",
        Some("context_docs"),
        &["docs", "context", "knowledge"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::tool_catalog::CapabilityTool::primary(),
        "execution_resource_catalog",
        Some("capability"),
        &["catalog", "tool", "mcp", "skill", "capability"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::repo_history::RepoHistoryTool::new(),
        "filesystem_discovery",
        Some("history"),
        &["repo", "history", "git"],
    )
    .await;
    #[cfg(feature = "code-intel")]
    register_builtin_tool(
        &registry,
        crate::ast_grep_search::AstGrepSearchTool::new(),
        "code_intelligence",
        Some("ast_search"),
        &["code", "ast", "search"],
    )
    .await;
    #[cfg(feature = "code-intel")]
    register_builtin_tool(
        &registry,
        crate::ast_grep_replace::AstGrepReplaceTool::new(),
        "code_intelligence",
        Some("ast_replace"),
        &["code", "ast", "replace"],
    )
    .await;
    register_builtin_tool(
        &registry,
        crate::invalid::InvalidTool,
        "internal_sentinel",
        Some("fallback"),
        &["internal", "sentinel", "fallback"],
    )
    .await;

    // Auto-register plugin custom tools (may override same-named built-in tools)
    if let Some(loader) = agendao_plugin::global_loader() {
        register_plugin_tools(&registry, loader).await;
    }

    if let Some(config) = config {
        apply_disabled_tools_filter(&registry, &config.disabled_tools).await;
    }

    registry
}

/// Facade/bridge tools that must survive `disabled_tools`. In facade mode the
/// model can only reach other tools through `capability`, and can only
/// reach skill content through the `skills_*` discovery tools plus `skill_view` — disabling those
/// would cut the model off from everything behind them.
pub fn is_protected_facade_tool(name: &str) -> bool {
    name == crate::tool_catalog::CAPABILITY_TOOL_ID
        || name.starts_with("skills_")
        || name == "skill_view"
}

/// Removes tools listed in `disabled_tools` (exact tool id or `family/*`
/// category wildcard matched against catalog metadata) from a fully assembled
/// registry. Protected facade/bridge tools are never removed.
async fn apply_disabled_tools_filter(registry: &ToolRegistry, disabled: &[String]) {
    if disabled.is_empty() {
        return;
    }

    for tool in registry.list().await {
        let id = tool.id().to_string();
        let matched = agendao_config::matching::matching_disabled_pattern(disabled, &id)
            .map(|pattern| pattern.to_string())
            .or_else(|| {
                tool.catalog_metadata()
                    .and_then(|catalog| catalog.family)
                    .and_then(|family| {
                        agendao_config::matching::matching_disabled_pattern(disabled, &family)
                    })
                    .map(|pattern| pattern.to_string())
            });
        let Some(pattern) = matched else {
            continue;
        };

        if is_protected_facade_tool(&id) {
            tracing::debug!(
                tool = %id,
                pattern = %pattern,
                "disabled_tools entry ignored: facade/bridge tool is exempt"
            );
            continue;
        }

        registry.unregister(&id).await;
        tracing::debug!(
            tool = %id,
            pattern = %pattern,
            "tool removed from registry via disabled_tools"
        );
    }
}

async fn register_plugin_tools(
    registry: &ToolRegistry,
    loader: Arc<agendao_plugin::subprocess::loader::PluginLoader>,
) {
    let mut tools = loader.collect_plugin_tools().await;
    // Sort by plugin_id for stable override order when tool names collide
    tools.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.cmp(&b.0)));

    let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (tool_id, def, plugin_id) in tools {
        if let Some(prev_plugin) = seen.get(&tool_id) {
            tracing::warn!(
                tool = %tool_id,
                prev_plugin = %prev_plugin,
                new_plugin = %plugin_id,
                "plugin tool name conflict: later plugin_id wins"
            );
        }
        seen.insert(tool_id.clone(), plugin_id.clone());
        tracing::info!(tool = %tool_id, plugin_id = %plugin_id, "registering plugin custom tool");
        registry
            .register(crate::plugin_tool::PluginTool::new(
                tool_id,
                plugin_id,
                &def,
                Arc::clone(&loader),
            ))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_access::{self, ToolAccessKey, ToolAccessOutcome};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    struct CaptureTool {
        captured: Arc<Mutex<Option<serde_json::Value>>>,
        id: &'static str,
    }

    #[async_trait]
    impl Tool for CaptureTool {
        fn id(&self) -> &str {
            self.id
        }

        fn description(&self) -> &str {
            "Captures args for testing"
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }

        async fn execute(
            &self,
            args: serde_json::Value,
            _ctx: ToolContext,
        ) -> Result<ToolResult, ToolError> {
            *self.captured.lock().expect("lock should succeed") = Some(args.clone());
            let primary = args
                .get("file_path")
                .or_else(|| args.get("filePath"))
                .or_else(|| args.get("command"))
                .or_else(|| args.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            Ok(ToolResult::simple("ok", primary))
        }
    }

    async fn setup_capture_registry() -> (ToolRegistry, Arc<Mutex<Option<serde_json::Value>>>) {
        let registry = ToolRegistry::new();
        let captured = Arc::new(Mutex::new(None));
        registry
            .register(CaptureTool {
                captured: captured.clone(),
                id: "capture",
            })
            .await;
        (registry, captured)
    }

    #[tokio::test]
    async fn non_read_tool_execution_resets_repeated_access_counter() {
        let session_id = "registry-reset-repeated-access";
        tool_access::clear_tool_access_tracker(session_id);
        let key = ToolAccessKey::Read {
            path: "/tmp/demo.txt".to_string(),
            offset: 1,
            limit: 2000,
        };
        let (registry, _captured) = setup_capture_registry().await;

        assert_eq!(
            tool_access::record_tool_access(session_id, key.clone()),
            ToolAccessOutcome::Fresh { consecutive: 1 }
        );
        assert_eq!(
            tool_access::record_tool_access(session_id, key.clone()),
            ToolAccessOutcome::Fresh { consecutive: 2 }
        );

        registry
            .execute(
                "capture",
                serde_json::json!({}),
                ToolContext::new(
                    session_id.to_string(),
                    "message-1".to_string(),
                    ".".to_string(),
                ),
            )
            .await
            .expect("capture tool should execute");

        assert_eq!(
            tool_access::record_tool_access(session_id, key),
            ToolAccessOutcome::Fresh { consecutive: 1 }
        );
        tool_access::clear_tool_access_tracker(session_id);
    }

    #[tokio::test]
    async fn create_default_registry_assigns_catalog_to_builtin_tools() {
        let registry = create_default_registry().await;
        let schemas = registry.list_schemas().await;

        let read = schemas
            .iter()
            .find(|schema| schema.name == "read")
            .expect("read schema should exist");
        let bash = schemas
            .iter()
            .find(|schema| schema.name == "bash")
            .expect("bash schema should exist");
        let skills_list = schemas
            .iter()
            .find(|schema| schema.name == "skills_list")
            .expect("skills_list schema should exist");
        let skill_search = schemas
            .iter()
            .find(|schema| schema.name == "skill_search")
            .expect("skill_search schema should exist");

        assert_eq!(read.source_kind, crate::ToolSchemaSourceKind::BuiltIn);
        assert_eq!(
            read.catalog
                .as_ref()
                .and_then(|catalog| catalog.family.as_deref()),
            Some("filesystem_edit")
        );
        assert_eq!(
            bash.catalog
                .as_ref()
                .and_then(|catalog| catalog.family.as_deref()),
            Some("shell_execution")
        );
        assert_eq!(
            skills_list
                .catalog
                .as_ref()
                .and_then(|catalog| catalog.family.as_deref()),
            Some("skill_knowledge")
        );
        assert_eq!(
            skill_search
                .catalog
                .as_ref()
                .and_then(|catalog| catalog.family.as_deref()),
            Some("skill_knowledge")
        );
        assert!(schemas
            .iter()
            .filter(|schema| schema.source_kind == crate::ToolSchemaSourceKind::BuiltIn)
            .all(|schema| schema.catalog.is_some()));
    }

    #[tokio::test]
    async fn create_default_registry_lists_canonical_capability_tool() {
        let registry = create_default_registry().await;
        let schemas = registry.list_schemas().await;
        let names = schemas
            .iter()
            .map(|schema| schema.name.as_str())
            .collect::<Vec<_>>();

        assert!(names.contains(&crate::tool_catalog::CAPABILITY_TOOL_ID));
        assert!(!names.iter().any(|name| name.starts_with("tool_catalog_")));
    }

    #[cfg(not(feature = "lsp"))]
    #[tokio::test]
    async fn create_default_registry_omits_lsp_when_feature_is_disabled() {
        let registry = create_default_registry().await;
        assert!(registry.get("lsp").await.is_none());
    }

    #[cfg(feature = "lsp")]
    #[tokio::test]
    async fn create_default_registry_includes_lsp_when_feature_is_enabled() {
        let registry = create_default_registry().await;
        assert!(registry.get("lsp").await.is_some());
    }

    fn test_tool_context() -> ToolContext {
        ToolContext::new(
            "ses_test".to_string(),
            "msg_test".to_string(),
            ".".to_string(),
        )
    }

    #[tokio::test]
    async fn execute_passes_canonical_object_arguments_unchanged() {
        let (registry, captured) = setup_capture_registry().await;
        let args = serde_json::json!({
            "file_path": "/tmp/a.html",
            "content": "hello"
        });

        registry
            .execute("capture", args.clone(), test_tool_context())
            .await
            .expect("canonical object should execute");

        assert_eq!(
            captured.lock().expect("lock should succeed").as_ref(),
            Some(&args)
        );
    }

    #[tokio::test]
    async fn disabled_tools_are_removed_by_exact_name_and_family_wildcard() {
        let config = agendao_config::Config {
            disabled_tools: vec!["bash".to_string(), "task_governance/*".to_string()],
            ..Default::default()
        };
        let registry = create_default_registry_with_config(Some(&config)).await;
        let ids = registry.list_ids().await;

        assert!(!ids.iter().any(|id| id == "bash"));
        // family wildcard removes every member of task_governance
        for family_member in ["todoread", "todowrite", "batch"] {
            assert!(
                !ids.iter().any(|id| id == family_member),
                "{family_member} should be disabled via task_governance/*"
            );
        }
        // unrelated tools survive
        assert!(ids.iter().any(|id| id == "read"));
        assert!(ids.iter().any(|id| id == "grep"));
    }

    #[tokio::test]
    async fn disabled_tools_cannot_remove_facade_or_bridge_tools() {
        let config = agendao_config::Config {
            disabled_tools: vec![
                "capability".to_string(),
                "skills_list".to_string(),
                "skill_view".to_string(),
                "skill_knowledge/*".to_string(),
                "execution_resource_catalog/*".to_string(),
            ],
            ..Default::default()
        };
        let registry = create_default_registry_with_config(Some(&config)).await;
        let ids = registry.list_ids().await;

        // Facade/bridge tools survive even when listed explicitly or via a
        // family wildcard covering their catalog family.
        for protected in [
            "capability",
            "skills_categories",
            "skills_list",
            "skill_view",
        ] {
            assert!(
                ids.iter().any(|id| id == protected),
                "{protected} must be exempt from disabled_tools"
            );
        }
        // Non-bridge members of the same family are still removable.
        assert!(!ids.iter().any(|id| id == "skill_hub"));
        assert!(!ids.iter().any(|id| id == "skill_manage"));
    }

    #[tokio::test]
    async fn empty_disabled_tools_leaves_registry_untouched() {
        let config = agendao_config::Config::default();
        let registry = create_default_registry_with_config(Some(&config)).await;
        let ids = registry.list_ids().await;
        assert!(ids.iter().any(|id| id == "bash"));
        assert!(ids.iter().any(|id| id == "todoread"));
    }
}

#[cfg(test)]
mod default_registry_tests {
    #[tokio::test]
    async fn create_default_registry_registers_context_docs() {
        let registry = super::create_default_registry().await;
        let ids = registry.list_ids().await;
        assert!(ids.iter().any(|id| id == "context_docs"));
        #[cfg(feature = "code-intel")]
        assert!(ids.iter().any(|id| id == "ast_grep_replace"));
        assert!(ids.iter().any(|id| id == "repo_history"));
        #[cfg(feature = "terminal-tools")]
        assert!(ids.iter().any(|id| id == "shell_session"));
        #[cfg(feature = "web-tools")]
        assert!(ids.iter().any(|id| id == "browser_session"));
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::*;
    use async_trait::async_trait;

    struct ExternalTestTool;

    #[async_trait]
    impl Tool for ExternalTestTool {
        fn id(&self) -> &str {
            "external_test"
        }

        fn description(&self) -> &str {
            "Returns external test content"
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }

        fn source_kind(&self) -> crate::ToolSchemaSourceKind {
            crate::ToolSchemaSourceKind::Plugin
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: ToolContext,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::simple("external", "payload"))
        }
    }

    #[test]
    fn external_tool_sources_receive_typed_untrusted_provenance() {
        for (source, expected) in [
            (
                crate::ToolSchemaSourceKind::Mcp,
                ExternalContentSourceKind::Mcp,
            ),
            (
                crate::ToolSchemaSourceKind::Plugin,
                ExternalContentSourceKind::Plugin,
            ),
            (
                crate::ToolSchemaSourceKind::Dynamic,
                ExternalContentSourceKind::DynamicTool,
            ),
        ] {
            let provenance = external_content_provenance(
                "external_tool",
                source,
                &serde_json::json!({"uri": "resource/42"}),
                99,
            )
            .expect("external source");
            assert_eq!(provenance.source_kind, expected);
            assert_eq!(provenance.resource_id, "resource/42");
            assert!(provenance.untrusted_external);
        }
        assert!(external_content_provenance(
            "read",
            crate::ToolSchemaSourceKind::BuiltIn,
            &serde_json::json!({"file_path": "/tmp/a"}),
            99,
        )
        .is_none());
    }

    #[test]
    fn built_in_web_and_remote_skill_reads_are_external() {
        let web = external_content_provenance(
            "webfetch",
            crate::ToolSchemaSourceKind::BuiltIn,
            &serde_json::json!({"url": "https://example.test/doc"}),
            1,
        )
        .expect("web provenance");
        assert_eq!(web.source_kind, ExternalContentSourceKind::Web);
        let skill = external_content_provenance(
            "skill_hub",
            crate::ToolSchemaSourceKind::BuiltIn,
            &serde_json::json!({"action": "search", "query": "review"}),
            1,
        )
        .expect("remote skill provenance");
        assert_eq!(skill.source_kind, ExternalContentSourceKind::RemoteSkill);
    }

    #[tokio::test]
    async fn registry_execution_stamps_external_tool_result() {
        let registry = ToolRegistry::new();
        registry.register(ExternalTestTool).await;

        let result = registry
            .execute(
                "external_test",
                serde_json::json!({"uri": "plugin/resource"}),
                ToolContext::new(
                    "ses_external".to_string(),
                    "msg_external".to_string(),
                    ".".to_string(),
                ),
            )
            .await
            .expect("external tool should execute");
        let provenance = ExternalContentProvenance::from_metadata(&result.metadata)
            .expect("registry must stamp provenance");
        assert_eq!(provenance.source_kind, ExternalContentSourceKind::Plugin);
        assert_eq!(provenance.resource_id, "plugin/resource");
        assert!(provenance.untrusted_external);
    }
}
