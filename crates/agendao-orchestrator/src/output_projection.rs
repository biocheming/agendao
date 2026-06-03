use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY: &str =
    "scheduler_output_projection_policy";
pub const SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY: &str = "scheduler_model_context_summary";
pub const SCHEDULER_OUTPUT_ARTIFACTS_METADATA_KEY: &str = "scheduler_output_artifacts";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextProjectionPolicy {
    Full,
    Summary,
    ReferenceOnly,
    Hidden,
    OnDemandArtifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactKind {
    Report,
    Evidence,
    Log,
    Trace,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactSectionRef {
    pub anchor: String,
    pub title: String,
    pub token_estimate: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub id: String,
    pub kind: ArtifactKind,
    pub summary: String,
    pub sections: Vec<ArtifactSectionRef>,
    pub token_estimate: Option<u64>,
    pub provenance: Vec<SourceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactLoadPolicy {
    SummaryOnly,
    SectionByAnchor(Vec<String>),
    EvidenceOnly,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantOutputProjection {
    pub visible_text: String,
    pub model_context_summary: String,
    pub artifacts: Vec<ArtifactRef>,
    pub model_context_policy: ContextProjectionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantOutputProjectionOptions {
    pub large_output_char_threshold: usize,
    pub summary_char_limit: usize,
}

impl Default for AssistantOutputProjectionOptions {
    fn default() -> Self {
        Self {
            large_output_char_threshold: 8_000,
            summary_char_limit: 1_200,
        }
    }
}

pub fn project_assistant_output(
    content: &str,
    options: &AssistantOutputProjectionOptions,
) -> AssistantOutputProjection {
    if content.chars().count() <= options.large_output_char_threshold {
        return AssistantOutputProjection {
            visible_text: content.to_string(),
            model_context_summary: content.to_string(),
            artifacts: Vec::new(),
            model_context_policy: ContextProjectionPolicy::Full,
        };
    }

    let artifact = ArtifactRef {
        id: artifact_id_for_content(content),
        kind: ArtifactKind::Report,
        summary: projection_summary(content, options.summary_char_limit),
        sections: Vec::new(),
        token_estimate: Some(estimate_tokens(content)),
        provenance: Vec::new(),
    };

    AssistantOutputProjection {
        visible_text: content.to_string(),
        model_context_summary: format!(
            "Large assistant output stored as artifact `{}`. Summary:\n{}",
            artifact.id, artifact.summary
        ),
        artifacts: vec![artifact],
        model_context_policy: ContextProjectionPolicy::OnDemandArtifact,
    }
}

pub fn append_assistant_output_projection(
    metadata: &mut HashMap<String, Value>,
    content: &str,
    options: &AssistantOutputProjectionOptions,
) {
    let projection = project_assistant_output(content, options);

    if let Ok(value) = serde_json::to_value(&projection.model_context_policy) {
        metadata.insert(
            SCHEDULER_OUTPUT_PROJECTION_POLICY_METADATA_KEY.to_string(),
            value,
        );
    }

    if projection.model_context_policy != ContextProjectionPolicy::Full {
        metadata.insert(
            SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY.to_string(),
            Value::String(projection.model_context_summary),
        );
    } else {
        metadata.remove(SCHEDULER_MODEL_CONTEXT_SUMMARY_METADATA_KEY);
    }

    if projection.artifacts.is_empty() {
        metadata.remove(SCHEDULER_OUTPUT_ARTIFACTS_METADATA_KEY);
    } else if let Ok(value) = serde_json::to_value(projection.artifacts) {
        metadata.insert(SCHEDULER_OUTPUT_ARTIFACTS_METADATA_KEY.to_string(), value);
    }
}

fn projection_summary(content: &str, limit: usize) -> String {
    let trimmed = content.trim();
    let mut summary = trimmed.chars().take(limit).collect::<String>();
    if trimmed.chars().count() > limit {
        summary.push_str("\n[truncated]");
    }
    summary
}

fn artifact_id_for_content(content: &str) -> String {
    format!("art_assistant_{:016x}", stable_content_hash(content))
}

fn stable_content_hash(content: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn estimate_tokens(content: &str) -> u64 {
    content.chars().count().div_ceil(4) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_assistant_output_remains_full_context() {
        let projection = project_assistant_output(
            "short answer",
            &AssistantOutputProjectionOptions {
                large_output_char_threshold: 20,
                summary_char_limit: 10,
            },
        );

        assert_eq!(projection.visible_text, "short answer");
        assert_eq!(projection.model_context_summary, "short answer");
        assert!(projection.artifacts.is_empty());
        assert_eq!(
            projection.model_context_policy,
            ContextProjectionPolicy::Full
        );
    }

    #[test]
    fn large_assistant_output_uses_artifact_projection() {
        let content = "0123456789abcdef0123456789abcdef";
        let projection = project_assistant_output(
            content,
            &AssistantOutputProjectionOptions {
                large_output_char_threshold: 12,
                summary_char_limit: 8,
            },
        );

        assert_eq!(projection.visible_text, content);
        assert_ne!(projection.model_context_summary, content);
        assert_eq!(
            projection.model_context_policy,
            ContextProjectionPolicy::OnDemandArtifact
        );
        assert_eq!(projection.artifacts.len(), 1);
        assert!(projection.model_context_summary.contains("01234567"));
        assert!(projection.model_context_summary.contains("[truncated]"));
    }
}
