use base64::{engine::general_purpose::STANDARD, Engine};
use std::io::Write;
use std::process::{Command, Stdio};

pub(crate) struct Clipboard;

impl Clipboard {
    pub(crate) fn write_text(text: &str) -> anyhow::Result<()> {
        write_osc52(text);

        if cfg!(target_os = "macos") {
            return write_with_command("pbcopy", &[], text);
        }

        if cfg!(target_os = "windows") {
            return write_with_command(
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    "[Console]::InputEncoding = [System.Text.Encoding]::UTF8; Set-Clipboard -Value ([Console]::In.ReadToEnd())",
                ],
                text,
            );
        }

        if std::env::var("WAYLAND_DISPLAY").is_ok()
            && write_with_command("wl-copy", &[], text).is_ok()
        {
            return Ok(());
        }

        if write_with_command("xclip", &["-selection", "clipboard"], text).is_ok() {
            return Ok(());
        }

        if write_with_command("xsel", &["--clipboard", "--input"], text).is_ok() {
            return Ok(());
        }

        Ok(())
    }
}

fn write_osc52(text: &str) {
    let encoded = STANDARD.encode(text.as_bytes());
    let osc52 = format!("\x1b]52;c;{encoded}\x07");
    let sequence = if std::env::var("TMUX").is_ok() || std::env::var("STY").is_ok() {
        format!("\x1bPtmux;\x1b{osc52}\x1b\\")
    } else {
        osc52
    };

    let _ = std::io::stdout()
        .write_all(sequence.as_bytes())
        .and_then(|_| std::io::stdout().flush());
}

fn write_with_command(program: &str, args: &[&str], text: &str) -> anyhow::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            anyhow::anyhow!("failed to execute clipboard write command `{program}`: {error}")
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes()).map_err(|error| {
            anyhow::anyhow!("failed to write text to clipboard command stdin `{program}`: {error}")
        })?;
    }

    let status = child.wait().map_err(|error| {
        anyhow::anyhow!("failed waiting for clipboard command `{program}`: {error}")
    })?;
    if !status.success() {
        anyhow::bail!(
            "clipboard write command `{}` failed with status {}",
            program,
            status
        );
    }

    Ok(())
}
