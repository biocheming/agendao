use agendao_config::{Config, VoiceCommandConfig, VoiceConfig};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedVoiceConfig {
    pub duration_seconds: u64,
    pub attach_audio: bool,
    pub mime: String,
    pub language: Option<String>,
    pub record: Option<VoiceCommandConfig>,
    pub transcribe: Option<VoiceCommandConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedMultimodalConfig {
    pub voice: ResolvedVoiceConfig,
    pub max_input_bytes: Option<usize>,
    pub max_attachments_per_prompt: Option<usize>,
    pub allow_audio_input: bool,
    pub allow_image_input: bool,
    pub allow_file_input: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultimodalConfigView {
    pub voice: ResolvedVoiceConfig,
    pub max_input_bytes: Option<usize>,
    pub max_attachments_per_prompt: Option<usize>,
    pub allow_audio_input: bool,
    pub allow_image_input: bool,
    pub allow_file_input: bool,
}

impl ResolvedMultimodalConfig {
    pub fn from_config(config: &Config) -> Self {
        let multimodal = config.multimodal.clone().unwrap_or_default();
        let voice = Self::merged_voice_config(config);
        Self {
            voice: ResolvedVoiceConfig {
                duration_seconds: voice.duration_seconds.unwrap_or(15).max(1),
                attach_audio: voice.attach_audio.unwrap_or(true),
                mime: voice.mime.unwrap_or_else(|| "audio/wav".to_string()),
                language: voice.language,
                record: voice.record,
                transcribe: voice.transcribe,
            },
            max_input_bytes: multimodal
                .limits
                .as_ref()
                .and_then(|limits| limits.max_input_bytes),
            max_attachments_per_prompt: multimodal
                .limits
                .as_ref()
                .and_then(|limits| limits.max_attachments_per_prompt),
            allow_audio_input: multimodal
                .policy
                .as_ref()
                .and_then(|policy| policy.allow_audio_input)
                .unwrap_or(true),
            allow_image_input: multimodal
                .policy
                .as_ref()
                .and_then(|policy| policy.allow_image_input)
                .unwrap_or(true),
            allow_file_input: multimodal
                .policy
                .as_ref()
                .and_then(|policy| policy.allow_file_input)
                .unwrap_or(true),
        }
    }

    pub fn merged_voice_config(config: &Config) -> VoiceConfig {
        let mut merged = config.voice.clone().unwrap_or_default();
        merge_voice_config(
            &mut merged,
            config
                .multimodal
                .clone()
                .and_then(|multimodal| multimodal.voice),
        );
        merged
    }
}

impl MultimodalConfigView {
    pub fn from_resolved(resolved: &ResolvedMultimodalConfig) -> Self {
        Self {
            voice: resolved.voice.clone(),
            max_input_bytes: resolved.max_input_bytes,
            max_attachments_per_prompt: resolved.max_attachments_per_prompt,
            allow_audio_input: resolved.allow_audio_input,
            allow_image_input: resolved.allow_image_input,
            allow_file_input: resolved.allow_file_input,
        }
    }
}

fn merge_voice_config(target: &mut VoiceConfig, overlay: Option<VoiceConfig>) {
    let Some(overlay) = overlay else {
        return;
    };

    if overlay.duration_seconds.is_some() {
        target.duration_seconds = overlay.duration_seconds;
    }
    if overlay.attach_audio.is_some() {
        target.attach_audio = overlay.attach_audio;
    }
    if overlay.mime.is_some() {
        target.mime = overlay.mime;
    }
    if overlay.language.is_some() {
        target.language = overlay.language;
    }
    merge_voice_command(&mut target.record, overlay.record);
    merge_voice_command(&mut target.transcribe, overlay.transcribe);
}

fn merge_voice_command(
    target: &mut Option<VoiceCommandConfig>,
    overlay: Option<VoiceCommandConfig>,
) {
    let Some(overlay) = overlay else {
        return;
    };

    let target_command = target.get_or_insert_with(VoiceCommandConfig::default);
    if !overlay.command.is_empty() {
        target_command.command = overlay.command;
    }
    for (key, value) in overlay.env {
        target_command.env.insert(key, value);
    }
}
