use ast_grep_core::Pattern as AstPattern;
use ast_grep_language::{LanguageExt, SupportLang};
use async_trait::async_trait;
use glob::Pattern;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

use crate::ast_grep_common::{
    compile_pattern, count_placeholders, display_path, resolve_directory_root, should_visit,
    AstGrepLanguage, DEFAULT_GLOB, DEFAULT_MAX_RESULTS, MAX_CONTEXT_LINES, MAX_RESULTS_LIMIT,
    SUPPORTED_LANGUAGES,
};
use crate::{
    assert_external_directory, ExternalDirectoryKind, ExternalDirectoryOptions, Metadata,
    PermissionRequest, Tool, ToolContext, ToolError, ToolResult,
};

const DESCRIPTION: &str = r#"Structural code search using the ast-grep engine.

Phase 1 supports Rust syntax only. Use this tool when plain text grep is not precise enough and you need to find code by syntactic shape rather than substring matching. Typical use cases:
- Find function calls with a specific argument structure
- Find control-flow constructs with a specific shape
- Find repeated code patterns before refactoring
- Audit structurally similar implementations across files

This tool is read-only. It does not modify files."#;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AstGrepSearchInput {
    pattern: String,
    language: AstGrepLanguage,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    glob: Option<String>,
    #[serde(default = "default_max_results", alias = "max_results")]
    max_results: usize,
    #[serde(default, alias = "context_lines")]
    context_lines: usize,
}

fn default_max_results() -> usize {
    DEFAULT_MAX_RESULTS
}

#[derive(Debug, Clone)]
struct CompiledPattern {
    pattern: AstPattern,
    normalized_pattern: String,
    placeholder_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct AstMatch {
    file: String,
    line: usize,
    column: usize,
    end_line: usize,
    end_column: usize,
    kind: String,
    snippet: String,
    matched: String,
}

#[derive(Debug, Clone)]
struct SearchOutcome {
    matches: Vec<AstMatch>,
    truncated: bool,
}

pub struct AstGrepSearchTool;

impl AstGrepSearchTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AstGrepSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for AstGrepSearchTool {
    fn id(&self) -> &str {
        "ast_grep_search"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "AST pattern to search for"
                },
                "language": {
                    "type": "string",
                    "enum": SUPPORTED_LANGUAGES,
                    "description": "Language hint for the parser (Phase 1 currently supports rust only)"
                },
                "path": {
                    "type": "string",
                    "description": "Optional root path to scope the search. Defaults to current session directory."
                },
                "glob": {
                    "type": "string",
                    "description": "Optional file glob filter such as '**/*.rs'"
                },
                "maxResults": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 500,
                    "default": 100,
                    "description": "Maximum number of matches to return"
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 500,
                    "default": 100,
                    "description": "Maximum number of matches to return (snake_case alias)"
                },
                "contextLines": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 20,
                    "default": 0,
                    "description": "Surrounding context lines to include per match"
                },
                "context_lines": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 20,
                    "default": 0,
                    "description": "Surrounding context lines to include per match (snake_case alias)"
                }
            },
            "required": ["pattern", "language"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: AstGrepSearchInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;

        validate_input(&input)?;

        let search_root = resolve_directory_root(&ctx, input.path.as_deref())?;
        assert_external_directory(
            &ctx,
            Some(&search_root.to_string_lossy()),
            ExternalDirectoryOptions {
                bypass: false,
                kind: ExternalDirectoryKind::Directory,
            },
        )
        .await?;

        ctx.ask_permission(
            PermissionRequest::new("ast_grep_search")
                .with_pattern(&input.pattern)
                .with_metadata("language", serde_json::json!(input.language.as_str()))
                .with_metadata("path", serde_json::json!(search_root))
                .always_allow(),
        )
        .await?;

        let compiled = CompiledPattern {
            pattern: compile_pattern(&input.pattern, &input.language)?,
            normalized_pattern: input.pattern.trim().to_string(),
            placeholder_count: count_placeholders(&input.pattern),
        };
        let glob_pattern = Pattern::new(input.glob.as_deref().unwrap_or(DEFAULT_GLOB))
            .map_err(|e| ToolError::InvalidArguments(format!("Invalid glob pattern: {}", e)))?;

        let outcome = search_rust_files(
            &search_root,
            &glob_pattern,
            &compiled,
            input.max_results,
            input.context_lines,
        )?;

        let output = render_matches(
            &compiled,
            &outcome.matches,
            input.max_results,
            outcome.truncated,
        );

        let mut metadata = Metadata::new();
        metadata.insert("pattern".to_string(), serde_json::json!(input.pattern));
        metadata.insert(
            "normalized_pattern".to_string(),
            serde_json::json!(compiled.normalized_pattern),
        );
        metadata.insert(
            "placeholder_count".to_string(),
            serde_json::json!(compiled.placeholder_count),
        );
        metadata.insert(
            "language".to_string(),
            serde_json::json!(input.language.as_str()),
        );
        metadata.insert("engine".to_string(), serde_json::json!("ast-grep"));
        metadata.insert(
            "path".to_string(),
            serde_json::json!(search_root.to_string_lossy().to_string()),
        );
        metadata.insert("glob".to_string(), serde_json::json!(input.glob));
        metadata.insert(
            "count".to_string(),
            serde_json::json!(outcome.matches.len()),
        );
        metadata.insert(
            "truncated".to_string(),
            serde_json::json!(outcome.truncated),
        );
        metadata.insert(
            "matches".to_string(),
            serde_json::to_value(&outcome.matches).unwrap_or_else(|_| serde_json::json!([])),
        );
        metadata.insert("implemented".to_string(), serde_json::json!(true));

        Ok(ToolResult {
            title: format!("ast_grep_search '{}'", compiled.normalized_pattern),
            output,
            metadata,
            truncated: outcome.truncated,
        })
    }
}

fn validate_input(input: &AstGrepSearchInput) -> Result<(), ToolError> {
    if input.pattern.trim().is_empty() {
        return Err(ToolError::InvalidArguments(
            "pattern cannot be empty".to_string(),
        ));
    }
    if input.max_results == 0 || input.max_results > MAX_RESULTS_LIMIT {
        return Err(ToolError::InvalidArguments(format!(
            "maxResults must be between 1 and {}",
            MAX_RESULTS_LIMIT
        )));
    }
    if input.context_lines > MAX_CONTEXT_LINES {
        return Err(ToolError::InvalidArguments(format!(
            "contextLines must be between 0 and {}",
            MAX_CONTEXT_LINES
        )));
    }
    Ok(())
}

fn search_rust_files(
    base_dir: &Path,
    glob_pattern: &Pattern,
    compiled: &CompiledPattern,
    max_results: usize,
    context_lines: usize,
) -> Result<SearchOutcome, ToolError> {
    let mut matches = Vec::new();
    let mut truncated = false;

    for entry in WalkDir::new(base_dir)
        .follow_links(true)
        .into_iter()
        .filter_entry(|entry| should_visit(entry, base_dir))
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };

        if matches.len() >= max_results {
            truncated = true;
            break;
        }

        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let rel = path.strip_prefix(base_dir).unwrap_or(path);
        if !glob_pattern.matches_path(rel) {
            continue;
        }

        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(_) => continue,
        };

        let root = SupportLang::Rust.ast_grep(&source);
        for matched in root.root().find_all(&compiled.pattern) {
            matches.push(node_to_match(base_dir, path, &matched, context_lines));
            if matches.len() >= max_results {
                truncated = true;
                break;
            }
        }
    }

    Ok(SearchOutcome { matches, truncated })
}

fn node_to_match(
    base_dir: &Path,
    path: &Path,
    node: &ast_grep_core::NodeMatch<'_, ast_grep_core::tree_sitter::StrDoc<SupportLang>>,
    context_lines: usize,
) -> AstMatch {
    let start = node.start_pos();
    let end = node.end_pos();
    let display = node
        .get_node()
        .display_context(context_lines, context_lines);
    let snippet = format!("{}{}{}", display.leading, display.matched, display.trailing)
        .trim_end_matches('\n')
        .to_string();

    AstMatch {
        file: display_path(base_dir, path),
        line: start.line() + 1,
        column: start.column(node.get_node()) + 1,
        end_line: end.line() + 1,
        end_column: end.column(node.get_node()) + 1,
        kind: node.kind().into_owned(),
        snippet,
        matched: node.text().to_string(),
    }
}

fn render_matches(
    compiled: &CompiledPattern,
    matches: &[AstMatch],
    max_results: usize,
    truncated: bool,
) -> String {
    if matches.is_empty() {
        return format!(
            "No matches found for Rust ast-grep pattern: {}",
            compiled.normalized_pattern
        );
    }

    let mut lines = vec![format!(
        "Found {} Rust AST matches for pattern '{}'{}",
        matches.len(),
        compiled.normalized_pattern,
        if truncated {
            format!(" (showing first {})", max_results)
        } else {
            String::new()
        }
    )];

    for (idx, m) in matches.iter().enumerate() {
        lines.push(String::new());
        lines.push(format!("{}. {}:{}:{}", idx + 1, m.file, m.line, m.column));
        lines.push(format!("   kind: {}", m.kind));
        lines.push("   snippet:".to_string());
        for line in m.snippet.lines() {
            lines.push(format!("   {}", line));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn tool_id_and_required_fields_are_stable() {
        let tool = AstGrepSearchTool::new();
        assert_eq!(tool.id(), "ast_grep_search");

        let schema = tool.parameters();
        let required = schema["required"]
            .as_array()
            .expect("required should be an array");
        assert!(required.iter().any(|v| v == "pattern"));
        assert!(required.iter().any(|v| v == "language"));
    }

    #[test]
    fn schema_exposes_rust_only_phase_one_language_and_aliases() {
        let tool = AstGrepSearchTool::new();
        let schema = tool.parameters();
        let language_enum = schema["properties"]["language"]["enum"]
            .as_array()
            .expect("language enum should be an array");

        assert_eq!(language_enum.len(), 1);
        assert!(language_enum.iter().any(|v| v == "rust"));
        assert!(schema["properties"].get("maxResults").is_some());
        assert!(schema["properties"].get("max_results").is_some());
        assert!(schema["properties"].get("contextLines").is_some());
        assert!(schema["properties"].get("context_lines").is_some());
    }

    #[test]
    fn compile_pattern_accepts_placeholders() {
        let compiled = CompiledPattern {
            pattern: compile_pattern("foo($A)", &AstGrepLanguage::Rust)
                .expect("pattern should compile"),
            normalized_pattern: "foo($A)".to_string(),
            placeholder_count: count_placeholders("foo($A)"),
        };
        assert_eq!(compiled.placeholder_count, 1);
        assert_eq!(compiled.normalized_pattern, "foo($A)");
    }

    #[tokio::test]
    async fn execute_finds_rust_expression_matches() {
        let dir = tempdir().expect("tempdir should exist");
        let file = dir.path().join("sample.rs");
        fs::write(
            &file,
            r#"
fn demo() {
    foo(bar);
    foo(baz(1));
    qux();
}
"#,
        )
        .expect("fixture should write");

        let tool = AstGrepSearchTool::new();
        let ctx = ToolContext::new(
            "session".to_string(),
            "message".to_string(),
            dir.path().to_string_lossy().to_string(),
        );

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "foo($X)",
                    "language": "rust",
                    "maxResults": 10
                }),
                ctx,
            )
            .await
            .expect("search should succeed");

        assert!(result.output.contains("Found 2 Rust AST matches"));
        assert!(result.output.contains("sample.rs:3"));
        assert!(result.output.contains("sample.rs:4"));
        assert_eq!(
            result.metadata.get("implemented"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(result.metadata.get("count"), Some(&serde_json::json!(2)));
        assert_eq!(
            result.metadata.get("engine"),
            Some(&serde_json::json!("ast-grep"))
        );
    }
}
