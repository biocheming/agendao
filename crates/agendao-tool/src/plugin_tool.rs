//! Bridge between plugin-registered custom tools and the agendao tool registry.
//!
//! Each `PluginTool` holds an `Arc<PluginLoader>` (not a direct client reference)
//! so that idle-shutdown recovery via `ensure_started()` works transparently.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use agendao_plugin::subprocess::client::PluginToolDef;
use agendao_plugin::subprocess::loader::PluginLoader;
use agendao_types::ToolCatalogMetadata;

use crate::truncation;
use crate::{Metadata, Tool, ToolContext, ToolError, ToolResult, ToolSchemaSourceKind};

pub struct PluginTool {
    tool_id: String,
    plugin_id: String,
    description: String,
    parameters: Value,
    catalog: Option<ToolCatalogMetadata>,
    loader: Arc<PluginLoader>,
}

impl PluginTool {
    pub fn new(
        tool_id: String,
        plugin_id: String,
        def: &PluginToolDef,
        loader: Arc<PluginLoader>,
    ) -> Self {
        Self {
            tool_id,
            plugin_id,
            description: def.description.clone(),
            parameters: def.parameters.clone(),
            catalog: def.catalog.clone(),
            loader,
        }
    }
}

#[async_trait]
impl Tool for PluginTool {
    fn id(&self) -> &str {
        &self.tool_id
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        self.parameters.clone()
    }

    fn source_kind(&self) -> ToolSchemaSourceKind {
        ToolSchemaSourceKind::Plugin
    }

    fn catalog_metadata(&self) -> Option<ToolCatalogMetadata> {
        self.catalog.clone()
    }

    async fn execute(&self, args: Value, ctx: ToolContext) -> Result<ToolResult, ToolError> {
        let context = serde_json::json!({
            "sessionID": ctx.session_id,
            "messageID": ctx.message_id,
            "agent": ctx.agent,
            "directory": ctx.directory,
            "worktree": ctx.worktree,
        });
        let result = self
            .loader
            .invoke_plugin_tool(&self.plugin_id, &self.tool_id, args, context)
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!(
                    "plugin tool `{}` (plugin `{}`): {}",
                    self.tool_id, self.plugin_id, e
                ))
            })?;
        let output = match result {
            Value::String(s) => s,
            Value::Null => String::new(),
            other => serde_json::to_string_pretty(&other).unwrap_or_default(),
        };

        // Large output → save FULL content to file, return preview + path in output,
        // full content reference in metadata.attachments (attachment demotion).
        let original_bytes = output.len();
        let original_lines = output.lines().count();
        let needs_demotion =
            original_bytes > truncation::MAX_BYTES || original_lines > truncation::MAX_LINES;

        if !needs_demotion {
            return Ok(ToolResult::simple("", output));
        }

        let saved_path = save_full_output(&output, &ctx.directory)
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!("failed to persist plugin tool output: {e}"))
            })?;

        let summary = format!(
            "Output too large ({} bytes, {} lines). Full output saved to: {}\n\n{}",
            original_bytes,
            original_lines,
            saved_path.display(),
            preview_lines(&output, 20),
        );

        let path_str = saved_path.display().to_string();
        let mut attachment = serde_json::Map::new();
        attachment.insert("type".into(), serde_json::json!("file"));
        attachment.insert("path".into(), serde_json::json!(path_str));
        attachment.insert("original_bytes".into(), serde_json::json!(original_bytes));
        attachment.insert("original_lines".into(), serde_json::json!(original_lines));
        let attachment_value = Value::Object(attachment);

        let mut metadata = Metadata::new();
        metadata.insert("truncated".into(), serde_json::json!(true));
        metadata.insert("original_bytes".into(), serde_json::json!(original_bytes));
        metadata.insert("original_lines".into(), serde_json::json!(original_lines));
        metadata.insert("attachment".into(), attachment_value.clone());
        metadata.insert("attachments".into(), serde_json::json!([attachment_value]));

        Ok(ToolResult {
            title: String::new(),
            output: summary,
            metadata,
            truncated: true,
        })
    }
}

/// Save the FULL (untruncated) output to disk. Tries project dir first, falls back to /tmp.
async fn save_full_output(content: &str, session_dir: &str) -> std::io::Result<PathBuf> {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::fs;
    use tokio::io::AsyncWriteExt;

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let filename = format!("plugin_output_{}_{}_{}.txt", timestamp, pid, seq);

    // Primary: project dir
    let primary = if !session_dir.is_empty() {
        Some(
            PathBuf::from(session_dir)
                .join(".agendao")
                .join("plugin-tool-output"),
        )
    } else {
        None
    };

    // Fallback: system temp dir
    let fallback = std::env::temp_dir()
        .join("agendao")
        .join("plugin-tool-output");

    let dirs_to_try: Vec<PathBuf> = primary
        .into_iter()
        .chain(std::iter::once(fallback))
        .collect();

    let mut last_err = std::io::Error::other("no candidate dirs");
    for dir in &dirs_to_try {
        if let Err(e) = fs::create_dir_all(dir).await {
            last_err = e;
            continue;
        }
        let path = dir.join(&filename);
        match fs::File::create(&path).await {
            Ok(mut file) => {
                // Write errors also fall through to the next candidate dir.
                if let Err(e) = file.write_all(content.as_bytes()).await {
                    last_err = e;
                    continue;
                }
                if let Err(e) = file.flush().await {
                    last_err = e;
                    continue;
                }
                return Ok(path);
            }
            Err(e) => {
                last_err = e;
            }
        }
    }
    Err(last_err)
}

/// Hard byte-cap for the preview portion of the summary.
const PREVIEW_MAX_BYTES: usize = 8 * 1024;

/// First N lines as a preview, capped at `PREVIEW_MAX_BYTES`.
fn preview_lines(content: &str, n: usize) -> String {
    let lines: Vec<&str> = content.lines().take(n + 1).collect();
    let mut preview = if lines.len() > n {
        let mut p = lines[..n].join("\n");
        p.push_str("\n...");
        p
    } else {
        lines.join("\n")
    };
    // Hard byte-cap: single-line mega-outputs or many long lines can still blow up.
    if preview.len() > PREVIEW_MAX_BYTES {
        // Find the largest valid char boundary at or below the limit.
        let mut cut = PREVIEW_MAX_BYTES;
        while cut > 0 && !preview.is_char_boundary(cut) {
            cut -= 1;
        }
        preview.truncate(cut);
        preview.push_str("\n...(preview truncated)");
    }
    preview
}
