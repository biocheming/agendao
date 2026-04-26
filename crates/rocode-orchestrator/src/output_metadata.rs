use crate::runtime::events::StepUsage;
use crate::workflow_artifacts::{WorkflowModeArtifact, WorkflowModeArtifactEntry};
use crate::workflow_mode::{mode_artifacts_from_metadata, WORKFLOW_MODE_ARTIFACTS_METADATA_KEY};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const CONTINUATION_TARGETS_METADATA_KEY: &str = "continuationTargets";
pub const OUTPUT_USAGE_METADATA_KEY: &str = "usage";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinuationTarget {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(
        rename = "agentTaskId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub agent_task_id: Option<String>,
    #[serde(rename = "toolName", default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    #[serde(default)]
    pub context_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
}

impl OutputUsage {
    pub fn is_zero(&self) -> bool {
        self.prompt_tokens == 0
            && self.completion_tokens == 0
            && self.context_tokens == 0
            && self.reasoning_tokens == 0
            && self.cache_read_tokens == 0
            && self.cache_write_tokens == 0
    }

    pub fn accumulate(&mut self, other: &OutputUsage) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.context_tokens = self
            .context_tokens
            .max(other.context_tokens.max(other.prompt_tokens));
        self.reasoning_tokens += other.reasoning_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_write_tokens += other.cache_write_tokens;
    }
}

impl From<StepUsage> for OutputUsage {
    fn from(value: StepUsage) -> Self {
        Self {
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            context_tokens: value.context_tokens.max(value.prompt_tokens),
            reasoning_tokens: value.reasoning_tokens,
            cache_read_tokens: value.cache_read_tokens,
            cache_write_tokens: value.cache_write_tokens,
        }
    }
}

impl From<&StepUsage> for OutputUsage {
    fn from(value: &StepUsage) -> Self {
        Self {
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            context_tokens: value.context_tokens.max(value.prompt_tokens),
            reasoning_tokens: value.reasoning_tokens,
            cache_read_tokens: value.cache_read_tokens,
            cache_write_tokens: value.cache_write_tokens,
        }
    }
}

impl ContinuationTarget {
    fn key(&self) -> (&str, Option<&str>, Option<&str>) {
        (
            self.session_id.as_str(),
            self.agent_task_id.as_deref(),
            self.tool_name.as_deref(),
        )
    }
}

pub fn continuation_target_from_tool_metadata(
    tool_name: &str,
    metadata: Option<&Value>,
) -> Option<ContinuationTarget> {
    let metadata = metadata?;
    let session_id = metadata
        .get("sessionId")
        .or_else(|| metadata.get("session_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    let agent_task_id = metadata
        .get("agentTaskId")
        .or_else(|| metadata.get("agent_task_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    Some(ContinuationTarget {
        session_id: session_id.to_string(),
        agent_task_id,
        tool_name: Some(tool_name.to_string()),
    })
}

pub fn continuation_targets(metadata: &HashMap<String, Value>) -> Vec<ContinuationTarget> {
    metadata
        .get(CONTINUATION_TARGETS_METADATA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<ContinuationTarget>>(value).ok())
        .unwrap_or_default()
}

pub fn output_usage(metadata: &HashMap<String, Value>) -> Option<OutputUsage> {
    metadata
        .get(OUTPUT_USAGE_METADATA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value::<OutputUsage>(value).ok())
}

pub fn append_output_usage(metadata: &mut HashMap<String, Value>, usage: &OutputUsage) {
    let mut merged = output_usage(metadata).unwrap_or_default();
    merged.accumulate(usage);
    if merged.is_zero() {
        metadata.remove(OUTPUT_USAGE_METADATA_KEY);
    } else if let Ok(value) = serde_json::to_value(merged) {
        metadata.insert(OUTPUT_USAGE_METADATA_KEY.to_string(), value);
    }
}

pub fn append_continuation_target(
    metadata: &mut HashMap<String, Value>,
    target: ContinuationTarget,
) {
    let mut merged = continuation_targets(metadata);
    if !merged.iter().any(|existing| existing.key() == target.key()) {
        merged.push(target);
    }
    if let Ok(value) = serde_json::to_value(merged) {
        metadata.insert(CONTINUATION_TARGETS_METADATA_KEY.to_string(), value);
    }
}

pub(crate) fn append_workflow_mode_artifacts(
    metadata: &mut HashMap<String, Value>,
    artifacts: &[WorkflowModeArtifact],
) {
    let mut merged = mode_artifacts_from_metadata(metadata);
    for artifact in artifacts {
        merge_workflow_mode_artifact(&mut merged, artifact);
    }
    if merged.is_empty() {
        metadata.remove(WORKFLOW_MODE_ARTIFACTS_METADATA_KEY);
    } else if let Ok(value) = serde_json::to_value(merged) {
        metadata.insert(WORKFLOW_MODE_ARTIFACTS_METADATA_KEY.to_string(), value);
    }
}

pub fn merge_output_metadata(target: &mut HashMap<String, Value>, source: &HashMap<String, Value>) {
    for continuation in continuation_targets(source) {
        append_continuation_target(target, continuation);
    }
    if let Some(usage) = output_usage(source) {
        append_output_usage(target, &usage);
    }
    let mode_artifacts = mode_artifacts_from_metadata(source);
    if !mode_artifacts.is_empty() {
        append_workflow_mode_artifacts(target, &mode_artifacts);
    }
}

fn merge_workflow_mode_artifact(
    merged: &mut Vec<WorkflowModeArtifact>,
    incoming: &WorkflowModeArtifact,
) {
    if let Some(existing) = merged
        .iter_mut()
        .find(|artifact| artifact.name == incoming.name)
    {
        if !incoming.description.trim().is_empty() {
            existing.description = incoming.description.clone();
        }
        for entry in &incoming.entries {
            merge_workflow_mode_entry(&mut existing.entries, entry);
        }
    } else {
        merged.push(incoming.clone());
    }
}

fn merge_workflow_mode_entry(
    entries: &mut Vec<WorkflowModeArtifactEntry>,
    incoming: &WorkflowModeArtifactEntry,
) {
    if let Some(existing) = entries.iter_mut().find(|entry| entry.key == incoming.key) {
        existing.iteration = incoming.iteration.or(existing.iteration);
        existing.status = incoming.status.clone();
        if !incoming.title.trim().is_empty() {
            existing.title = incoming.title.clone();
        }
        if !incoming.detail.trim().is_empty() {
            existing.detail = incoming.detail.clone();
        }
        for evidence in &incoming.evidence {
            if !existing.evidence.iter().any(|item| item == evidence) {
                existing.evidence.push(evidence.clone());
            }
        }
    } else {
        entries.push(incoming.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_output_metadata_accumulates_usage() {
        let mut target = HashMap::new();
        append_output_usage(
            &mut target,
            &OutputUsage {
                prompt_tokens: 10,
                completion_tokens: 4,
                context_tokens: 10,
                reasoning_tokens: 2,
                cache_read_tokens: 1,
                cache_write_tokens: 0,
            },
        );

        let mut source = HashMap::new();
        append_output_usage(
            &mut source,
            &OutputUsage {
                prompt_tokens: 7,
                completion_tokens: 3,
                context_tokens: 7,
                reasoning_tokens: 1,
                cache_read_tokens: 0,
                cache_write_tokens: 5,
            },
        );

        merge_output_metadata(&mut target, &source);
        let usage = output_usage(&target).expect("usage should exist");
        assert_eq!(
            usage,
            OutputUsage {
                prompt_tokens: 17,
                completion_tokens: 7,
                context_tokens: 10,
                reasoning_tokens: 3,
                cache_read_tokens: 1,
                cache_write_tokens: 5,
            }
        );
    }

    #[test]
    fn merge_output_metadata_merges_workflow_mode_artifacts() {
        let mut target = HashMap::from([(
            WORKFLOW_MODE_ARTIFACTS_METADATA_KEY.to_string(),
            json!([{
                "name": "finding-registry",
                "description": "base",
                "entries": [{
                    "iteration": 1,
                    "key": "active-finding",
                    "status": "open",
                    "title": "Open finding",
                    "detail": "base detail",
                    "evidence": ["attack-scenario"]
                }]
            }]),
        )]);
        let source = HashMap::from([(
            WORKFLOW_MODE_ARTIFACTS_METADATA_KEY.to_string(),
            json!([{
                "name": "finding-registry",
                "description": "updated",
                "entries": [
                    {
                        "iteration": 2,
                        "key": "active-finding",
                        "status": "verified",
                        "title": "Verified finding",
                        "detail": "updated detail",
                        "evidence": ["file-line"]
                    },
                    {
                        "iteration": 2,
                        "key": "coverage-review",
                        "status": "needs-evidence",
                        "title": "Coverage review",
                        "detail": "still incomplete",
                        "evidence": ["required-evidence"]
                    }
                ]
            }]),
        )]);

        merge_output_metadata(&mut target, &source);

        let artifacts = mode_artifacts_from_metadata(&target);
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].description, "updated");
        assert_eq!(artifacts[0].entries.len(), 2);
        let finding = artifacts[0]
            .entries
            .iter()
            .find(|entry| entry.key == "active-finding")
            .expect("active finding should exist");
        assert_eq!(finding.status, "verified");
        assert!(finding.evidence.contains(&"attack-scenario".to_string()));
        assert!(finding.evidence.contains(&"file-line".to_string()));
    }
}
