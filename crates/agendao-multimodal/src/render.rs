use crate::MultimodalPart;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultimodalDisplaySummary {
    pub primary_text: String,
    pub attachment_count: usize,
    pub badges: Vec<String>,
    pub compact_label: String,
    pub kinds: Vec<String>,
}

impl MultimodalDisplaySummary {
    pub fn from_parts(primary_text: Option<&str>, parts: &[MultimodalPart]) -> Self {
        let normalized_primary = primary_text
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_default()
            .to_string();
        let kinds = unique_kinds(parts);
        let attachment_count = parts.len();
        let compact_label = if !normalized_primary.is_empty() {
            normalized_primary.clone()
        } else if attachment_count == 1 {
            format!(
                "[{} input]",
                kinds.first().map(String::as_str).unwrap_or("attachment")
            )
        } else if attachment_count > 1 {
            format!("[{} attachments]", attachment_count)
        } else {
            String::new()
        };

        Self {
            primary_text: normalized_primary,
            attachment_count,
            badges: kinds.clone(),
            compact_label,
            kinds,
        }
    }
}

fn unique_kinds(parts: &[MultimodalPart]) -> Vec<String> {
    let mut kinds = Vec::new();
    for kind in parts.iter().map(|part| part.kind().badge().to_string()) {
        if !kinds.iter().any(|existing| existing == &kind) {
            kinds.push(kind);
        }
    }
    kinds
}
