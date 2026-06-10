use std::path::Path;
use std::str::FromStr;

use super::{
    scheduler_preset_definition, SchedulerConfig, SchedulerConfigError, SchedulerPresetDefinition,
    SchedulerProfileConfig, SchedulerProfileOrchestrator, SchedulerProfilePlan, SchedulerStageKind,
    SchedulerStageObservability, SchedulerStageOverride, StageEntry, StageToolPolicyOverride,
};
use crate::iterative_workflow::{IterativeWorkflowMode, WorkflowBasePreset};
use crate::skill_tree::SkillTreeRequestPlan;
use crate::tool_runner::ToolRunner;

#[derive(Debug, Clone, Default)]
pub struct SchedulerRequestDefaults {
    pub profile_name: Option<String>,
    pub root_agent_name: Option<String>,
    pub skill_tree_plan: Option<SkillTreeRequestPlan>,
}

pub const AUTO_SCHEDULER_PROFILE_NAME: &str = "auto";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerPresetMetadata {
    pub public: bool,
    pub router_recommended: bool,
    pub deprecated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerPresetKind {
    Sisyphus,
    Prometheus,
    Atlas,
    Hephaestus,
    Verifier,
}

const ALL_SCHEDULER_PRESETS: [SchedulerPresetKind; 5] = [
    SchedulerPresetKind::Sisyphus,
    SchedulerPresetKind::Prometheus,
    SchedulerPresetKind::Atlas,
    SchedulerPresetKind::Hephaestus,
    SchedulerPresetKind::Verifier,
];

const PUBLIC_SCHEDULER_PRESETS: [SchedulerPresetKind; 5] = [
    SchedulerPresetKind::Sisyphus,
    SchedulerPresetKind::Prometheus,
    SchedulerPresetKind::Atlas,
    SchedulerPresetKind::Hephaestus,
    SchedulerPresetKind::Verifier,
];

const ROUTER_RECOMMENDED_SCHEDULER_PRESETS: [SchedulerPresetKind; 5] = [
    SchedulerPresetKind::Sisyphus,
    SchedulerPresetKind::Prometheus,
    SchedulerPresetKind::Atlas,
    SchedulerPresetKind::Hephaestus,
    SchedulerPresetKind::Verifier,
];

impl SchedulerPresetKind {
    pub fn all() -> &'static [Self] {
        &ALL_SCHEDULER_PRESETS
    }

    pub fn public_presets() -> &'static [Self] {
        &PUBLIC_SCHEDULER_PRESETS
    }

    pub fn router_recommended_presets() -> &'static [Self] {
        &ROUTER_RECOMMENDED_SCHEDULER_PRESETS
    }

    pub fn definition(self) -> SchedulerPresetDefinition {
        scheduler_preset_definition(self)
    }

    pub fn stage_observability(self, stage: SchedulerStageKind) -> SchedulerStageObservability {
        self.definition().stage_observability(stage)
    }

    pub fn metadata(self) -> SchedulerPresetMetadata {
        self.definition().metadata
    }

    pub fn is_public(self) -> bool {
        self.metadata().public
    }

    pub fn is_router_recommended(self) -> bool {
        self.metadata().router_recommended
    }

    pub fn is_deprecated(self) -> bool {
        self.metadata().deprecated
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sisyphus => "sisyphus",
            Self::Prometheus => "prometheus",
            Self::Atlas => "atlas",
            Self::Hephaestus => "hephaestus",
            Self::Verifier => "verifier",
        }
    }

    pub fn from_profile_config(
        profile: &SchedulerProfileConfig,
    ) -> Result<Self, SchedulerConfigError> {
        if let Some(orchestrator) = profile.orchestrator.as_deref() {
            return orchestrator.parse();
        }

        if let Some(workflow) = profile.workflow() {
            if workflow.workflow.mode == IterativeWorkflowMode::Verify {
                return Ok(Self::Verifier);
            }
            return Ok(match workflow.base_preset_hint() {
                WorkflowBasePreset::Prometheus => Self::Prometheus,
                WorkflowBasePreset::Atlas => Self::Atlas,
                WorkflowBasePreset::Hephaestus => Self::Hephaestus,
            });
        }

        Ok(Self::Sisyphus)
    }

    pub fn plan_from_profile(
        self,
        profile_name: Option<String>,
        profile: &SchedulerProfileConfig,
    ) -> SchedulerProfilePlan {
        let definition = self.definition();
        let mut plan = SchedulerProfilePlan::from_profile_config(
            profile_name,
            definition.default_stage_kinds(),
            profile,
        );
        plan.stages = definition.resolved_stage_kinds(profile);
        if plan.orchestrator.is_none() && profile.workflow().is_some() {
            plan.orchestrator = Some(self.as_str().to_string());
        }
        plan
    }

    pub fn orchestrator_from_profile(
        self,
        profile_name: Option<String>,
        profile: &SchedulerProfileConfig,
        tool_runner: ToolRunner,
    ) -> SchedulerProfileOrchestrator {
        SchedulerProfileOrchestrator::new(
            self.plan_from_profile(profile_name, profile),
            tool_runner,
        )
    }
}

pub fn scheduler_stage_observability(
    scheduler_profile: &str,
    stage_name: &str,
) -> Option<SchedulerStageObservability> {
    let stage = SchedulerStageKind::from_event_name(stage_name)?;
    if scheduler_profile == AUTO_SCHEDULER_PROFILE_NAME {
        let profile = scheduler_auto_profile_config();
        let plan =
            scheduler_plan_from_profile(Some(scheduler_profile.to_string()), &profile).ok()?;
        let policy = plan.stage_policy(stage);
        return Some(SchedulerStageObservability {
            projection: policy.session_projection.label().to_string(),
            tool_policy: policy.tool_policy.label(),
            loop_budget: policy.loop_budget.label(),
        });
    }

    let preset = scheduler_profile.parse::<SchedulerPresetKind>().ok()?;
    Some(preset.stage_observability(stage))
}

pub fn scheduler_auto_profile_config() -> SchedulerProfileConfig {
    SchedulerProfileConfig {
        orchestrator: Some(SchedulerPresetKind::Sisyphus.as_str().to_string()),
        description: Some(
            "Automatic scheduler routing: analyze the request, run Route for preset selection, then execute the selected workflow.".to_string(),
        ),
        stages: vec![
            StageEntry::Plain(SchedulerStageKind::RequestAnalysis),
            StageEntry::Override(Box::new(SchedulerStageOverride {
                kind: SchedulerStageKind::Route,
                tool_policy: Some(StageToolPolicyOverride::DisableAll),
                loop_budget: None,
                session_projection: None,
                agent_tree: None,
                agents: Vec::new(),
                skill_list: Vec::new(),
            })),
            StageEntry::Plain(SchedulerStageKind::ExecutionOrchestration),
        ],
        ..Default::default()
    }
}

pub fn scheduler_plan_from_profile(
    profile_name: Option<String>,
    profile: &SchedulerProfileConfig,
) -> Result<SchedulerProfilePlan, SchedulerConfigError> {
    Ok(SchedulerPresetKind::from_profile_config(profile)?.plan_from_profile(profile_name, profile))
}

pub fn scheduler_plan_from_config(
    config: &SchedulerConfig,
) -> Result<SchedulerProfilePlan, SchedulerConfigError> {
    let profile_name = config
        .default_profile_key()
        .ok_or_else(|| SchedulerConfigError::ProfileNotFound("<default>".to_string()))?;
    let profile = config.profile(profile_name)?;
    scheduler_plan_from_profile(Some(profile_name.to_string()), profile)
}

pub fn scheduler_plan_from_file(
    path: impl AsRef<Path>,
) -> Result<SchedulerProfilePlan, SchedulerConfigError> {
    let config = SchedulerConfig::load_from_file(path)?;
    scheduler_plan_from_config(&config)
}

impl FromStr for SchedulerPresetKind {
    type Err = SchedulerConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "sisyphus" => Ok(Self::Sisyphus),
            "prometheus" => Ok(Self::Prometheus),
            "atlas" => Ok(Self::Atlas),
            "hephaestus" => Ok(Self::Hephaestus),
            "verifier" => Ok(Self::Verifier),
            other => Err(SchedulerConfigError::UnknownOrchestrator(other.to_string())),
        }
    }
}

pub fn scheduler_request_defaults_from_plan(
    plan: &SchedulerProfilePlan,
) -> SchedulerRequestDefaults {
    SchedulerRequestDefaults {
        profile_name: plan.profile_name.clone(),
        root_agent_name: plan
            .agent_tree
            .as_ref()
            .map(|node| node.agent.name.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_string),
        skill_tree_plan: plan.skill_tree.clone(),
    }
}

pub fn scheduler_request_defaults_from_config(
    config: &SchedulerConfig,
) -> Result<SchedulerRequestDefaults, SchedulerConfigError> {
    let plan = scheduler_plan_from_config(config)?;
    Ok(scheduler_request_defaults_from_plan(&plan))
}

pub fn scheduler_request_defaults_from_file(
    path: impl AsRef<Path>,
) -> Result<SchedulerRequestDefaults, SchedulerConfigError> {
    let config = SchedulerConfig::load_from_file(path)?;
    scheduler_request_defaults_from_config(&config)
}

pub fn scheduler_orchestrator_from_profile(
    profile_name: Option<String>,
    profile: &SchedulerProfileConfig,
    tool_runner: ToolRunner,
) -> Result<SchedulerProfileOrchestrator, SchedulerConfigError> {
    Ok(
        SchedulerPresetKind::from_profile_config(profile)?.orchestrator_from_profile(
            profile_name,
            profile,
            tool_runner,
        ),
    )
}

pub fn scheduler_orchestrator_from_plan(
    plan: SchedulerProfilePlan,
    tool_runner: ToolRunner,
) -> SchedulerProfileOrchestrator {
    SchedulerProfileOrchestrator::new(plan, tool_runner)
}

/// Build the typed preset prompt extension for a resolved scheduler plan.
///
/// This is the single authority for projecting scheduler preset identity,
/// capability summary, and extra sections into a `PresetPromptExtension`
/// without flattening the surface into an already-rendered charter string.
pub fn scheduler_preset_extension_from_plan(
    plan: &SchedulerProfilePlan,
) -> Option<PresetPromptExtension> {
    let preset = plan
        .orchestrator
        .as_deref()
        .and_then(|value| value.parse::<SchedulerPresetKind>().ok())?;

    let execution_skills =
        plan.effective_skill_list(Some(crate::scheduler::SchedulerStageKind::ExecutionOrchestration));

    Some(match preset {
        SchedulerPresetKind::Sisyphus => {
            crate::scheduler::presets::sisyphus_prompt_extension(
                &plan.available_agents,
                &plan.available_categories,
                execution_skills,
            )
        }
        SchedulerPresetKind::Atlas => crate::scheduler::presets::atlas_prompt_extension(
            &plan.available_agents,
            &plan.available_categories,
            execution_skills,
        ),
        SchedulerPresetKind::Hephaestus => {
            crate::scheduler::presets::hephaestus_prompt_extension(
                &plan.available_agents,
                &plan.available_categories,
                execution_skills,
            )
        }
        SchedulerPresetKind::Prometheus => PresetPromptExtension::new(
            "prometheus",
            "planning-first orchestration and handoff workflow",
        )
        .with_tone_augment(
            "Preserve planner discipline: interview before planning, review before handoff, and never pretend execution completed unless downstream stages actually ran.",
        ),
        SchedulerPresetKind::Verifier => {
            PresetPromptExtension::new("verifier", "verification-focused review workflow")
                .with_tone_augment(
                    "Bias toward evidence review, contradiction surfacing, and explicit uncertainty when proof is incomplete.",
                )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::SchedulerStageKind;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn checked_in_scheduler_example_path(relative_path: &str) -> PathBuf {
        repo_root()
            .join("docs/examples/scheduler")
            .join(relative_path)
    }

    fn assert_checked_in_public_scheduler_example(
        relative_path: &str,
        profile_name: &str,
        orchestrator: &str,
        expected_stages: Vec<SchedulerStageKind>,
    ) {
        let path = checked_in_scheduler_example_path(relative_path);
        let config = SchedulerConfig::load_from_file(&path)
            .unwrap_or_else(|err| panic!("example {} should parse: {}", path.display(), err));
        let profile = config.default_profile().unwrap_or_else(|err| {
            panic!(
                "example {} should resolve default profile: {}",
                path.display(),
                err
            )
        });
        let plan = scheduler_plan_from_config(&config).unwrap_or_else(|err| {
            panic!(
                "example {} should resolve scheduler plan: {}",
                path.display(),
                err
            )
        });

        assert_eq!(config.default_profile_key(), Some(profile_name));
        assert_eq!(profile.orchestrator.as_deref(), Some(orchestrator));
        assert_eq!(profile.stage_kinds(), expected_stages);
        assert_eq!(plan.profile_name.as_deref(), Some(profile_name));
        assert_eq!(plan.orchestrator.as_deref(), Some(orchestrator));
        assert_eq!(plan.stages, expected_stages);
    }

    fn write_temp_scheduler(content: &str) -> std::path::PathBuf {
        let unique = format!(
            "agendao_scheduler_preset_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock error")
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).expect("temp dir should exist");
        let path = dir.join("scheduler.jsonc");
        fs::write(&path, content).expect("scheduler file should write");
        path
    }

    #[test]
    fn scheduler_preset_defaults_to_sisyphus() {
        let profile = SchedulerProfileConfig::default();
        let plan = scheduler_plan_from_profile(Some("default".to_string()), &profile).unwrap();
        assert_eq!(plan.profile_name.as_deref(), Some("default"));
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::ExecutionOrchestration,
            ]
        );
    }

    #[test]
    fn scheduler_auto_profile_uses_route_for_preset_selection() {
        let profile = scheduler_auto_profile_config();
        let plan =
            scheduler_plan_from_profile(Some(AUTO_SCHEDULER_PROFILE_NAME.to_string()), &profile)
                .unwrap();
        assert_eq!(
            plan.profile_name.as_deref(),
            Some(AUTO_SCHEDULER_PROFILE_NAME)
        );
        assert_eq!(plan.orchestrator.as_deref(), Some("sisyphus"));
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::Route,
                SchedulerStageKind::ExecutionOrchestration,
            ]
        );
    }

    #[test]
    fn scheduler_auto_profile_uses_disable_all_route_observability() {
        let route = scheduler_stage_observability(AUTO_SCHEDULER_PROFILE_NAME, "route")
            .expect("auto route stage should expose observability");
        let sisyphus_route =
            SchedulerPresetKind::Sisyphus.stage_observability(SchedulerStageKind::Route);
        let sisyphus_execution = SchedulerPresetKind::Sisyphus
            .stage_observability(SchedulerStageKind::ExecutionOrchestration);
        let auto_execution =
            scheduler_stage_observability(AUTO_SCHEDULER_PROFILE_NAME, "execution-orchestration")
                .expect("auto execution stage should expose observability");

        assert_eq!(route.tool_policy, "disable-all");
        assert_ne!(route, sisyphus_route);
        assert_eq!(auto_execution, sisyphus_execution);
    }

    #[test]
    fn scheduler_preset_can_resolve_prometheus() {
        let profile = SchedulerProfileConfig {
            orchestrator: Some("prometheus".to_string()),
            ..Default::default()
        };
        let plan = scheduler_plan_from_profile(Some("planner".to_string()), &profile).unwrap();
        assert_eq!(plan.profile_name.as_deref(), Some("planner"));
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::Interview,
                SchedulerStageKind::Plan,
                SchedulerStageKind::Review,
                SchedulerStageKind::Handoff,
            ]
        );
    }

    #[test]
    fn scheduler_prometheus_ignores_custom_execution_stages() {
        use crate::scheduler::StageEntry;
        let profile = SchedulerProfileConfig {
            orchestrator: Some("prometheus".to_string()),
            stages: vec![
                StageEntry::Plain(SchedulerStageKind::RequestAnalysis),
                StageEntry::Plain(SchedulerStageKind::ExecutionOrchestration),
                StageEntry::Plain(SchedulerStageKind::Synthesis),
            ],
            ..Default::default()
        };
        let plan = scheduler_plan_from_profile(Some("planner".to_string()), &profile).unwrap();
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::Interview,
                SchedulerStageKind::Plan,
                SchedulerStageKind::Review,
                SchedulerStageKind::Handoff,
            ]
        );
    }

    #[test]
    fn scheduler_preset_can_resolve_atlas() {
        let profile = SchedulerProfileConfig {
            orchestrator: Some("atlas".to_string()),
            ..Default::default()
        };
        let plan = scheduler_plan_from_profile(Some("atlas".to_string()), &profile).unwrap();
        assert_eq!(plan.profile_name.as_deref(), Some("atlas"));
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::ExecutionOrchestration,
                SchedulerStageKind::Synthesis,
            ]
        );
    }

    #[test]
    fn scheduler_preset_can_resolve_hephaestus() {
        let profile = SchedulerProfileConfig {
            orchestrator: Some("hephaestus".to_string()),
            ..Default::default()
        };
        let plan = scheduler_plan_from_profile(Some("hephaestus".to_string()), &profile).unwrap();
        assert_eq!(plan.profile_name.as_deref(), Some("hephaestus"));
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::ExecutionOrchestration,
            ]
        );
    }

    #[test]
    fn scheduler_preset_infers_prometheus_from_plan_workflow() {
        let profile = SchedulerProfileConfig {
            workflow: Some(crate::IterativeWorkflowSource::Inline(
                crate::IterativeWorkflowConfig::load_from_str(
                    r#"{
                      "workflow": { "kind": "autoresearch", "mode": "plan" }
                    }"#,
                )
                .unwrap(),
            )),
            ..Default::default()
        };
        let plan = scheduler_plan_from_profile(Some("planner".to_string()), &profile).unwrap();
        assert_eq!(plan.orchestrator.as_deref(), Some("prometheus"));
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::Interview,
                SchedulerStageKind::Plan,
                SchedulerStageKind::Review,
                SchedulerStageKind::Handoff,
            ]
        );
    }

    #[test]
    fn scheduler_preset_infers_hephaestus_from_run_workflow() {
        let profile = SchedulerProfileConfig {
            workflow: Some(crate::IterativeWorkflowSource::Inline(
                crate::IterativeWorkflowConfig::load_from_str(
                    r#"{
                      "workflow": { "kind": "autoresearch", "mode": "run" },
                      "objective": {
                        "goal": "Improve tests",
                        "scope": { "include": ["src/**/*.rs"] },
                        "direction": "higher-is-better",
                        "metric": { "kind": "numeric-extract", "pattern": "([0-9]+)" },
                        "verify": { "command": "cargo test" }
                      },
                      "iterationPolicy": { "mode": "bounded", "maxIterations": 3 },
                      "decisionPolicy": {},
                      "workspacePolicy": { "snapshotStrategy": "patch-file" }
                    }"#,
                )
                .unwrap(),
            )),
            ..Default::default()
        };
        let plan = scheduler_plan_from_profile(Some("run".to_string()), &profile).unwrap();
        assert_eq!(plan.orchestrator.as_deref(), Some("hephaestus"));
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::ExecutionOrchestration,
            ]
        );
    }

    #[test]
    fn scheduler_preset_infers_verifier_from_verify_workflow() {
        let profile = SchedulerProfileConfig {
            workflow: Some(crate::IterativeWorkflowSource::Inline(
                crate::IterativeWorkflowConfig::load_from_str(
                    r#"{
                      "workflow": { "kind": "autoresearch", "mode": "verify" },
                      "objective": {
                        "goal": "Choose the better patch",
                        "scope": { "include": ["src/**/*.rs"] },
                        "direction": "higher-is-better",
                        "metric": { "kind": "numeric-extract", "pattern": "([0-9]+)" },
                        "verify": { "command": "cargo test" }
                      },
                      "decisionPolicy": {},
                      "workspacePolicy": { "snapshotStrategy": "patch-file" },
                      "verifier": {
                        "model": { "providerId": "openai", "modelId": "gpt-5" },
                        "criteria": [
                          {
                            "id": "spec",
                            "name": "Spec",
                            "description": "Prefer the candidate that best satisfies the request."
                          }
                        ]
                      }
                    }"#,
                )
                .unwrap(),
            )),
            ..Default::default()
        };
        let plan = scheduler_plan_from_profile(Some("verify".to_string()), &profile).unwrap();
        assert_eq!(plan.orchestrator.as_deref(), Some("verifier"));
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::ExecutionOrchestration,
            ]
        );
    }

    #[test]
    fn scheduler_preset_infers_verifier_from_external_verify_workflow_path() {
        let unique = format!(
            "agendao_scheduler_verifier_workflow_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock error")
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).expect("temp dir should exist");
        let workflow_path = dir.join("workflow.jsonc");
        fs::write(
            &workflow_path,
            r#"{
              "workflow": { "kind": "autoresearch", "mode": "verify" },
              "objective": {
                "goal": "Pick the strongest implementation",
                "scope": { "include": ["src/**/*.rs"] },
                "direction": "higher-is-better",
                "metric": { "kind": "numeric-extract", "pattern": "([0-9]+)" },
                "verify": { "command": "cargo test" }
              },
              "decisionPolicy": {},
              "workspacePolicy": { "snapshotStrategy": "patch-file" },
              "verifier": {
                "model": { "providerId": "openai", "modelId": "gpt-5" },
                "criteria": [
                  {
                    "id": "spec",
                    "name": "Spec",
                    "description": "Prefer the candidate that best satisfies the request."
                  }
                ]
              }
            }"#,
        )
        .expect("workflow file should write");

        let scheduler_path = dir.join("scheduler.jsonc");
        fs::write(
            &scheduler_path,
            r#"{
              "defaults": { "profile": "verify-path" },
              "profiles": {
                "verify-path": {
                  "workflowPath": "./workflow.jsonc"
                }
              }
            }"#,
        )
        .expect("scheduler file should write");

        let plan = scheduler_plan_from_file(&scheduler_path).unwrap();
        assert_eq!(plan.profile_name.as_deref(), Some("verify-path"));
        assert_eq!(plan.orchestrator.as_deref(), Some("verifier"));
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::ExecutionOrchestration,
            ]
        );

        let _ = fs::remove_file(&scheduler_path);
        let _ = fs::remove_file(&workflow_path);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scheduler_public_presets_only_include_omo_presets() {
        assert_eq!(
            SchedulerPresetKind::public_presets()
                .iter()
                .map(|preset| preset.as_str())
                .collect::<Vec<_>>(),
            vec!["sisyphus", "prometheus", "atlas", "hephaestus", "verifier"]
        );
        assert_eq!(
            SchedulerPresetKind::router_recommended_presets()
                .iter()
                .map(|preset| preset.as_str())
                .collect::<Vec<_>>(),
            vec!["sisyphus", "prometheus", "atlas", "hephaestus", "verifier"]
        );
    }

    #[test]
    fn scheduler_omo_presets_are_public_and_recommended() {
        for preset in SchedulerPresetKind::public_presets() {
            assert!(preset.is_public(), "{} should stay public", preset.as_str());
            assert!(
                preset.is_router_recommended(),
                "{} should stay router recommended",
                preset.as_str()
            );
            assert!(
                !preset.is_deprecated(),
                "{} should not be deprecated",
                preset.as_str()
            );
        }
    }

    #[test]
    fn scheduler_plan_from_config_uses_default_profile_key() {
        let config = SchedulerConfig::load_from_str(
            r#"{
                "defaults": { "profile": "planner" },
                "profiles": {
                    "planner": { "orchestrator": "prometheus" }
                }
            }"#,
        )
        .unwrap();

        let plan = scheduler_plan_from_config(&config).unwrap();
        assert_eq!(plan.profile_name.as_deref(), Some("planner"));
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::Interview,
                SchedulerStageKind::Plan,
                SchedulerStageKind::Review,
                SchedulerStageKind::Handoff,
            ]
        );
    }

    #[test]
    fn scheduler_request_defaults_extract_root_agent_and_skill_tree() {
        let path = write_temp_scheduler(
            r#"{
                "defaults": { "profile": "delivery" },
                "profiles": {
                    "delivery": {
                        "orchestrator": "sisyphus",
                        "skillTree": { "contextMarkdown": "External scheduler context" },
                        "agentTree": {
                            "agent": { "name": "deep-worker" }
                        }
                    }
                }
            }"#,
        );

        let defaults = scheduler_request_defaults_from_file(&path).unwrap();
        assert_eq!(defaults.profile_name.as_deref(), Some("delivery"));
        assert_eq!(defaults.root_agent_name.as_deref(), Some("deep-worker"));
        assert_eq!(
            defaults
                .skill_tree_plan
                .as_ref()
                .map(|tree| tree.context_markdown.as_str()),
            Some("External scheduler context")
        );
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn checked_in_public_scheduler_examples_align_with_runtime_defaults() {
        assert_checked_in_public_scheduler_example(
            "presets/sisyphus.example.jsonc",
            "sisyphus-default",
            "sisyphus",
            crate::scheduler::sisyphus_default_stages(),
        );
        assert_checked_in_public_scheduler_example(
            "presets/prometheus.example.jsonc",
            "prometheus-default",
            "prometheus",
            crate::scheduler::prometheus_default_stages(),
        );
        assert_checked_in_public_scheduler_example(
            "presets/atlas.example.jsonc",
            "atlas-default",
            "atlas",
            crate::scheduler::atlas_default_stages(),
        );
        assert_checked_in_public_scheduler_example(
            "presets/hephaestus.example.jsonc",
            "hephaestus-default",
            "hephaestus",
            crate::scheduler::hephaestus_default_stages(),
        );
        assert_checked_in_public_scheduler_example(
            "verifier/minimal.example.jsonc",
            "verifier-simple",
            "verifier",
            crate::scheduler::verifier_default_stages(),
        );
        assert_checked_in_public_scheduler_example(
            "verifier/profile.example.jsonc",
            "verifier-default",
            "verifier",
            crate::scheduler::verifier_default_stages(),
        );
    }

    #[test]
    fn verifier_example_exposes_verify_workflow_defaults() {
        let path = checked_in_scheduler_example_path("verifier/profile.example.jsonc");
        let config = SchedulerConfig::load_from_file(&path)
            .unwrap_or_else(|err| panic!("verifier example should parse: {}", err));
        let profile = config.default_profile().unwrap();
        let workflow = profile
            .workflow()
            .expect("verifier example should include inline workflow config");
        let verifier = workflow
            .verifier
            .as_ref()
            .expect("verify mode should include verifier block");

        assert_eq!(workflow.workflow.mode, IterativeWorkflowMode::Verify);
        assert_eq!(verifier.repetitions, Some(3));
        assert_eq!(verifier.use_logprobs, Some(true));
        assert_eq!(verifier.granularity, Some(20));
        assert_eq!(
            verifier.selection,
            Some(crate::iterative_workflow::VerifierSelectionStrategy::RoundRobin)
        );
        assert_eq!(
            verifier.trace_format,
            Some(crate::iterative_workflow::VerifierTraceFormat::Compact)
        );
        assert_eq!(verifier.criteria.len(), 2);
        assert_eq!(verifier.criteria[0].weight, Some(1.0));
        assert_eq!(
            verifier.criteria[1].aggregation,
            Some(crate::iterative_workflow::VerifierCriterionAggregation::WinnerVote)
        );
    }

    #[test]
    fn verifier_simple_example_keeps_user_facing_defaults_minimal() {
        let path = checked_in_scheduler_example_path("verifier/minimal.example.jsonc");
        let config = SchedulerConfig::load_from_file(&path)
            .unwrap_or_else(|err| panic!("verifier simple example should parse: {}", err));
        let profile = config.default_profile().unwrap();
        let workflow = profile
            .workflow()
            .expect("verifier simple example should include inline workflow config");
        let verifier = workflow
            .verifier
            .as_ref()
            .expect("verify mode should include verifier block");

        assert_eq!(config.default_profile_key(), Some("verifier-simple"));
        assert_eq!(workflow.workflow.mode, IterativeWorkflowMode::Verify);
        assert_eq!(verifier.criteria.len(), 1);
        assert_eq!(verifier.repetitions, Some(1));
        assert_eq!(verifier.max_candidates, Some(3));
        assert_eq!(verifier.use_logprobs, None);
        assert_eq!(verifier.granularity, None);
    }

    #[test]
    fn verifier_example_custom_profile_resolves_external_workflow() {
        let path = checked_in_scheduler_example_path("verifier/profile.example.jsonc");
        let config = SchedulerConfig::load_from_file(&path)
            .unwrap_or_else(|err| panic!("verifier example should parse: {}", err));
        let profile = config.profile("verifier-custom").unwrap();
        let workflow = profile
            .workflow()
            .expect("verifier custom profile should resolve workflowPath");
        let verifier = workflow
            .verifier
            .as_ref()
            .expect("verify workflow should include verifier block");

        assert_eq!(workflow.workflow.mode, IterativeWorkflowMode::Verify);
        assert_eq!(
            verifier.selection,
            Some(crate::iterative_workflow::VerifierSelectionStrategy::Tournament)
        );
        assert_eq!(verifier.max_candidates, Some(4));
        assert_eq!(verifier.use_logprobs, Some(true));
        assert_eq!(verifier.granularity, Some(20));
        assert_eq!(verifier.criteria.len(), 3);
    }

    #[test]
    fn scheduler_preset_rejects_unknown_orchestrator() {
        let profile = SchedulerProfileConfig {
            orchestrator: Some("unknown".to_string()),
            ..Default::default()
        };
        let err = scheduler_plan_from_profile(None, &profile).unwrap_err();
        assert!(err
            .to_string()
            .contains("unsupported scheduler orchestrator"));
    }

    #[test]
    fn pso_example_parses_and_resolves_agent_tree_paths() {
        let path = checked_in_scheduler_example_path("pso/example.jsonc");
        let config = SchedulerConfig::load_from_file(&path)
            .unwrap_or_else(|err| panic!("PSO example should parse: {}", err));

        // Default profile is pso-3iter
        assert_eq!(config.default_profile_key(), Some("pso-3iter"));

        let profile = config.default_profile().unwrap();
        assert_eq!(profile.orchestrator.as_deref(), Some("atlas"));

        // pso-3iter has 7 stages: request-analysis + 3×(execution-orchestration, synthesis)
        assert_eq!(profile.stages.len(), 7);
        assert_eq!(
            profile.stage_kinds(),
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::ExecutionOrchestration,
                SchedulerStageKind::Synthesis,
                SchedulerStageKind::ExecutionOrchestration,
                SchedulerStageKind::Synthesis,
                SchedulerStageKind::ExecutionOrchestration,
                SchedulerStageKind::Synthesis,
            ]
        );

        // Profile-level agent tree is absent (all trees are per-stage)
        assert!(profile.agent_tree.is_none());

        // Each execution-orchestration stage override has a resolved inline agent tree
        for entry in &profile.stages {
            if let crate::scheduler::StageEntry::Override(o) = entry {
                let source = o
                    .agent_tree
                    .as_ref()
                    .expect("execution stage should have agent tree");
                assert!(
                    source.is_inline(),
                    "agent tree path should be resolved to inline"
                );
                let tree = source.as_inline().unwrap();
                assert_eq!(tree.agent.name, "swarm-coordinator");
                assert_eq!(tree.children.len(), 3);
                assert_eq!(tree.children[0].agent.name, "particle-alpha");
                assert_eq!(tree.children[1].agent.name, "particle-beta");
                assert_eq!(tree.children[2].agent.name, "particle-gamma");
            }
        }

        // pso-5iter also parses
        let profile5 = config.profile("pso-5iter").unwrap();
        // 11 stages: request-analysis + 5×(execution-orchestration, synthesis)
        assert_eq!(profile5.stages.len(), 11);
    }

    #[test]
    fn render_preset_prompt_extension_consumes_all_typed_fields() {
        let rendered = super::render_preset_prompt_extension(
            &super::PresetPromptExtension::new("atlas", "coordination orchestrator")
                .with_section("Identity", "<identity>Atlas</identity>")
                .with_section("Constraints", "<Constraints>\nNever guess.\n</Constraints>")
                .with_capability("Agents: explore, review.")
                .with_tone_augment("Be concise. No flattery."),
        );

        assert!(rendered.contains("## Preset Role Summary"));
        assert!(rendered.contains("coordination orchestrator"));
        assert!(rendered.contains("<identity>Atlas</identity>"));
        assert!(rendered.contains("## Capability Projection"));
        assert!(rendered.contains("Agents: explore, review."));
        assert!(rendered.contains("## Tone Augment"));
        assert!(rendered.contains("Be concise. No flattery."));
        assert!(rendered.contains("<Constraints>"));
        assert!(
            rendered.find("## Tone Augment").unwrap()
                < rendered.find("## Capability Projection").unwrap()
        );
        assert!(
            rendered.find("<Constraints>").unwrap()
                < rendered.find("## Capability Projection").unwrap()
        );
    }

    #[test]
    fn scheduler_preset_extension_from_plan_builds_sisyphus_extension() {
        let mut profile = SchedulerProfileConfig {
            orchestrator: Some("sisyphus".to_string()),
            ..Default::default()
        };
        profile.available_agents = vec![crate::scheduler::AvailableAgentMeta {
            name: "explorer".to_string(),
            description: "Exploration agent".to_string(),
            mode: "subagent".to_string(),
            cost: "CHEAP".to_string(),
        }];
        let plan = scheduler_plan_from_profile(Some("sisyphus".to_string()), &profile).unwrap();

        let ext = scheduler_preset_extension_from_plan(&plan).expect("extension should exist");

        assert_eq!(ext.preset_name, "sisyphus");
        assert_eq!(ext.role_summary, "delegation-first execution orchestrator");
        assert!(!ext.extra_sections.is_empty());
        assert!(ext.extra_sections.iter().any(|(_, body)| !body.trim().is_empty()));
    }
}

// ── Preset prompt extension (Commit 6) ─────────────────────────────────
//
// PresetPromptExtension is defined in agendao-types (no circular dep).
// Re-exported here so presets can use `crate::scheduler::PresetPromptExtension`.
pub use agendao_types::PresetPromptExtension;

/// Render a preset prompt extension into the exact scheduler charter text.
///
/// The extension already carries fully rendered section bodies in the
/// correct order. This helper is the single renderer for the typed
/// contract, so every populated field must become model-visible text.
pub fn render_preset_prompt_extension(extension: &PresetPromptExtension) -> String {
    let mut sections = Vec::new();

    let role_summary = extension.role_summary.trim();
    if !role_summary.is_empty() {
        sections.push(format!("## Preset Role Summary\n{role_summary}"));
    }

    sections.extend(
        extension
            .extra_sections
            .iter()
            .map(|(_, body)| body.trim())
            .filter(|body| !body.is_empty())
            .map(str::to_string),
    );

    if let Some(tone_augment) = extension
        .tone_augment
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(format!("## Tone Augment\n{tone_augment}"));
    }

    // Keep large runtime capability catalogs late so the prompt prefix
    // stays anchored by higher-stability preset governance text.
    if let Some(capability_projection) = extension
        .capability_projection
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(format!(
            "## Capability Projection\n{capability_projection}"
        ));
    }

    sections.join("\n\n")
}
