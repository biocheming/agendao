use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use agendao_config::{ExternalToolConfig, ExternalToolExecutionKind, ResolvedExternalToolCatalog};
use agendao_types::ToolCatalogMetadata;
use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::timeout;

use crate::{
    assert_external_directory, bash::authorize_bash_command, ExternalDirectoryKind,
    ExternalDirectoryOptions, Metadata, Tool, ToolContext, ToolError, ToolResult,
    ToolSchemaSourceKind,
};

pub const MCP_SEARCH_TOOL_ID: &str = "mcp_search";
pub const MCP_DESCRIBE_TOOL_ID: &str = "mcp_describe";
pub const MCP_CALL_TOOL_ID: &str = "mcp_call";
pub const TOOL_CATALOG_FACADE_TOOL_IDS: &[&str] =
    &[MCP_SEARCH_TOOL_ID, MCP_DESCRIBE_TOOL_ID, MCP_CALL_TOOL_ID];

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
    TOOL_CATALOG_FACADE_TOOL_IDS.contains(&name)
}

pub struct McpSearchTool;

pub struct McpDescribeTool;

pub struct McpCallTool;

#[derive(Debug, Deserialize)]
struct McpSearchInput {
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
struct McpDescribeInput {
    tool: String,
}

#[derive(Debug, Deserialize)]
struct McpCallInput {
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

#[async_trait]
impl Tool for McpSearchTool {
    fn id(&self) -> &str {
        MCP_SEARCH_TOOL_ID
    }

    fn description(&self) -> &str {
        "Search the execution resource catalog by name, description, domain, family, or tag. Use this first when the tool catalog is large."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "domain": { "type": "string" },
                "family": { "type": "string" },
                "subfamily": { "type": "string" },
                "tag": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 8 },
                "offset": { "type": "integer", "minimum": 0, "default": 0 }
            }
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: McpSearchInput = serde_json::from_value(args)
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
                serde_json::json!(results
                    .iter()
                    .map(|entry| entry_json(entry))
                    .collect::<Vec<_>>()),
            )
            .with_metadata("count", serde_json::json!(results.len()))
            .with_metadata("offset", serde_json::json!(offset))
            .with_metadata("limit", serde_json::json!(limit))
            .with_metadata("total_matches", serde_json::json!(total_matches)))
    }
}

#[async_trait]
impl Tool for McpDescribeTool {
    fn id(&self) -> &str {
        MCP_DESCRIBE_TOOL_ID
    }

    fn description(&self) -> &str {
        "Describe one execution resource in detail, including schema, catalog metadata, and whether it is executable."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "tool": { "type": "string", "description": "Exact tool name from mcp_search results" }
            },
            "required": ["tool"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: McpDescribeInput = serde_json::from_value(args)
            .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
        let entries = collect_catalog_entries(&ctx).await?;
        let Some(entry) = entries.into_iter().find(|entry| entry.name == input.tool) else {
            return Err(ToolError::InvalidArguments(format!(
                "execution resource `{}` not found; use {} first",
                input.tool, MCP_SEARCH_TOOL_ID
            )));
        };

        let resource = entry_json(&entry);
        let output =
            serde_json::to_string_pretty(&resource).unwrap_or_else(|_| format!("{:?}", entry.name));
        Ok(ToolResult::simple("Execution resource detail", output)
            .with_metadata("resource", resource))
    }
}

#[async_trait]
impl Tool for McpCallTool {
    fn id(&self) -> &str {
        MCP_CALL_TOOL_ID
    }

    fn description(&self) -> &str {
        "Call an execution resource returned by mcp_search after inspecting it with mcp_describe."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "tool": { "type": "string" },
                "arguments": { "type": "object", "additionalProperties": true }
            },
            "required": ["tool", "arguments"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: McpCallInput = serde_json::from_value(args)
            .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
        if is_tool_catalog_facade_tool(&input.tool) {
            return Err(ToolError::InvalidArguments(
                "mcp_call cannot target catalog facade tools".to_string(),
            ));
        }

        if let Some(registry) = ctx.registry.clone() {
            if registry.get(&input.tool).await.is_some() {
                return registry.execute(&input.tool, input.arguments, ctx).await;
            }
        }

        let external_catalogs = load_external_catalogs(&ctx)?;
        if let Some(config) = find_external_catalog_config(&external_catalogs, &input.tool) {
            if config.is_executable() {
                return execute_external_catalog_tool(&input.tool, config, input.arguments, &ctx)
                    .await;
            }
            let entry = find_external_catalog_entry(&external_catalogs, &input.tool)
                .expect("entry should exist when config exists");
            return Err(ToolError::ExecutionError(format!(
                "execution resource `{}` is catalog-only right now; no execution adapter is registered yet{}",
                input.tool,
                entry.source_path
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
}

async fn collect_catalog_entries(ctx: &ToolContext) -> Result<Vec<CatalogEntry>, ToolError> {
    let mut entries = BTreeMap::new();

    if let Some(registry) = ctx.registry.clone() {
        for id in registry.list_ids().await {
            if is_tool_catalog_facade_tool(&id) || id == "invalid" {
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
            entries.insert(
                tool_name.clone(),
                external_catalog_entry(tool_name, &config),
            );
        }
    }

    Ok(entries.into_values().collect())
}

fn load_external_catalogs(
    ctx: &ToolContext,
) -> Result<Vec<ResolvedExternalToolCatalog>, ToolError> {
    let project_root = if !ctx.project_root.trim().is_empty() {
        PathBuf::from(ctx.project_root.trim())
    } else if let Some(config_store) = ctx.config_store.as_ref() {
        config_store
            .project_dir()
            .unwrap_or_else(|| PathBuf::from(ctx.directory.clone()))
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

    let mut cmd = tokio::process::Command::new(runtime);
    cmd.arg(entry);
    cmd.arg(compact_args);
    cmd.current_dir(&workdir);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|error| {
        ToolError::ExecutionError(format!("Failed to spawn process: {}", error))
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ToolError::ExecutionError("failed to capture stdout".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ToolError::ExecutionError("failed to capture stderr".to_string()))?;
    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();
    let abort_token = ctx.abort.clone();
    let mut output = String::new();
    let mut stderr_output = String::new();
    let timeout_ms = 30_000;

    let result = timeout(Duration::from_millis(timeout_ms), async {
        loop {
            tokio::select! {
                _ = abort_token.cancelled() => {
                    if let Err(error) = child.kill().await {
                        tracing::debug!(%error, "failed to kill external catalog tool after cancellation");
                    }
                    return Err(ToolError::Cancelled);
                }
                line = stdout_reader.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            output.push_str(&line);
                            output.push('\n');
                        }
                        Ok(None) => break,
                        Err(error) => return Err(ToolError::ExecutionError(format!("failed to read external tool stdout: {}", error))),
                    }
                }
                line = stderr_reader.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            stderr_output.push_str(&line);
                            stderr_output.push('\n');
                        }
                        Ok(None) => break,
                        Err(error) => return Err(ToolError::ExecutionError(format!("failed to read external tool stderr: {}", error))),
                    }
                }
            }
        }
        Ok::<(), ToolError>(())
    })
    .await;

    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(error),
        Err(_) => {
            if let Err(error) = child.kill().await {
                tracing::debug!(%error, "failed to kill external catalog tool after timeout");
            }
            return Err(ToolError::Timeout(format!(
                "external catalog tool `{}` timed out after {}ms",
                tool_name, timeout_ms
            )));
        }
    }

    let status = child.wait().await.map_err(|error| {
        ToolError::ExecutionError(format!("Failed to wait for process: {}", error))
    })?;

    if !status.success() {
        let mut message = format!(
            "external catalog tool `{}` exited with code {}",
            tool_name,
            status.code().unwrap_or(-1)
        );
        if !stderr_output.trim().is_empty() {
            message.push_str(": ");
            message.push_str(stderr_output.trim());
        }
        return Err(ToolError::ExecutionError(message));
    }

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
    use crate::ToolRegistry;
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

    async fn test_tool_context_with_registry(tools: Vec<CatalogTestTool>) -> ToolContext {
        let registry = Arc::new(ToolRegistry::new());
        for tool in tools {
            registry.register(tool).await;
        }
        ToolContext::new(
            "ses_tool_catalog".to_string(),
            "msg_tool_catalog".to_string(),
            ".".to_string(),
        )
        .with_registry(registry)
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

        let result = McpSearchTool
            .execute(serde_json::json!({"query": "dock", "limit": 10}), ctx)
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

        let result = McpSearchTool
            .execute(
                serde_json::json!({"family": "docking", "limit": 1, "offset": 1}),
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

        let result = McpDescribeTool
            .execute(serde_json::json!({"tool": "dock_pose"}), ctx)
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
    async fn mcp_call_executes_registry_tool_when_present() {
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

        let result = McpCallTool
            .execute(
                serde_json::json!({"tool": "dock_pose", "arguments": {"query": "x"}}),
                ctx,
            )
            .await
            .expect("registry tool should execute");

        assert_eq!(result.output, "dock_pose");
    }

    #[tokio::test]
    async fn mcp_call_rejects_catalog_only_external_tool() {
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

        let store = Arc::new(
            agendao_config::ConfigStore::from_project_dir(&temp.path).expect("config store"),
        );
        let ctx = ToolContext::new(
            "ses_tool_catalog".to_string(),
            "msg_tool_catalog".to_string(),
            temp.path.to_string_lossy().to_string(),
        )
        .with_config_store(store);

        let error = McpCallTool
            .execute(
                serde_json::json!({"tool": "dock_pose", "arguments": {"query": "x"}}),
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
    async fn mcp_call_executes_first_supported_external_adapter() {
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

        let store = Arc::new(
            agendao_config::ConfigStore::from_project_dir(&temp.path).expect("config store"),
        );
        let ctx = ToolContext::new(
            "ses_tool_catalog".to_string(),
            "msg_tool_catalog".to_string(),
            temp.path.to_string_lossy().to_string(),
        )
        .with_config_store(store)
        .with_ask(|_request| async move { Ok(()) });

        let result = McpCallTool
            .execute(
                serde_json::json!({"tool": "dock_pose", "arguments": {"query": "pose-ok"}}),
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
        let temp = TestDir::new("agendao_tool_catalog_describe_states");
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

        let store = Arc::new(
            agendao_config::ConfigStore::from_project_dir(&temp.path).expect("config store"),
        );
        let ctx = ToolContext::new(
            "ses_tool_catalog".to_string(),
            "msg_tool_catalog".to_string(),
            temp.path.to_string_lossy().to_string(),
        )
        .with_config_store(store);

        let dock = McpDescribeTool
            .execute(serde_json::json!({"tool": "dock_pose"}), ctx.clone())
            .await
            .expect("describe dock_pose");
        let score = McpDescribeTool
            .execute(serde_json::json!({"tool": "score_pose"}), ctx)
            .await
            .expect("describe score_pose");

        assert_eq!(dock.metadata["resource"]["executable"], serde_json::json!(false));
        assert_eq!(score.metadata["resource"]["executable"], serde_json::json!(true));
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

        let store = Arc::new(
            agendao_config::ConfigStore::from_project_dir(&temp.path).expect("config store"),
        );
        let ctx = ToolContext::new(
            "ses_tool_catalog".to_string(),
            "msg_tool_catalog".to_string(),
            temp.path.to_string_lossy().to_string(),
        )
        .with_config_store(store);

        let result = McpSearchTool
            .execute(
                serde_json::json!({"family": "molecular_docking", "limit": 10}),
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
