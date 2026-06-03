use super::{SchedulerSkillRef, SchedulerStageKind};
use crate::agent_tree::AgentTreeNode;
use crate::iterative_workflow::{IterativeWorkflowConfig, IterativeWorkflowSource};
use crate::scheduler::{AvailableAgentMeta, AvailableCategoryMeta};
use crate::skill_graph::SkillGraphDefinition;
use crate::skill_tree::SkillTreeRequestPlan;
use crate::ModelRef;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

// ─── Agent tree source ────────────────────────────────────────────────

/// An `agentTree` field can be specified inline or as a file path.
///
/// ```jsonc
/// // Inline:
/// "agentTree": { "agent": { "name": "deep-worker" }, "children": [...] }
///
/// // File path (resolved relative to the config file):
/// "agentTree": "./trees/coordinator-tree.json"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AgentTreeSource {
    /// Inline agent tree definition.
    Inline(AgentTreeNode),
    /// Path to an external JSON/JSONC file containing an `AgentTreeNode`.
    Path(String),
}

impl AgentTreeSource {
    /// Return the inline tree if present. Panics on unresolved paths —
    /// call `SchedulerConfig::resolve_agent_tree_paths()` first.
    pub fn as_inline(&self) -> Option<&AgentTreeNode> {
        match self {
            Self::Inline(tree) => Some(tree),
            Self::Path(_) => None,
        }
    }

    /// Extract the inline tree, consuming self.
    pub fn into_inline(self) -> Option<AgentTreeNode> {
        match self {
            Self::Inline(tree) => Some(tree),
            Self::Path(_) => None,
        }
    }

    /// True if this source has been resolved to an inline tree.
    pub fn is_inline(&self) -> bool {
        matches!(self, Self::Inline(_))
    }

    /// True if this source is still an unresolved path.
    pub fn is_path(&self) -> bool {
        matches!(self, Self::Path(_))
    }
}

// ─── Per-stage override ────────────────────────────────────────────────

/// An entry in the `stages` array: either a plain stage-kind string
/// (`"plan"`) or an object with per-stage overrides.
///
/// ```jsonc
/// "stages": [
///   "request-analysis",                  // plain string
///   {                                    // object with overrides
///     "kind": "execution-orchestration",
///     "toolPolicy": "allow-all",
///     "loopBudget": "step-limit:10",
///     "agentTree": { "agent": { "name": "coordinator" } }
///   }
/// ]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StageEntry {
    /// Shorthand: just the stage kind as a kebab-case string.
    Plain(SchedulerStageKind),
    /// Long form: stage kind plus optional per-stage overrides.
    Override(Box<SchedulerStageOverride>),
}

impl StageEntry {
    /// Extract the stage kind regardless of variant.
    pub fn kind(&self) -> SchedulerStageKind {
        match self {
            Self::Plain(kind) => *kind,
            Self::Override(o) => o.kind,
        }
    }

    /// Return the override if present, `None` for plain entries.
    pub fn as_override(&self) -> Option<&SchedulerStageOverride> {
        match self {
            Self::Plain(_) => None,
            Self::Override(o) => Some(o),
        }
    }
}

/// Per-stage policy overrides that users can set in JSON.
///
/// Every field except `kind` is optional — omitted fields fall through to
/// the preset → hardcoded-default chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerStageOverride {
    pub kind: SchedulerStageKind,

    #[serde(default, alias = "toolPolicy", skip_serializing_if = "Option::is_none")]
    pub tool_policy: Option<StageToolPolicyOverride>,

    #[serde(default, alias = "loopBudget", skip_serializing_if = "Option::is_none")]
    pub loop_budget: Option<String>,

    #[serde(
        default,
        alias = "sessionProjection",
        skip_serializing_if = "Option::is_none"
    )]
    pub session_projection: Option<String>,

    #[serde(default, alias = "agentTree", skip_serializing_if = "Option::is_none")]
    pub agent_tree: Option<AgentTreeSource>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<String>,

    #[serde(default, alias = "skillList", skip_serializing_if = "Vec::is_empty")]
    pub skill_list: Vec<SchedulerSkillRef>,
}

/// JSON-friendly tool-policy values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StageToolPolicyOverride {
    AllowAll,
    AllowReadOnly,
    DisableAll,
}

#[derive(Debug, thiserror::Error)]
pub enum SchedulerConfigError {
    #[error("failed to read scheduler config: {0}")]
    Read(#[from] std::io::Error),

    #[error("failed to parse scheduler config as jsonc: {0}")]
    Parse(String),

    #[error("failed to deserialize scheduler config: {0}")]
    Deserialize(#[from] serde_json::Error),

    #[error("scheduler profile not found: {0}")]
    ProfileNotFound(String),

    #[error("unsupported scheduler orchestrator: {0}")]
    UnknownOrchestrator(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SchedulerConfig {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub defaults: Option<SchedulerDefaults>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub profiles: HashMap<String, SchedulerProfileConfig>,
}

impl SchedulerConfig {
    pub fn load_from_str(content: &str) -> Result<Self, SchedulerConfigError> {
        let parse_options = jsonc_parser::ParseOptions {
            allow_trailing_commas: true,
            ..Default::default()
        };
        let value = jsonc_parser::parse_to_serde_value(content, &parse_options)
            .map_err(|err| SchedulerConfigError::Parse(err.to_string()))?
            .ok_or_else(|| SchedulerConfigError::Parse("empty scheduler config".to_string()))?;

        let config: Self = serde_json::from_value(value)?;
        config.validate_inline_workflows()?;
        Ok(config)
    }

    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self, SchedulerConfigError> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)?;
        let mut config = Self::load_from_str(&content)?;
        // Resolve any `agentTree` file paths relative to the config file's directory.
        let base_dir = path.parent().unwrap_or(Path::new("."));
        config.resolve_agent_tree_paths(base_dir)?;
        config.resolve_workflow_paths(base_dir)?;
        Ok(config)
    }

    /// Walk all profiles and stages, resolving `AgentTreeSource::Path` entries
    /// by loading the referenced JSON/JSONC file relative to `base_dir`.
    pub fn resolve_agent_tree_paths(
        &mut self,
        base_dir: &Path,
    ) -> Result<(), SchedulerConfigError> {
        for profile in self.profiles.values_mut() {
            // Resolve profile-level agentTree.
            if let Some(source) = profile.agent_tree.take() {
                profile.agent_tree = Some(Self::resolve_agent_tree_source(source, base_dir)?);
            }
            // Resolve per-stage agentTree overrides.
            for entry in &mut profile.stages {
                if let StageEntry::Override(ref mut o) = entry {
                    if let Some(source) = o.agent_tree.take() {
                        o.agent_tree = Some(Self::resolve_agent_tree_source(source, base_dir)?);
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_inline_workflows(&self) -> Result<(), SchedulerConfigError> {
        for profile in self.profiles.values() {
            if let Some(source) = profile.workflow.as_ref() {
                if let Some(config) = source.as_inline() {
                    config
                        .validate()
                        .map_err(|err| SchedulerConfigError::Parse(err.to_string()))?;
                }
            }
        }
        Ok(())
    }

    pub fn resolve_workflow_paths(&mut self, base_dir: &Path) -> Result<(), SchedulerConfigError> {
        for profile in self.profiles.values_mut() {
            if let Some(source) = profile.workflow.take() {
                profile.workflow = Some(Self::resolve_workflow_source(source, base_dir)?);
            }
        }
        Ok(())
    }

    /// Resolve a single `AgentTreeSource`. Inline sources pass through;
    /// path sources load the referenced file.
    fn resolve_agent_tree_source(
        source: AgentTreeSource,
        base_dir: &Path,
    ) -> Result<AgentTreeSource, SchedulerConfigError> {
        match source {
            AgentTreeSource::Inline(_) => Ok(source),
            AgentTreeSource::Path(ref rel_path) => {
                let abs_path = base_dir.join(rel_path);
                let content = fs::read_to_string(&abs_path).map_err(|e| {
                    SchedulerConfigError::Parse(format!(
                        "failed to load agentTree from {}: {}",
                        abs_path.display(),
                        e
                    ))
                })?;
                let parse_options = jsonc_parser::ParseOptions {
                    allow_trailing_commas: true,
                    ..Default::default()
                };
                let value = jsonc_parser::parse_to_serde_value(&content, &parse_options)
                    .map_err(|err| {
                        SchedulerConfigError::Parse(format!(
                            "failed to parse agentTree file {}: {}",
                            abs_path.display(),
                            err
                        ))
                    })?
                    .ok_or_else(|| {
                        SchedulerConfigError::Parse(format!(
                            "agentTree file {} is empty",
                            abs_path.display()
                        ))
                    })?;
                let tree: AgentTreeNode = serde_json::from_value(value).map_err(|e| {
                    SchedulerConfigError::Parse(format!(
                        "failed to deserialize agentTree from {}: {}",
                        abs_path.display(),
                        e
                    ))
                })?;
                Ok(AgentTreeSource::Inline(tree))
            }
        }
    }

    fn resolve_workflow_source(
        source: IterativeWorkflowSource,
        base_dir: &Path,
    ) -> Result<IterativeWorkflowSource, SchedulerConfigError> {
        match source {
            IterativeWorkflowSource::Inline(config) => {
                config
                    .validate()
                    .map_err(|err| SchedulerConfigError::Parse(err.to_string()))?;
                Ok(IterativeWorkflowSource::Inline(config))
            }
            IterativeWorkflowSource::Path(rel_path) => {
                let abs_path = base_dir.join(&rel_path);
                let config = IterativeWorkflowConfig::load_from_file(&abs_path).map_err(|err| {
                    SchedulerConfigError::Parse(format!(
                        "failed to load workflow from {}: {}",
                        abs_path.display(),
                        err
                    ))
                })?;
                Ok(IterativeWorkflowSource::Inline(config))
            }
        }
    }

    pub fn profile(&self, key: &str) -> Result<&SchedulerProfileConfig, SchedulerConfigError> {
        self.profiles
            .get(key)
            .ok_or_else(|| SchedulerConfigError::ProfileNotFound(key.to_string()))
    }

    pub fn default_profile_key(&self) -> Option<&str> {
        self.defaults.as_ref()?.profile.as_deref()
    }

    pub fn default_profile(&self) -> Result<&SchedulerProfileConfig, SchedulerConfigError> {
        let key = self
            .default_profile_key()
            .ok_or_else(|| SchedulerConfigError::ProfileNotFound("<default>".to_string()))?;
        self.profile(key)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SchedulerDefaults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SchedulerProfileConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orchestrator: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelRef>,

    #[serde(default, alias = "skillList", skip_serializing_if = "Vec::is_empty")]
    pub skill_list: Vec<SchedulerSkillRef>,

    /// Stage sequence. Each entry is either a plain stage-kind string
    /// (`"plan"`) or an object with per-stage overrides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<StageEntry>,

    #[serde(default, alias = "agentTree", skip_serializing_if = "Option::is_none")]
    pub agent_tree: Option<AgentTreeSource>,

    #[serde(default, alias = "skillGraph", skip_serializing_if = "Option::is_none")]
    pub skill_graph: Option<SkillGraphDefinition>,

    #[serde(default, alias = "skillTree", skip_serializing_if = "Option::is_none")]
    pub skill_tree: Option<SkillTreeRequestPlan>,

    #[serde(
        default,
        alias = "workflowPath",
        alias = "iterativeWorkflow",
        skip_serializing_if = "Option::is_none"
    )]
    pub workflow: Option<IterativeWorkflowSource>,

    #[serde(
        default,
        alias = "availableAgents",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub available_agents: Vec<AvailableAgentMeta>,

    #[serde(
        default,
        alias = "availableCategories",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub available_categories: Vec<AvailableCategoryMeta>,
}

impl SchedulerProfileConfig {
    /// Extract the flat list of stage kinds (ignoring overrides).
    pub fn stage_kinds(&self) -> Vec<SchedulerStageKind> {
        self.stages.iter().map(|entry| entry.kind()).collect()
    }

    /// Collect per-stage overrides into a map keyed by stage kind.
    pub fn stage_overrides(&self) -> HashMap<SchedulerStageKind, &SchedulerStageOverride> {
        self.stages
            .iter()
            .filter_map(|entry| entry.as_override().map(|o| (o.kind, o)))
            .collect()
    }

    pub fn workflow(&self) -> Option<&IterativeWorkflowConfig> {
        self.workflow.as_ref()?.as_inline()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_config_parses_jsonc_profile() {
        let content = r#"
        {
          // comment
          "$schema": "https://agendao.dev/schemas/scheduler-profile.schema.json",
          "defaults": { "profile": "prometheus-default" },
          "profiles": {
            "prometheus-default": {
              "orchestrator": "prometheus",
              "model": {
                "providerId": "ethnopic",
                "modelId": "test-model-reasoning"
              },
              "skillList": ["request-analysis", "plan", "delegation"],
              "stages": ["request-analysis", "plan", "delegation", "review", "synthesis"],
              "skillTree": {
                "contextMarkdown": "Article 1: one execution kernel",
                "tokenBudget": 256,
                "truncationStrategy": "tail"
              },
              "agentTree": {
                "agent": { "name": "deep-worker" }
              },
              "skillGraph": {
                "entryNodeId": "review",
                "nodes": [
                  {
                    "id": "review",
                    "agent": { "name": "architecture-advisor" }
                  }
                ]
              }
            }
          }
        }
        "#;

        let config = SchedulerConfig::load_from_str(content).unwrap();
        let profile = config.default_profile().unwrap();
        assert_eq!(profile.orchestrator.as_deref(), Some("prometheus"));
        assert_eq!(
            profile
                .model
                .as_ref()
                .map(|model| model.provider_id.as_str()),
            Some("ethnopic")
        );
        assert_eq!(
            profile.model.as_ref().map(|model| model.model_id.as_str()),
            Some("test-model-reasoning")
        );
        assert_eq!(profile.skill_list.len(), 3);
        assert_eq!(profile.stages.len(), 5);
        assert!(profile.agent_tree.is_some());
        assert!(profile.skill_graph.is_some());
        assert_eq!(
            profile
                .skill_tree
                .as_ref()
                .map(|tree| tree.context_markdown.as_str()),
            Some("Article 1: one execution kernel")
        );
        assert_eq!(
            profile
                .skill_tree
                .as_ref()
                .and_then(|tree| tree.token_budget),
            Some(256)
        );
        assert_eq!(
            profile
                .skill_tree
                .as_ref()
                .map(|tree| tree.truncation_strategy),
            Some(crate::skill_tree::SkillTreeTruncationStrategy::Tail)
        );
    }

    #[test]
    fn scheduler_config_handles_empty_profiles() {
        let config = SchedulerConfig::load_from_str("{}").unwrap();
        assert!(config.defaults.is_none());
        assert!(config.profiles.is_empty());
    }

    // ── Per-stage override deserialization ──

    #[test]
    fn stages_mixed_plain_and_override() {
        let content = r#"
        {
          "defaults": { "profile": "custom" },
          "profiles": {
            "custom": {
              "orchestrator": "sisyphus",
              "stages": [
                "request-analysis",
                {
                  "kind": "plan",
                  "toolPolicy": "allow-all",
                  "loopBudget": "step-limit:5",
                  "sessionProjection": "hidden"
                },
                "execution-orchestration",
                "synthesis"
              ]
            }
          }
        }
        "#;

        let config = SchedulerConfig::load_from_str(content).unwrap();
        let profile = config.default_profile().unwrap();

        // 4 stage entries
        assert_eq!(profile.stages.len(), 4);

        // Flat stage_kinds extraction
        assert_eq!(
            profile.stage_kinds(),
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::Plan,
                SchedulerStageKind::ExecutionOrchestration,
                SchedulerStageKind::Synthesis,
            ]
        );

        // Only one override (the plan stage)
        let overrides = profile.stage_overrides();
        assert_eq!(overrides.len(), 1);

        let plan_override = overrides.get(&SchedulerStageKind::Plan).unwrap();
        assert_eq!(
            plan_override.tool_policy,
            Some(StageToolPolicyOverride::AllowAll)
        );
        assert_eq!(plan_override.loop_budget.as_deref(), Some("step-limit:5"));
        assert_eq!(plan_override.session_projection.as_deref(), Some("hidden"));
    }

    #[test]
    fn stages_override_with_agent_tree() {
        let content = r#"
        {
          "defaults": { "profile": "custom" },
          "profiles": {
            "custom": {
              "stages": [
                {
                  "kind": "execution-orchestration",
                  "agentTree": {
                    "agent": { "name": "coordinator" },
                    "children": [
                      { "agent": { "name": "worker-a" }, "prompt": "Do A" },
                      { "agent": { "name": "worker-b" }, "prompt": "Do B" }
                    ]
                  }
                }
              ]
            }
          }
        }
        "#;

        let config = SchedulerConfig::load_from_str(content).unwrap();
        let profile = config.default_profile().unwrap();

        assert_eq!(
            profile.stage_kinds(),
            vec![SchedulerStageKind::ExecutionOrchestration]
        );

        let overrides = profile.stage_overrides();
        let exec_override = overrides
            .get(&SchedulerStageKind::ExecutionOrchestration)
            .unwrap();
        let tree = exec_override
            .agent_tree
            .as_ref()
            .unwrap()
            .as_inline()
            .unwrap();
        assert_eq!(tree.agent.name, "coordinator");
        assert_eq!(tree.children.len(), 2);
        assert_eq!(tree.children[0].agent.name, "worker-a");
        assert_eq!(tree.children[1].agent.name, "worker-b");
    }

    #[test]
    fn stages_plain_strings_backward_compatible() {
        let content = r#"
        {
          "profiles": {
            "basic": {
              "orchestrator": "sisyphus",
              "stages": ["request-analysis", "route", "execution-orchestration"]
            }
          }
        }
        "#;

        let config = SchedulerConfig::load_from_str(content).unwrap();
        let profile = config.profile("basic").unwrap();

        assert_eq!(
            profile.stage_kinds(),
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::Route,
                SchedulerStageKind::ExecutionOrchestration,
            ]
        );
        assert!(profile.stage_overrides().is_empty());
    }

    #[test]
    fn stage_override_partial_fields() {
        let content = r#"
        {
          "profiles": {
            "partial": {
              "stages": [
                { "kind": "review", "toolPolicy": "disable-all" }
              ]
            }
          }
        }
        "#;

        let config = SchedulerConfig::load_from_str(content).unwrap();
        let profile = config.profile("partial").unwrap();

        let overrides = profile.stage_overrides();
        let review = overrides.get(&SchedulerStageKind::Review).unwrap();
        assert_eq!(
            review.tool_policy,
            Some(StageToolPolicyOverride::DisableAll)
        );
        // Omitted fields are None
        assert!(review.loop_budget.is_none());
        assert!(review.session_projection.is_none());
        assert!(review.agent_tree.is_none());
        assert!(review.agents.is_empty());
        assert!(review.skill_list.is_empty());
    }

    #[test]
    fn stage_tool_policy_override_serde_roundtrip() {
        let cases = [
            (StageToolPolicyOverride::AllowAll, "\"allow-all\""),
            (
                StageToolPolicyOverride::AllowReadOnly,
                "\"allow-read-only\"",
            ),
            (StageToolPolicyOverride::DisableAll, "\"disable-all\""),
        ];
        for (policy, expected_json) in cases {
            let json = serde_json::to_string(&policy).unwrap();
            assert_eq!(json, expected_json);
            let back: StageToolPolicyOverride = serde_json::from_str(&json).unwrap();
            assert_eq!(policy, back);
        }
    }

    // ── AgentTreeSource file path loading ──

    #[test]
    fn agent_tree_source_inline_parses_from_object() {
        let content = r#"
        {
          "profiles": {
            "test": {
              "orchestrator": "sisyphus",
              "agentTree": {
                "agent": { "name": "deep-worker" },
                "children": [
                  { "agent": { "name": "explorer" }, "prompt": "Explore." }
                ]
              }
            }
          }
        }
        "#;

        let config = SchedulerConfig::load_from_str(content).unwrap();
        let profile = config.profile("test").unwrap();
        let source = profile.agent_tree.as_ref().unwrap();
        assert!(source.is_inline());
        let tree = source.as_inline().unwrap();
        assert_eq!(tree.agent.name, "deep-worker");
        assert_eq!(tree.children.len(), 1);
        assert_eq!(tree.children[0].agent.name, "explorer");
    }

    #[test]
    fn agent_tree_source_path_parses_from_string() {
        let content = r#"
        {
          "profiles": {
            "test": {
              "orchestrator": "sisyphus",
              "agentTree": "./trees/coordinator.json"
            }
          }
        }
        "#;

        let config = SchedulerConfig::load_from_str(content).unwrap();
        let profile = config.profile("test").unwrap();
        let source = profile.agent_tree.as_ref().unwrap();
        assert!(source.is_path());
        match source {
            AgentTreeSource::Path(p) => assert_eq!(p, "./trees/coordinator.json"),
            _ => panic!("expected Path variant"),
        }
    }

    #[test]
    fn agent_tree_source_path_in_stage_override() {
        let content = r#"
        {
          "profiles": {
            "test": {
              "stages": [
                {
                  "kind": "execution-orchestration",
                  "agentTree": "./trees/exec-tree.json"
                }
              ]
            }
          }
        }
        "#;

        let config = SchedulerConfig::load_from_str(content).unwrap();
        let profile = config.profile("test").unwrap();
        let overrides = profile.stage_overrides();
        let exec = overrides
            .get(&SchedulerStageKind::ExecutionOrchestration)
            .unwrap();
        let source = exec.agent_tree.as_ref().unwrap();
        assert!(source.is_path());
        match source {
            AgentTreeSource::Path(p) => assert_eq!(p, "./trees/exec-tree.json"),
            _ => panic!("expected Path variant"),
        }
    }

    #[test]
    fn resolve_agent_tree_paths_loads_external_file() {
        let dir = std::env::temp_dir().join("agendao_test_agent_tree_resolve");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Write an agent tree JSON file
        let tree_path = dir.join("my-tree.json");
        let tree_json = r#"{
            "agent": { "name": "loaded-coordinator" },
            "children": [
                { "agent": { "name": "loaded-worker" }, "prompt": "Do work." }
            ]
        }"#;
        std::fs::write(&tree_path, tree_json).unwrap();

        // Write a scheduler config referencing the tree
        let config_path = dir.join("scheduler.jsonc");
        let config_json = r#"{
            "profiles": {
                "test": {
                    "orchestrator": "sisyphus",
                    "agentTree": "./my-tree.json",
                    "stages": [
                        {
                            "kind": "execution-orchestration",
                            "agentTree": "./my-tree.json"
                        }
                    ]
                }
            }
        }"#;
        std::fs::write(&config_path, config_json).unwrap();

        let config = SchedulerConfig::load_from_file(&config_path).unwrap();
        let profile = config.profile("test").unwrap();

        // Profile-level tree is resolved
        let source = profile.agent_tree.as_ref().unwrap();
        assert!(source.is_inline());
        let tree = source.as_inline().unwrap();
        assert_eq!(tree.agent.name, "loaded-coordinator");
        assert_eq!(tree.children.len(), 1);
        assert_eq!(tree.children[0].agent.name, "loaded-worker");

        // Per-stage tree is also resolved
        let overrides = profile.stage_overrides();
        let exec = overrides
            .get(&SchedulerStageKind::ExecutionOrchestration)
            .unwrap();
        let stage_tree = exec.agent_tree.as_ref().unwrap().as_inline().unwrap();
        assert_eq!(stage_tree.agent.name, "loaded-coordinator");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_agent_tree_paths_errors_on_missing_file() {
        let dir = std::env::temp_dir().join("agendao_test_agent_tree_missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let config_path = dir.join("scheduler.jsonc");
        let config_json = r#"{
            "profiles": {
                "test": {
                    "agentTree": "./nonexistent.json"
                }
            }
        }"#;
        std::fs::write(&config_path, config_json).unwrap();

        let result = SchedulerConfig::load_from_file(&config_path);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("nonexistent.json"),
            "error should mention the file: {err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_agent_tree_paths_supports_jsonc() {
        let dir = std::env::temp_dir().join("agendao_test_agent_tree_jsonc");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // JSONC with comments and trailing commas
        let tree_path = dir.join("tree.jsonc");
        let tree_jsonc = r#"{
            // root agent
            "agent": { "name": "jsonc-agent" },
        }"#;
        std::fs::write(&tree_path, tree_jsonc).unwrap();

        let config_path = dir.join("scheduler.jsonc");
        let config_json = r#"{
            "profiles": {
                "test": {
                    "agentTree": "./tree.jsonc"
                }
            }
        }"#;
        std::fs::write(&config_path, config_json).unwrap();

        let config = SchedulerConfig::load_from_file(&config_path).unwrap();
        let profile = config.profile("test").unwrap();
        let tree = profile.agent_tree.as_ref().unwrap().as_inline().unwrap();
        assert_eq!(tree.agent.name, "jsonc-agent");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scheduler_config_parses_inline_workflow() {
        let content = r#"
        {
          "profiles": {
            "test": {
              "workflow": {
                "workflow": { "kind": "autoresearch", "mode": "plan" }
              }
            }
          }
        }
        "#;

        let config = SchedulerConfig::load_from_str(content).unwrap();
        let profile = config.profile("test").unwrap();
        let workflow = profile.workflow().unwrap();
        assert_eq!(workflow.workflow.mode.as_str(), "plan");
    }

    #[test]
    fn resolve_workflow_paths_loads_external_file() {
        let dir = std::env::temp_dir().join("agendao_test_workflow_resolve");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let workflow_path = dir.join("workflow.jsonc");
        let workflow_json = r#"{
            "workflow": { "kind": "autoresearch", "mode": "run" },
            "objective": {
                "goal": "Improve coverage",
                "scope": { "include": ["src/**/*.rs"] },
                "direction": "higher-is-better",
                "metric": { "kind": "numeric-extract", "pattern": "([0-9]+)" },
                "verify": { "command": "cargo test" }
            },
            "iterationPolicy": { "mode": "bounded", "maxIterations": 3 },
            "decisionPolicy": {},
            "workspacePolicy": { "snapshotStrategy": "patch-file" }
        }"#;
        std::fs::write(&workflow_path, workflow_json).unwrap();

        let config_path = dir.join("scheduler.jsonc");
        let config_json = r#"{
            "profiles": {
                "test": {
                    "workflowPath": "./workflow.jsonc"
                }
            }
        }"#;
        std::fs::write(&config_path, config_json).unwrap();

        let config = SchedulerConfig::load_from_file(&config_path).unwrap();
        let profile = config.profile("test").unwrap();
        let source = profile.workflow.as_ref().unwrap();
        assert!(source.is_inline());
        assert_eq!(profile.workflow().unwrap().workflow.mode.as_str(), "run");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
