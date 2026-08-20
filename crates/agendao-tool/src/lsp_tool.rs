use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use lsp_types;

use crate::{Metadata, Tool, ToolContext, ToolError, ToolResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LspParams {
    pub operation: LspOperation,
    pub file_path: String,
    pub line: Option<u32>,
    pub character: Option<u32>,
    pub query: Option<String>,
    pub new_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LspOperation {
    GoToDefinition,
    FindReferences,
    Hover,
    DocumentSymbol,
    WorkspaceSymbol,
    GoToImplementation,
    TypeDefinition,
    Rename,
    Diagnostics,
    PrepareCallHierarchy,
    IncomingCalls,
    OutgoingCalls,
}

pub struct LspTool;

#[async_trait]
impl Tool for LspTool {
    fn id(&self) -> &str {
        "lsp"
    }

    fn description(&self) -> &str {
        "Language Server Protocol operations for code navigation and analysis. Supports goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, goToImplementation, typeDefinition, rename, diagnostics, prepareCallHierarchy, incomingCalls, and outgoingCalls."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["goToDefinition", "findReferences", "hover", "documentSymbol", "workspaceSymbol", "goToImplementation", "typeDefinition", "rename", "diagnostics", "prepareCallHierarchy", "incomingCalls", "outgoingCalls"],
                    "description": "The LSP operation to perform"
                },
                "filePath": {
                    "type": "string",
                    "description": "The absolute or relative path to the file"
                },
                "line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "The line number (1-based, as shown in editors)"
                },
                "character": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "The character offset (1-based, as shown in editors)"
                },
                "query": {
                    "type": "string",
                    "description": "Query string for workspaceSymbol operation"
                },
                "newName": {
                    "type": "string",
                    "description": "New name for rename operation"
                }
            },
            "required": ["operation", "filePath"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let params: LspParams = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArguments(format!("Invalid parameters: {}", e)))?;

        let path = PathBuf::from(&params.file_path);
        let path_str = path.to_string_lossy().to_string();

        if !path.exists() {
            return Err(ToolError::FileNotFound(format!(
                "File not found: {}",
                params.file_path
            )));
        }

        if ctx.is_external_path(&path_str) {
            ctx.ask_permission(
                crate::PermissionRequest::new("external_directory")
                    .with_pattern(&path_str)
                    .with_scope_key(crate::external_fs_scope_key(&path_str))
                    .with_metadata("filepath", serde_json::json!(&path_str)),
            )
            .await?;
        }

        ctx.ask_permission(
            crate::PermissionRequest::new("lsp")
                .with_scope_key(crate::workspace_scope_key(&ctx.project_root, &path_str))
                .with_patterns(vec!["*".to_string()])
                .always_allow(),
        )
        .await?;

        let line = params.line.map(|l| l.saturating_sub(1)).unwrap_or(0);
        let character = params.character.map(|c| c.saturating_sub(1)).unwrap_or(0);

        execute_with_lsp(&params, &path, line, character, &ctx).await
    }
}

async fn execute_with_lsp(
    params: &LspParams,
    path: &Path,
    line: u32,
    character: u32,
    ctx: &ToolContext,
) -> Result<ToolResult, ToolError> {
    use agendao_lsp::detect_language;

    let registry = ctx.lsp_registry.as_ref().ok_or_else(|| {
        ToolError::ExecutionError("LSP registry is not available in this runtime".to_string())
    })?;
    if !registry.has_clients(path).await {
        return Err(ToolError::ExecutionError(
            "No LSP server available for this file type.".to_string(),
        ));
    }

    // A successful result proves that a configured client executed the requested
    // LSP operation. Compile-time feature availability alone proves nothing.
    registry
        .touch_file(path, true)
        .await
        .map_err(|e| ToolError::ExecutionError(format!("Failed to touch file in LSP: {e}")))?;

    let language = detect_language(path);
    let clients = registry.list().await;
    let client = clients
        .iter()
        .find(|(id, _)| id.contains(language))
        .map(|(_, client)| client.clone())
        .ok_or_else(|| {
            ToolError::ExecutionError(format!("No LSP client available for language: {language}"))
        })?;

    let output = match &params.operation {
        LspOperation::GoToDefinition => match client.goto_definition(path, line, character).await {
            Ok(Some(loc)) => format_location_result("Definition", loc),
            Ok(None) => "No definition found.".to_string(),
            Err(e) => return Err(lsp_error(e)),
        },
        LspOperation::FindReferences => match client.references(path, line, character).await {
            Ok(locs) if !locs.is_empty() => locs
                .iter()
                .map(format_location)
                .collect::<Vec<_>>()
                .join("\n"),
            Ok(_) => "No references found.".to_string(),
            Err(e) => return Err(lsp_error(e)),
        },
        LspOperation::Hover => match client.hover(path, line, character).await {
            Ok(Some(hover)) => format_hover_result(hover),
            Ok(None) => "No hover information available.".to_string(),
            Err(e) => return Err(lsp_error(e)),
        },
        LspOperation::DocumentSymbol => match client.document_symbol(path).await {
            Ok(symbols) if !symbols.is_empty() => symbols
                .iter()
                .map(|s| format!("{} ({:?})", s.name, s.kind))
                .collect::<Vec<_>>()
                .join("\n"),
            Ok(_) => "No document symbols found.".to_string(),
            Err(e) => return Err(lsp_error(e)),
        },
        LspOperation::WorkspaceSymbol => {
            let query = params.query.as_deref().unwrap_or("");
            match client.workspace_symbol(query).await {
                Ok(symbols) if !symbols.is_empty() => symbols
                    .iter()
                    .map(|s| format!("{} ({:?})", s.name, s.kind))
                    .collect::<Vec<_>>()
                    .join("\n"),
                Ok(_) => "No workspace symbols found.".to_string(),
                Err(e) => return Err(lsp_error(e)),
            }
        }
        LspOperation::GoToImplementation => {
            match client.goto_implementation(path, line, character).await {
                Ok(locs) if !locs.is_empty() => locs
                    .iter()
                    .map(format_location)
                    .collect::<Vec<_>>()
                    .join("\n"),
                Ok(_) => "No implementations found.".to_string(),
                Err(e) => return Err(lsp_error(e)),
            }
        }
        LspOperation::TypeDefinition => match client.type_definition(path, line, character).await {
            Ok(locs) if !locs.is_empty() => locs
                .iter()
                .map(format_location)
                .collect::<Vec<_>>()
                .join("\n"),
            Ok(_) => "No type definitions found.".to_string(),
            Err(e) => return Err(lsp_error(e)),
        },
        LspOperation::Rename => {
            let new_name = params.new_name.as_deref().unwrap_or("new_name");
            match client.rename(path, line, character, new_name).await {
                Ok(Some(_)) => format!(
                    "Rename preview available. Workspace edit ready for: {}",
                    new_name
                ),
                Ok(None) => "Cannot rename symbol at this location.".to_string(),
                Err(e) => return Err(lsp_error(e)),
            }
        }
        LspOperation::Diagnostics => {
            let diags = client.get_diagnostics(path).await;
            if diags.is_empty() {
                "No diagnostics available.".to_string()
            } else {
                diags
                    .iter()
                    .map(|d| format!("{:?}: {}", d.severity, d.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        LspOperation::PrepareCallHierarchy => {
            match client.prepare_call_hierarchy(path, line, character).await {
                Ok(items) if !items.is_empty() => items
                    .iter()
                    .map(|item| {
                        format!(
                            "{} ({:?}) - {}:{}",
                            item.name,
                            item.kind,
                            *item.uri,
                            item.range.start.line + 1
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                Ok(_) => "No call hierarchy items found at this location.".to_string(),
                Err(e) => return Err(lsp_error(e)),
            }
        }
        LspOperation::IncomingCalls => match client.incoming_calls(path, line, character).await {
            Ok(calls) if !calls.is_empty() => calls
                .iter()
                .map(|call| {
                    let from = &call.from;
                    format!(
                        "{} ({:?}) calls from {}:{}",
                        from.name,
                        from.kind,
                        *from.uri,
                        from.range.start.line + 1
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Ok(_) => "No incoming calls found.".to_string(),
            Err(e) => return Err(lsp_error(e)),
        },
        LspOperation::OutgoingCalls => match client.outgoing_calls(path, line, character).await {
            Ok(calls) if !calls.is_empty() => calls
                .iter()
                .map(|call| {
                    let to = &call.to;
                    format!(
                        "{} ({:?}) calls to {}:{}",
                        to.name,
                        to.kind,
                        *to.uri,
                        to.range.start.line + 1
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Ok(_) => "No outgoing calls found.".to_string(),
            Err(e) => return Err(lsp_error(e)),
        },
    };

    let mut metadata = Metadata::new();
    metadata.insert("operation".to_string(), serde_json::json!(params.operation));
    metadata.insert("file_path".to_string(), serde_json::json!(params.file_path));

    Ok(ToolResult {
        output,
        title: format!("LSP: {:?} {}", params.operation, params.file_path),
        metadata,
        truncated: false,
    })
}

fn format_location(loc: &lsp_types::Location) -> String {
    let path = loc.uri.to_string();
    let line = loc.range.start.line + 1;
    let character = loc.range.start.character + 1;
    format!("{}:{}:{}", path, line, character)
}

fn lsp_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::ExecutionError(format!("LSP error: {error}"))
}

fn format_location_result(label: &str, loc: lsp_types::Location) -> String {
    format!("{} found at:\n{}", label, format_location(&loc))
}

fn format_hover_result(hover: lsp_types::Hover) -> String {
    match hover.contents {
        lsp_types::HoverContents::Scalar(markup) => format_markup(markup),
        lsp_types::HoverContents::Array(markups) => markups
            .into_iter()
            .map(format_markup)
            .collect::<Vec<_>>()
            .join("\n"),
        lsp_types::HoverContents::Markup(content) => content.value,
    }
}

fn format_markup(markup: lsp_types::MarkedString) -> String {
    match markup {
        lsp_types::MarkedString::String(s) => s,
        lsp_types::MarkedString::LanguageString(ls) => {
            format!("```{}\n{}\n```", ls.language, ls.value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LspParams, LspTool};
    use crate::{Tool, ToolContext, ToolError};

    #[test]
    fn lsp_rejects_filepath_alias() {
        let error = serde_json::from_value::<LspParams>(serde_json::json!({
            "operation": "hover",
            "filepath": "src/main.rs"
        }))
        .expect_err("filepath alias should be rejected");
        assert!(error.to_string().contains("unknown field"));
    }

    #[tokio::test]
    async fn lsp_rejects_execution_without_runtime_registry() {
        let file = tempfile::NamedTempFile::new().expect("temporary source file");
        let error = LspTool
            .execute(
                serde_json::json!({
                    "operation": "hover",
                    "filePath": file.path(),
                    "line": 1,
                    "character": 1
                }),
                ToolContext::new("session".into(), "message".into(), ".".into()),
            )
            .await
            .expect_err("missing LSP registry must not produce a successful tool result");
        assert!(
            matches!(error, ToolError::ExecutionError(message) if message.contains("not available"))
        );
    }
}
