use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::context_docs_backend::{
    load_registered_docs_source, resolve_registered_docs_source_display,
};
use crate::{Metadata, Tool, ToolContext, ToolError, ToolResult};

const MAX_LIMIT: usize = 20;
const DEFAULT_LIMIT: usize = 5;
const MAX_PAGE_OUTPUT_CHARS: usize = 20_000;

const DESCRIPTION: &str = r#"Docs-aware official documentation lookup.

This tool is the AgenDao authority for docs source resolution and structured docs queries.

Implemented operations:
- resolve_library
- query_docs
- get_page

Current backend scope:
- registry-driven docs sources
- local docs index JSON files
- local markdown bundle directories
- remote docs index URLs

Not yet included:
- richer remote docs providers
- automatic crawling/indexing"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ContextDocsOperation {
    ResolveLibrary,
    QueryDocs,
    GetPage,
}

impl ContextDocsOperation {
    fn as_str(&self) -> &'static str {
        match self {
            Self::ResolveLibrary => "resolve_library",
            Self::QueryDocs => "query_docs",
            Self::GetPage => "get_page",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextDocsInput {
    operation: ContextDocsOperation,
    #[serde(default)]
    library: Option<String>,
    #[serde(default, alias = "library_id")]
    library_id: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default, alias = "page_id")]
    page_id: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct ContextDocsRegistry {
    libraries: Vec<RegisteredLibrary>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegisteredLibrary {
    pub(crate) library_id: String,
    pub(crate) display_name: String,
    #[serde(default)]
    pub(crate) aliases: Vec<String>,
    pub(crate) source_family: String,
    #[serde(default)]
    pub(crate) default_version: Option<String>,
    #[serde(default)]
    pub(crate) homepage: Option<String>,
    pub(crate) index_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DocsIndex {
    #[serde(default)]
    pub(crate) library_id: Option<String>,
    #[serde(default)]
    pub(crate) version: Option<String>,
    pub(crate) pages: Vec<DocsPage>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DocsPage {
    pub(crate) page_id: String,
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) content: String,
    #[serde(default)]
    pub(crate) summary: Option<String>,
    #[serde(default)]
    pub(crate) version: Option<String>,
    #[serde(default)]
    pub(crate) headings: Vec<String>,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextDocsRegistryValidationSummary {
    pub valid: bool,
    pub registry_path: String,
    pub library_count: usize,
    pub libraries: Vec<ContextDocsLibraryValidationSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextDocsLibraryValidationSummary {
    pub library_id: String,
    pub display_name: String,
    pub source_family: String,
    pub index_path: String,
    pub resolved_index_path: String,
    pub page_count: usize,
    pub index_library_id: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextDocsIndexValidationSummary {
    pub valid: bool,
    pub index_path: String,
    pub library_id: Option<String>,
    pub version: Option<String>,
    pub page_count: usize,
    pub page_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct LibraryMatch {
    library_id: String,
    display_name: String,
    source: String,
    default_version: Option<String>,
    homepage: Option<String>,
    aliases: Vec<String>,
    score: i64,
}

#[derive(Debug, Clone, Serialize)]
struct DocsHit {
    page_id: String,
    title: String,
    url: String,
    snippet: String,
    version: Option<String>,
    score: i64,
}

#[derive(Debug, Clone, Serialize)]
struct PageView {
    page_id: String,
    title: String,
    url: String,
    content: String,
    summary: Option<String>,
    version: Option<String>,
    headings: Vec<String>,
    tags: Vec<String>,
}

fn default_limit() -> usize {
    DEFAULT_LIMIT
}

pub(crate) struct ContextDocsTool;

impl ContextDocsTool {
    pub fn new() -> Self {
        Self
    }

    async fn execute_impl(
        &self,
        input: &ContextDocsInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let registry_path = context_docs_registry_path(ctx)?;
        let registry = load_registry(&registry_path)?;

        match input.operation {
            ContextDocsOperation::ResolveLibrary => {
                execute_resolve_library(input, &registry, &registry_path)
            }
            ContextDocsOperation::QueryDocs => execute_query_docs(input, &registry, &registry_path),
            ContextDocsOperation::GetPage => execute_get_page(input, &registry, &registry_path),
        }
    }
}

impl Default for ContextDocsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ContextDocsTool {
    fn id(&self) -> &str {
        "context_docs"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["resolve_library", "query_docs", "get_page"],
                    "description": "Docs-aware operation to execute"
                },
                "library": {
                    "type": "string",
                    "description": "Human-facing library or product name for resolve_library"
                },
                "library_id": {
                    "type": "string",
                    "description": "Resolved docs source id for query_docs or get_page"
                },
                "query": {
                    "type": "string",
                    "description": "Docs query text for query_docs"
                },
                "page_id": {
                    "type": "string",
                    "description": "Canonical page id for get_page"
                },
                "version": {
                    "type": "string",
                    "description": "Optional version hint"
                },
                "source": {
                    "type": "string",
                    "description": "Optional source family hint"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 20,
                    "default": 5,
                    "description": "Maximum number of matches to return"
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: ContextDocsInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        validate_input(&input)?;

        let mut permission = crate::PermissionRequest::new("context_docs")
            .with_metadata("operation", serde_json::json!(input.operation.as_str()))
            .with_metadata("limit", serde_json::json!(input.limit))
            .always_allow();
        if let Some(library) = input.library.as_ref() {
            permission = permission.with_metadata("library", serde_json::json!(library));
        }
        if let Some(library_id) = input.library_id.as_ref() {
            permission = permission.with_metadata("library_id", serde_json::json!(library_id));
        }
        if let Some(query) = input.query.as_ref() {
            permission = permission.with_metadata("query", serde_json::json!(query));
        }
        if let Some(page_id) = input.page_id.as_ref() {
            permission = permission.with_metadata("page_id", serde_json::json!(page_id));
        }
        if let Some(version) = input.version.as_ref() {
            permission = permission.with_metadata("version", serde_json::json!(version));
        }
        if let Some(source) = input.source.as_ref() {
            permission = permission.with_metadata("source", serde_json::json!(source));
        }
        ctx.ask_permission(permission).await?;

        self.execute_impl(&input, &ctx).await
    }
}

fn validate_input(input: &ContextDocsInput) -> Result<(), ToolError> {
    if input.limit == 0 || input.limit > MAX_LIMIT {
        return Err(ToolError::InvalidArguments(format!(
            "limit must be between 1 and {}",
            MAX_LIMIT
        )));
    }

    match input.operation {
        ContextDocsOperation::ResolveLibrary => {
            let library = input.library.as_deref().unwrap_or_default().trim();
            if library.is_empty() {
                return Err(ToolError::InvalidArguments(
                    "library is required for resolve_library".to_string(),
                ));
            }
        }
        ContextDocsOperation::QueryDocs => {
            let library_id = input.library_id.as_deref().unwrap_or_default().trim();
            let query = input.query.as_deref().unwrap_or_default().trim();
            if library_id.is_empty() {
                return Err(ToolError::InvalidArguments(
                    "library_id is required for query_docs".to_string(),
                ));
            }
            if query.is_empty() {
                return Err(ToolError::InvalidArguments(
                    "query is required for query_docs".to_string(),
                ));
            }
        }
        ContextDocsOperation::GetPage => {
            let library_id = input.library_id.as_deref().unwrap_or_default().trim();
            let page_id = input.page_id.as_deref().unwrap_or_default().trim();
            if library_id.is_empty() {
                return Err(ToolError::InvalidArguments(
                    "library_id is required for get_page".to_string(),
                ));
            }
            if page_id.is_empty() {
                return Err(ToolError::InvalidArguments(
                    "page_id is required for get_page".to_string(),
                ));
            }
        }
    }

    Ok(())
}

fn execute_resolve_library(
    input: &ContextDocsInput,
    registry: &ContextDocsRegistry,
    registry_path: &Path,
) -> Result<ToolResult, ToolError> {
    let requested_library = input.library.as_deref().unwrap_or_default().trim();
    let requested_version = normalized_optional(input.version.as_deref());
    let requested_source = normalized_optional(input.source.as_deref());

    let mut matches = registry
        .libraries
        .iter()
        .filter(|entry| source_matches(&requested_source, &entry.source_family))
        .filter_map(|entry| {
            let score = library_match_score(requested_library, entry, requested_version.as_deref());
            if score <= 0 {
                return None;
            }
            Some(LibraryMatch {
                library_id: entry.library_id.clone(),
                display_name: entry.display_name.clone(),
                source: entry.source_family.clone(),
                default_version: entry.default_version.clone(),
                homepage: entry.homepage.clone(),
                aliases: entry.aliases.clone(),
                score,
            })
        })
        .collect::<Vec<_>>();

    matches.sort_by(|left, right| {
        cmp_scored(
            right.score,
            &right.display_name,
            left.score,
            &left.display_name,
        )
    });
    let truncated = matches.len() > input.limit;
    matches.truncate(input.limit);

    let title = format!("Resolve docs library: {}", requested_library);
    let output = if matches.is_empty() {
        format!(
            "No docs source matched `{}` in registry `{}`.",
            requested_library,
            registry_path.display()
        )
    } else {
        let mut lines = vec![format!(
            "Resolved `{}` against docs registry `{}`:",
            requested_library,
            registry_path.display()
        )];
        for matched in &matches {
            let version = matched.default_version.as_deref().unwrap_or("unknown");
            let homepage = matched.homepage.as_deref().unwrap_or("-");
            lines.push(format!(
                "- {} ({}) source={} default_version={} score={} homepage={}",
                matched.display_name,
                matched.library_id,
                matched.source,
                version,
                matched.score,
                homepage
            ));
        }
        lines.join("\n")
    };

    let mut metadata = Metadata::new();
    metadata.insert(
        "operation".to_string(),
        serde_json::json!(ContextDocsOperation::ResolveLibrary.as_str()),
    );
    metadata.insert("library".to_string(), serde_json::json!(requested_library));
    metadata.insert("matches".to_string(), serde_json::json!(matches));
    metadata.insert("truncated".to_string(), serde_json::json!(truncated));
    metadata.insert(
        "registry_path".to_string(),
        serde_json::json!(registry_path.to_string_lossy().to_string()),
    );

    Ok(ToolResult {
        title,
        output,
        metadata,
        truncated,
    })
}

fn execute_query_docs(
    input: &ContextDocsInput,
    registry: &ContextDocsRegistry,
    registry_path: &Path,
) -> Result<ToolResult, ToolError> {
    let library_id = input.library_id.as_deref().unwrap_or_default().trim();
    let entry = find_library_entry(registry, library_id, input.source.as_deref())?;
    let (index, _, backend) = load_registered_docs_source(registry_path, entry)?;
    let requested_version = normalized_optional(input.version.as_deref());
    let query = input.query.as_deref().unwrap_or_default().trim();

    let mut hits = index
        .pages
        .iter()
        .filter(|page| page_version_matches(page, &index, requested_version.as_deref(), entry))
        .filter_map(|page| {
            let score = docs_page_score(query, page);
            if score <= 0 {
                return None;
            }
            Some(DocsHit {
                page_id: page.page_id.clone(),
                title: page.title.clone(),
                url: page.url.clone(),
                snippet: build_page_snippet(page, query),
                version: resolved_page_version(page, &index, entry),
                score,
            })
        })
        .collect::<Vec<_>>();

    hits.sort_by(|left, right| cmp_scored(right.score, &right.title, left.score, &left.title));
    let truncated = hits.len() > input.limit;
    hits.truncate(input.limit);

    let title = format!("Query docs: {}", entry.display_name);
    let output = if hits.is_empty() {
        format!(
            "No docs results for `{}` in `{}` (library_id=`{}`).",
            query, entry.display_name, entry.library_id
        )
    } else {
        let mut lines = vec![format!(
            "Docs results for `{}` in {}:",
            query, entry.display_name
        )];
        for hit in &hits {
            let version = hit.version.as_deref().unwrap_or("unknown");
            lines.push(format!(
                "- {} ({}) version={} score={}\n  {}\n  {}",
                hit.title, hit.page_id, version, hit.score, hit.url, hit.snippet
            ));
        }
        lines.join("\n")
    };

    let mut metadata = Metadata::new();
    metadata.insert(
        "operation".to_string(),
        serde_json::json!(ContextDocsOperation::QueryDocs.as_str()),
    );
    metadata.insert(
        "library_id".to_string(),
        serde_json::json!(entry.library_id),
    );
    metadata.insert("query".to_string(), serde_json::json!(query));
    metadata.insert("results".to_string(), serde_json::json!(hits));
    metadata.insert("backend".to_string(), serde_json::json!(backend.as_str()));
    metadata.insert("truncated".to_string(), serde_json::json!(truncated));
    metadata.insert(
        "registry_path".to_string(),
        serde_json::json!(registry_path.to_string_lossy().to_string()),
    );

    Ok(ToolResult {
        title,
        output,
        metadata,
        truncated,
    })
}

fn execute_get_page(
    input: &ContextDocsInput,
    registry: &ContextDocsRegistry,
    registry_path: &Path,
) -> Result<ToolResult, ToolError> {
    let library_id = input.library_id.as_deref().unwrap_or_default().trim();
    let entry = find_library_entry(registry, library_id, input.source.as_deref())?;
    let (index, _, backend) = load_registered_docs_source(registry_path, entry)?;
    let requested_version = normalized_optional(input.version.as_deref());
    let requested_page_id = input.page_id.as_deref().unwrap_or_default().trim();

    let page = index
        .pages
        .iter()
        .find(|page| {
            normalized(&page.page_id) == normalized(requested_page_id)
                && page_version_matches(page, &index, requested_version.as_deref(), entry)
        })
        .ok_or_else(|| {
            ToolError::ExecutionError(format!(
                "page `{}` was not found for library_id `{}`",
                requested_page_id, entry.library_id
            ))
        })?;

    let truncated = page.content.chars().count() > MAX_PAGE_OUTPUT_CHARS;
    let rendered_content = if truncated {
        truncate_chars(&page.content, MAX_PAGE_OUTPUT_CHARS)
    } else {
        page.content.clone()
    };

    let view = PageView {
        page_id: page.page_id.clone(),
        title: page.title.clone(),
        url: page.url.clone(),
        content: rendered_content.clone(),
        summary: page.summary.clone(),
        version: resolved_page_version(page, &index, entry),
        headings: page.headings.clone(),
        tags: page.tags.clone(),
    };

    let mut output_lines = vec![format!("{} ({})\n{}", view.title, view.page_id, view.url)];
    if let Some(version) = view.version.as_deref() {
        output_lines.push(format!("version: {}", version));
    }
    if let Some(summary) = view.summary.as_deref() {
        output_lines.push(format!("summary: {}", summary));
    }
    if !view.headings.is_empty() {
        output_lines.push(format!("headings: {}", view.headings.join(", ")));
    }
    if !view.tags.is_empty() {
        output_lines.push(format!("tags: {}", view.tags.join(", ")));
    }
    output_lines.push(String::new());
    output_lines.push(rendered_content);
    if truncated {
        output_lines.push(format!(
            "\n[output truncated at {} characters]",
            MAX_PAGE_OUTPUT_CHARS
        ));
    }

    let mut metadata = Metadata::new();
    metadata.insert(
        "operation".to_string(),
        serde_json::json!(ContextDocsOperation::GetPage.as_str()),
    );
    metadata.insert(
        "library_id".to_string(),
        serde_json::json!(entry.library_id),
    );
    metadata.insert("page".to_string(), serde_json::json!(view));
    metadata.insert("backend".to_string(), serde_json::json!(backend.as_str()));
    metadata.insert("truncated".to_string(), serde_json::json!(truncated));
    metadata.insert(
        "registry_path".to_string(),
        serde_json::json!(registry_path.to_string_lossy().to_string()),
    );

    Ok(ToolResult {
        title: format!("Docs page: {}", page.title),
        output: output_lines.join("\n"),
        metadata,
        truncated,
    })
}

fn context_docs_registry_path(ctx: &ToolContext) -> Result<PathBuf, ToolError> {
    let configured = ctx
        .runtime_config
        .context_docs_registry_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ToolError::ExecutionError(
                "context_docs registry path is not configured; set docs.contextDocsRegistryPath in agendao config"
                    .to_string(),
            )
        })?;
    let path = PathBuf::from(configured);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(PathBuf::from(&ctx.project_root).join(path))
    }
}

pub fn validate_registry_file(
    path: &Path,
) -> Result<ContextDocsRegistryValidationSummary, ToolError> {
    let registry = load_registry(path)?;
    validate_registry_summary(path, &registry)
}

pub fn validate_docs_index_file(
    path: &Path,
) -> Result<ContextDocsIndexValidationSummary, ToolError> {
    validate_docs_index_file_with_expected_library(path, None)
}

fn load_registry(path: &Path) -> Result<ContextDocsRegistry, ToolError> {
    let contents = fs::read_to_string(path).map_err(|err| {
        ToolError::ExecutionError(format!(
            "failed to read context_docs registry `{}`: {}",
            path.display(),
            err
        ))
    })?;
    let registry: ContextDocsRegistry = serde_json::from_str(&contents).map_err(|err| {
        ToolError::ExecutionError(format!(
            "failed to parse context_docs registry `{}`: {}",
            path.display(),
            err
        ))
    })?;
    if registry.libraries.is_empty() {
        return Err(ToolError::ExecutionError(format!(
            "context_docs registry `{}` does not define any libraries",
            path.display()
        )));
    }
    Ok(registry)
}

fn validate_registry_summary(
    registry_path: &Path,
    registry: &ContextDocsRegistry,
) -> Result<ContextDocsRegistryValidationSummary, ToolError> {
    let mut seen_library_keys = BTreeSet::new();
    let mut libraries = Vec::with_capacity(registry.libraries.len());

    for entry in &registry.libraries {
        validate_registered_library(entry, registry_path)?;
        let unique_key = format!(
            "{}::{}",
            normalized(&entry.library_id),
            normalized(&entry.source_family)
        );
        if !seen_library_keys.insert(unique_key) {
            return Err(ToolError::ExecutionError(format!(
                "context_docs registry `{}` has duplicate library/source entry for `{}::{}`",
                registry_path.display(),
                entry.library_id,
                entry.source_family
            )));
        }

        let resolved_index_path = resolve_registered_docs_source_display(registry_path, entry);
        let (_, index_summary, _) = load_registered_docs_source(registry_path, entry)?;

        libraries.push(ContextDocsLibraryValidationSummary {
            library_id: entry.library_id.clone(),
            display_name: entry.display_name.clone(),
            source_family: entry.source_family.clone(),
            index_path: entry.index_path.clone(),
            resolved_index_path,
            page_count: index_summary.page_count,
            index_library_id: index_summary.library_id.clone(),
            version: index_summary.version.clone(),
        });
    }

    Ok(ContextDocsRegistryValidationSummary {
        valid: true,
        registry_path: registry_path.display().to_string(),
        library_count: libraries.len(),
        libraries,
    })
}

fn validate_registered_library(
    entry: &RegisteredLibrary,
    registry_path: &Path,
) -> Result<(), ToolError> {
    validate_non_empty(
        "library_id",
        &entry.library_id,
        &format!("registry `{}`", registry_path.display()),
    )?;
    validate_non_empty(
        "display_name",
        &entry.display_name,
        &format!(
            "registry `{}` library `{}`",
            registry_path.display(),
            entry.library_id
        ),
    )?;
    validate_non_empty(
        "source_family",
        &entry.source_family,
        &format!(
            "registry `{}` library `{}`",
            registry_path.display(),
            entry.library_id
        ),
    )?;
    validate_non_empty(
        "index_path",
        &entry.index_path,
        &format!(
            "registry `{}` library `{}`",
            registry_path.display(),
            entry.library_id
        ),
    )?;
    for alias in &entry.aliases {
        validate_non_empty(
            "alias",
            alias,
            &format!(
                "registry `{}` library `{}`",
                registry_path.display(),
                entry.library_id
            ),
        )?;
    }
    Ok(())
}

pub(crate) fn resolve_registry_index_path(registry_path: &Path, index_path: &str) -> PathBuf {
    let registry_dir = registry_path.parent().unwrap_or_else(|| Path::new("."));
    registry_dir.join(index_path)
}

fn validate_docs_index_file_with_expected_library(
    path: &Path,
    expected_library_id: Option<&str>,
) -> Result<ContextDocsIndexValidationSummary, ToolError> {
    let (_, summary) = load_docs_index_from_path(path, expected_library_id)?;
    Ok(summary)
}

pub(crate) fn load_docs_index_from_path(
    path: &Path,
    expected_library_id: Option<&str>,
) -> Result<(DocsIndex, ContextDocsIndexValidationSummary), ToolError> {
    let contents = fs::read_to_string(path).map_err(|err| {
        ToolError::ExecutionError(format!(
            "failed to read docs index `{}`: {}",
            path.display(),
            err
        ))
    })?;
    let index: DocsIndex = serde_json::from_str(&contents).map_err(|err| {
        ToolError::ExecutionError(format!(
            "failed to parse docs index `{}`: {}",
            path.display(),
            err
        ))
    })?;
    let summary = validate_docs_index_summary(path, &index, expected_library_id)?;
    Ok((index, summary))
}

pub(crate) fn validate_docs_index_summary(
    index_path: &Path,
    index: &DocsIndex,
    expected_library_id: Option<&str>,
) -> Result<ContextDocsIndexValidationSummary, ToolError> {
    if let Some(index_library_id) = index.library_id.as_deref() {
        validate_non_empty(
            "library_id",
            index_library_id,
            &format!("docs index `{}`", index_path.display()),
        )?;
    }

    if let Some(expected_library_id) = expected_library_id {
        if let Some(index_library_id) = index.library_id.as_deref() {
            if normalized(index_library_id) != normalized(expected_library_id) {
                return Err(ToolError::ExecutionError(format!(
                    "docs index `{}` belongs to library_id `{}` but registry entry expected `{}`",
                    index_path.display(),
                    index_library_id,
                    expected_library_id
                )));
            }
        }
    }

    if index.pages.is_empty() {
        return Err(ToolError::ExecutionError(format!(
            "docs index `{}` does not define any pages",
            index_path.display()
        )));
    }

    let mut seen_page_ids = BTreeSet::new();
    let mut page_ids = Vec::with_capacity(index.pages.len());
    for page in &index.pages {
        validate_docs_page(page, index_path)?;
        let normalized_page_id = normalized(&page.page_id);
        if !seen_page_ids.insert(normalized_page_id) {
            return Err(ToolError::ExecutionError(format!(
                "docs index `{}` has duplicate page_id `{}`",
                index_path.display(),
                page.page_id
            )));
        }
        page_ids.push(page.page_id.clone());
    }

    Ok(ContextDocsIndexValidationSummary {
        valid: true,
        index_path: index_path.display().to_string(),
        library_id: index.library_id.clone(),
        version: index.version.clone(),
        page_count: page_ids.len(),
        page_ids,
    })
}

fn validate_docs_page(page: &DocsPage, index_path: &Path) -> Result<(), ToolError> {
    let context = format!(
        "docs index `{}` page `{}`",
        index_path.display(),
        page.page_id
    );
    validate_non_empty("page_id", &page.page_id, &context)?;
    validate_non_empty("title", &page.title, &context)?;
    validate_non_empty("url", &page.url, &context)?;
    validate_non_empty("content", &page.content, &context)?;
    if let Some(summary) = page.summary.as_deref() {
        validate_non_empty("summary", summary, &context)?;
    }
    if let Some(version) = page.version.as_deref() {
        validate_non_empty("version", version, &context)?;
    }
    for heading in &page.headings {
        validate_non_empty("heading", heading, &context)?;
    }
    for tag in &page.tags {
        validate_non_empty("tag", tag, &context)?;
    }
    Ok(())
}

fn validate_non_empty(field_name: &str, value: &str, context: &str) -> Result<(), ToolError> {
    if value.trim().is_empty() {
        return Err(ToolError::ExecutionError(format!(
            "{} must be non-empty in {}",
            field_name, context
        )));
    }
    Ok(())
}

fn find_library_entry<'a>(
    registry: &'a ContextDocsRegistry,
    library_id: &str,
    source: Option<&str>,
) -> Result<&'a RegisteredLibrary, ToolError> {
    let requested_library_id = normalized(library_id);
    let requested_source = normalized_optional(source);
    registry
        .libraries
        .iter()
        .find(|entry| {
            normalized(&entry.library_id) == requested_library_id
                && source_matches(&requested_source, &entry.source_family)
        })
        .ok_or_else(|| {
            ToolError::ExecutionError(format!(
                "library_id `{}` was not found in context_docs registry",
                library_id
            ))
        })
}

fn source_matches(requested_source: &Option<String>, actual_source: &str) -> bool {
    match requested_source.as_deref() {
        Some(expected) => normalized(actual_source) == expected,
        None => true,
    }
}

fn library_match_score(
    requested_library: &str,
    entry: &RegisteredLibrary,
    requested_version: Option<&str>,
) -> i64 {
    let requested = normalized(requested_library);
    if requested.is_empty() {
        return 0;
    }

    let mut score = 0;
    let tokens = tokenize(requested_library);
    let display_name = normalized(&entry.display_name);
    let library_id = normalized(&entry.library_id);

    if requested == library_id {
        score += 1_000;
    }
    if requested == display_name {
        score += 950;
    }
    if library_id.contains(&requested) {
        score += 250;
    }
    if display_name.contains(&requested) {
        score += 225;
    }

    for alias in &entry.aliases {
        let normalized_alias = normalized(alias);
        if normalized_alias == requested {
            score += 900;
        } else if normalized_alias.contains(&requested) || requested.contains(&normalized_alias) {
            score += 200;
        }
        score += token_overlap_score(&tokens, &tokenize(alias), 120);
    }

    score += token_overlap_score(&tokens, &tokenize(&entry.display_name), 100);
    score += token_overlap_score(&tokens, &tokenize(&entry.library_id), 90);

    if let Some(version) = requested_version {
        if entry.default_version.as_deref().map(normalized).as_deref() == Some(version) {
            score += 50;
        }
    }

    score
}

fn docs_page_score(query: &str, page: &DocsPage) -> i64 {
    let normalized_query = normalized(query);
    if normalized_query.is_empty() {
        return 0;
    }

    let query_tokens = tokenize(query);
    let mut score = 0;

    let title = normalized(&page.title);
    let page_id = normalized(&page.page_id);
    let summary = normalized_optional(page.summary.as_deref()).unwrap_or_default();
    let content = normalized(&page.content);

    if title.contains(&normalized_query) {
        score += 500;
    }
    if page_id.contains(&normalized_query) {
        score += 350;
    }
    if summary.contains(&normalized_query) {
        score += 250;
    }
    if content.contains(&normalized_query) {
        score += 100;
    }

    score += token_overlap_score(&query_tokens, &tokenize(&page.title), 120);
    score += token_overlap_score(&query_tokens, &tokenize(&page.page_id), 100);
    score += token_overlap_score(&query_tokens, &tokenize(&page.content), 10);

    for heading in &page.headings {
        let normalized_heading = normalized(heading);
        if normalized_heading.contains(&normalized_query) {
            score += 180;
        }
        score += token_overlap_score(&query_tokens, &tokenize(heading), 80);
    }
    for tag in &page.tags {
        let normalized_tag = normalized(tag);
        if normalized_tag.contains(&normalized_query) {
            score += 140;
        }
        score += token_overlap_score(&query_tokens, &tokenize(tag), 70);
    }

    score
}

fn page_version_matches(
    page: &DocsPage,
    index: &DocsIndex,
    requested_version: Option<&str>,
    entry: &RegisteredLibrary,
) -> bool {
    match requested_version {
        Some(requested) => {
            resolved_page_version(page, index, entry)
                .as_deref()
                .map(normalized)
                .as_deref()
                == Some(requested)
        }
        None => true,
    }
}

fn resolved_page_version(
    page: &DocsPage,
    index: &DocsIndex,
    entry: &RegisteredLibrary,
) -> Option<String> {
    page.version
        .clone()
        .or_else(|| index.version.clone())
        .or_else(|| entry.default_version.clone())
}

fn build_page_snippet(page: &DocsPage, query: &str) -> String {
    if let Some(summary) = page
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return summary.to_string();
    }

    let normalized_query = normalized(query);
    let lower_content = page.content.to_lowercase();
    if let Some(position) = lower_content.find(&normalized_query) {
        let char_start = page.content[..position].chars().count();
        let start = char_start.saturating_sub(80);
        let excerpt = truncate_chars(&page.content.chars().skip(start).collect::<String>(), 220);
        return excerpt.trim().to_string();
    }

    truncate_chars(page.content.trim(), 220)
}

fn normalized(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalized_optional(value: Option<&str>) -> Option<String> {
    value
        .map(normalized)
        .filter(|normalized| !normalized.is_empty())
}

fn tokenize(value: &str) -> Vec<String> {
    normalized(value)
        .split_whitespace()
        .map(|token| token.to_string())
        .collect()
}

fn token_overlap_score(left: &[String], right: &[String], weight: i64) -> i64 {
    if left.is_empty() || right.is_empty() {
        return 0;
    }
    left.iter()
        .filter(|token| right.iter().any(|candidate| candidate == *token))
        .count() as i64
        * weight
}

fn cmp_scored(left_score: i64, left_label: &str, right_score: i64, right_label: &str) -> Ordering {
    left_score
        .cmp(&right_score)
        .then_with(|| left_label.cmp(right_label))
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut iter = value.chars();
    let truncated: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn base_input(operation: ContextDocsOperation) -> ContextDocsInput {
        ContextDocsInput {
            operation,
            library: None,
            library_id: None,
            query: None,
            page_id: None,
            version: None,
            source: None,
            limit: 5,
        }
    }

    fn write_markdown_bundle_registry() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().expect("tempdir");
        let registry_path = dir.path().join("context-docs-registry.json");
        let bundle_dir = dir.path().join("react-router-docs");
        fs::create_dir_all(bundle_dir.join("guides")).expect("bundle dir should create");
        fs::write(
            &registry_path,
            serde_json::json!({
                "libraries": [
                    {
                        "libraryId": "react-router",
                        "displayName": "React Router",
                        "aliases": ["react router", "rr"],
                        "sourceFamily": "official_docs",
                        "defaultVersion": "7",
                        "homepage": "https://reactrouter.com/",
                        "indexPath": "react-router-docs"
                    }
                ]
            })
            .to_string(),
        )
        .expect("registry should write");
        fs::write(
            bundle_dir.join("index.md"),
            "# React Router\n\nReact Router documentation home.\n\n## Overview\n\nRouting for React apps.\n",
        )
        .expect("index markdown should write");
        fs::write(
            bundle_dir.join("guides/loaders.md"),
            "# Data Loading\n\nUse loaders to fetch data before rendering routes. Redirect from a loader when a route should navigate instead of rendering.\n\n## Loader\n\nLoaders run before render.\n\n## Redirect\n\nUse redirect for navigation in a loader.\n",
        )
        .expect("guide markdown should write");
        (dir, registry_path)
    }

    fn write_fixture_registry() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().expect("tempdir");
        let registry_path = dir.path().join("context-docs-registry.json");
        let index_path = dir.path().join("react-router.json");
        fs::write(
            &registry_path,
            serde_json::json!({
                "libraries": [
                    {
                        "libraryId": "react-router",
                        "displayName": "React Router",
                        "aliases": ["react router", "rr"],
                        "sourceFamily": "official_docs",
                        "defaultVersion": "7",
                        "homepage": "https://reactrouter.com/",
                        "indexPath": "react-router.json"
                    },
                    {
                        "libraryId": "tokio",
                        "displayName": "Tokio",
                        "aliases": ["tokio-rs"],
                        "sourceFamily": "library_docs",
                        "defaultVersion": "1.x",
                        "homepage": "https://docs.rs/tokio/latest/tokio/",
                        "indexPath": "tokio.json"
                    }
                ]
            })
            .to_string(),
        )
        .expect("registry should write");
        fs::write(
            &index_path,
            serde_json::json!({
                "libraryId": "react-router",
                "version": "7",
                "pages": [
                    {
                        "pageId": "guides/data-loading",
                        "title": "Data Loading",
                        "url": "https://reactrouter.com/start/data-loading",
                        "summary": "Use loaders to fetch data before rendering routes.",
                        "content": "Loaders let you fetch data before rendering. Use redirect from a loader when a route should navigate instead of rendering.",
                        "headings": ["loader", "redirect"],
                        "tags": ["data", "navigation"]
                    },
                    {
                        "pageId": "api/components/router-provider",
                        "title": "RouterProvider",
                        "url": "https://reactrouter.com/api/components/RouterProvider",
                        "summary": "RouterProvider renders the current route tree.",
                        "content": "RouterProvider provides the routing context and renders matches.",
                        "headings": ["provider", "router"],
                        "tags": ["components"]
                    }
                ]
            })
            .to_string(),
        )
        .expect("index should write");
        fs::write(
            dir.path().join("tokio.json"),
            serde_json::json!({
                "libraryId": "tokio",
                "version": "1.x",
                "pages": [
                    {
                        "pageId": "runtime/builder",
                        "title": "Runtime Builder",
                        "url": "https://docs.rs/tokio/latest/tokio/runtime/struct.Builder.html",
                        "summary": "Configure a Tokio runtime.",
                        "content": "Builder configures the multi-threaded runtime.",
                        "headings": ["builder"],
                        "tags": ["runtime"]
                    }
                ]
            })
            .to_string(),
        )
        .expect("tokio index should write");
        (dir, registry_path)
    }

    fn tool_ctx_with_registry(registry_path: &Path) -> ToolContext {
        ToolContext::new("session-1".into(), "message-1".into(), ".".into())
            .with_tool_runtime_config(crate::ToolRuntimeConfig {
                context_docs_registry_path: Some(registry_path.to_string_lossy().to_string()),
            })
    }

    #[test]
    fn schema_exposes_expected_operations() {
        let tool = ContextDocsTool::new();
        let schema = tool.parameters();
        let ops = schema["properties"]["operation"]["enum"]
            .as_array()
            .expect("enum should exist");
        assert!(ops.iter().any(|v| v == "resolve_library"));
        assert!(ops.iter().any(|v| v == "query_docs"));
        assert!(ops.iter().any(|v| v == "get_page"));
    }

    #[test]
    fn resolve_library_requires_library_name() {
        let input = base_input(ContextDocsOperation::ResolveLibrary);
        match validate_input(&input) {
            Err(ToolError::InvalidArguments(msg)) => assert!(msg.contains("library is required")),
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn query_docs_requires_library_id_and_query() {
        let input = base_input(ContextDocsOperation::QueryDocs);
        match validate_input(&input) {
            Err(ToolError::InvalidArguments(msg)) => {
                assert!(msg.contains("library_id is required"))
            }
            other => panic!("unexpected result: {:?}", other),
        }

        let mut input = base_input(ContextDocsOperation::QueryDocs);
        input.library_id = Some("react-router".to_string());
        match validate_input(&input) {
            Err(ToolError::InvalidArguments(msg)) => assert!(msg.contains("query is required")),
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn get_page_requires_library_id_and_page_id() {
        let mut input = base_input(ContextDocsOperation::GetPage);
        input.page_id = Some("routing/overview".to_string());
        match validate_input(&input) {
            Err(ToolError::InvalidArguments(msg)) => {
                assert!(msg.contains("library_id is required"))
            }
            other => panic!("unexpected result: {:?}", other),
        }

        let mut input = base_input(ContextDocsOperation::GetPage);
        input.library_id = Some("react-router".to_string());
        match validate_input(&input) {
            Err(ToolError::InvalidArguments(msg)) => assert!(msg.contains("page_id is required")),
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn limit_must_stay_within_bounds() {
        let mut input = base_input(ContextDocsOperation::ResolveLibrary);
        input.library = Some("react-router".to_string());
        input.limit = 0;
        assert!(validate_input(&input).is_err());
        input.limit = 21;
        assert!(validate_input(&input).is_err());
        input.limit = 5;
        assert!(validate_input(&input).is_ok());
    }

    #[tokio::test]
    async fn resolve_library_reads_registry_fixture() {
        let (_dir, registry_path) = write_fixture_registry();
        let tool = ContextDocsTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "operation": "resolve_library",
                    "library": "react router"
                }),
                tool_ctx_with_registry(&registry_path),
            )
            .await
            .expect("resolve_library should succeed");

        assert!(result.output.contains("React Router"));
        assert_eq!(
            result.metadata["matches"][0]["library_id"],
            serde_json::json!("react-router")
        );
    }

    #[tokio::test]
    async fn query_docs_reads_index_fixture() {
        let (_dir, registry_path) = write_fixture_registry();
        let tool = ContextDocsTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "operation": "query_docs",
                    "library_id": "react-router",
                    "query": "loader redirect"
                }),
                tool_ctx_with_registry(&registry_path),
            )
            .await
            .expect("query_docs should succeed");

        assert!(result.output.contains("Data Loading"));
        assert_eq!(
            result.metadata["results"][0]["page_id"],
            serde_json::json!("guides/data-loading")
        );
    }

    #[tokio::test]
    async fn get_page_reads_full_page_fixture() {
        let (_dir, registry_path) = write_fixture_registry();
        let tool = ContextDocsTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "operation": "get_page",
                    "library_id": "react-router",
                    "page_id": "guides/data-loading"
                }),
                tool_ctx_with_registry(&registry_path),
            )
            .await
            .expect("get_page should succeed");

        assert!(result.output.contains("Use redirect from a loader"));
        assert_eq!(
            result.metadata["page"]["title"],
            serde_json::json!("Data Loading")
        );
    }

    #[tokio::test]
    async fn query_docs_reads_markdown_bundle_fixture() {
        let (_dir, registry_path) = write_markdown_bundle_registry();
        let tool = ContextDocsTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "operation": "query_docs",
                    "library_id": "react-router",
                    "query": "loader redirect"
                }),
                tool_ctx_with_registry(&registry_path),
            )
            .await
            .expect("query_docs should succeed for markdown bundle");

        assert_eq!(
            result.metadata["backend"],
            serde_json::json!("markdown_bundle")
        );
        assert_eq!(
            result.metadata["results"][0]["page_id"],
            serde_json::json!("guides/loaders")
        );
        assert!(result.output.contains("Data Loading"));
    }

    #[tokio::test]
    async fn get_page_reads_markdown_bundle_fixture() {
        let (_dir, registry_path) = write_markdown_bundle_registry();
        let tool = ContextDocsTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "operation": "get_page",
                    "library_id": "react-router",
                    "page_id": "guides/loaders"
                }),
                tool_ctx_with_registry(&registry_path),
            )
            .await
            .expect("get_page should succeed for markdown bundle");

        assert_eq!(
            result.metadata["backend"],
            serde_json::json!("markdown_bundle")
        );
        assert_eq!(result.metadata["page"]["version"], serde_json::json!("7"));
        assert!(result
            .output
            .contains("Use redirect for navigation in a loader."));
    }

    #[tokio::test]
    async fn execute_requires_registry_path_in_context() {
        let tool = ContextDocsTool::new();
        let error = tool
            .execute(
                serde_json::json!({
                    "operation": "resolve_library",
                    "library": "react router"
                }),
                ToolContext::new("session-1".into(), "message-1".into(), ".".into()),
            )
            .await
            .expect_err("missing registry path should fail");

        match error {
            ToolError::ExecutionError(message) => {
                assert!(message.contains("docs.contextDocsRegistryPath"))
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validate_registry_file_summarizes_fixture() {
        let (_dir, registry_path) = write_fixture_registry();
        let summary =
            validate_registry_file(&registry_path).expect("fixture registry should validate");

        assert!(summary.valid);
        assert_eq!(summary.library_count, 2);
        assert_eq!(summary.libraries[0].library_id, "react-router");
        assert_eq!(summary.libraries[0].page_count, 2);
    }

    #[test]
    fn validate_registry_file_supports_markdown_bundle() {
        let (_dir, registry_path) = write_markdown_bundle_registry();
        let summary = validate_registry_file(&registry_path)
            .expect("markdown bundle registry should validate");

        assert!(summary.valid);
        assert_eq!(summary.library_count, 1);
        assert_eq!(summary.libraries[0].library_id, "react-router");
        assert_eq!(summary.libraries[0].page_count, 2);
        assert!(summary.libraries[0]
            .resolved_index_path
            .ends_with("react-router-docs"));
    }

    #[test]
    fn validate_docs_index_file_rejects_duplicate_page_ids() {
        let dir = tempfile::tempdir().expect("tempdir should create");
        let index_path = dir.path().join("duplicate.json");
        fs::write(
            &index_path,
            serde_json::json!({
                "libraryId": "demo",
                "pages": [
                    {
                        "pageId": "dup",
                        "title": "Page One",
                        "url": "https://example.com/one",
                        "content": "content"
                    },
                    {
                        "pageId": "dup",
                        "title": "Page Two",
                        "url": "https://example.com/two",
                        "content": "content"
                    }
                ]
            })
            .to_string(),
        )
        .expect("index should write");

        let error =
            validate_docs_index_file(&index_path).expect_err("duplicate page ids should fail");
        match error {
            ToolError::ExecutionError(message) => {
                assert!(message.contains("duplicate page_id"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validate_checked_in_context_docs_examples() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let registry_path =
            repo_root.join("docs/examples/context_docs/context-docs-registry.example.json");
        let index_path =
            repo_root.join("docs/examples/context_docs/react-router.docs-index.example.json");

        let registry_summary = validate_registry_file(&registry_path)
            .expect("checked-in registry example should validate");
        let index_summary = validate_docs_index_file(&index_path)
            .expect("checked-in index example should validate");

        assert!(registry_summary.valid);
        assert!(index_summary.valid);
        assert_eq!(index_summary.library_id.as_deref(), Some("react-router"));
    }
}
