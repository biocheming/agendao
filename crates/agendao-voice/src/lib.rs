use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use agendao_config::{VoiceCommandConfig, VoiceConfig};
use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;

#[derive(Debug, Clone)]
pub struct VoiceCaptureOptions {
    pub config: Option<VoiceConfig>,
}

#[derive(Debug, Clone)]
pub struct VoiceAttachment {
    pub filename: String,
    pub mime: String,
    pub data_url: String,
    pub bytes: usize,
}

#[derive(Debug, Clone)]
pub struct VoiceCaptureResult {
    pub transcript: Option<String>,
    pub attachment: Option<VoiceAttachment>,
    pub duration_seconds: u64,
    pub recorder_label: String,
    pub transcriber_label: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedCommand {
    label: String,
    command: Vec<String>,
    env: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct CommandContext {
    file: PathBuf,
    duration_seconds: u64,
    language: Option<String>,
    mime: String,
}

fn cleanup_voice_file(file: &std::path::Path, reason: &str) {
    if let Err(error) = fs::remove_file(file) {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(
                path = %file.display(),
                %error,
                reason,
                "failed to remove voice temp file"
            );
        }
    }
}

pub fn capture_voice(options: VoiceCaptureOptions) -> Result<VoiceCaptureResult> {
    let config = options.config.unwrap_or_default();
    let duration_seconds = config.duration_seconds.unwrap_or(15).max(1);
    let mime = config
        .mime
        .clone()
        .unwrap_or_else(|| "audio/wav".to_string());
    let extension = extension_for_mime(&mime);
    let file = std::env::temp_dir().join(format!(
        "agendao-voice-{}.{}",
        uuid::Uuid::new_v4(),
        extension
    ));
    let command_context = CommandContext {
        file: file.clone(),
        duration_seconds,
        language: config.language.clone(),
        mime: mime.clone(),
    };

    let recorder = run_recorder_command(config.record.as_ref(), &command_context)?;

    let metadata = fs::metadata(&file).with_context(|| {
        format!(
            "voice recorder completed but no audio file was produced at {}",
            file.display()
        )
    })?;
    if metadata.len() == 0 {
        cleanup_voice_file(&file, "empty recording");
        return Err(anyhow!(
            "voice recorder completed but produced an empty audio file"
        ));
    }

    let transcript = if let Some(transcribe) = config.transcribe.as_ref() {
        let transcriber = resolve_custom_command(transcribe, &command_context)
            .context("invalid voice transcribe command")?;
        let output = run_command(&transcriber, true)
            .with_context(|| format!("voice transcription failed via {}", transcriber.label))?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            cleanup_voice_file(&file, "empty transcript");
            return Err(anyhow!(
                "voice transcribe command completed but returned an empty transcript"
            ));
        }
        Some((stdout, transcriber.label))
    } else {
        None
    };

    let attachment = if config.attach_audio.unwrap_or(true) {
        let bytes = fs::read(&file)
            .with_context(|| format!("failed to read recorded audio file {}", file.display()))?;
        Some(VoiceAttachment {
            filename: format!("voice-input.{}", extension),
            mime: mime.clone(),
            data_url: format!(
                "data:{};base64,{}",
                mime,
                BASE64_STANDARD.encode(bytes.as_slice())
            ),
            bytes: bytes.len(),
        })
    } else {
        None
    };

    cleanup_voice_file(&file, "capture complete");

    if attachment.is_none() && transcript.is_none() {
        return Err(anyhow!(
            "voice capture produced neither transcript nor audio attachment"
        ));
    }

    Ok(VoiceCaptureResult {
        transcript: transcript.as_ref().map(|value| value.0.clone()),
        attachment,
        duration_seconds,
        recorder_label: recorder.label,
        transcriber_label: transcript.map(|value| value.1),
    })
}

fn run_recorder_command(
    configured: Option<&VoiceCommandConfig>,
    context: &CommandContext,
) -> Result<ResolvedCommand> {
    if let Some(configured) = configured {
        let command =
            resolve_custom_command(configured, context).context("invalid voice record command")?;
        run_command(&command, false)
            .with_context(|| format!("voice record failed via {}", command.label))?;
        return Ok(command);
    }

    let mut attempted = Vec::new();
    for candidate in default_recorders() {
        if which::which(candidate.program).is_ok() {
            let command = ResolvedCommand {
                label: candidate.label.to_string(),
                command: expand_tokens(candidate.command, context),
                env: HashMap::new(),
            };
            match run_command(&command, false) {
                Ok(_) => return Ok(command),
                Err(error) => {
                    cleanup_voice_file(&context.file, "recorder attempt failed");
                    attempted.push(format!("{}: {}", command.label, error));
                }
            }
        }
    }

    if attempted.is_empty() {
        Err(anyhow!(
            "no recorder available; configure `voice.record.command` or install one of: ffmpeg, rec, sox, arecord"
        ))
    } else {
        Err(anyhow!(
            "no autodetected recorder succeeded: {}",
            attempted.join(" | ")
        ))
    }
}

fn resolve_custom_command(
    configured: &VoiceCommandConfig,
    context: &CommandContext,
) -> Result<ResolvedCommand> {
    if configured.command.is_empty() {
        return Err(anyhow!("command must not be empty"));
    }

    Ok(ResolvedCommand {
        label: configured.command.join(" "),
        command: expand_tokens(configured.command.as_slice(), context),
        env: configured.env.clone(),
    })
}

fn expand_tokens<T: AsRef<str>>(parts: &[T], context: &CommandContext) -> Vec<String> {
    parts
        .iter()
        .map(|part| {
            part.as_ref()
                .replace("{file}", &context.file.display().to_string())
                .replace("{duration_seconds}", &context.duration_seconds.to_string())
                .replace(
                    "{language}",
                    context.language.as_deref().unwrap_or_default(),
                )
                .replace("{mime}", &context.mime)
        })
        .collect()
}

fn run_command(command: &ResolvedCommand, capture_stdout: bool) -> Result<Output> {
    let mut process = Command::new(
        command
            .command
            .first()
            .ok_or_else(|| anyhow!("command must not be empty"))?,
    );
    process.args(command.command.iter().skip(1));
    process.envs(&command.env);
    if capture_stdout {
        process.stdout(Stdio::piped());
    } else {
        process.stdout(Stdio::null());
    }
    process.stderr(Stdio::piped());

    let output = process
        .output()
        .with_context(|| format!("failed to spawn command `{}`", command.command.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            format!("command exited with status {}", output.status)
        } else {
            stderr
        };
        return Err(anyhow!(message));
    }

    Ok(output)
}

fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/ogg" => "ogg",
        "audio/flac" => "flac",
        "audio/webm" => "webm",
        _ => "wav",
    }
}

struct DefaultRecorder {
    program: &'static str,
    label: &'static str,
    command: &'static [&'static str],
}

fn default_recorders() -> &'static [DefaultRecorder] {
    &[
        DefaultRecorder {
            program: "ffmpeg",
            label: "ffmpeg (pulse)",
            command: &[
                "ffmpeg",
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "pulse",
                "-i",
                "default",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-t",
                "{duration_seconds}",
                "{file}",
            ],
        },
        DefaultRecorder {
            program: "rec",
            label: "rec (sox)",
            command: &[
                "rec",
                "-q",
                "-c",
                "1",
                "-r",
                "16000",
                "{file}",
                "trim",
                "0",
                "{duration_seconds}",
            ],
        },
        DefaultRecorder {
            program: "sox",
            label: "sox",
            command: &[
                "sox",
                "-q",
                "-d",
                "-c",
                "1",
                "-r",
                "16000",
                "{file}",
                "trim",
                "0",
                "{duration_seconds}",
            ],
        },
        DefaultRecorder {
            program: "arecord",
            label: "arecord",
            command: &[
                "arecord",
                "-q",
                "-f",
                "S16_LE",
                "-r",
                "16000",
                "-c",
                "1",
                "-d",
                "{duration_seconds}",
                "{file}",
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn expands_command_tokens() {
        let context = CommandContext {
            file: Path::new("/tmp/voice.wav").to_path_buf(),
            duration_seconds: 12,
            language: Some("zh".to_string()),
            mime: "audio/wav".to_string(),
        };
        let expanded = expand_tokens(
            &[
                "ffmpeg".to_string(),
                "{file}".to_string(),
                "{duration_seconds}".to_string(),
                "{language}".to_string(),
                "{mime}".to_string(),
            ],
            &context,
        );
        assert_eq!(
            expanded,
            vec![
                "ffmpeg".to_string(),
                "/tmp/voice.wav".to_string(),
                "12".to_string(),
                "zh".to_string(),
                "audio/wav".to_string()
            ]
        );
    }

    #[test]
    fn maps_mime_to_extension() {
        assert_eq!(extension_for_mime("audio/wav"), "wav");
        assert_eq!(extension_for_mime("audio/mpeg"), "mp3");
        assert_eq!(extension_for_mime("audio/webm"), "webm");
    }
}
