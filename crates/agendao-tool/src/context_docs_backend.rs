use std::fs;
use std::path::Path;
use std::time::Duration;

use reqwest::blocking::Client;
use walkdir::WalkDir;

use crate::context_docs::{
    load_docs_index_from_path, resolve_registry_index_path, validate_docs_index_summary,
    ContextDocsIndexValidationSummary, DocsIndex, DocsPage, RegisteredLibrary,
};
use crate::ToolError;

const REMOTE_DOCS_INDEX_TIMEOUT_SECS: u64 = 20;
const MAX_REMOTE_DOCS_INDEX_BYTES: usize = 2 * 1024 * 1024;
const DOCS_FETCH_USER_AGENT: &str = "AgenDao context_docs/2026.3.4";

#[derive(Debug, Clone, Copy)]
pub(crate) enum ContextDocsBackendKind {
    DocsIndex,
    MarkdownBundle,
    RemoteDocsIndex,
}

impl ContextDocsBackendKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::DocsIndex => "docs_index",
            Self::MarkdownBundle => "markdown_bundle",
            Self::RemoteDocsIndex => "remote_docs_index",
        }
    }
}

pub(crate) fn load_registered_docs_source(
    registry_path: &Path,
    entry: &RegisteredLibrary,
) -> Result<
    (
        DocsIndex,
        ContextDocsIndexValidationSummary,
        ContextDocsBackendKind,
    ),
    ToolError,
> {
    if is_remote_docs_index(&entry.index_path) {
        return load_remote_docs_index(&entry.index_path, &entry.library_id)
            .map(|(index, summary)| (index, summary, ContextDocsBackendKind::RemoteDocsIndex));
    }

    let source_path = resolve_registry_index_path(registry_path, &entry.index_path);
    if source_path.is_dir() {
        load_markdown_bundle_from_dir(&source_path, entry)
    } else {
        load_docs_index_from_path(&source_path, Some(&entry.library_id))
            .map(|(index, summary)| (index, summary, ContextDocsBackendKind::DocsIndex))
    }
}

pub(crate) fn resolve_registered_docs_source_display(
    registry_path: &Path,
    entry: &RegisteredLibrary,
) -> String {
    if is_remote_docs_index(&entry.index_path) {
        entry.index_path.clone()
    } else {
        resolve_registry_index_path(registry_path, &entry.index_path)
            .display()
            .to_string()
    }
}

fn load_remote_docs_index(
    url: &str,
    expected_library_id: &str,
) -> Result<(DocsIndex, ContextDocsIndexValidationSummary), ToolError> {
    let client = Client::builder()
        .user_agent(DOCS_FETCH_USER_AGENT)
        .timeout(Duration::from_secs(REMOTE_DOCS_INDEX_TIMEOUT_SECS))
        .build()
        .map_err(|err| {
            ToolError::ExecutionError(format!("failed to build HTTP client: {}", err))
        })?;

    let response = client.get(url).send().map_err(|err| {
        ToolError::ExecutionError(format!("failed to fetch docs index `{}`: {}", url, err))
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(ToolError::ExecutionError(format!(
            "failed to fetch docs index `{}`: HTTP {}",
            url, status
        )));
    }

    let bytes = response.bytes().map_err(|err| {
        ToolError::ExecutionError(format!(
            "failed to read docs index response `{}`: {}",
            url, err
        ))
    })?;

    parse_remote_docs_index_bytes(url, &bytes, expected_library_id)
}

fn parse_remote_docs_index_bytes(
    source_label: &str,
    bytes: &[u8],
    expected_library_id: &str,
) -> Result<(DocsIndex, ContextDocsIndexValidationSummary), ToolError> {
    if bytes.len() > MAX_REMOTE_DOCS_INDEX_BYTES {
        return Err(ToolError::ExecutionError(format!(
            "docs index `{}` exceeded {} bytes",
            source_label, MAX_REMOTE_DOCS_INDEX_BYTES
        )));
    }

    let index: DocsIndex = serde_json::from_slice(bytes).map_err(|err| {
        ToolError::ExecutionError(format!(
            "failed to parse docs index `{}`: {}",
            source_label, err
        ))
    })?;
    let summary =
        validate_docs_index_summary(Path::new(source_label), &index, Some(expected_library_id))?;
    Ok((index, summary))
}

fn is_remote_docs_index(index_path: &str) -> bool {
    index_path.starts_with("http://") || index_path.starts_with("https://")
}

fn load_markdown_bundle_from_dir(
    dir: &Path,
    entry: &RegisteredLibrary,
) -> Result<
    (
        DocsIndex,
        ContextDocsIndexValidationSummary,
        ContextDocsBackendKind,
    ),
    ToolError,
> {
    let mut pages = Vec::new();
    for item in WalkDir::new(dir).follow_links(true) {
        let item = match item {
            Ok(item) => item,
            Err(_) => continue,
        };
        let path = item.path();
        if !path.is_file() || !is_markdown_file(path) {
            continue;
        }
        pages.push(markdown_file_to_page(dir, path, entry)?);
    }

    pages.sort_by(|left, right| left.page_id.cmp(&right.page_id));

    let index = DocsIndex {
        library_id: Some(entry.library_id.clone()),
        version: entry.default_version.clone(),
        pages,
    };
    let summary = validate_docs_index_summary(dir, &index, Some(&entry.library_id))?;
    Ok((index, summary, ContextDocsBackendKind::MarkdownBundle))
}

fn markdown_file_to_page(
    bundle_root: &Path,
    path: &Path,
    entry: &RegisteredLibrary,
) -> Result<DocsPage, ToolError> {
    let content = fs::read_to_string(path).map_err(|err| {
        ToolError::ExecutionError(format!(
            "failed to read markdown docs page `{}`: {}",
            path.display(),
            err
        ))
    })?;
    let rel = path.strip_prefix(bundle_root).unwrap_or(path);
    let page_id = relative_markdown_page_id(rel);
    let headings = extract_markdown_headings(&content);
    let title = headings
        .first()
        .cloned()
        .unwrap_or_else(|| fallback_title(rel));
    let summary = extract_markdown_summary(&content);
    let url = build_page_url(entry, &page_id, rel);

    Ok(DocsPage {
        page_id,
        title,
        url,
        content,
        summary,
        version: entry.default_version.clone(),
        headings,
        tags: Vec::new(),
    })
}

fn is_markdown_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("md") | Some("mdx")
    )
}

fn relative_markdown_page_id(rel: &Path) -> String {
    let mut raw = rel.to_string_lossy().replace('\\', "/");
    if raw.ends_with(".md") {
        raw.truncate(raw.len() - 3);
    } else if raw.ends_with(".mdx") {
        raw.truncate(raw.len() - 4);
    }
    if raw == "index" {
        return "index".to_string();
    }
    if let Some(stripped) = raw.strip_suffix("/index") {
        if stripped.is_empty() {
            "index".to_string()
        } else {
            stripped.to_string()
        }
    } else {
        raw
    }
}

fn fallback_title(rel: &Path) -> String {
    let stem = rel
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("index")
        .replace(['-', '_'], " ");
    stem.split_whitespace()
        .map(capitalize)
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalize(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn extract_markdown_headings(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with('#') {
                return None;
            }
            let text = trimmed.trim_start_matches('#').trim();
            if text.is_empty() {
                None
            } else {
                Some(text.to_string())
            }
        })
        .collect()
}

fn extract_markdown_summary(content: &str) -> Option<String> {
    let mut lines = Vec::new();
    let mut in_code_block = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block || trimmed.is_empty() || trimmed.starts_with('#') {
            if !lines.is_empty() {
                break;
            }
            continue;
        }
        lines.push(trimmed.to_string());
        if lines.len() >= 3 {
            break;
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(truncate_chars(&lines.join(" "), 220))
    }
}

fn build_page_url(entry: &RegisteredLibrary, page_id: &str, rel: &Path) -> String {
    if let Some(homepage) = entry
        .homepage
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let base = homepage.trim_end_matches('/');
        if page_id == "index" {
            homepage.to_string()
        } else {
            format!("{}/{}", base, page_id)
        }
    } else {
        format!("file://{}", rel.to_string_lossy())
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_remote_docs_index_bytes_validates_payload() {
        let bytes = serde_json::json!({
            "libraryId": "react-router",
            "version": "7",
            "pages": [
                {
                    "pageId": "guides/data-loading",
                    "title": "Data Loading",
                    "url": "https://reactrouter.com/start/data-loading",
                    "content": "Loaders let you fetch data before rendering routes."
                }
            ]
        })
        .to_string()
        .into_bytes();

        let (index, summary) = parse_remote_docs_index_bytes(
            "https://example.com/react-router.json",
            &bytes,
            "react-router",
        )
        .expect("remote docs payload should parse");

        assert_eq!(summary.library_id.as_deref(), Some("react-router"));
        assert_eq!(summary.page_count, 1);
        assert_eq!(index.pages[0].page_id, "guides/data-loading");
    }

    #[test]
    fn resolve_registered_docs_source_display_keeps_remote_url() {
        let entry = RegisteredLibrary {
            library_id: "react-router".to_string(),
            display_name: "React Router".to_string(),
            aliases: vec!["react router".to_string()],
            source_family: "official_docs".to_string(),
            default_version: Some("7".to_string()),
            homepage: Some("https://reactrouter.com/".to_string()),
            index_path: "https://example.com/react-router.json".to_string(),
        };

        let resolved = resolve_registered_docs_source_display(
            Path::new("/tmp/context-docs-registry.json"),
            &entry,
        );

        assert_eq!(resolved, "https://example.com/react-router.json");
    }
}
