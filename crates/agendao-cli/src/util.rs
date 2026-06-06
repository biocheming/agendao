#[cfg(feature = "run-core")]
use std::fs;
#[cfg(feature = "run-core")]
use std::io::{self, IsTerminal, Read};
#[cfg(feature = "run-core")]
use std::path::PathBuf;

#[cfg(feature = "run-core")]
use agendao_grep::Ripgrep;

#[cfg(feature = "run-remote-stream")]
pub(super) fn parse_bool_env(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(feature = "run-core")]
pub(super) fn append_cli_file_attachments(
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

#[cfg(feature = "run-core")]
pub(super) fn collect_run_input(message: Vec<String>) -> anyhow::Result<String> {
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

pub(super) fn truncate_text(input: &str, max_chars: usize) -> String {
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
