use std::collections::HashSet;
use std::path::Path;

use base64::Engine;

use crate::system::SystemPrompt;
use crate::SessionMessage;

use super::SessionPrompt;

impl SessionPrompt {
    pub(super) async fn add_file_part(
        &self,
        msg: &mut SessionMessage,
        raw_url: &str,
        filename: Option<&str>,
        mime: Option<&str>,
        project_root: &str,
    ) {
        let filename = filename
            .filter(|name| !name.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| Self::filename_from_url(raw_url));
        let mime = mime
            .filter(|m| !m.is_empty())
            .unwrap_or("application/octet-stream")
            .to_string();

        if raw_url.starts_with("mcp://") {
            self.add_mcp_resource_part(msg, raw_url, &filename, &mime)
                .await;
            return;
        }

        if raw_url.starts_with("data:") {
            self.add_data_url_part(msg, raw_url, &filename, &mime).await;
            return;
        }

        if raw_url.starts_with("file://") {
            self.add_file_url_part(msg, raw_url, &filename, &mime, project_root)
                .await;
            return;
        }

        msg.add_file(raw_url.to_string(), filename, mime);
    }

    async fn add_mcp_resource_part(
        &self,
        msg: &mut SessionMessage,
        raw_url: &str,
        filename: &str,
        mime: &str,
    ) {
        let Some((client_name, uri)) = Self::parse_mcp_resource_url(raw_url) else {
            msg.add_text(format!("Failed to parse MCP resource URL: {}", raw_url));
            msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
            return;
        };

        msg.add_text(format!("Reading MCP resource: {} ({})", filename, uri));

        let Some(registry) = &self.mcp_clients else {
            msg.add_text(
                "MCP client registry is not configured; unable to read resource content."
                    .to_string(),
            );
            msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
            return;
        };

        let Some(client) = registry.get(&client_name).await else {
            msg.add_text(format!("MCP client `{}` is not connected.", client_name));
            msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
            return;
        };

        match client.read_resource(&uri).await {
            Ok(result) => {
                let mut text_chunks = Vec::new();
                let mut binary_chunks = Vec::new();
                for content in result.contents {
                    if let Some(text) = content.text {
                        if !text.trim().is_empty() {
                            text_chunks.push(text);
                        }
                        continue;
                    }

                    if content.blob.is_some() {
                        binary_chunks.push(
                            content
                                .mime_type
                                .clone()
                                .unwrap_or_else(|| mime.to_string()),
                        );
                    }
                }

                if !text_chunks.is_empty() {
                    msg.add_text(SystemPrompt::mcp_resource_reminder(
                        filename,
                        &uri,
                        &text_chunks.join("\n\n"),
                    ));
                }

                let has_binary = !binary_chunks.is_empty();
                for mime in binary_chunks {
                    msg.add_text(format!("[Binary content: {}]", mime));
                }

                if text_chunks.is_empty() && !has_binary {
                    msg.add_text(format!("MCP resource `{}` returned no readable text.", uri));
                }
            }
            Err(err) => {
                msg.add_text(format!("Failed to read MCP resource `{}`: {}", uri, err));
            }
        }

        msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
    }

    async fn add_data_url_part(
        &self,
        msg: &mut SessionMessage,
        raw_url: &str,
        filename: &str,
        mime: &str,
    ) {
        if let Some(text) = Self::decode_data_url_text(raw_url, mime) {
            msg.add_text(format!(
                "Called the Read tool with the following input: {}",
                serde_json::json!({ "filePath": filename })
            ));
            msg.add_text(text);
        }

        msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
    }

    async fn add_file_url_part(
        &self,
        msg: &mut SessionMessage,
        raw_url: &str,
        filename: &str,
        mime: &str,
        project_root: &str,
    ) {
        let parsed = match url::Url::parse(raw_url) {
            Ok(url) => url,
            Err(err) => {
                msg.add_text(format!("Invalid file URL `{}`: {}", raw_url, err));
                msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
                return;
            }
        };

        let file_path = match parsed.to_file_path() {
            Ok(path) => path,
            Err(_) => {
                msg.add_text(format!("Invalid file path URL `{}`", raw_url));
                msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
                return;
            }
        };

        let metadata = match tokio::fs::metadata(&file_path).await {
            Ok(meta) => meta,
            Err(err) => {
                msg.add_text(format!(
                    "Read tool failed to read {} with error: {}",
                    file_path.display(),
                    err
                ));
                msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
                return;
            }
        };

        if metadata.is_dir() {
            let listing = Self::read_directory_preview(&file_path).await;
            msg.add_text(format!(
                "Called the Read tool with the following input: {}",
                serde_json::json!({ "filePath": file_path.display().to_string() })
            ));
            msg.add_text(listing);
            msg.add_file(
                raw_url.to_string(),
                filename.to_string(),
                "application/x-directory".to_string(),
            );
            return;
        }

        let bytes = match tokio::fs::read(&file_path).await {
            Ok(bytes) => bytes,
            Err(err) => {
                msg.add_text(format!(
                    "Read tool failed to read {} with error: {}",
                    file_path.display(),
                    err
                ));
                msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
                return;
            }
        };

        if Self::is_binary_asset_mime(mime) {
            let data_url = format!(
                "data:{};base64,{}",
                mime,
                base64::engine::general_purpose::STANDARD.encode(bytes)
            );
            msg.add_file(data_url, filename.to_string(), mime.to_string());
            return;
        }

        let mut text = String::from_utf8_lossy(&bytes).to_string();
        let mut read_args = serde_json::json!({
            "filePath": file_path.display().to_string(),
        });

        if let Some((start, end)) = self.resolve_file_line_window(&file_path, &parsed).await {
            text = Self::slice_lines(&text, start, end);
            if let Some(obj) = read_args.as_object_mut() {
                obj.insert("offset".to_string(), serde_json::json!(start));
                if let Some(end) = end {
                    obj.insert(
                        "limit".to_string(),
                        serde_json::json!(end.saturating_sub(start).saturating_add(1)),
                    );
                }
            }
        }

        msg.add_text(format!(
            "Called the Read tool with the following input: {}",
            read_args
        ));
        msg.add_text(text);
        Self::inject_instruction_prompt(msg, &file_path, Path::new(project_root));
        msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
    }

    pub(super) fn inject_instruction_prompt(
        msg: &mut SessionMessage,
        file_path: &Path,
        project_root: &Path,
    ) {
        let mut loaded = Self::loaded_instruction_paths(msg);
        let mut prompt_chunks = Vec::new();

        for instruction in crate::instruction::resolve_agents_for_file(file_path, project_root) {
            if loaded.insert(instruction.path.clone()) {
                prompt_chunks.push(format!(
                    "Instructions from: {}\n{}",
                    instruction.path, instruction.content
                ));
            }
        }

        if prompt_chunks.is_empty() {
            return;
        }

        msg.add_text(SystemPrompt::system_reminder(&prompt_chunks.join("\n\n")));
        Self::store_loaded_instruction_paths(msg, loaded);
    }

    pub(super) fn loaded_instruction_paths(msg: &SessionMessage) -> HashSet<String> {
        msg.metadata
            .get("loaded_instruction_files")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn store_loaded_instruction_paths(
        msg: &mut SessionMessage,
        loaded: HashSet<String>,
    ) {
        if loaded.is_empty() {
            return;
        }

        let mut paths: Vec<String> = loaded.into_iter().collect();
        paths.sort();
        msg.metadata.insert(
            "loaded_instruction_files".to_string(),
            serde_json::json!(paths),
        );
    }

    async fn resolve_file_line_window(
        &self,
        file_path: &Path,
        file_url: &url::Url,
    ) -> Option<(usize, Option<usize>)> {
        let (start, mut end) = Self::parse_line_window(file_url)?;
        if end == Some(start) {
            if let Some(symbol_end) = self.lookup_symbol_end_line(file_path, start).await {
                end = Some(symbol_end);
            }
        }
        Some((start, end))
    }

    async fn lookup_symbol_end_line(&self, file_path: &Path, start_line: usize) -> Option<usize> {
        let registry = self.lsp_registry.as_ref()?;
        let clients = registry.list().await;
        if clients.is_empty() {
            return None;
        }

        let content = tokio::fs::read_to_string(file_path).await.ok();
        for (_, client) in clients {
            if let Some(content) = content.as_deref() {
                let language = rocode_lsp::detect_language(file_path);
                if let Err(error) = client.open_document(file_path, content, language).await {
                    tracing::debug!(
                        path = %file_path.display(),
                        error = %error,
                        "Failed to sync file to LSP before symbol lookup"
                    );
                }
            }

            let symbols = match client.document_symbol(file_path).await {
                Ok(symbols) => symbols,
                Err(_) => continue,
            };

            for symbol in symbols {
                let symbol_start = symbol.location.range.start.line as usize + 1;
                if symbol_start != start_line {
                    continue;
                }

                let symbol_end = symbol.location.range.end.line as usize + 1;
                if symbol_end >= start_line {
                    return Some(symbol_end);
                }
            }
        }

        None
    }

    pub(super) fn parse_line_window(file_url: &url::Url) -> Option<(usize, Option<usize>)> {
        let start = file_url.query_pairs().find_map(|(key, value)| {
            if key != "start" {
                return None;
            }
            value.parse::<usize>().ok().map(|n| n.max(1))
        })?;

        let end = file_url.query_pairs().find_map(|(key, value)| {
            if key != "end" {
                return None;
            }
            value.parse::<usize>().ok().map(|n| n.max(1))
        });

        Some((start, end))
    }

    pub(super) fn decode_data_url_text(url: &str, mime: &str) -> Option<String> {
        if !Self::is_text_mime(mime) {
            return None;
        }

        let (metadata, payload) = url.split_once(',')?;
        if metadata.contains(";base64") {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(payload.as_bytes())
                .ok()?;
            return Some(String::from_utf8_lossy(&bytes).to_string());
        }

        Some(payload.to_string())
    }

    pub(super) fn parse_mcp_resource_url(url: &str) -> Option<(String, String)> {
        let parsed = url::Url::parse(url).ok()?;
        if parsed.scheme() != "mcp" {
            return None;
        }

        let client_name = parsed.host_str()?.to_string();
        let mut uri = parsed.path().trim_start_matches('/').to_string();
        if let Some(query) = parsed.query() {
            if !query.is_empty() {
                if !uri.is_empty() {
                    uri.push('?');
                }
                uri.push_str(query);
            }
        }

        if uri.is_empty() {
            return None;
        }

        Some((client_name, uri))
    }

    pub(super) fn filename_from_url(url: &str) -> String {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(last) = parsed
                .path_segments()
                .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
            {
                return last.to_string();
            }
        }
        String::new()
    }

    pub(super) fn is_text_mime(mime: &str) -> bool {
        mime.starts_with("text/")
            || matches!(
                mime,
                "application/json"
                    | "application/xml"
                    | "application/javascript"
                    | "application/typescript"
                    | "application/x-sh"
                    | "application/x-shellscript"
            )
    }

    pub(super) fn is_binary_asset_mime(mime: &str) -> bool {
        mime.starts_with("image/") || mime == "application/pdf"
    }

    pub(super) fn slice_lines(text: &str, start: usize, end: Option<usize>) -> String {
        let lines: Vec<&str> = text.lines().collect();
        if lines.is_empty() {
            return String::new();
        }

        let start_idx = start.saturating_sub(1).min(lines.len());
        let end_idx = end.unwrap_or(lines.len()).min(lines.len());
        if start_idx >= end_idx {
            return String::new();
        }

        lines[start_idx..end_idx].join("\n")
    }

    pub(super) async fn read_directory_preview(path: &Path) -> String {
        let mut entries = match tokio::fs::read_dir(path).await {
            Ok(entries) => entries,
            Err(err) => return format!("Failed to list directory {}: {}", path.display(), err),
        };

        let mut names = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            names.push(entry.file_name().to_string_lossy().to_string());
            if names.len() >= 200 {
                names.push("... (truncated)".to_string());
                break;
            }
        }

        if names.is_empty() {
            return format!("Directory is empty: {}", path.display());
        }

        names.sort();
        names.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::super::PromptInput;
    use super::*;
    use crate::prompt::PartInput;
    use crate::{PartType, Session};

    #[tokio::test]
    async fn create_user_message_decodes_text_data_url() {
        let prompt = SessionPrompt::default();
        let mut session = Session::new("proj", ".");
        let input = PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            tools: None,
            ingress: None,
            parts: vec![PartInput::File {
                url: "data:text/plain;base64,SGVsbG8=".to_string(),
                filename: Some("inline.txt".to_string()),
                mime: Some("text/plain".to_string()),
            }],
        };

        prompt
            .create_user_message(&input, &mut session)
            .await
            .expect("create_user_message should succeed");

        let message = session.messages.last().expect("message should exist");
        let text = message
            .parts
            .iter()
            .filter_map(|part| match &part.part_type {
                PartType::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("Hello"));
    }
    // PLACEHOLDER_FP_TESTS_1

    #[tokio::test]
    async fn create_user_message_file_url_with_range_reads_only_requested_lines() {
        let prompt = SessionPrompt::default();
        let mut session = Session::new("proj", ".");
        let temp_dir = tempfile::tempdir().expect("tempdir should create");
        let file_path = temp_dir.path().join("sample.rs");
        let content = (1..=30)
            .map(|n| format!("L{:02}", n))
            .collect::<Vec<_>>()
            .join("\n");
        tokio::fs::write(&file_path, content)
            .await
            .expect("write should succeed");

        let mut url = url::Url::from_file_path(&file_path).expect("file path should convert");
        url.set_query(Some("start=10&end=20"));

        let input = PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            tools: None,
            ingress: None,
            parts: vec![PartInput::File {
                url: url.to_string(),
                filename: Some("sample.rs".to_string()),
                mime: Some("text/plain".to_string()),
            }],
        };

        prompt
            .create_user_message(&input, &mut session)
            .await
            .expect("create_user_message should succeed");

        let message = session.messages.last().expect("message should exist");
        let text = message
            .parts
            .iter()
            .filter_map(|part| match &part.part_type {
                PartType::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("L10"));
        assert!(text.contains("L20"));
        assert!(!text.contains("L09"));
        assert!(!text.contains("L21"));
    }
    // PLACEHOLDER_FP_TESTS_2

    #[tokio::test]
    async fn create_user_message_file_url_injects_nearby_instructions() {
        let prompt = SessionPrompt::default();
        let temp_dir = tempfile::tempdir().expect("tempdir should create");
        let project_root = temp_dir.path().to_path_buf();
        let src_dir = project_root.join("src");
        tokio::fs::create_dir_all(&src_dir)
            .await
            .expect("src directory should create");
        tokio::fs::write(src_dir.join("AGENTS.md"), "Prefer immutable updates")
            .await
            .expect("instructions should write");

        let file_path = src_dir.join("sample.rs");
        tokio::fs::write(&file_path, "fn main() {}")
            .await
            .expect("file should write");

        let mut session = Session::new("proj", project_root.to_string_lossy().to_string());
        let file_url = url::Url::from_file_path(&file_path)
            .expect("file path should convert")
            .to_string();
        let input = PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            tools: None,
            ingress: None,
            parts: vec![PartInput::File {
                url: file_url,
                filename: Some("sample.rs".to_string()),
                mime: Some("text/plain".to_string()),
            }],
        };

        prompt
            .create_user_message(&input, &mut session)
            .await
            .expect("create_user_message should succeed");

        let message = session.messages.last().expect("message should exist");
        let text = message
            .parts
            .iter()
            .filter_map(|part| match &part.part_type {
                PartType::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("<system-reminder>"));
        assert!(text.contains("Instructions from:"));
        assert!(text.contains("Prefer immutable updates"));
    }
    // PLACEHOLDER_FP_TESTS_3

    #[tokio::test]
    async fn create_user_message_file_url_dedupes_instruction_injection_per_message() {
        let prompt = SessionPrompt::default();
        let temp_dir = tempfile::tempdir().expect("tempdir should create");
        let project_root = temp_dir.path().to_path_buf();
        let src_dir = project_root.join("src");
        tokio::fs::create_dir_all(&src_dir)
            .await
            .expect("src directory should create");
        tokio::fs::write(src_dir.join("AGENTS.md"), "Shared file rules")
            .await
            .expect("instructions should write");

        let file_a = src_dir.join("a.rs");
        let file_b = src_dir.join("b.rs");
        tokio::fs::write(&file_a, "fn a() {}")
            .await
            .expect("file a should write");
        tokio::fs::write(&file_b, "fn b() {}")
            .await
            .expect("file b should write");

        let mut session = Session::new("proj", project_root.to_string_lossy().to_string());
        let input = PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            tools: None,
            ingress: None,
            parts: vec![
                PartInput::File {
                    url: url::Url::from_file_path(&file_a)
                        .expect("file a path should convert")
                        .to_string(),
                    filename: Some("a.rs".to_string()),
                    mime: Some("text/plain".to_string()),
                },
                PartInput::File {
                    url: url::Url::from_file_path(&file_b)
                        .expect("file b path should convert")
                        .to_string(),
                    filename: Some("b.rs".to_string()),
                    mime: Some("text/plain".to_string()),
                },
            ],
        };

        prompt
            .create_user_message(&input, &mut session)
            .await
            .expect("create_user_message should succeed");
        // PLACEHOLDER_FP_TESTS_4

        let message = session.messages.last().expect("message should exist");
        let text = message
            .parts
            .iter()
            .filter_map(|part| match &part.part_type {
                PartType::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(text.matches("Instructions from:").count(), 1);
    }

    #[tokio::test]
    async fn create_user_message_file_url_injects_agents_but_not_claude_for_file_scope() {
        let prompt = SessionPrompt::default();
        let temp_dir = tempfile::tempdir().expect("tempdir should create");
        let project_root = temp_dir.path().to_path_buf();
        let src_dir = project_root.join("src");
        tokio::fs::create_dir_all(&src_dir)
            .await
            .expect("src directory should create");
        tokio::fs::write(project_root.join("AGENTS.md"), "Root agents rule")
            .await
            .expect("root AGENTS should write");
        tokio::fs::write(src_dir.join("CLAUDE.md"), "src claude rule")
            .await
            .expect("src CLAUDE should write");

        let file_path = src_dir.join("sample.rs");
        tokio::fs::write(&file_path, "fn main() {}")
            .await
            .expect("file should write");

        let mut session = Session::new("proj", project_root.to_string_lossy().to_string());
        let file_url = url::Url::from_file_path(&file_path)
            .expect("file path should convert")
            .to_string();
        let input = PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            tools: None,
            ingress: None,
            parts: vec![PartInput::File {
                url: file_url,
                filename: Some("sample.rs".to_string()),
                mime: Some("text/plain".to_string()),
            }],
        };

        prompt
            .create_user_message(&input, &mut session)
            .await
            .expect("create_user_message should succeed");

        let message = session.messages.last().expect("message should exist");
        let text = message
            .parts
            .iter()
            .filter_map(|part| match &part.part_type {
                PartType::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("Root agents rule"));
        assert!(!text.contains("src claude rule"));
    }
}
