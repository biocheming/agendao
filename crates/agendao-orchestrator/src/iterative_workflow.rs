use crate::types::ModelRef;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const VERIFIER_MAX_CRITERIA: usize = 8;
const VERIFIER_MAX_REPETITIONS: u32 = 10;
const VERIFIER_MAX_CANDIDATES: u32 = 12;
const VERIFIER_MAX_PAIRWISE_JUDGE_JOBS: u32 = 2_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IterativeWorkflowSource {
    Inline(IterativeWorkflowConfig),
    Path(String),
}

impl IterativeWorkflowSource {
    pub fn as_inline(&self) -> Option<&IterativeWorkflowConfig> {
        match self {
            Self::Inline(config) => Some(config),
            Self::Path(_) => None,
        }
    }

    pub fn is_inline(&self) -> bool {
        matches!(self, Self::Inline(_))
    }

    pub fn is_path(&self) -> bool {
        matches!(self, Self::Path(_))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IterativeWorkflowConfigError {
    #[error("failed to read iterative workflow config: {0}")]
    Read(#[from] std::io::Error),

    #[error("failed to parse iterative workflow config as jsonc: {0}")]
    Parse(String),

    #[error("failed to deserialize iterative workflow config: {0}")]
    Deserialize(#[from] serde_json::Error),

    #[error("invalid iterative workflow config: {0}")]
    Validation(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowBasePreset {
    Prometheus,
    Atlas,
    Hephaestus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IterativeWorkflowConfig {
    pub workflow: WorkflowDescriptor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<ObjectiveDefinition>,
    #[serde(
        default,
        alias = "iterationPolicy",
        skip_serializing_if = "Option::is_none"
    )]
    pub iteration_policy: Option<IterationPolicyDefinition>,
    #[serde(
        default,
        alias = "decisionPolicy",
        skip_serializing_if = "Option::is_none"
    )]
    pub decision_policy: Option<DecisionPolicyDefinition>,
    #[serde(
        default,
        alias = "workspacePolicy",
        skip_serializing_if = "Option::is_none"
    )]
    pub workspace_policy: Option<WorkspacePolicyDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<ArtifactDefinition>,
    #[serde(
        default,
        alias = "approvalPolicy",
        skip_serializing_if = "Option::is_none"
    )]
    pub approval_policy: Option<ApprovalPolicyDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<SecurityConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<DebugConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<FixConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier: Option<VerifierConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ship: Option<ShipConfig>,
}

impl IterativeWorkflowConfig {
    pub fn load_from_str(content: &str) -> Result<Self, IterativeWorkflowConfigError> {
        let parse_options = jsonc_parser::ParseOptions {
            allow_trailing_commas: true,
            ..Default::default()
        };
        let value = jsonc_parser::parse_to_serde_value(content, &parse_options)
            .map_err(|err| IterativeWorkflowConfigError::Parse(err.to_string()))?
            .ok_or_else(|| {
                IterativeWorkflowConfigError::Parse("empty iterative workflow config".to_string())
            })?;
        let config: Self = serde_json::from_value(value)?;
        config.validate()?;
        Ok(config)
    }

    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self, IterativeWorkflowConfigError> {
        let content = fs::read_to_string(path)?;
        Self::load_from_str(&content)
    }

    pub fn base_preset_hint(&self) -> WorkflowBasePreset {
        match self.workflow.mode {
            IterativeWorkflowMode::Plan => WorkflowBasePreset::Prometheus,
            IterativeWorkflowMode::Security | IterativeWorkflowMode::Ship => {
                WorkflowBasePreset::Atlas
            }
            IterativeWorkflowMode::Run
            | IterativeWorkflowMode::Verify
            | IterativeWorkflowMode::Debug
            | IterativeWorkflowMode::Fix => WorkflowBasePreset::Hephaestus,
        }
    }

    pub fn validate(&self) -> Result<(), IterativeWorkflowConfigError> {
        self.validate_mode_requirements()?;

        if let Some(objective) = &self.objective {
            objective.validate()?;
        }
        if let Some(policy) = &self.iteration_policy {
            policy.validate()?;
        }
        if let Some(policy) = &self.decision_policy {
            policy.validate()?;
        }
        if let Some(config) = &self.fix {
            config.validate()?;
        }
        if let Some(config) = &self.verifier {
            config.validate()?;
        }

        Ok(())
    }

    fn validate_mode_requirements(&self) -> Result<(), IterativeWorkflowConfigError> {
        let missing = match self.workflow.mode {
            IterativeWorkflowMode::Run => required_fields(&[
                ("objective", self.objective.is_some()),
                ("iterationPolicy", self.iteration_policy.is_some()),
                ("decisionPolicy", self.decision_policy.is_some()),
                ("workspacePolicy", self.workspace_policy.is_some()),
            ]),
            IterativeWorkflowMode::Plan => Vec::new(),
            IterativeWorkflowMode::Security => {
                required_fields(&[("security", self.security.is_some())])
            }
            IterativeWorkflowMode::Debug => required_fields(&[("debug", self.debug.is_some())]),
            IterativeWorkflowMode::Fix => required_fields(&[
                ("objective", self.objective.is_some()),
                ("decisionPolicy", self.decision_policy.is_some()),
                ("workspacePolicy", self.workspace_policy.is_some()),
            ]),
            IterativeWorkflowMode::Verify => required_fields(&[
                ("objective", self.objective.is_some()),
                ("decisionPolicy", self.decision_policy.is_some()),
                ("workspacePolicy", self.workspace_policy.is_some()),
                ("verifier", self.verifier.is_some()),
            ]),
            IterativeWorkflowMode::Ship => required_fields(&[
                ("ship", self.ship.is_some()),
                ("approvalPolicy", self.approval_policy.is_some()),
            ]),
        };

        if missing.is_empty() {
            Ok(())
        } else {
            Err(IterativeWorkflowConfigError::Validation(format!(
                "workflow mode '{}' requires: {}",
                self.workflow.mode.as_str(),
                missing.join(", ")
            )))
        }
    }
}

fn required_fields(fields: &[(&str, bool)]) -> Vec<String> {
    fields
        .iter()
        .filter_map(|(name, present)| (!present).then_some((*name).to_string()))
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowDescriptor {
    pub kind: IterativeWorkflowKind,
    pub mode: IterativeWorkflowMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IterativeWorkflowKind {
    Autoresearch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IterativeWorkflowMode {
    Run,
    Plan,
    Security,
    Debug,
    Fix,
    Verify,
    Ship,
}

impl IterativeWorkflowMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Plan => "plan",
            Self::Security => "security",
            Self::Debug => "debug",
            Self::Fix => "fix",
            Self::Verify => "verify",
            Self::Ship => "ship",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectiveDefinition {
    pub goal: String,
    pub scope: ScopeDefinition,
    pub direction: ObjectiveDirection,
    pub metric: MetricDefinition,
    pub verify: CommandDefinition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<CommandDefinition>,
    #[serde(
        default,
        alias = "satisfiedWhen",
        skip_serializing_if = "Option::is_none"
    )]
    pub satisfied_when: Option<SatisfiedWhenDefinition>,
}

impl ObjectiveDefinition {
    fn validate(&self) -> Result<(), IterativeWorkflowConfigError> {
        self.metric.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopeDefinition {
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ObjectiveDirection {
    HigherIsBetter,
    LowerIsBetter,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetricDefinition {
    pub kind: MetricKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(
        default,
        alias = "countPattern",
        skip_serializing_if = "Option::is_none"
    )]
    pub count_pattern: Option<String>,
    #[serde(default, alias = "jsonPath", skip_serializing_if = "Option::is_none")]
    pub json_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

impl MetricDefinition {
    fn validate(&self) -> Result<(), IterativeWorkflowConfigError> {
        let required = match self.kind {
            MetricKind::NumericExtract if self.pattern.is_none() => Some("pattern"),
            MetricKind::CountLines if self.count_pattern.is_none() => Some("countPattern"),
            MetricKind::JsonPath if self.json_path.is_none() => Some("jsonPath"),
            _ => None,
        };
        if let Some(field) = required {
            Err(IterativeWorkflowConfigError::Validation(format!(
                "metric kind '{}' requires field '{}'",
                self.kind.as_str(),
                field
            )))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MetricKind {
    NumericExtract,
    CountLines,
    ExitCode,
    JsonPath,
}

impl MetricKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::NumericExtract => "numeric-extract",
            Self::CountLines => "count-lines",
            Self::ExitCode => "exit-code",
            Self::JsonPath => "json-path",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandDefinition {
    pub command: String,
    #[serde(default, alias = "timeoutMs", skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    #[serde(
        default,
        alias = "workingDirectory",
        skip_serializing_if = "Option::is_none"
    )]
    pub working_directory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SatisfiedWhenDefinition {
    #[serde(
        default,
        alias = "metricAtLeast",
        skip_serializing_if = "Option::is_none"
    )]
    pub metric_at_least: Option<f64>,
    #[serde(
        default,
        alias = "metricAtMost",
        skip_serializing_if = "Option::is_none"
    )]
    pub metric_at_most: Option<f64>,
    #[serde(
        default,
        alias = "metricEquals",
        skip_serializing_if = "Option::is_none"
    )]
    pub metric_equals: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IterationPolicyDefinition {
    pub mode: IterationMode,
    #[serde(
        default,
        alias = "maxIterations",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_iterations: Option<u32>,
    #[serde(
        default,
        alias = "stopConditions",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub stop_conditions: Vec<StopCondition>,
    #[serde(
        default,
        alias = "stuckThreshold",
        skip_serializing_if = "Option::is_none"
    )]
    pub stuck_threshold: Option<u32>,
    #[serde(
        default,
        alias = "progressReportEvery",
        skip_serializing_if = "Option::is_none"
    )]
    pub progress_report_every: Option<u32>,
}

impl IterationPolicyDefinition {
    fn validate(&self) -> Result<(), IterativeWorkflowConfigError> {
        if matches!(self.mode, IterationMode::Bounded) && self.max_iterations.is_none() {
            return Err(IterativeWorkflowConfigError::Validation(
                "bounded iterationPolicy requires maxIterations".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IterationMode {
    Bounded,
    Unbounded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum StopCondition {
    ObjectiveSatisfied,
    NoProgress,
    ErrorCountZero,
    MaxIterationsReached,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionPolicyDefinition {
    #[serde(
        default,
        alias = "baselineStrategy",
        skip_serializing_if = "Option::is_none"
    )]
    pub baseline_strategy: Option<BaselineStrategy>,
    #[serde(
        default,
        alias = "baselineValue",
        skip_serializing_if = "Option::is_none"
    )]
    pub baseline_value: Option<f64>,
    #[serde(
        default,
        alias = "keepConditions",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub keep_conditions: Vec<KeepCondition>,
    #[serde(
        default,
        alias = "discardConditions",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub discard_conditions: Vec<DiscardCondition>,
    #[serde(
        default,
        alias = "reworkPolicy",
        skip_serializing_if = "Option::is_none"
    )]
    pub rework_policy: Option<AttemptPolicy>,
    #[serde(
        default,
        alias = "crashRetryPolicy",
        skip_serializing_if = "Option::is_none"
    )]
    pub crash_retry_policy: Option<AttemptPolicy>,
    #[serde(
        default,
        alias = "simplicityOverride",
        skip_serializing_if = "Option::is_none"
    )]
    pub simplicity_override: Option<SimplicityOverrideDefinition>,
}

impl DecisionPolicyDefinition {
    fn validate(&self) -> Result<(), IterativeWorkflowConfigError> {
        if matches!(self.baseline_strategy, Some(BaselineStrategy::FromConfig))
            && self.baseline_value.is_none()
        {
            return Err(IterativeWorkflowConfigError::Validation(
                "baselineStrategy 'from-config' requires baselineValue".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BaselineStrategy {
    CaptureBeforeFirstIteration,
    FromConfig,
    FromLastRun,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum KeepCondition {
    MetricImproved,
    MetricUnchangedButSimpler,
    VerifyPassed,
    GuardPassed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DiscardCondition {
    MetricRegressed,
    MetricUnchanged,
    VerifyFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptPolicy {
    #[serde(
        default,
        alias = "maxAttempts",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_attempts: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SimplicityOverrideDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(
        default,
        alias = "minImprovementPercent",
        skip_serializing_if = "Option::is_none"
    )]
    pub min_improvement_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspacePolicyDefinition {
    #[serde(
        default,
        alias = "mutationMode",
        skip_serializing_if = "Option::is_none"
    )]
    pub mutation_mode: Option<MutationMode>,
    #[serde(
        default,
        alias = "protectedPaths",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub protected_paths: Vec<String>,
    #[serde(alias = "snapshotStrategy")]
    pub snapshot_strategy: SnapshotStrategy,
    #[serde(
        default,
        alias = "commitPolicy",
        skip_serializing_if = "Option::is_none"
    )]
    pub commit_policy: Option<CommitPolicyDefinition>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MutationMode {
    Tracked,
    Untracked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SnapshotStrategy {
    GitBranchPerIteration,
    GitStashStack,
    PatchFile,
    WorktreeFork,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitPolicyDefinition {
    #[serde(
        default,
        alias = "commitKeptIterations",
        skip_serializing_if = "Option::is_none"
    )]
    pub commit_kept_iterations: Option<bool>,
    #[serde(
        default,
        alias = "messageTemplate",
        skip_serializing_if = "Option::is_none"
    )]
    pub message_template: Option<String>,
    #[serde(
        default,
        alias = "squashOnCompletion",
        skip_serializing_if = "Option::is_none"
    )]
    pub squash_on_completion: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactDefinition {
    #[serde(default, alias = "rootDir", skip_serializing_if = "Option::is_none")]
    pub root_dir: Option<String>,
    #[serde(default, alias = "runDir", skip_serializing_if = "Option::is_none")]
    pub run_dir: Option<String>,
    #[serde(
        default,
        alias = "iterationLog",
        skip_serializing_if = "Option::is_none"
    )]
    pub iteration_log: Option<ArtifactFileDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<ArtifactFileDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactFileDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalPolicyDefinition {
    #[serde(
        default,
        alias = "requireHumanApprovalFor",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub require_human_approval_for: Vec<ApprovalAction>,
    #[serde(
        default,
        alias = "allowAutoApproveWhen",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub allow_auto_approve_when: Vec<AutoApproveCondition>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalAction {
    ShipAction,
    RollbackAction,
    ExternalWrite,
    DestructiveRefactor,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AutoApproveCondition {
    DryRunPassed,
    NoBlockers,
    AllGuardsPassed,
    MetricThresholdMet,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityConfig {
    #[serde(
        default,
        alias = "coverageTargets",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub coverage_targets: Vec<SecurityCoverageTarget>,
    #[serde(
        default,
        alias = "failOnSeverity",
        skip_serializing_if = "Option::is_none"
    )]
    pub fail_on_severity: Option<SeverityLevel>,
    #[serde(default, alias = "diffMode", skip_serializing_if = "Option::is_none")]
    pub diff_mode: Option<bool>,
    #[serde(default, alias = "autoFix", skip_serializing_if = "Option::is_none")]
    pub auto_fix: Option<bool>,
    #[serde(
        default,
        alias = "requiredEvidence",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub required_evidence: Vec<SecurityEvidenceRequirement>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SecurityCoverageTarget {
    OwaspTop10,
    Stride,
    SupplyChain,
    CweTop25,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SeverityLevel {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SecurityEvidenceRequirement {
    FileLine,
    AttackScenario,
    SeverityJustification,
    CweId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DebugConfig {
    pub symptom: String,
    #[serde(
        default,
        alias = "reproCommand",
        skip_serializing_if = "Option::is_none"
    )]
    pub repro_command: Option<CommandDefinition>,
    #[serde(
        default,
        alias = "minSeverity",
        skip_serializing_if = "Option::is_none"
    )]
    pub min_severity: Option<DebugSeverityLevel>,
    #[serde(
        default,
        alias = "requiredEvidence",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub required_evidence: Vec<DebugEvidenceRequirement>,
    #[serde(
        default,
        alias = "maxHypotheses",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_hypotheses: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DebugSeverityLevel {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DebugEvidenceRequirement {
    FileLine,
    ReproSteps,
    HypothesisLog,
    StackTrace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<FixTarget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<FixCategory>,
    #[serde(default, alias = "fromDebug", skip_serializing_if = "Option::is_none")]
    pub from_debug: Option<bool>,
    #[serde(
        default,
        alias = "debugRunRef",
        skip_serializing_if = "Option::is_none"
    )]
    pub debug_run_ref: Option<String>,
    #[serde(default, alias = "stopOnZero", skip_serializing_if = "Option::is_none")]
    pub stop_on_zero: Option<bool>,
}

impl FixConfig {
    fn validate(&self) -> Result<(), IterativeWorkflowConfigError> {
        if self.from_debug.unwrap_or(false) && self.debug_run_ref.is_none() {
            return Err(IterativeWorkflowConfigError::Validation(
                "fix.fromDebug=true requires debugRunRef".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FixTarget {
    AutoDetect,
    Explicit,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FixCategory {
    Test,
    Type,
    Lint,
    Build,
    Runtime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifierConfig {
    pub model: ModelRef,
    pub criteria: Vec<VerifierCriterionDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granularity: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repetitions: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<VerifierSelectionStrategy>,
    #[serde(
        default,
        alias = "maxCandidates",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_candidates: Option<u32>,
    #[serde(
        default,
        alias = "useLogprobs",
        skip_serializing_if = "Option::is_none"
    )]
    pub use_logprobs: Option<bool>,
    #[serde(
        default,
        alias = "traceFormat",
        skip_serializing_if = "Option::is_none"
    )]
    pub trace_format: Option<VerifierTraceFormat>,
}

impl VerifierConfig {
    fn validate(&self) -> Result<(), IterativeWorkflowConfigError> {
        if self.criteria.is_empty() {
            return Err(IterativeWorkflowConfigError::Validation(
                "verifier.criteria must contain at least one criterion".to_string(),
            ));
        }
        if self.criteria.len() > VERIFIER_MAX_CRITERIA {
            return Err(IterativeWorkflowConfigError::Validation(format!(
                "verifier.criteria must contain at most {VERIFIER_MAX_CRITERIA} criteria"
            )));
        }
        for criterion in &self.criteria {
            criterion.validate()?;
        }
        if matches!(self.repetitions, Some(0)) {
            return Err(IterativeWorkflowConfigError::Validation(
                "verifier.repetitions must be at least 1".to_string(),
            ));
        }
        if matches!(self.repetitions, Some(value) if value > VERIFIER_MAX_REPETITIONS) {
            return Err(IterativeWorkflowConfigError::Validation(format!(
                "verifier.repetitions must be at most {VERIFIER_MAX_REPETITIONS}"
            )));
        }
        if matches!(self.max_candidates, Some(0 | 1)) {
            return Err(IterativeWorkflowConfigError::Validation(
                "verifier.maxCandidates must be at least 2".to_string(),
            ));
        }
        if matches!(self.max_candidates, Some(value) if value > VERIFIER_MAX_CANDIDATES) {
            return Err(IterativeWorkflowConfigError::Validation(format!(
                "verifier.maxCandidates must be at most {VERIFIER_MAX_CANDIDATES}"
            )));
        }
        if matches!(self.granularity, Some(0) | Some(21..)) {
            return Err(IterativeWorkflowConfigError::Validation(
                "verifier.granularity must be between 1 and 20".to_string(),
            ));
        }
        let max_candidates = self.max_candidates.unwrap_or(3);
        let repetitions = self.repetitions.unwrap_or(1);
        let pair_count = max_candidates.saturating_mul(max_candidates.saturating_sub(1)) / 2;
        let judge_jobs = pair_count
            .saturating_mul(self.criteria.len() as u32)
            .saturating_mul(repetitions);
        if judge_jobs > VERIFIER_MAX_PAIRWISE_JUDGE_JOBS {
            return Err(IterativeWorkflowConfigError::Validation(format!(
                "verifier pairwise judge budget {judge_jobs} exceeds limit {VERIFIER_MAX_PAIRWISE_JUDGE_JOBS}; reduce maxCandidates, criteria, or repetitions"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifierCriterionDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregation: Option<VerifierCriterionAggregation>,
}

impl VerifierCriterionDefinition {
    fn validate(&self) -> Result<(), IterativeWorkflowConfigError> {
        if matches!(self.weight, Some(weight) if weight <= 0.0) {
            return Err(IterativeWorkflowConfigError::Validation(format!(
                "verifier criterion '{}' must have weight > 0",
                self.id
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum VerifierCriterionAggregation {
    WinnerVote,
    ScoreMargin,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VerifierSelectionStrategy {
    RoundRobin,
    Tournament,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VerifierTraceFormat {
    Compact,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShipConfig {
    #[serde(rename = "type")]
    pub ship_type: ShipType,
    #[serde(default, alias = "dryRun", skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(
        default,
        alias = "autoApprove",
        skip_serializing_if = "Option::is_none"
    )]
    pub auto_approve: Option<bool>,
    #[serde(
        default,
        alias = "rollbackEnabled",
        skip_serializing_if = "Option::is_none"
    )]
    pub rollback_enabled: Option<bool>,
    #[serde(
        default,
        alias = "monitorDurationMs",
        skip_serializing_if = "Option::is_none"
    )]
    pub monitor_duration_ms: Option<u64>,
    #[serde(
        default,
        alias = "monitorCommand",
        skip_serializing_if = "Option::is_none"
    )]
    pub monitor_command: Option<CommandDefinition>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ShipType {
    Deployment,
    Release,
    Publish,
    Migration,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iterative_workflow_run_requires_core_blocks() {
        let err = IterativeWorkflowConfig::load_from_str(
            r#"{
              "workflow": { "kind": "autoresearch", "mode": "run" }
            }"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("objective"));
        assert!(err.contains("iterationPolicy"));
    }

    #[test]
    fn iterative_workflow_plan_loads_without_objective() {
        let config = IterativeWorkflowConfig::load_from_str(
            r#"{
              "workflow": { "kind": "autoresearch", "mode": "plan" }
            }"#,
        )
        .unwrap();
        assert_eq!(config.base_preset_hint(), WorkflowBasePreset::Prometheus);
    }

    #[test]
    fn iterative_workflow_validates_metric_requirements() {
        let err = IterativeWorkflowConfig::load_from_str(
            r#"{
              "workflow": { "kind": "autoresearch", "mode": "run" },
              "objective": {
                "goal": "Improve test coverage",
                "scope": { "include": ["src/**/*.rs"] },
                "direction": "higher-is-better",
                "metric": { "kind": "numeric-extract" },
                "verify": { "command": "cargo test" }
              },
              "iterationPolicy": { "mode": "bounded", "maxIterations": 3 },
              "decisionPolicy": {},
              "workspacePolicy": { "snapshotStrategy": "patch-file" }
            }"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("pattern"));
    }

    #[test]
    fn iterative_workflow_verify_requires_verifier_block() {
        let err = IterativeWorkflowConfig::load_from_str(
            r#"{
              "workflow": { "kind": "autoresearch", "mode": "verify" },
              "objective": {
                "goal": "Improve test coverage",
                "scope": { "include": ["src/**/*.rs"] },
                "direction": "higher-is-better",
                "metric": { "kind": "numeric-extract", "pattern": "score=(\\d+)" },
                "verify": { "command": "cargo test" }
              },
              "decisionPolicy": {},
              "workspacePolicy": { "snapshotStrategy": "patch-file" }
            }"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("verifier"));
    }

    #[test]
    fn iterative_workflow_verify_loads_with_verifier_block() {
        let config = IterativeWorkflowConfig::load_from_str(
            r#"{
              "workflow": { "kind": "autoresearch", "mode": "verify" },
              "objective": {
                "goal": "Choose the better patch",
                "scope": { "include": ["src/**/*.rs"] },
                "direction": "higher-is-better",
                "metric": { "kind": "numeric-extract", "pattern": "score=(\\d+)" },
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
                    "description": "Choose the candidate that best satisfies the request."
                  }
                ],
                "repetitions": 1,
                "maxCandidates": 2,
                "selection": "round-robin",
                "traceFormat": "compact"
              }
            }"#,
        )
        .unwrap();
        assert_eq!(config.workflow.mode, IterativeWorkflowMode::Verify);
        assert_eq!(config.base_preset_hint(), WorkflowBasePreset::Hephaestus);
        assert!(config.verifier.is_some());
    }

    #[test]
    fn iterative_workflow_verify_enforces_cost_controls() {
        let err = IterativeWorkflowConfig::load_from_str(
            r#"{
              "workflow": { "kind": "autoresearch", "mode": "verify" },
              "objective": {
                "goal": "Choose the better patch",
                "scope": { "include": ["src/**/*.rs"] },
                "direction": "higher-is-better",
                "metric": { "kind": "numeric-extract", "pattern": "score=(\\d+)" },
                "verify": { "command": "cargo test" }
              },
              "decisionPolicy": {},
              "workspacePolicy": { "snapshotStrategy": "patch-file" },
              "verifier": {
                "model": { "providerId": "openai", "modelId": "gpt-5" },
                "criteria": [
                  { "id": "spec", "name": "Spec", "description": "Spec adherence" }
                ],
                "repetitions": 11,
                "maxCandidates": 2
              }
            }"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("repetitions"));
    }
}
