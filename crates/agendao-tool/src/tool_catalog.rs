use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use agendao_config::{ExternalToolConfig, ExternalToolExecutionKind, ResolvedExternalToolCatalog};
use agendao_sandbox::ProfileKind;
use agendao_types::ToolCatalogMetadata;
use async_trait::async_trait;
use serde::Deserialize;

use crate::{
    assert_external_directory, bash::authorize_bash_command, ExternalDirectoryKind,
    ExternalDirectoryOptions, Metadata, Tool, ToolContext, ToolError, ToolResult,
    ToolSchemaSourceKind,
};

pub const CAPABILITY_TOOL_ID: &str = "capability";
pub const CAPABILITY_ALLOWED_TOOL_IDS_KEY: &str = "capability_allowed_tool_ids";
pub const CORE_MODEL_TOOL_IDS: &[&str] =
    &[CAPABILITY_TOOL_ID, "bash", "read", "apply_patch", "grep"];

#[derive(Debug, Clone)]
struct CatalogEntry {
    name: String,
    description: String,
    parameters: serde_json::Value,
    source_kind: ToolSchemaSourceKind,
    catalog: Option<ToolCatalogMetadata>,
    executable: bool,
    source_path: Option<String>,
    manifest_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CatalogEntryScore {
    exact_name: u8,
    prefix_name: u8,
    exact_catalog: u8,
    tag_match: u8,
    fuzzy_match: u8,
}

pub fn is_tool_catalog_facade_tool(name: &str) -> bool {
    name == CAPABILITY_TOOL_ID
}

pub fn is_model_visible_tool_catalog_facade_tool(name: &str) -> bool {
    name == CAPABILITY_TOOL_ID
}

pub fn is_core_model_tool(name: &str) -> bool {
    CORE_MODEL_TOOL_IDS.contains(&name)
}

pub struct CapabilityTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CapabilityAction {
    Search,
    Describe,
    Call,
}

#[derive(Debug, Deserialize)]
struct CapabilityInput {
    action: CapabilityAction,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    subfamily: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default = "default_offset")]
    offset: usize,
    #[serde(default)]
    arguments: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct CapabilitySearchInput {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    subfamily: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default = "default_offset")]
    offset: usize,
}

#[derive(Debug, Deserialize)]
struct CapabilityDescribeInput {
    tool: String,
}

#[derive(Debug, Deserialize)]
struct CapabilityCallInput {
    tool: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

fn default_limit() -> usize {
    8
}

fn default_offset() -> usize {
    0
}

const MAX_LIMIT: usize = 50;

fn looks_like_skill_file_or_catalog_path(name: &str) -> bool {
    name.contains(':') || name.contains('/')
}

impl CapabilityTool {
    pub const fn primary() -> Self {
        Self
    }
}

fn capability_parameters() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["search", "describe", "call"],
                "description": "Search the local capability catalog, describe one exact result, or call it."
            },
            "tool": {
                "type": "string",
                "description": "Exact tool or MCP capability id returned by search; required for describe and call."
            },
            "query": { "type": "string" },
            "domain": { "type": "string" },
            "family": { "type": "string" },
            "subfamily": { "type": "string" },
            "tag": { "type": "string" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 8 },
            "offset": { "type": "integer", "minimum": 0, "default": 0 },
            "arguments": { "type": "object", "additionalProperties": true }
        },
        "required": ["action"],
        "additionalProperties": false
    })
}

fn required_capability_tool(tool: Option<String>, action: &str) -> Result<String, ToolError> {
    tool.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ToolError::InvalidArguments(format!(
                "tool is required for capability action `{action}`"
            ))
        })
}

async fn execute_capability(
    args: serde_json::Value,
    ctx: ToolContext,
) -> Result<ToolResult, ToolError> {
    let input: CapabilityInput = serde_json::from_value(args)
        .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
    match input.action {
        CapabilityAction::Search => {
            execute_capability_search(
                serde_json::json!({
                    "query": input.query,
                    "domain": input.domain,
                    "family": input.family,
                    "subfamily": input.subfamily,
                    "tag": input.tag,
                    "limit": input.limit,
                    "offset": input.offset,
                }),
                ctx,
            )
            .await
        }
        CapabilityAction::Describe => {
            let tool = required_capability_tool(input.tool, "describe")?;
            execute_capability_describe(serde_json::json!({"tool": tool}), ctx).await
        }
        CapabilityAction::Call => {
            let tool = required_capability_tool(input.tool, "call")?;
            execute_capability_call(
                serde_json::json!({"tool": tool, "arguments": input.arguments}),
                ctx,
            )
            .await
        }
    }
}

async fn execute_capability_search(
    args: serde_json::Value,
    ctx: ToolContext,
) -> Result<ToolResult, ToolError> {
    let input: CapabilitySearchInput = serde_json::from_value(args)
        .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
    let mut entries = collect_catalog_entries(&ctx).await?;
    let limit = input.limit.clamp(1, MAX_LIMIT);
    let offset = input.offset;
    let query = input
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let domain = input
        .domain
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let family = input
        .family
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let subfamily = input
        .subfamily
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let tag = input
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    entries.retain(|entry| {
        matches_structured_filter(entry, domain, family, subfamily, tag)
            && matches_free_text_query(entry, query)
    });
    sort_catalog_entries(&mut entries, query, domain, family, subfamily, tag);

    let total_matches = entries.len();
    let results = entries
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let lines = results
        .iter()
        .map(|entry| {
            let catalog = entry.catalog.as_ref();
            let domain = catalog
                .and_then(|value| value.domain.as_deref())
                .unwrap_or("unknown");
            let family = catalog
                .and_then(|value| value.family.as_deref())
                .unwrap_or("uncategorized");
            let subfamily = catalog
                .and_then(|value| value.subfamily.as_deref())
                .unwrap_or("-");
            let executable = if entry.executable {
                "yes"
            } else {
                "catalog-only"
            };
            format!(
                "- `{}` [{}/{}/{}] executable={} — {}",
                entry.name, domain, family, subfamily, executable, entry.description
            )
        })
        .collect::<Vec<_>>();
    let output = if lines.is_empty() {
        "No matching execution resources found.".to_string()
    } else {
        lines.join("\n")
    };

    Ok(ToolResult::simple("Catalog search results", output)
        .with_metadata(
            "results",
            serde_json::json!(results.iter().map(entry_json).collect::<Vec<_>>()),
        )
        .with_metadata("count", serde_json::json!(results.len()))
        .with_metadata("offset", serde_json::json!(offset))
        .with_metadata("limit", serde_json::json!(limit))
        .with_metadata("total_matches", serde_json::json!(total_matches)))
}

async fn execute_capability_describe(
    args: serde_json::Value,
    ctx: ToolContext,
) -> Result<ToolResult, ToolError> {
    let input: CapabilityDescribeInput = serde_json::from_value(args)
        .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
    let entries = collect_catalog_entries(&ctx).await?;
    let Some(entry) = entries.into_iter().find(|entry| entry.name == input.tool) else {
        return Err(ToolError::InvalidArguments(format!(
            "execution resource `{}` not found; use {} first",
            input.tool, CAPABILITY_TOOL_ID
        )));
    };

    let resource = entry_json(&entry);
    let output =
        serde_json::to_string_pretty(&resource).unwrap_or_else(|_| format!("{:?}", entry.name));
    Ok(ToolResult::simple("Execution resource detail", output).with_metadata("resource", resource))
}

async fn execute_capability_call(
    args: serde_json::Value,
    ctx: ToolContext,
) -> Result<ToolResult, ToolError> {
    let input: CapabilityCallInput = serde_json::from_value(args)
        .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
    if is_tool_catalog_facade_tool(&input.tool) {
        return Err(ToolError::InvalidArguments(
            "capability cannot target itself".to_string(),
        ));
    }
    if !capability_target_is_allowed(&ctx, &input.tool) {
        return Err(ToolError::PermissionDenied(format!(
            "capability target `{}` is outside the active agent policy",
            input.tool
        )));
    }

    if let Some(registry) = ctx.registry.clone() {
        if registry.get(&input.tool).await.is_some() {
            return registry.execute(&input.tool, input.arguments, ctx).await;
        }
    }

    let external_catalogs = load_external_catalogs(&ctx)?;
    if let Some(config) = find_external_catalog_config(&external_catalogs, &input.tool) {
        if config.is_executable() {
            return execute_external_catalog_tool(&input.tool, config, input.arguments, &ctx).await;
        }
        let entry = find_external_catalog_entry(&external_catalogs, &input.tool)
            .expect("entry should exist when config exists");
        return Err(ToolError::ExecutionError(format!(
            "execution resource `{}` is catalog-only right now; no execution adapter is registered yet{}",
            input.tool,
            entry
                .source_path
                .as_deref()
                .map(|path| format!(" (source: {path})"))
                .unwrap_or_default()
        )));
    }

    let suggestions = if let Some(registry) = ctx.registry.as_ref() {
        registry.suggest_tools(&input.tool).await
    } else {
        Vec::new()
    };
    if looks_like_skill_file_or_catalog_path(&input.tool) {
        return Err(ToolError::InvalidArguments(format!(
            "execution resource `{}` not found. This looks like a skill file or catalog path, not an execution resource id. Use `skill_view(name, file_path)` to inspect skill-owned files, or call `capability` with action `search` and pass the exact `results[].name` into action `call`.",
            input.tool
        )));
    }
    if suggestions.is_empty() {
        Err(ToolError::InvalidArguments(format!(
            "execution resource `{}` not found",
            input.tool
        )))
    } else {
        Err(ToolError::InvalidArguments(format!(
            "execution resource `{}` not found. Suggestions: {}",
            input.tool,
            suggestions.join(", ")
        )))
    }
}

#[async_trait]
impl Tool for CapabilityTool {
    fn id(&self) -> &str {
        CAPABILITY_TOOL_ID
    }

    fn description(&self) -> &str {
        "Discover and use tools, MCP capabilities, and imported execution resources without loading every schema into model context. Search first, describe an exact result when needed, then call it."
    }

    fn parameters(&self) -> serde_json::Value {
        capability_parameters()
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        execute_capability(args, ctx).await
    }
}

async fn collect_catalog_entries(ctx: &ToolContext) -> Result<Vec<CatalogEntry>, ToolError> {
    let mut entries = BTreeMap::new();

    if let Some(registry) = ctx.registry.clone() {
        for id in registry.list_ids().await {
            if is_tool_catalog_facade_tool(&id) || id == "invalid" {
                continue;
            }
            if !capability_target_is_allowed(ctx, &id) {
                continue;
            }
            let Some(tool) = registry.get(&id).await else {
                continue;
            };
            entries.insert(
                id.clone(),
                CatalogEntry {
                    name: id,
                    description: tool.description().to_string(),
                    parameters: tool.parameters(),
                    source_kind: tool.source_kind(),
                    catalog: tool.catalog_metadata(),
                    executable: true,
                    source_path: None,
                    manifest_path: None,
                },
            );
        }
    }

    for catalog in load_external_catalogs(ctx)? {
        for (tool_name, config) in catalog.tools {
            if entries.contains_key(&tool_name) {
                continue;
            }
            if !capability_target_is_allowed(ctx, &tool_name) {
                continue;
            }
            entries.insert(
                tool_name.clone(),
                external_catalog_entry(tool_name, &config),
            );
        }
    }

    Ok(entries.into_values().collect())
}

fn capability_allowed_tool_ids(ctx: &ToolContext) -> Option<HashSet<&str>> {
    ctx.extra
        .get(CAPABILITY_ALLOWED_TOOL_IDS_KEY)
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect()
        })
}

fn capability_target_is_allowed(ctx: &ToolContext, tool: &str) -> bool {
    capability_allowed_tool_ids(ctx)
        .map(|allowed| allowed.contains(tool))
        .unwrap_or(false)
}

fn load_external_catalogs(
    ctx: &ToolContext,
) -> Result<Vec<ResolvedExternalToolCatalog>, ToolError> {
    if let Some(config_store) = ctx.config_store.as_ref() {
        let config = config_store.config();
        if config.tool_imports.is_empty() {
            return Ok(Vec::new());
        }
        let project_root = config_store
            .project_dir()
            .unwrap_or_else(|| PathBuf::from(ctx.directory.clone()));
        return agendao_config::load_external_tool_catalogs_from_imports(
            project_root,
            &config.tool_imports,
        )
        .map_err(|error| {
            ToolError::ExecutionError(format!("failed to load external tool catalogs: {error}"))
        });
    }

    let project_root = if !ctx.project_root.trim().is_empty() {
        PathBuf::from(ctx.project_root.trim())
    } else {
        PathBuf::from(ctx.directory.clone())
    };
    agendao_config::load_external_tool_catalogs_for_project(project_root).map_err(|error| {
        ToolError::ExecutionError(format!("failed to load external tool catalogs: {error}"))
    })
}

fn find_external_catalog_entry(
    catalogs: &[ResolvedExternalToolCatalog],
    tool_name: &str,
) -> Option<CatalogEntry> {
    catalogs.iter().find_map(|catalog| {
        catalog
            .tools
            .get(tool_name)
            .map(|config| external_catalog_entry(tool_name.to_string(), config))
    })
}

fn external_catalog_entry(tool_name: String, config: &ExternalToolConfig) -> CatalogEntry {
    CatalogEntry {
        name: tool_name,
        description: "External catalog tool discovered from toolImports".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "additionalProperties": true
        }),
        source_kind: ToolSchemaSourceKind::Dynamic,
        catalog: config.catalog.clone(),
        executable: config.is_executable(),
        source_path: config
            .source
            .as_ref()
            .and_then(|source| source.path.clone()),
        manifest_path: config
            .source
            .as_ref()
            .and_then(|source| source.manifest.clone()),
    }
}

fn find_external_catalog_config<'a>(
    catalogs: &'a [ResolvedExternalToolCatalog],
    tool_name: &str,
) -> Option<&'a ExternalToolConfig> {
    catalogs
        .iter()
        .find_map(|catalog| catalog.tools.get(tool_name))
}

async fn execute_external_catalog_tool(
    tool_name: &str,
    config: &ExternalToolConfig,
    arguments: serde_json::Value,
    ctx: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let execution = config.execution.as_ref().ok_or_else(|| {
        ToolError::ExecutionError(format!(
            "execution resource `{}` is catalog-only right now; no execution adapter is registered yet",
            tool_name
        ))
    })?;

    match execution.kind {
        ExternalToolExecutionKind::ScriptRunner => {
            execute_script_runner_external_tool(tool_name, config, execution, arguments, ctx).await
        }
    }
}

async fn execute_script_runner_external_tool(
    tool_name: &str,
    config: &ExternalToolConfig,
    execution: &agendao_config::ExternalToolExecution,
    arguments: serde_json::Value,
    ctx: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let entry = execution
        .entry
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            ToolError::ExecutionError(format!(
                "execution resource `{}` is missing execution.entry",
                tool_name
            ))
        })?;

    assert_external_directory(
        ctx,
        Some(entry),
        ExternalDirectoryOptions {
            bypass: false,
            kind: ExternalDirectoryKind::File,
        },
    )
    .await?;

    let runtime = execution.runtime.as_deref().unwrap_or("python3");
    let workdir = ctx.directory.clone();
    let compact_args = serde_json::to_string(&arguments)
        .map_err(|error| ToolError::ExecutionError(error.to_string()))?;
    let command = format!(
        "{} '{}' '{}'",
        runtime,
        escape_single_quoted_shell(entry),
        escape_single_quoted_shell(&compact_args)
    );
    authorize_bash_command(
        &command,
        &format!("Execute external catalog tool `{}`", tool_name),
        ctx,
    )
    .await?;

    // The sandbox boundary is the only launch path for model-reachable
    // execution — external catalog tools run contained by default under
    // the same authority as bash, resolving to native only when the host
    // has authorized it for the session (sandbox plan §4.4; Phase 8
    // closed the last direct-spawn path here). Tools never self-widen.
    let profile = if ctx.sandbox_native_allowed {
        ProfileKind::Native
    } else {
        ProfileKind::WorkspaceWrite
    };
    let result = agendao_tool_core::run_contained_query(
        agendao_tool_core::ContainedQuerySpec {
            program: runtime.to_string(),
            args: vec![entry.to_string(), compact_args],
            cwd: Some(workdir.into()),
            env_overrides: Default::default(),
            label: format!("external catalog tool `{}`", tool_name),
        },
        ctx,
        Duration::from_millis(30_000),
        profile,
    )
    .await?;

    if !result.success {
        let mut message = format!(
            "external catalog tool `{}` exited with code {}",
            tool_name,
            result.code.unwrap_or(-1)
        );
        if !result.stderr.trim().is_empty() {
            message.push_str(": ");
            message.push_str(result.stderr.trim());
        }
        return Err(ToolError::ExecutionError(message));
    }

    let output = result.stdout;
    let stderr_output = result.stderr;
    let trimmed_output = output.trim().to_string();
    let title = format!("External execution resource `{}`", tool_name);
    let mut metadata = Metadata::new();
    metadata.insert("source".to_string(), serde_json::json!("external_catalog"));
    metadata.insert("tool".to_string(), serde_json::json!(tool_name));
    metadata.insert("runtime".to_string(), serde_json::json!(runtime));
    metadata.insert("entry".to_string(), serde_json::json!(entry));
    metadata.insert(
        "catalog".to_string(),
        serde_json::to_value(&config.catalog).unwrap_or(serde_json::Value::Null),
    );
    if !stderr_output.trim().is_empty() {
        metadata.insert(
            "stderr".to_string(),
            serde_json::json!(stderr_output.trim()),
        );
    }

    Ok(ToolResult {
        title,
        output: trimmed_output,
        metadata,
        truncated: false,
    })
}

fn escape_single_quoted_shell(input: &str) -> String {
    input.replace('\'', "'\"'\"'")
}

fn matches_structured_filter(
    entry: &CatalogEntry,
    domain: Option<&str>,
    family: Option<&str>,
    subfamily: Option<&str>,
    tag: Option<&str>,
) -> bool {
    let Some(catalog) = entry.catalog.as_ref() else {
        return domain.is_none() && family.is_none() && subfamily.is_none() && tag.is_none();
    };

    if let Some(domain) = domain {
        if catalog.domain.as_deref() != Some(domain) {
            return false;
        }
    }
    if let Some(family) = family {
        if catalog.family.as_deref() != Some(family) {
            return false;
        }
    }
    if let Some(subfamily) = subfamily {
        if catalog.subfamily.as_deref() != Some(subfamily) {
            return false;
        }
    }
    if let Some(tag) = tag {
        if !catalog.tags.iter().any(|value| value == tag) {
            return false;
        }
    }
    true
}

fn matches_free_text_query(entry: &CatalogEntry, query: Option<&str>) -> bool {
    let Some(query) = query.map(str::to_ascii_lowercase) else {
        return true;
    };
    let haystacks = [
        entry.name.to_ascii_lowercase(),
        entry.description.to_ascii_lowercase(),
        entry
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.domain.clone())
            .unwrap_or_default()
            .to_ascii_lowercase(),
        entry
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.family.clone())
            .unwrap_or_default()
            .to_ascii_lowercase(),
        entry
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.subfamily.clone())
            .unwrap_or_default()
            .to_ascii_lowercase(),
        entry
            .catalog
            .as_ref()
            .map(|catalog| catalog.tags.join(" "))
            .unwrap_or_default()
            .to_ascii_lowercase(),
    ];
    haystacks
        .iter()
        .any(|value: &String| value.contains(query.as_str()))
}

fn score_catalog_entry(
    entry: &CatalogEntry,
    query: Option<&str>,
    domain: Option<&str>,
    family: Option<&str>,
    subfamily: Option<&str>,
    tag: Option<&str>,
) -> CatalogEntryScore {
    let normalized_name = entry.name.to_ascii_lowercase();
    let normalized_description = entry.description.to_ascii_lowercase();
    let normalized_query = query.map(str::to_ascii_lowercase);
    let normalized_domain = domain.map(str::to_ascii_lowercase);
    let normalized_family = family.map(str::to_ascii_lowercase);
    let normalized_subfamily = subfamily.map(str::to_ascii_lowercase);
    let normalized_tag = tag.map(str::to_ascii_lowercase);
    let catalog = entry.catalog.as_ref();
    let catalog_domain = catalog
        .and_then(|value| value.domain.as_deref())
        .map(str::to_ascii_lowercase);
    let catalog_family = catalog
        .and_then(|value| value.family.as_deref())
        .map(str::to_ascii_lowercase);
    let catalog_subfamily = catalog
        .and_then(|value| value.subfamily.as_deref())
        .map(str::to_ascii_lowercase);
    let catalog_tags = catalog
        .map(|value| {
            value
                .tags
                .iter()
                .map(|tag: &String| tag.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let exact_name = normalized_query
        .as_deref()
        .map(|value| u8::from(normalized_name == value))
        .unwrap_or(0);
    let prefix_name = normalized_query
        .as_deref()
        .map(|value| u8::from(normalized_name.starts_with(value)))
        .unwrap_or(0);
    let exact_catalog = u8::from(
        normalized_domain
            .as_deref()
            .zip(catalog_domain.as_deref())
            .map(|(expected, actual)| expected == actual)
            .unwrap_or(false)
            || normalized_family
                .as_deref()
                .zip(catalog_family.as_deref())
                .map(|(expected, actual)| expected == actual)
                .unwrap_or(false)
            || normalized_subfamily
                .as_deref()
                .zip(catalog_subfamily.as_deref())
                .map(|(expected, actual)| expected == actual)
                .unwrap_or(false),
    );
    let tag_match = normalized_tag
        .as_deref()
        .map(|expected| u8::from(catalog_tags.iter().any(|actual| actual == expected)))
        .unwrap_or(0);
    let fuzzy_match = normalized_query
        .as_deref()
        .map(|value| {
            u8::from(
                normalized_description.contains(value)
                    || catalog_domain
                        .as_deref()
                        .map(|actual: &str| actual.contains(value))
                        .unwrap_or(false)
                    || catalog_family
                        .as_deref()
                        .map(|actual: &str| actual.contains(value))
                        .unwrap_or(false)
                    || catalog_subfamily
                        .as_deref()
                        .map(|actual: &str| actual.contains(value))
                        .unwrap_or(false)
                    || catalog_tags
                        .iter()
                        .any(|actual: &String| actual.contains(value)),
            )
        })
        .unwrap_or(0);

    CatalogEntryScore {
        exact_name,
        prefix_name,
        exact_catalog,
        tag_match,
        fuzzy_match,
    }
}

fn sort_catalog_entries(
    entries: &mut [CatalogEntry],
    query: Option<&str>,
    domain: Option<&str>,
    family: Option<&str>,
    subfamily: Option<&str>,
    tag: Option<&str>,
) {
    entries.sort_by(|left, right| {
        let left_score = score_catalog_entry(left, query, domain, family, subfamily, tag);
        let right_score = score_catalog_entry(right, query, domain, family, subfamily, tag);
        right_score
            .cmp(&left_score)
            .then_with(|| right.executable.cmp(&left.executable))
            .then_with(|| left.name.cmp(&right.name))
    });
}

fn entry_json(entry: &CatalogEntry) -> serde_json::Value {
    serde_json::json!({
        "name": entry.name,
        "description": entry.description,
        "executable": entry.executable,
        "source_kind": format!("{:?}", entry.source_kind),
        "catalog": entry.catalog,
        "parameters": entry.parameters,
        "source_path": entry.source_path,
        "manifest_path": entry.manifest_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::NativeTestAuthority;
    use crate::ToolRegistry;
    use agendao_tool_core::SandboxExecutionBoundary;
    use async_trait::async_trait;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn matches_catalog_filter_checks_query_and_family() {
        let entry = CatalogEntry {
            name: "dock_pose".to_string(),
            description: "Protein-ligand docking".to_string(),
            parameters: serde_json::json!({}),
            source_kind: ToolSchemaSourceKind::Dynamic,
            catalog: Some(ToolCatalogMetadata {
                domain: Some("cadd".to_string()),
                family: Some("molecular_docking".to_string()),
                subfamily: Some("protein_ligand".to_string()),
                tags: vec!["pose".to_string(), "gnina".to_string()],
                provenance: Some("tool_import".to_string()),
            }),
            executable: false,
            source_path: None,
            manifest_path: None,
        };

        assert!(matches_structured_filter(
            &entry,
            Some("cadd"),
            Some("molecular_docking"),
            None,
            None
        ));
        assert!(matches_free_text_query(&entry, Some("dock")));
        assert!(!matches_free_text_query(&entry, Some("dynamics")));
        assert!(!matches_structured_filter(
            &entry,
            Some("biology"),
            Some("molecular_docking"),
            None,
            None
        ));
    }

    struct CatalogTestTool {
        id: &'static str,
        description: &'static str,
        catalog: Option<ToolCatalogMetadata>,
    }

    #[async_trait]
    impl Tool for CatalogTestTool {
        fn id(&self) -> &str {
            self.id
        }

        fn description(&self) -> &str {
            self.description
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                }
            })
        }

        fn source_kind(&self) -> ToolSchemaSourceKind {
            ToolSchemaSourceKind::BuiltIn
        }

        fn catalog_metadata(&self) -> Option<ToolCatalogMetadata> {
            self.catalog.clone()
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: ToolContext,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::simple("ok", self.id))
        }
    }

    fn catalog_metadata(
        domain: &str,
        family: &str,
        subfamily: &str,
        tags: &[&str],
    ) -> ToolCatalogMetadata {
        ToolCatalogMetadata {
            domain: Some(domain.to_string()),
            family: Some(family.to_string()),
            subfamily: Some(subfamily.to_string()),
            tags: tags.iter().map(|value| value.to_string()).collect(),
            provenance: Some("test".to_string()),
        }
    }

    /// Attach the shared native test authority so tools that drive real
    /// subprocesses (git, external catalog runners) launch through the
    /// boundary, not a direct spawn (Phase 8).
    fn with_test_authority(context: ToolContext) -> ToolContext {
        let authority: Arc<dyn SandboxExecutionBoundary> = Arc::new(NativeTestAuthority::new());
        context.with_sandbox_execution_boundary(authority)
    }

    async fn test_tool_context_with_registry(tools: Vec<CatalogTestTool>) -> ToolContext {
        let registry = Arc::new(ToolRegistry::new());
        let allowed = tools.iter().map(|tool| tool.id).collect::<Vec<_>>();
        for tool in tools {
            registry.register(tool).await;
        }
        let config_store = Arc::new(agendao_config::ConfigStore::new(
            agendao_config::Config::default(),
        ));
        let mut context = ToolContext::new(
            "ses_tool_catalog".to_string(),
            "msg_tool_catalog".to_string(),
            ".".to_string(),
        )
        .with_registry(registry)
        .with_config_store(config_store);
        context.extra.insert(
            CAPABILITY_ALLOWED_TOOL_IDS_KEY.to_string(),
            serde_json::json!(allowed),
        );
        with_test_authority(context)
    }

    fn allow_capability_targets(mut context: ToolContext, targets: &[&str]) -> ToolContext {
        context.extra.insert(
            CAPABILITY_ALLOWED_TOOL_IDS_KEY.to_string(),
            serde_json::json!(targets),
        );
        context
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let unique = format!(
                "{}_{}_{}",
                prefix,
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock error")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            std::fs::create_dir_all(&path).expect("failed to create test temp dir");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn local_config_store(temp: &TestDir) -> Arc<agendao_config::ConfigStore> {
        let mut loader = agendao_config::ConfigLoader::new();
        loader
            .load_from_file(temp.path.join(".agendao/agendao.jsonc"))
            .expect("local test config");
        Arc::new(agendao_config::ConfigStore::new(loader.into_config()))
    }

    #[tokio::test]
    async fn search_results_are_ranked_stably() {
        let ctx = test_tool_context_with_registry(vec![
            CatalogTestTool {
                id: "dock",
                description: "Exact match built-in docking tool",
                catalog: Some(catalog_metadata(
                    "cadd",
                    "docking",
                    "protein_ligand",
                    &["pose"],
                )),
            },
            CatalogTestTool {
                id: "dock_pose",
                description: "Prefix match docking tool",
                catalog: Some(catalog_metadata(
                    "cadd",
                    "docking",
                    "protein_ligand",
                    &["pose"],
                )),
            },
            CatalogTestTool {
                id: "ligand_dock_helper",
                description: "Fuzzy docking helper",
                catalog: Some(catalog_metadata("cadd", "screening", "ligand", &["dock"])),
            },
        ])
        .await;

        let result = CapabilityTool::primary()
            .execute(
                serde_json::json!({"action": "search", "query": "dock", "limit": 10}),
                ctx,
            )
            .await
            .expect("search should succeed");
        let names = result.metadata["results"]
            .as_array()
            .expect("results should be an array")
            .iter()
            .map(|entry| {
                entry["name"]
                    .as_str()
                    .expect("name should be present")
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["dock", "dock_pose", "ligand_dock_helper"]);
    }

    #[tokio::test]
    async fn search_supports_limit_and_offset() {
        let ctx = test_tool_context_with_registry(vec![
            CatalogTestTool {
                id: "alpha",
                description: "Alpha docking tool",
                catalog: Some(catalog_metadata("cadd", "docking", "a", &["pose"])),
            },
            CatalogTestTool {
                id: "beta",
                description: "Beta docking tool",
                catalog: Some(catalog_metadata("cadd", "docking", "b", &["pose"])),
            },
            CatalogTestTool {
                id: "gamma",
                description: "Gamma docking tool",
                catalog: Some(catalog_metadata("cadd", "docking", "c", &["pose"])),
            },
        ])
        .await;

        let result = CapabilityTool::primary()
            .execute(
                serde_json::json!({"action": "search", "family": "docking", "limit": 1, "offset": 1}),
                ctx,
            )
            .await
            .expect("search should succeed");
        let names = result.metadata["results"]
            .as_array()
            .expect("results should be an array")
            .iter()
            .map(|entry| {
                entry["name"]
                    .as_str()
                    .expect("name should be present")
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["beta"]);
        assert_eq!(result.metadata["count"], serde_json::json!(1));
        assert_eq!(result.metadata["offset"], serde_json::json!(1));
        assert_eq!(result.metadata["limit"], serde_json::json!(1));
        assert_eq!(result.metadata["total_matches"], serde_json::json!(3));
    }

    #[tokio::test]
    async fn describe_returns_fixed_shape() {
        let ctx = test_tool_context_with_registry(vec![CatalogTestTool {
            id: "dock_pose",
            description: "Protein-ligand docking",
            catalog: Some(catalog_metadata(
                "cadd",
                "molecular_docking",
                "protein_ligand",
                &["pose", "gnina"],
            )),
        }])
        .await;

        let result = CapabilityTool::primary()
            .execute(
                serde_json::json!({"action": "describe", "tool": "dock_pose"}),
                ctx,
            )
            .await
            .expect("describe should succeed");
        let resource = result.metadata["resource"]
            .as_object()
            .expect("resource metadata should be an object");

        assert_eq!(
            resource.keys().cloned().collect::<Vec<_>>(),
            vec![
                "catalog".to_string(),
                "description".to_string(),
                "executable".to_string(),
                "manifest_path".to_string(),
                "name".to_string(),
                "parameters".to_string(),
                "source_kind".to_string(),
                "source_path".to_string(),
            ]
        );
        assert_eq!(resource.get("name"), Some(&serde_json::json!("dock_pose")));
        assert_eq!(resource.get("executable"), Some(&serde_json::json!(true)));
    }

    #[tokio::test]
    async fn capability_call_executes_registry_tool_when_present() {
        let ctx = test_tool_context_with_registry(vec![CatalogTestTool {
            id: "dock_pose",
            description: "Protein-ligand docking",
            catalog: Some(catalog_metadata(
                "cadd",
                "molecular_docking",
                "protein_ligand",
                &["pose"],
            )),
        }])
        .await;

        let result = CapabilityTool::primary()
            .execute(
                serde_json::json!({"action": "call", "tool": "dock_pose", "arguments": {"query": "x"}}),
                ctx,
            )
            .await
            .expect("registry tool should execute");

        assert_eq!(result.output, "dock_pose");
    }

    #[tokio::test]
    async fn capability_cannot_discover_or_call_targets_outside_agent_policy() {
        let mut ctx = test_tool_context_with_registry(vec![CatalogTestTool {
            id: "dangerous_hidden_tool",
            description: "must remain hidden",
            catalog: None,
        }])
        .await;
        ctx.extra.insert(
            CAPABILITY_ALLOWED_TOOL_IDS_KEY.to_string(),
            serde_json::json!(["read"]),
        );

        let search = CapabilityTool::primary()
            .execute(
                serde_json::json!({"action": "search", "query": "dangerous_hidden_tool"}),
                ctx.clone(),
            )
            .await
            .expect("filtered search should succeed");
        assert_eq!(search.metadata["count"], serde_json::json!(0));

        let error = CapabilityTool::primary()
            .execute(
                serde_json::json!({
                    "action": "call",
                    "tool": "dangerous_hidden_tool",
                    "arguments": {}
                }),
                ctx,
            )
            .await
            .expect_err("guessed hidden tool id must remain denied");
        assert!(matches!(error, ToolError::PermissionDenied(_)));
    }

    #[tokio::test]
    async fn capability_call_rejects_catalog_only_external_tool() {
        let temp = TestDir::new("agendao_tool_catalog_catalog_only");
        let config_dir = temp.path.join(".agendao");
        let tools_dir = config_dir.join("tools");
        std::fs::create_dir_all(&tools_dir).expect("tools dir");
        std::fs::write(
            config_dir.join("agendao.jsonc"),
            r#"{ "toolImports": ["tools/catalog.jsonc"] }"#,
        )
        .expect("config");
        std::fs::write(
            tools_dir.join("catalog.jsonc"),
            r#"{
  "tools": {
    "dock_pose": {
      "catalog": { "domain": "cadd", "family": "molecular_docking" }
    }
  }
}"#,
        )
        .expect("catalog");

        let store = local_config_store(&temp);
        let ctx = allow_capability_targets(
            ToolContext::new(
                "ses_tool_catalog".to_string(),
                "msg_tool_catalog".to_string(),
                temp.path.to_string_lossy().to_string(),
            )
            .with_config_store(store),
            &["dock_pose"],
        );

        let error = CapabilityTool::primary()
            .execute(
                serde_json::json!({"action": "call", "tool": "dock_pose", "arguments": {"query": "x"}}),
                ctx,
            )
            .await
            .expect_err("catalog-only tool should reject execution");

        match error {
            ToolError::ExecutionError(message) => {
                assert!(message.contains("catalog-only right now"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn capability_call_rejects_skill_file_like_identifier_with_guidance() {
        let ctx = allow_capability_targets(
            test_tool_context_with_registry(vec![CatalogTestTool {
                id: "dock_pose",
                description: "Protein-ligand docking",
                catalog: Some(catalog_metadata(
                    "cadd",
                    "molecular_docking",
                    "protein_ligand",
                    &["pose"],
                )),
            }])
            .await,
            &["dock_pose", "semantic-scholar:s2_search.py"],
        );

        let error = CapabilityTool::primary()
            .execute(
                serde_json::json!({
                    "action": "call",
                    "tool": "semantic-scholar:s2_search.py",
                    "arguments": {"query": "xu ximing"}
                }),
                ctx,
            )
            .await
            .expect_err("skill-file-like identifiers should reject with guidance");

        match error {
            ToolError::InvalidArguments(message) => {
                assert!(message.contains("skill file or catalog path"));
                assert!(message.contains("skill_view(name, file_path)"));
                assert!(message.contains("results[].name"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn capability_call_executes_first_supported_external_adapter() {
        let temp = TestDir::new("agendao_tool_catalog_external_exec");
        let config_dir = temp.path.join(".agendao");
        let tools_dir = config_dir.join("tools/cadd");
        std::fs::create_dir_all(&tools_dir).expect("tools dir");
        std::fs::write(
            config_dir.join("agendao.jsonc"),
            r#"{ "toolImports": ["tools/cadd/tools.jsonc"] }"#,
        )
        .expect("config");
        std::fs::write(
            tools_dir.join("echo_tool.py"),
            r#"import json
import sys

payload = json.loads(sys.argv[1])
print(payload["query"])
"#,
        )
        .expect("script");
        std::fs::write(
            tools_dir.join("tools.jsonc"),
            r#"{
  "tools": {
    "dock_pose": {
      "catalog": { "domain": "cadd", "family": "molecular_docking" },
      "execution": {
        "kind": "script_runner",
        "entry": "./echo_tool.py"
      }
    }
  }
}"#,
        )
        .expect("catalog");

        let store = local_config_store(&temp);
        let ctx = with_test_authority(allow_capability_targets(
            ToolContext::new(
                "ses_tool_catalog".to_string(),
                "msg_tool_catalog".to_string(),
                temp.path.to_string_lossy().to_string(),
            )
            .with_config_store(store)
            .with_ask(|_request| async move { Ok(()) })
            .with_sandbox_native_allowed(true),
            &["dock_pose"],
        ));

        let result = CapabilityTool::primary()
            .execute(
                serde_json::json!({"action": "call", "tool": "dock_pose", "arguments": {"query": "pose-ok"}}),
                ctx,
            )
            .await
            .expect("external executable should run");

        assert_eq!(result.output, "pose-ok");
        assert_eq!(
            result.metadata.get("source"),
            Some(&serde_json::json!("external_catalog"))
        );
    }

    #[tokio::test]
    async fn describe_surfaces_catalog_only_vs_executable_state() {
        let temp = TestDir::new("agendao_capability_describe_states");
        let config_dir = temp.path.join(".agendao");
        let tools_dir = config_dir.join("tools/cadd");
        std::fs::create_dir_all(&tools_dir).expect("tools dir");
        std::fs::write(
            config_dir.join("agendao.jsonc"),
            r#"{ "toolImports": ["tools/cadd/tools.jsonc"] }"#,
        )
        .expect("config");
        std::fs::write(
            tools_dir.join("tools.jsonc"),
            r#"{
  "tools": {
    "dock_pose": {
      "catalog": { "domain": "cadd", "family": "molecular_docking" }
    },
    "score_pose": {
      "catalog": { "domain": "cadd", "family": "scoring" },
      "execution": { "kind": "script_runner", "entry": "./score_pose.py" }
    }
  }
}"#,
        )
        .expect("catalog");

        let store = local_config_store(&temp);
        let ctx = allow_capability_targets(
            ToolContext::new(
                "ses_tool_catalog".to_string(),
                "msg_tool_catalog".to_string(),
                temp.path.to_string_lossy().to_string(),
            )
            .with_config_store(store),
            &["dock_pose", "score_pose"],
        );

        let dock = CapabilityTool::primary()
            .execute(
                serde_json::json!({"action": "describe", "tool": "dock_pose"}),
                ctx.clone(),
            )
            .await
            .expect("describe dock_pose");
        let score = CapabilityTool::primary()
            .execute(
                serde_json::json!({"action": "describe", "tool": "score_pose"}),
                ctx,
            )
            .await
            .expect("describe score_pose");

        assert_eq!(
            dock.metadata["resource"]["executable"],
            serde_json::json!(false)
        );
        assert_eq!(
            score.metadata["resource"]["executable"],
            serde_json::json!(true)
        );
    }

    #[tokio::test]
    async fn search_finds_imported_tool_by_directory_inferred_family() {
        let temp = TestDir::new("agendao_tool_catalog_inferred_family_search");
        let config_dir = temp.path.join(".agendao");
        let tools_dir = config_dir.join("tools/cadd/molecular_docking");
        std::fs::create_dir_all(&tools_dir).expect("tools dir");
        std::fs::write(
            config_dir.join("agendao.jsonc"),
            r#"{ "toolImports": ["tools/catalog.jsonc"] }"#,
        )
        .expect("config");
        std::fs::write(
            config_dir.join("tools/catalog.jsonc"),
            r#"{
  "tools": {
    "dock_pose": {
      "source": { "path": "./cadd/molecular_docking/dock_pose.py" },
      "catalog": {}
    }
  }
}"#,
        )
        .expect("catalog");

        let store = local_config_store(&temp);
        let ctx = allow_capability_targets(
            ToolContext::new(
                "ses_tool_catalog".to_string(),
                "msg_tool_catalog".to_string(),
                temp.path.to_string_lossy().to_string(),
            )
            .with_config_store(store),
            &["dock_pose"],
        );

        let result = CapabilityTool::primary()
            .execute(
                serde_json::json!({"action": "search", "family": "molecular_docking", "limit": 10}),
                ctx,
            )
            .await
            .expect("search should succeed");

        let names = result.metadata["results"]
            .as_array()
            .expect("results array")
            .iter()
            .map(|entry| entry["name"].as_str().expect("name").to_string())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["dock_pose"]);
    }
}
