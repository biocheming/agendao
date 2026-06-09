use super::*;

impl App {
    pub(super) fn image_mime_from_path(
        path: &std::path::Path,
        bytes: &[u8],
    ) -> Option<&'static str> {
        if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            return Some("image/png");
        }
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Some("image/jpeg");
        }
        if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            return Some("image/gif");
        }
        if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            return Some("image/webp");
        }
        if bytes.starts_with(b"BM") {
            return Some("image/bmp");
        }
        if let Ok(prefix) = std::str::from_utf8(&bytes[..bytes.len().min(512)]) {
            let trimmed =
                prefix.trim_start_matches(|ch: char| ch.is_ascii_whitespace() || ch == '\u{feff}');
            if trimmed.starts_with("<svg")
                || trimmed.starts_with("<?xml")
                    && (trimmed.contains("<svg") || trimmed.contains("<svg:svg"))
            {
                return Some("image/svg+xml");
            }
        }

        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "png" => Some("image/png"),
            "jpg" | "jpeg" => Some("image/jpeg"),
            "gif" => Some("image/gif"),
            "webp" => Some("image/webp"),
            "svg" => Some("image/svg+xml"),
            "bmp" => Some("image/bmp"),
            _ => None,
        }
    }

    pub(super) fn attach_image_path(&mut self, raw_path: &str) {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            self.toast
                .show(ToastVariant::Warning, "Usage: /image <path>", 2400);
            return;
        }

        let base_dir = {
            let directory = self.context.directory.read().clone();
            if directory.trim().is_empty() {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
            } else {
                PathBuf::from(directory)
            }
        };
        let candidate = PathBuf::from(trimmed);
        let resolved = if candidate.is_absolute() {
            candidate
        } else {
            base_dir.join(candidate)
        };

        if !resolved.is_file() {
            self.toast.show(
                ToastVariant::Error,
                &format!("Image not found: {}", resolved.display()),
                3200,
            );
            return;
        }

        let bytes = match std::fs::read(&resolved) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to read image: {}", error),
                    3200,
                );
                return;
            }
        };

        let Some(mime) = Self::image_mime_from_path(&resolved, &bytes) else {
            self.toast.show(
                ToastVariant::Warning,
                "Unsupported image type. Use png, jpg, jpeg, gif, webp, svg, or bmp.",
                3400,
            );
            return;
        };

        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let filename = resolved
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("image")
            .to_string();
        self.prompt_draft.push_attachment(crate::api::PromptPart::File {
            url: format!("data:{};base64,{}", mime, encoded),
            filename: Some(filename.clone()),
            mime: Some(mime.to_string()),
        });
        self.sync_prompt_draft_hint();
        self.toast.show(
            ToastVariant::Info,
            &format!("Attached image: {}", filename),
            2200,
        );
    }

    pub(super) fn sync_prompt_draft_hint(&mut self) {
        self.prompt
            .set_attachment_status_hint(self.prompt_draft.attachment_hint());
        self.sync_return_flow();
    }

    pub(super) fn sync_return_flow(&mut self) {
        let lifecycle = self.context.session_context_compaction_lifecycle_summary();
        let compaction_in_progress = matches!(
            lifecycle.as_ref().map(|value| value.status),
            Some(agendao_types::ContextCompactionLifecycleStatus::Started)
        );
        let compaction_just_finished = matches!(
            lifecycle.as_ref().map(|value| value.status),
            Some(agendao_types::ContextCompactionLifecycleStatus::Installed)
        );
        let last_turn_tokens = self.current_session_id().and_then(|session_id| {
            let session_ctx = self.context.session.read();
            let messages = session_ctx.data.messages.get(&session_id)?;
            messages
                .iter()
                .rev()
                .find(|message| matches!(message.role, MessageRole::Assistant))
                .map(|message| message.tokens.clone())
        });
        let item = resolve_return_flow_strip(
            compaction_in_progress,
            compaction_just_finished,
            None,
            None,
            self.prompt_draft.attachment_count(),
            self.prompt_draft.image_count(),
            last_turn_tokens.as_ref(),
        );
        self.prompt
            .set_return_flow_text(item.as_ref().map(format_return_flow_item));
    }

    pub(super) fn clear_prompt_attachments(&mut self) {
        self.prompt_draft.clear_attachments();
        self.sync_prompt_draft_hint();
    }

    pub(super) fn discard_prompt_draft(&mut self) {
        self.prompt.clear();
        self.clear_prompt_attachments();
    }

    pub(super) fn handle_prompt_route_change(&mut self) {
        if !self.prompt.get_input().is_empty() || self.prompt_draft.has_attachments() {
            self.discard_prompt_draft();
            self.event_caused_change = true;
        }
    }

    pub(super) fn navigate_session_with_prompt_cleanup(&mut self, session_id: String) {
        self.handle_prompt_route_change();
        self.context.navigate_session(session_id);
    }

    pub(super) fn navigate_home_with_prompt_cleanup(&mut self) {
        self.handle_prompt_route_change();
        self.context.navigate_home();
    }

    pub(super) fn queue_clipboard_image_attachment(
        &mut self,
        content: crate::render::ClipboardContent,
    ) {
        let filename = format!("clipboard-image-{}.png", chrono::Utc::now().timestamp_millis());
        let data_url = format!("data:{};base64,{}", content.mime, content.data);
        self.prompt_draft.push_attachment(crate::api::PromptPart::File {
            url: data_url,
            filename: Some(filename),
            mime: Some(content.mime),
        });
        self.sync_prompt_draft_hint();
        self.toast.show(
            ToastVariant::Info,
            "Clipboard image attached to the next prompt.",
            2200,
        );
    }
}
