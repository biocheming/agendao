use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::execution_preflight::{
    execute_registry_tool_execution_preflight, ExecutionPreflightReport, ExecutionPreflightRunner,
};
use crate::path_guard::{resolve_user_path, RootPathFallbackPolicy};
use crate::{Metadata, PermissionRequest, Tool, ToolContext, ToolError, ToolResult};

const DEFAULT_QUESTION: &str =
    "Describe the relevant contents of this media file and answer the user's need concisely.";
const DESCRIPTION: &str = r#"Inspect a local media file by delegating to the `media-reader` agent.

Phase 2 scope:
- accepts a local file path
- preflights the target through the authoritative `read` tool when registry access is available
- preserves discovered attachment payloads on the `media_inspect` tool result
- creates a `media-reader` subsession with explicit preflight media context
- still relies on the existing attachment-aware `read` tool for image/PDF payload delivery inside the delegated session

This tool does not perform OCR or binary parsing itself."#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaInspectInput {
    #[serde(alias = "file_path")]
    file_path: String,
    #[serde(default)]
    question: Option<String>,
}

struct MediaReadPreflight {
    resolved_path: PathBuf,
}

pub struct MediaInspectTool;

impl MediaInspectTool {
    pub fn new() -> Self {
        Self
    }

    async fn execute_impl(
        &self,
        input: &MediaInspectInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let resolved_path = resolve_media_path(input, ctx)?;
        let preflight = run_media_preflight(&resolved_path, ctx).await?;
        let agent = ctx.do_get_agent_info("media-reader").await.ok_or_else(|| {
            ToolError::ExecutionError("media-reader agent is not configured".to_string())
        })?;
        let preferred_model = if let Some(model) = agent
            .model
            .as_ref()
            .map(|m| format!("{}:{}", m.provider_id, m.model_id))
        {
            Some(model)
        } else {
            ctx.do_get_last_model().await
        };

        let session_id = ctx
            .do_create_subsession(
                "media-reader".to_string(),
                Some(format_media_title(&resolved_path)),
                preferred_model.clone(),
                Vec::new(),
            )
            .await?;
        let prompt = build_media_prompt(&resolved_path, input.question.as_deref(), &preflight);
        let result_text = ctx.do_prompt_subsession(session_id.clone(), prompt).await?;

        let mut metadata = Metadata::new();
        metadata.insert("agent".to_string(), serde_json::json!("media-reader"));
        metadata.insert("sessionId".to_string(), serde_json::json!(session_id));
        metadata.insert(
            "filePath".to_string(),
            serde_json::json!(resolved_path.to_string_lossy().to_string()),
        );
        if let Some(model) = preferred_model {
            metadata.insert("model".to_string(), serde_json::json!(model));
        }
        if let Some(question) = input
            .question
            .as_ref()
            .map(|q| q.trim())
            .filter(|q| !q.is_empty())
        {
            metadata.insert("question".to_string(), serde_json::json!(question));
        }
        preflight.attach_to_metadata(&mut metadata);
        attach_media_preflight_attachments(&preflight, &mut metadata);

        Ok(ToolResult {
            title: format_media_title(&resolved_path),
            output: result_text,
            metadata,
            truncated: false,
        })
    }
}

impl Default for MediaInspectTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MediaInspectTool {
    fn id(&self) -> &str {
        "media_inspect"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Absolute path or session-relative local media file path"
                },
                "question": {
                    "type": "string",
                    "description": "Optional question to answer about the media file"
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: MediaInspectInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        validate_input(&input)?;

        let mut permission = PermissionRequest::new("media_inspect")
            .with_pattern(&input.file_path)
            .with_metadata("file_path", serde_json::json!(&input.file_path))
            .always_allow();
        if let Some(question) = input.question.as_ref() {
            permission = permission.with_metadata("question", serde_json::json!(question));
        }
        ctx.ask_permission(permission).await?;

        self.execute_impl(&input, &ctx).await
    }
}

fn validate_input(input: &MediaInspectInput) -> Result<(), ToolError> {
    if input.file_path.trim().is_empty() {
        return Err(ToolError::InvalidArguments(
            "file_path cannot be empty".to_string(),
        ));
    }
    Ok(())
}

fn resolve_media_path(input: &MediaInspectInput, ctx: &ToolContext) -> Result<PathBuf, ToolError> {
    let base_dir = if ctx.directory.is_empty() {
        Path::new(".")
    } else {
        Path::new(&ctx.directory)
    };
    let resolved = resolve_user_path(
        input.file_path.trim(),
        base_dir,
        RootPathFallbackPolicy::ExistingFallbackOnly,
    );
    if !resolved.resolved.exists() {
        return Err(ToolError::FileNotFound(format!(
            "media file not found: {}",
            resolved.resolved.display()
        )));
    }
    Ok(resolved.resolved)
}

async fn run_media_preflight(
    resolved_path: &Path,
    ctx: &ToolContext,
) -> Result<ExecutionPreflightReport, ToolError> {
    MediaReadPreflight {
        resolved_path: resolved_path.to_path_buf(),
    }
    .run(ctx)
    .await
}

fn build_media_prompt(
    path: &Path,
    question: Option<&str>,
    preflight: &ExecutionPreflightReport,
) -> String {
    let question = question
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .unwrap_or(DEFAULT_QUESTION);

    let mut prompt = format!(
        "Inspect the local media file at `{}`. First call the `read` tool on this exact path. Then answer this question:\n\n{}",
        path.display(),
        question
    );

    if preflight.has_guidance() {
        let attachment_summary = summarize_attachment_payloads(&preflight.attachments);
        prompt.push_str("\n\nPreflight media context from the authoritative `read` tool:\n");
        prompt.push_str(&format!(
            "- read output: {}\n",
            sanitize_prompt_line(&preflight.output)
        ));
        if let Some(mime) = preflight.metadata.get("mime").and_then(|v| v.as_str()) {
            prompt.push_str(&format!("- mime: {}\n", mime));
        }
        if let Some(size) = preflight.metadata.get("size") {
            prompt.push_str(&format!("- size: {}\n", size));
        }
        if let Some(summary) = attachment_summary {
            prompt.push_str(&format!("- attachment summary: {}\n", summary));
        }
        prompt.push_str(
            "Use this preflight only as guidance. You must still call `read` on the exact same file path so the session can obtain the attachment payload for interpretation.",
        );
    } else {
        prompt.push_str(
            " If the read result includes an image or PDF attachment payload, use that attachment to interpret the file.",
        );
    }

    prompt
}

#[async_trait]
impl ExecutionPreflightRunner for MediaReadPreflight {
    async fn run(&self, ctx: &ToolContext) -> Result<ExecutionPreflightReport, ToolError> {
        let subject = self.resolved_path.to_string_lossy().to_string();
        let Some(registry) = ctx.registry.as_ref().map(Arc::clone) else {
            return Ok(ExecutionPreflightReport::new("read", subject).advisory(
                "registry_unavailable",
                "tool registry unavailable; skipped authoritative `read` preflight",
            ));
        };

        let mut report = execute_registry_tool_execution_preflight(
            &registry,
            "read",
            serde_json::json!({
                "file_path": subject.clone(),
            }),
            ctx,
            "read",
            subject,
        )
        .await?;

        if report.attachments.is_empty() {
            report = report.soft_warn(
                "attachment_missing",
                "authoritative `read` preflight did not return an attachment payload",
            );
        }

        Ok(report)
    }
}

fn attach_media_preflight_attachments(
    preflight: &ExecutionPreflightReport,
    metadata: &mut Metadata,
) {
    if preflight.attachments.is_empty() {
        return;
    }
    metadata.insert(
        "attachments".to_string(),
        serde_json::Value::Array(preflight.attachments.clone()),
    );
    if let Some(first) = preflight.attachments.first() {
        metadata.insert("attachment".to_string(), first.clone());
    }
}

fn summarize_attachment_payloads(attachments: &[serde_json::Value]) -> Option<String> {
    let first = attachments.first()?;
    let mime = first
        .get("mime")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let filename = first
        .get("filename")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let url_kind = first
        .get("url")
        .and_then(|value| value.as_str())
        .map(|url| {
            if url.starts_with("data:") {
                "data-url payload"
            } else {
                "remote/file url"
            }
        })
        .unwrap_or("unknown url kind");
    Some(format!(
        "{} attachment(s), first mime={}, filename={}, payload={}.",
        attachments.len(),
        mime,
        filename,
        url_kind
    ))
}

fn sanitize_prompt_line(value: &str) -> String {
    value.replace('\n', " ").trim().to_string()
}

fn format_media_title(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("media file");
    format!("Media Inspect: {}", name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{read::ReadTool, TaskAgentInfo, ToolContext, ToolRegistry};
    use std::fs;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::Mutex;

    #[test]
    fn schema_requires_file_path() {
        let schema = MediaInspectTool::new().parameters();
        assert_eq!(schema["required"][0], serde_json::json!("file_path"));
    }

    #[test]
    fn prompt_mentions_preflight_context_when_available() {
        let mut preflight = ExecutionPreflightReport::new("read", "/tmp/sample.pdf");
        preflight.output = "PDF read successfully (12 bytes)".to_string();
        preflight
            .metadata
            .insert("mime".to_string(), serde_json::json!("application/pdf"));
        preflight
            .metadata
            .insert("size".to_string(), serde_json::json!(12));
        preflight.attachments.push(serde_json::json!({
            "mime": "application/pdf",
            "filename": "sample.pdf",
            "url": "data:application/pdf;base64,AA=="
        }));
        let prompt = build_media_prompt(
            Path::new("/tmp/sample.pdf"),
            Some("Summarize the first page"),
            &preflight,
        );
        assert!(prompt.contains("Preflight media context"));
        assert!(prompt.contains("mime: application/pdf"));
        assert!(prompt.contains("attachment summary"));
        assert!(prompt.contains("Summarize the first page"));
    }

    #[test]
    fn advisory_only_preflight_does_not_render_guidance_block() {
        let preflight = ExecutionPreflightReport::new("read", "/tmp/sample.pdf")
            .advisory("registry_unavailable", "tool registry unavailable");
        let prompt = build_media_prompt(
            Path::new("/tmp/sample.pdf"),
            Some("Summarize the first page"),
            &preflight,
        );

        assert!(!prompt.contains("Preflight media context"));
        assert!(prompt.contains("First call the `read` tool on this exact path."));
    }

    #[test]
    fn media_attachment_projection_remains_adopter_policy() {
        let mut preflight = ExecutionPreflightReport::new("read", "/tmp/sample.pdf");
        preflight.attachments.push(serde_json::json!({
            "mime": "application/pdf",
            "filename": "sample.pdf",
            "url": "data:application/pdf;base64,AA=="
        }));

        let mut metadata = Metadata::new();
        preflight.attach_to_metadata(&mut metadata);
        assert!(!metadata.contains_key("attachments"));

        attach_media_preflight_attachments(&preflight, &mut metadata);
        assert!(metadata.contains_key("attachments"));
        assert!(metadata.contains_key("attachment"));
    }

    #[tokio::test]
    async fn execute_delegates_to_media_reader_with_preflight_attachments() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("sample.pdf");
        fs::write(&file_path, b"%PDF-1.7\n").expect("fixture media should write");

        let registry = Arc::new(ToolRegistry::new());
        registry.register(ReadTool::new()).await;

        let create_calls = Arc::new(Mutex::new(Vec::<(
            String,
            Option<String>,
            Option<String>,
            Vec<String>,
        )>::new()));
        let prompt_calls = Arc::new(Mutex::new(Vec::<(String, String)>::new()));

        let ctx = ToolContext::new(
            "session-1".into(),
            "message-1".into(),
            dir.path().to_string_lossy().to_string(),
        )
        .with_registry(registry)
        .with_get_agent_info(|name| async move {
            if name == "media-reader" {
                Ok(Some(TaskAgentInfo {
                    name: "media-reader".to_string(),
                    model: None,
                    can_use_task: false,
                    steps: Some(12),
                    execution: None,
                    max_tokens: None,
                    temperature: None,
                    top_p: None,
                    variant: None,
                }))
            } else {
                Ok(None)
            }
        })
        .with_get_last_model(|_session_id| async move { Ok(Some("provider-x:model-y".into())) })
        .with_create_subsession({
            let create_calls = create_calls.clone();
            move |agent, title, model, disabled_tools| {
                let create_calls = create_calls.clone();
                async move {
                    create_calls
                        .lock()
                        .await
                        .push((agent, title, model, disabled_tools));
                    Ok("media_reader_session".to_string())
                }
            }
        })
        .with_prompt_subsession({
            let prompt_calls = prompt_calls.clone();
            move |session_id, prompt| {
                let prompt_calls = prompt_calls.clone();
                async move {
                    prompt_calls.lock().await.push((session_id, prompt));
                    Ok("media findings".to_string())
                }
            }
        });

        let result = MediaInspectTool::new()
            .execute(
                serde_json::json!({
                    "file_path": "sample.pdf",
                    "question": "What does this document contain?"
                }),
                ctx,
            )
            .await
            .expect("media inspect should succeed");

        assert_eq!(result.output, "media findings");
        assert_eq!(result.metadata["agent"], serde_json::json!("media-reader"));
        assert_eq!(
            result.metadata["sessionId"],
            serde_json::json!("media_reader_session")
        );
        assert!(result.metadata.contains_key("preflight"));
        assert_eq!(
            result.metadata["preflight"]["status"],
            serde_json::json!("ready")
        );
        let attachments = result
            .metadata
            .get("attachments")
            .and_then(|value| value.as_array())
            .expect("attachments should exist after preflight read");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0]["mime"], serde_json::json!("application/pdf"));

        let create_calls = create_calls.lock().await.clone();
        assert_eq!(create_calls.len(), 1);
        assert_eq!(create_calls[0].0, "media-reader");
        assert_eq!(create_calls[0].2, Some("provider-x:model-y".to_string()));

        let prompt_calls = prompt_calls.lock().await.clone();
        assert_eq!(prompt_calls.len(), 1);
        assert_eq!(prompt_calls[0].0, "media_reader_session");
        assert!(prompt_calls[0].1.contains("sample.pdf"));
        assert!(prompt_calls[0]
            .1
            .contains("What does this document contain?"));
        assert!(prompt_calls[0].1.contains("Preflight media context"));
        assert!(prompt_calls[0].1.contains("attachment summary"));
    }
}
