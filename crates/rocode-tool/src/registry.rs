use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::repair_telemetry::ToolArgumentNormalizationTelemetry;
use crate::tool_access;
use crate::{Tool, ToolContext, ToolError, ToolResult};
use rocode_plugin::{HookContext, HookEvent};

/// Tools that should not appear in suggestion lists when a tool is not found.
const FILTERED_FROM_SUGGESTIONS: &[&str] = &["invalid", "patch", "batch"];

fn looks_like_jsonish_payload(s: &str) -> bool {
    let trimmed = s.trim_start();
    trimmed.starts_with('{')
        || trimmed.starts_with('[')
        || trimmed.starts_with("\"{")
        || trimmed.starts_with("\"[")
        || s.contains("\":")
        || s.contains("\\\":")
        || s.contains("\"file_path\"")
        || s.contains("\\\"file_path\\\"")
        || s.contains("\"filePath\"")
        || s.contains("\\\"filePath\\\"")
        || s.contains("\"content\"")
        || s.contains("\\\"content\\\"")
}

fn parse_jsonish_string_field(input: &str, field: &str) -> Option<String> {
    let needle = format!("\"{}\"", field);
    let field_idx = input.find(&needle)?;
    let after_field = &input[field_idx + needle.len()..];
    let colon_idx = after_field.find(':')?;
    let mut chars = after_field[colon_idx + 1..].chars().peekable();

    while matches!(chars.peek(), Some(c) if c.is_whitespace()) {
        chars.next();
    }
    if !matches!(chars.next(), Some('"')) {
        return None;
    }

    let mut out = String::new();
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if escaped {
            match ch {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push('\u{08}'),
                'f' => out.push('\u{0c}'),
                'u' => {
                    let mut hex = String::new();
                    for _ in 0..4 {
                        match chars.peek().copied() {
                            Some(c) if c.is_ascii_hexdigit() => {
                                hex.push(c);
                                chars.next();
                            }
                            _ => break,
                        }
                    }
                    if hex.len() == 4 {
                        if let Ok(code) = u32::from_str_radix(&hex, 16) {
                            if let Some(decoded) = char::from_u32(code) {
                                out.push(decoded);
                            }
                        }
                    } else {
                        out.push('u');
                        out.push_str(&hex);
                    }
                }
                other => out.push(other),
            }
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(out),
            other => out.push(other),
        }
    }

    // Unterminated JSON string: return best-effort content.
    Some(out)
}

fn recover_write_args_from_jsonish(input: &str) -> Option<serde_json::Value> {
    fn normalize_single_escaped_quotes(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        let mut prev: Option<char> = None;

        while let Some(ch) = chars.next() {
            if ch == '\\' && matches!(chars.peek(), Some('"')) && prev != Some('\\') {
                out.push('"');
                chars.next();
                prev = Some('"');
                continue;
            }
            out.push(ch);
            prev = Some(ch);
        }
        out
    }

    fn recover_once(input: &str) -> Option<serde_json::Value> {
        let file_path = parse_jsonish_string_field(input, "file_path")
            .or_else(|| parse_jsonish_string_field(input, "filePath"))?;
        let content = parse_jsonish_string_field(input, "content").unwrap_or_default();
        Some(serde_json::json!({
            "file_path": file_path,
            "content": content
        }))
    }

    if let Some(recovered) = recover_once(input) {
        return Some(recovered);
    }

    // If arguments were wrapped as a JSON string, unwrap one layer and retry.
    if let Ok(inner) = serde_json::from_str::<String>(input) {
        if let Some(recovered) = recover_once(&inner) {
            return Some(recovered);
        }
    }

    // Some malformed payloads preserve escaped quotes without a valid outer JSON
    // wrapper (e.g. {\"file_path\":\"...\" ...). Best-effort normalize and retry.
    if input.contains("\\\"") {
        let de_escaped_quotes = normalize_single_escaped_quotes(input);
        if let Some(recovered) = recover_once(&de_escaped_quotes) {
            return Some(recovered);
        }
    }

    None
}

fn recover_bash_args_from_jsonish(input: &str) -> Option<serde_json::Value> {
    fn normalize_single_escaped_quotes(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        let mut prev: Option<char> = None;

        while let Some(ch) = chars.next() {
            if ch == '\\' && matches!(chars.peek(), Some('"')) && prev != Some('\\') {
                out.push('"');
                chars.next();
                prev = Some('"');
                continue;
            }
            out.push(ch);
            prev = Some(ch);
        }
        out
    }

    fn recover_once(input: &str) -> Option<serde_json::Value> {
        let command = parse_jsonish_string_field(input, "command")
            .or_else(|| parse_jsonish_string_field(input, "cmd"))?;
        let description = parse_jsonish_string_field(input, "description")
            .unwrap_or_else(|| "Execute shell command".to_string());
        let mut obj = serde_json::Map::new();
        obj.insert("command".to_string(), serde_json::Value::String(command));
        obj.insert(
            "description".to_string(),
            serde_json::Value::String(description),
        );
        if let Some(workdir) = parse_jsonish_string_field(input, "workdir")
            .or_else(|| parse_jsonish_string_field(input, "cwd"))
        {
            obj.insert("workdir".to_string(), serde_json::Value::String(workdir));
        }
        Some(serde_json::Value::Object(obj))
    }

    if let Some(recovered) = recover_once(input) {
        return Some(recovered);
    }
    if let Ok(inner) = serde_json::from_str::<String>(input) {
        if let Some(recovered) = recover_once(&inner) {
            return Some(recovered);
        }
    }
    if input.contains("\\\"") {
        let de_escaped_quotes = normalize_single_escaped_quotes(input);
        if let Some(recovered) = recover_once(&de_escaped_quotes) {
            return Some(recovered);
        }
    }
    None
}

pub fn normalize_tool_arguments(tool_id: &str, args: serde_json::Value) -> serde_json::Value {
    normalize_tool_arguments_with_telemetry(tool_id, args).0
}

pub fn normalize_tool_arguments_with_telemetry(
    tool_id: &str,
    mut args: serde_json::Value,
) -> (serde_json::Value, ToolArgumentNormalizationTelemetry) {
    let mut telemetry = ToolArgumentNormalizationTelemetry::default();

    // Normalize: if args is a JSON array of objects, merge them into a single
    // object. Some models produce `[{"file_path":"x"},{"content":"y"}]` instead
    // of `{"file_path":"x","content":"y"}`.
    if let Some(arr) = args.as_array() {
        let mut merged = serde_json::Map::new();
        for item in arr {
            if let Some(obj) = item.as_object() {
                for (k, v) in obj {
                    merged.insert(k.clone(), v.clone());
                }
            }
        }
        if !merged.is_empty() {
            tracing::info!(
                tool = %tool_id,
                element_count = arr.len(),
                merged_keys = %merged.keys().cloned().collect::<Vec<_>>().join(","),
                "merged array tool arguments into single object"
            );
            telemetry.record("array_object_merge");
            args = serde_json::Value::Object(merged);
        }
    }

    // Normalize: if args is a JSON string that contains a valid object,
    // parse it into an actual object. This happens when the stream assembler
    // fails to parse tool call arguments during streaming and wraps the raw
    // text as Value::String.
    if let Some(s) = args.as_str().map(|s| s.to_owned()) {
        if let Some(parsed @ serde_json::Value::Object(_)) =
            rocode_util::json::try_parse_json_object_robust(&s)
        {
            tracing::info!(
                tool = %tool_id,
                "recovered tool arguments via robust JSON parser"
            );
            telemetry.record("robust_json_object_parse");
            args = parsed;
        } else {
            if let Some(parsed) =
                rocode_util::json::recover_tool_arguments_from_jsonish(tool_id, &s)
            {
                tracing::info!(
                    tool = %tool_id,
                    "recovered tool arguments from JSON-ish payload"
                );
                telemetry.record("jsonish_recovery");
                args = parsed;
            } else if tool_id == "write" {
                if let Some(parsed) = recover_write_args_from_jsonish(&s) {
                    tracing::info!(
                        tool = %tool_id,
                        "recovered write arguments from JSON-ish payload"
                    );
                    telemetry.record("write_jsonish_recovery");
                    args = parsed;
                }
            } else if tool_id == "bash" {
                if let Some(parsed) = recover_bash_args_from_jsonish(&s) {
                    tracing::info!(
                        tool = %tool_id,
                        "recovered bash arguments from JSON-ish payload"
                    );
                    telemetry.record("bash_jsonish_recovery");
                    args = parsed;
                }
            }
            // Ultra structural recovery: handles unescaped HTML quotes,
            // truncated JSON, markdown fences, reasoning pollution, etc.
            if args.is_string() {
                if let Some(parsed) = rocode_util::json::recover_tool_call_ultra(tool_id, &s) {
                    tracing::info!(
                        tool = %tool_id,
                        "recovered tool arguments via ultra structural recovery"
                    );
                    telemetry.record("ultra_structural_recovery");
                    args = parsed;
                }
            }
            // If still a string, try key=value fallback.
            if args.is_string() && !looks_like_jsonish_payload(&s) {
                // Some models (e.g. Qwen via LiteLLM) may send arguments in
                // non-JSON formats like "key=value" or "key: value". Try to
                // construct a JSON object from simple key=value pairs.
                let mut obj = serde_json::Map::new();
                for line in s.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Some((key, value)) = line.split_once('=') {
                        let key = key.trim().to_string();
                        let value = value.trim();
                        // Try to parse value as JSON, otherwise treat as string
                        let json_value = serde_json::from_str(value)
                            .unwrap_or_else(|_| serde_json::Value::String(value.to_string()));
                        obj.insert(key, json_value);
                    }
                }
                if !obj.is_empty() {
                    tracing::info!(
                        tool = %tool_id,
                        "normalized non-JSON tool arguments from key=value format"
                    );
                    telemetry.record("key_value_recovery");
                    args = serde_json::Value::Object(obj);
                }
            }
        }
    }

    // Normalize bash schema defaults regardless of which recovery path succeeded.
    if tool_id == "bash" {
        if let Some(obj) = args.as_object_mut() {
            let needs_description = obj
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().is_empty())
                .unwrap_or(true);
            if needs_description {
                obj.insert(
                    "description".to_string(),
                    serde_json::Value::String("Execute shell command".to_string()),
                );
                telemetry.record("bash_default_description");
            }
        }
    }
    (args, telemetry)
}

pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
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
            })
            .collect();

        // Trigger tool.definition hook for each schema so plugins can transform them
        for schema in &mut schemas {
            let hook_outputs = rocode_plugin::trigger_collect(
                HookContext::new(HookEvent::ToolDefinition)
                    .with_data("tool_id", serde_json::json!(&schema.name))
                    .with_data("description", serde_json::json!(&schema.description))
                    .with_data("parameters", schema.parameters.clone()),
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

        let mut args = normalize_tool_arguments(tool_id, args);

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
        if rocode_plugin::should_trigger_agent_hooks(
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
            let before_outputs = rocode_plugin::trigger_collect(before_hook_ctx).await;
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
        if rocode_plugin::should_trigger_agent_hooks(
            HookEvent::ToolExecuteAfter,
            Some(ctx.agent.as_str()),
        )
        .await
        {
            let mut hook_ctx = HookContext::new(HookEvent::ToolExecuteAfter)
                .with_session(&ctx.session_id)
                .with_data("tool", serde_json::json!(tool_id))
                .with_data("args", args);
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

            let after_outputs = rocode_plugin::trigger_collect(hook_ctx).await;
            if let Ok(tool_result) = &mut result {
                for output in after_outputs {
                    if let Some(payload) = output.payload.as_ref() {
                        apply_tool_after_payload(tool_result, payload);
                    }
                }
            }
        }

        result
    }
}

fn apply_tool_definition_payload(schema: &mut ToolSchema, payload: &serde_json::Value) {
    let Some(object) = rocode_plugin::hook_payload_object(payload) else {
        return;
    };
    if let Some(description) = object.get("description").and_then(|value| value.as_str()) {
        schema.description = description.to_string();
    }
    if let Some(parameters) = object.get("parameters") {
        schema.parameters = parameters.clone();
    }
}

fn apply_tool_before_payload(args: &mut serde_json::Value, payload: &serde_json::Value) {
    let Some(object) = rocode_plugin::hook_payload_object(payload) else {
        return;
    };
    if let Some(next_args) = object.get("args") {
        *args = next_args.clone();
    }
}

fn apply_tool_after_payload(result: &mut ToolResult, payload: &serde_json::Value) {
    let Some(object) = rocode_plugin::hook_payload_object(payload) else {
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub source_kind: crate::ToolSchemaSourceKind,
}

pub async fn create_default_registry() -> ToolRegistry {
    create_default_registry_with_config(None).await
}

/// Create the default tool registry, optionally using web search config from the
/// application config. When `config` is `None` or `config.web_search` is unset,
/// the web search tool falls back to the default Exa MCP endpoint.
pub async fn create_default_registry_with_config(
    config: Option<&rocode_config::Config>,
) -> ToolRegistry {
    let registry = ToolRegistry::new();

    let web_search_config = config.and_then(|c| c.web_search.as_ref());

    registry.register(crate::read::ReadTool::new()).await;
    registry.register(crate::write::WriteTool::new()).await;
    registry.register(crate::edit::EditTool::new()).await;
    registry.register(crate::bash::BashTool::new()).await;
    registry
        .register(crate::shell_session::ShellSessionTool::new())
        .await;
    registry.register(crate::glob_tool::GlobTool::new()).await;
    registry.register(crate::grep_tool::GrepTool::new()).await;
    registry.register(crate::ls::LsTool::new()).await;
    registry.register(crate::task::TaskTool::new()).await;
    registry
        .register(crate::task_flow::TaskFlowTool::new())
        .await;
    registry
        .register(crate::question::QuestionTool::new())
        .await;
    registry
        .register(crate::webfetch::WebFetchTool::new())
        .await;
    registry
        .register(crate::websearch::WebSearchTool::from_config(
            web_search_config,
        ))
        .await;
    registry.register(crate::todo::TodoReadTool).await;
    registry.register(crate::todo::TodoWriteTool).await;
    registry.register(crate::multiedit::MultiEditTool).await;
    registry.register(crate::apply_patch::ApplyPatchTool).await;
    registry
        .register(crate::skills_categories::SkillsCategoriesTool)
        .await;
    registry.register(crate::skills_list::SkillsListTool).await;
    registry.register(crate::skill_view::SkillViewTool).await;
    registry.register(crate::skill::SkillTool).await;
    registry.register(crate::skill_hub::SkillHubTool).await;
    registry
        .register(crate::skill_manage::SkillManageTool)
        .await;
    registry.register(crate::lsp_tool::LspTool).await;
    registry.register(crate::batch::BatchTool).await;
    registry.register(crate::codesearch::CodeSearchTool).await;
    registry
        .register(crate::context_docs::ContextDocsTool::new())
        .await;
    registry
        .register(crate::github_research::GitHubResearchTool::new())
        .await;
    registry
        .register(crate::repo_history::RepoHistoryTool::new())
        .await;
    registry
        .register(crate::media_inspect::MediaInspectTool::new())
        .await;
    registry
        .register(crate::browser_session::BrowserSessionTool::new())
        .await;
    registry
        .register(crate::ast_grep_search::AstGrepSearchTool::new())
        .await;
    registry
        .register(crate::ast_grep_replace::AstGrepReplaceTool::new())
        .await;
    registry.register(crate::plan::PlanEnterTool).await;
    registry.register(crate::plan::PlanExitTool).await;
    registry.register(crate::invalid::InvalidTool).await;

    // Auto-register plugin custom tools (may override same-named built-in tools)
    if let Some(loader) = rocode_plugin::global_loader() {
        register_plugin_tools(&registry, loader).await;
    }

    registry
}

async fn register_plugin_tools(
    registry: &ToolRegistry,
    loader: Arc<rocode_plugin::subprocess::loader::PluginLoader>,
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

    fn test_tool_context() -> ToolContext {
        ToolContext::new(
            "ses_test".to_string(),
            "msg_test".to_string(),
            ".".to_string(),
        )
    }

    #[tokio::test]
    async fn execute_recovers_stringified_json_object_arguments() {
        let (registry, captured) = setup_capture_registry().await;
        let inner = r#"{"file_path":"/tmp/a.html","content":"hello"}"#;
        let outer = serde_json::to_string(inner).expect("stringify should succeed");

        let result = registry
            .execute(
                "capture",
                serde_json::Value::String(outer),
                test_tool_context(),
            )
            .await
            .expect("tool should execute");

        assert_eq!(result.output, "/tmp/a.html");
        let captured_args = captured
            .lock()
            .expect("lock should succeed")
            .clone()
            .expect("args should be captured");
        assert!(captured_args.is_object());
        assert_eq!(captured_args["file_path"], "/tmp/a.html");
    }

    #[tokio::test]
    async fn execute_recovers_literal_control_characters_in_json_string_arguments() {
        let (registry, captured) = setup_capture_registry().await;
        let args = serde_json::Value::String(
            "{\"file_path\":\"/tmp/b.html\",\"content\":\"line1\nline2\"}".to_string(),
        );

        let result = registry
            .execute("capture", args, test_tool_context())
            .await
            .expect("tool should execute");

        assert_eq!(result.output, "/tmp/b.html");
        let captured_args = captured
            .lock()
            .expect("lock should succeed")
            .clone()
            .expect("args should be captured");
        assert_eq!(captured_args["file_path"], "/tmp/b.html");
        assert_eq!(captured_args["content"], "line1\nline2");
    }

    #[tokio::test]
    async fn execute_keeps_key_value_fallback_for_non_json_strings() {
        let (registry, captured) = setup_capture_registry().await;
        let args = serde_json::Value::String("file_path=/tmp/c.html\ncontent=hello".to_string());

        let result = registry
            .execute("capture", args, test_tool_context())
            .await
            .expect("tool should execute");

        assert_eq!(result.output, "/tmp/c.html");
        let captured_args = captured
            .lock()
            .expect("lock should succeed")
            .clone()
            .expect("args should be captured");
        assert_eq!(captured_args["file_path"], "/tmp/c.html");
        assert_eq!(captured_args["content"], "hello");
    }

    #[tokio::test]
    async fn execute_recovers_write_args_from_unterminated_jsonish_payload() {
        let registry = ToolRegistry::new();
        let captured = Arc::new(Mutex::new(None));
        registry
            .register(CaptureTool {
                captured: captured.clone(),
                id: "write",
            })
            .await;

        let malformed = serde_json::Value::String(
            "{\"file_path\":\"/tmp/d.html\",\"content\":\"<div class=\\\"x\\\">hello\nworld"
                .to_string(),
        );

        let result = registry
            .execute("write", malformed, test_tool_context())
            .await
            .expect("tool should execute");

        assert_eq!(result.output, "/tmp/d.html");
        let captured_args = captured
            .lock()
            .expect("lock should succeed")
            .clone()
            .expect("args should be captured");
        assert_eq!(captured_args["file_path"], "/tmp/d.html");
        assert_eq!(captured_args["content"], "<div class=\"x\">hello\nworld");
    }

    #[tokio::test]
    async fn execute_recovers_write_args_from_escaped_jsonish_payload() {
        let registry = ToolRegistry::new();
        let captured = Arc::new(Mutex::new(None));
        registry
            .register(CaptureTool {
                captured: captured.clone(),
                id: "write",
            })
            .await;

        let malformed = serde_json::Value::String(
            "{\\\"file_path\\\":\\\"/tmp/e.html\\\",\\\"content\\\":\\\"<div class=\\\\\\\"x\\\\\\\">hello\\nworld".to_string(),
        );

        let result = registry
            .execute("write", malformed, test_tool_context())
            .await
            .expect("tool should execute");

        assert_eq!(result.output, "/tmp/e.html");
        let captured_args = captured
            .lock()
            .expect("lock should succeed")
            .clone()
            .expect("args should be captured");
        assert_eq!(captured_args["file_path"], "/tmp/e.html");
        let content = captured_args["content"]
            .as_str()
            .expect("content should be string");
        assert!(content.contains("<div class="));
        assert!(content.contains("hello"));
    }

    #[tokio::test]
    async fn execute_recovers_bash_args_from_unterminated_jsonish_payload() {
        let registry = ToolRegistry::new();
        let captured = Arc::new(Mutex::new(None));
        registry
            .register(CaptureTool {
                captured: captured.clone(),
                id: "bash",
            })
            .await;

        let malformed = serde_json::Value::String(
            "{\"command\":\"cat > /tmp/f.html << 'EOF'\n<!doctype html>\n".to_string(),
        );

        let result = registry
            .execute("bash", malformed, test_tool_context())
            .await
            .expect("tool should execute");

        assert!(result.output.starts_with("cat > /tmp/f.html"));
        let captured_args = captured
            .lock()
            .expect("lock should succeed")
            .clone()
            .expect("args should be captured");
        assert_eq!(
            captured_args["description"]
                .as_str()
                .expect("description should be set"),
            "Execute shell command"
        );
    }

    #[test]
    fn normalize_tool_arguments_recovers_bash_jsonish_payload() {
        let malformed = serde_json::Value::String("{\"command\":\"echo hello\nworld".to_string());
        let normalized = normalize_tool_arguments("bash", malformed);
        assert!(normalized.is_object());
        assert_eq!(normalized["command"], "echo hello\nworld");
        assert_eq!(normalized["description"], "Execute shell command");
    }

    #[test]
    fn normalize_tool_arguments_merges_array_into_object() {
        let arr = serde_json::json!([
            {"file_path": "test.py"},
            {"content": "print('hello')"}
        ]);
        let normalized = normalize_tool_arguments("write", arr);
        assert!(normalized.is_object(), "array should be merged into object");
        assert_eq!(normalized["file_path"], "test.py");
        assert_eq!(normalized["content"], "print('hello')");
    }

    #[test]
    fn normalize_tool_arguments_merges_single_element_array() {
        let arr = serde_json::json!([
            {"file_path": "test.py", "content": "ok"}
        ]);
        let normalized = normalize_tool_arguments("write", arr);
        assert!(normalized.is_object());
        assert_eq!(normalized["file_path"], "test.py");
        assert_eq!(normalized["content"], "ok");
    }

    #[test]
    fn normalize_tool_arguments_ultra_recovers_unescaped_html_write() {
        // Simulates the exact bug: LLM sends write args with unescaped HTML quotes.
        // Existing recovery fails; ultra structural recovery should succeed.
        let raw = r#"{"content":"<html lang="en"><body>Hello</body></html>","file_path":"/tmp/test.html"}"#;
        let input = serde_json::Value::String(raw.to_string());
        let normalized = normalize_tool_arguments("write", input);
        assert!(normalized.is_object(), "ultra should recover to object");
        assert_eq!(normalized["file_path"], "/tmp/test.html");
        let content = normalized["content"].as_str().unwrap();
        assert!(content.contains("<html"));
    }
}

#[cfg(test)]
mod task_flow_registry_tests {
    #[tokio::test]
    async fn create_default_registry_registers_task_flow() {
        let registry = super::create_default_registry().await;
        let ids = registry.list_ids().await;
        assert!(ids.iter().any(|id| id == "task_flow"));
    }

    #[tokio::test]
    async fn create_default_registry_registers_context_docs() {
        let registry = super::create_default_registry().await;
        let ids = registry.list_ids().await;
        assert!(ids.iter().any(|id| id == "context_docs"));
        assert!(ids.iter().any(|id| id == "ast_grep_replace"));
        assert!(ids.iter().any(|id| id == "repo_history"));
        assert!(ids.iter().any(|id| id == "media_inspect"));
        assert!(ids.iter().any(|id| id == "shell_session"));
        assert!(ids.iter().any(|id| id == "browser_session"));
    }
}
