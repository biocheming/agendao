use serde::Deserialize;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;

use agendao_grep::Ripgrep;

pub(crate) fn parse_model_and_provider(model: Option<String>) -> (Option<String>, Option<String>) {
    let Some(raw) = model else {
        return (None, None);
    };
    let raw = raw.trim().to_string();
    if let Some((provider, model_id)) = raw.split_once('/') {
        return (
            Some(provider.trim().to_string()),
            Some(model_id.trim().to_string()),
        );
    }
    if let Some((provider, model_id)) = raw.split_once(':') {
        return (
            Some(provider.trim().to_string()),
            Some(model_id.trim().to_string()),
        );
    }
    (None, Some(raw))
}

pub(crate) fn parse_bool_env(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub(crate) fn append_cli_file_attachments(
    input: &mut String,
    files: &[PathBuf],
    base_dir: &PathBuf,
) -> anyhow::Result<()> {
    for file_path in files {
        let resolved = if file_path.is_absolute() {
            file_path.clone()
        } else {
            base_dir.join(file_path)
        };
        let metadata = fs::metadata(&resolved).map_err(|e| {
            anyhow::anyhow!(
                "Failed to read attachment metadata {}: {}",
                resolved.display(),
                e
            )
        })?;
        let display = resolved
            .strip_prefix(base_dir)
            .unwrap_or(&resolved)
            .display()
            .to_string();

        if metadata.is_dir() {
            let tree = Ripgrep::tree(&resolved, Some(150)).unwrap_or_else(|_| {
                format!("(directory listing unavailable for {})", resolved.display())
            });
            input.push_str("\n\n[Attachment: directory ");
            input.push_str(&display);
            input.push_str("]\n");
            input.push_str(&tree);
            continue;
        }

        let bytes = fs::read(&resolved).map_err(|e| {
            anyhow::anyhow!("Failed to read attachment {}: {}", resolved.display(), e)
        })?;
        let mut text = String::from_utf8_lossy(&bytes).to_string();
        const MAX_ATTACHMENT_BYTES: usize = 120_000;
        if text.len() > MAX_ATTACHMENT_BYTES {
            text.truncate(MAX_ATTACHMENT_BYTES);
            text.push_str("\n\n[truncated]");
        }
        input.push_str("\n\n[Attachment: file ");
        input.push_str(&display);
        input.push_str("]\n```text\n");
        input.push_str(&text);
        if !text.ends_with('\n') {
            input.push('\n');
        }
        input.push_str("```");
    }
    Ok(())
}

pub(crate) fn collect_run_input(message: Vec<String>) -> anyhow::Result<String> {
    let mut input = message.join(" ");
    if !io::stdin().is_terminal() {
        let mut piped = String::new();
        io::stdin().read_to_string(&mut piped)?;
        if !piped.trim().is_empty() {
            if !input.trim().is_empty() {
                input.push('\n');
            }
            input.push_str(piped.trim_end());
        }
    }
    Ok(input)
}

pub(crate) fn truncate_text(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut out = String::new();
    for c in input.chars().take(max_chars.saturating_sub(2)) {
        out.push(c);
    }
    out.push_str("..");
    out
}

pub(crate) fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub(crate) fn server_url(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

pub(crate) async fn parse_http_json<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> anyhow::Result<T> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        anyhow::bail!("Request failed ({}): {}", status, body);
    }
    Ok(serde_json::from_str(&body)?)
}
