use crate::iterative_workflow::{
    DebugConfig, FixConfig, IterativeWorkflowConfig, IterativeWorkflowMode, SecurityConfig,
    ShipConfig,
};
use crate::workflow_artifacts::{
    WorkflowModeArtifact, WorkflowModeArtifactEntry, WorkflowModeReport,
};
use crate::{ExecutionContext, OrchestratorOutput};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::collections::HashMap;

pub(crate) const WORKFLOW_MODE_ARTIFACTS_METADATA_KEY: &str = "workflowModeArtifacts";

pub(crate) struct ModeIterationContext {
    pub iteration: u32,
    pub decision: String,
    pub gate_status: String,
    pub objective_satisfied: bool,
    pub metric_value: Option<f64>,
    pub verify_passed: bool,
    pub guard_passed: Option<bool>,
    pub structured_artifacts: Vec<WorkflowModeArtifact>,
}

pub(crate) struct ModeFinalizeContext {
    pub iterations_completed: u32,
    pub objective_satisfied: bool,
    pub final_decision: Option<String>,
}

pub(crate) struct ModeGateAnnotation {
    pub summary_suffix: Option<String>,
    pub next_input_prefix: Option<String>,
    pub iteration_note: Option<String>,
}

pub(crate) trait WorkflowModeProtocol: Send + Sync {
    fn mode(&self) -> &'static str;
    fn protocol_name(&self) -> &'static str;
    fn config_notes(&self) -> Vec<String>;
    fn iteration_brief(&self, iteration: u32) -> Option<String>;
    fn annotate_gate(&self, ctx: &ModeIterationContext) -> ModeGateAnnotation;
    fn record_iteration(&mut self, ctx: &ModeIterationContext);
    fn export_artifacts(&self) -> Vec<WorkflowModeArtifact>;
    fn finalize_report(
        &self,
        ctx: &ModeFinalizeContext,
        iteration_notes: &[String],
    ) -> WorkflowModeReport;
}

pub(crate) fn mode_protocol_for(config: &IterativeWorkflowConfig) -> Box<dyn WorkflowModeProtocol> {
    match config.workflow.mode {
        IterativeWorkflowMode::Run => Box::new(RunModeProtocol::new(config)),
        IterativeWorkflowMode::Plan => Box::new(PlanModeProtocol::new()),
        IterativeWorkflowMode::Security => {
            Box::new(SecurityModeProtocol::new(config.security.as_ref()))
        }
        IterativeWorkflowMode::Debug => Box::new(DebugModeProtocol::new(config.debug.as_ref())),
        IterativeWorkflowMode::Fix => Box::new(FixModeProtocol::new(config.fix.as_ref())),
        IterativeWorkflowMode::Verify => Box::new(VerifyModeProtocol::new(config)),
        IterativeWorkflowMode::Ship => Box::new(ShipModeProtocol::new(config.ship.as_ref())),
    }
}

#[derive(Default)]
struct ModeArtifactRegistry {
    name: String,
    description: String,
    entries: BTreeMap<String, WorkflowModeArtifactEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkflowModeArtifactEnvelope {
    #[serde(default, alias = "protocolArtifacts", alias = "modeArtifacts")]
    workflow_mode_artifacts: Vec<WorkflowModeArtifact>,
}

impl ModeArtifactRegistry {
    fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            entries: BTreeMap::new(),
        }
    }

    fn upsert(
        &mut self,
        key: impl Into<String>,
        iteration: Option<u32>,
        status: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
        evidence: Vec<String>,
    ) {
        let key = key.into();
        let status = status.into();
        let title = title.into();
        let detail = detail.into();
        match self.entries.get_mut(&key) {
            Some(entry) => {
                entry.iteration = iteration.or(entry.iteration);
                entry.status = status;
                entry.title = title;
                entry.detail = detail;
                merge_evidence(&mut entry.evidence, evidence);
            }
            None => {
                self.entries.insert(
                    key.clone(),
                    WorkflowModeArtifactEntry {
                        iteration,
                        key,
                        status,
                        title,
                        detail,
                        evidence: dedupe_evidence(evidence),
                    },
                );
            }
        }
    }

    fn export(&self) -> WorkflowModeArtifact {
        WorkflowModeArtifact {
            name: self.name.clone(),
            description: self.description.clone(),
            entries: self.entries.values().cloned().collect(),
        }
    }

    fn status_summary(&self) -> String {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for entry in self.entries.values() {
            *counts.entry(entry.status.as_str()).or_default() += 1;
        }
        if counts.is_empty() {
            return "no entries recorded".to_string();
        }
        counts
            .into_iter()
            .map(|(status, count)| format!("{count} {status}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn merge_evidence(existing: &mut Vec<String>, incoming: Vec<String>) {
    for item in incoming {
        if !existing.iter().any(|current| current == &item) {
            existing.push(item);
        }
    }
}

fn mode_artifact_name(mode: IterativeWorkflowMode) -> &'static str {
    match mode {
        IterativeWorkflowMode::Run => "objective-log",
        IterativeWorkflowMode::Plan => "planning-log",
        IterativeWorkflowMode::Security => "finding-registry",
        IterativeWorkflowMode::Debug => "hypothesis-log",
        IterativeWorkflowMode::Fix => "repair-log",
        IterativeWorkflowMode::Verify => "candidate-registry",
        IterativeWorkflowMode::Ship => "ship-checklist",
    }
}

pub(crate) fn mode_artifact_contract(mode: IterativeWorkflowMode) -> String {
    let artifact_name = mode_artifact_name(mode);
    let mode_name = mode.as_str();
    format!(
        "## Workflow Mode Artifact Contract\n\
If you produce structured {mode_name} state, append a fenced `json` block with \
{{\"workflowModeArtifacts\":[{{\"name\":\"{artifact_name}\",\"description\":\"short description\",\"entries\":[{{\"key\":\"stable-id\",\"status\":\"state\",\"title\":\"short title\",\"detail\":\"what changed\",\"evidence\":[\"proof\"],\"iteration\":1}}]}}]}}.\n\
Only include artifacts you actually updated in this round. Keep free-form prose outside the JSON block."
    )
}

pub(crate) fn attach_mode_artifacts_metadata(
    output: &mut OrchestratorOutput,
    exec_ctx: &ExecutionContext,
) {
    let mode = exec_ctx
        .metadata
        .get("workflow_mode")
        .and_then(Value::as_str)
        .and_then(parse_mode_name);
    let Some(mode) = mode else {
        return;
    };
    let artifacts = extract_mode_artifacts_from_content(&output.content, mode);
    if artifacts.is_empty() {
        return;
    }
    output.metadata.insert(
        WORKFLOW_MODE_ARTIFACTS_METADATA_KEY.to_string(),
        json!(artifacts),
    );
}

pub(crate) fn mode_artifacts_from_metadata(
    metadata: &HashMap<String, Value>,
) -> Vec<WorkflowModeArtifact> {
    metadata
        .get(WORKFLOW_MODE_ARTIFACTS_METADATA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<WorkflowModeArtifact>>(value).ok())
        .unwrap_or_default()
}

pub(crate) fn mode_artifacts_from_outputs(
    outputs: &[&OrchestratorOutput],
) -> Vec<WorkflowModeArtifact> {
    let mut artifacts = Vec::new();
    for output in outputs {
        artifacts.extend(mode_artifacts_from_metadata(&output.metadata));
    }
    artifacts
}

fn parse_mode_name(value: &str) -> Option<IterativeWorkflowMode> {
    match value.trim() {
        "run" => Some(IterativeWorkflowMode::Run),
        "plan" => Some(IterativeWorkflowMode::Plan),
        "security" => Some(IterativeWorkflowMode::Security),
        "debug" => Some(IterativeWorkflowMode::Debug),
        "fix" => Some(IterativeWorkflowMode::Fix),
        "verify" => Some(IterativeWorkflowMode::Verify),
        "ship" => Some(IterativeWorkflowMode::Ship),
        _ => None,
    }
}

fn extract_mode_artifacts_from_content(
    content: &str,
    mode: IterativeWorkflowMode,
) -> Vec<WorkflowModeArtifact> {
    let mut artifacts = parse_mode_artifact_payload(content);
    if artifacts.is_empty() {
        for block in json_fenced_blocks(content) {
            let parsed = parse_mode_artifact_payload(block);
            if !parsed.is_empty() {
                artifacts.extend(parsed);
            }
        }
    }
    let expected_name = mode_artifact_name(mode);
    artifacts
        .into_iter()
        .filter(|artifact| artifact.name == expected_name)
        .collect()
}

fn parse_mode_artifact_payload(content: &str) -> Vec<WorkflowModeArtifact> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(envelope) = serde_json::from_str::<WorkflowModeArtifactEnvelope>(trimmed) {
        if !envelope.workflow_mode_artifacts.is_empty() {
            return envelope.workflow_mode_artifacts;
        }
    }
    if let Ok(artifacts) = serde_json::from_str::<Vec<WorkflowModeArtifact>>(trimmed) {
        if !artifacts.is_empty() {
            return artifacts;
        }
    }
    if let Ok(single) = serde_json::from_str::<WorkflowModeArtifact>(trimmed) {
        return vec![single];
    }
    Vec::new()
}

fn json_fenced_blocks(content: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find("```") {
        let after_tick = &rest[start + 3..];
        let Some(header_end) = after_tick.find('\n') else {
            break;
        };
        let header = after_tick[..header_end].trim().to_ascii_lowercase();
        let body = &after_tick[header_end + 1..];
        let Some(end) = body.find("```") else {
            break;
        };
        if header.is_empty() || header == "json" {
            blocks.push(body[..end].trim());
        }
        rest = &body[end + 3..];
    }
    blocks
}

fn dedupe_evidence(values: Vec<String>) -> Vec<String> {
    let mut merged = Vec::new();
    merge_evidence(&mut merged, values);
    merged
}

fn bool_evidence(label: &str, value: Option<bool>) -> String {
    match value {
        Some(true) => format!("{label}=true"),
        Some(false) => format!("{label}=false"),
        None => format!("{label}=unknown"),
    }
}

fn security_status(ctx: &ModeIterationContext) -> &'static str {
    let guard_clear = ctx.guard_passed.unwrap_or(true);
    if ctx.verify_passed && guard_clear {
        if ctx.objective_satisfied || ctx.gate_status == "done" {
            "verified"
        } else {
            "needs-evidence"
        }
    } else {
        "open"
    }
}

fn debug_status(ctx: &ModeIterationContext) -> &'static str {
    if ctx.gate_status == "blocked" {
        "stalled"
    } else if ctx.objective_satisfied || ctx.gate_status == "done" {
        "confirmed"
    } else if ctx.verify_passed && ctx.decision == "discard" {
        "disproven"
    } else {
        "active"
    }
}

fn fix_status(ctx: &ModeIterationContext) -> &'static str {
    if ctx.objective_satisfied || ctx.gate_status == "done" {
        "resolved"
    } else if ctx.verify_passed && matches!(ctx.decision.as_str(), "keep" | "stop-satisfied") {
        "reduced"
    } else {
        "open"
    }
}

fn merge_external_artifacts(
    registry: &mut ModeArtifactRegistry,
    artifacts: &[WorkflowModeArtifact],
    expected_name: &str,
) {
    for artifact in artifacts {
        if artifact.name != expected_name {
            continue;
        }
        for entry in &artifact.entries {
            registry.upsert(
                entry.key.clone(),
                entry.iteration,
                entry.status.clone(),
                entry.title.clone(),
                entry.detail.clone(),
                entry.evidence.clone(),
            );
        }
    }
}

struct RunModeProtocol {
    goal: Option<String>,
    entries: Vec<WorkflowModeArtifactEntry>,
}

impl RunModeProtocol {
    fn new(config: &IterativeWorkflowConfig) -> Self {
        Self {
            goal: config
                .objective
                .as_ref()
                .map(|objective| objective.goal.clone()),
            entries: Vec::new(),
        }
    }
}

impl WorkflowModeProtocol for RunModeProtocol {
    fn mode(&self) -> &'static str {
        "run"
    }

    fn protocol_name(&self) -> &'static str {
        "objective-verify-keep-discard"
    }

    fn config_notes(&self) -> Vec<String> {
        self.goal
            .as_ref()
            .map(|goal| vec![format!("Optimize objective: {goal}")])
            .unwrap_or_default()
    }

    fn iteration_brief(&self, iteration: u32) -> Option<String> {
        Some(format!(
            "Iteration {iteration}: improve the configured objective without regressing verification."
        ))
    }

    fn annotate_gate(&self, ctx: &ModeIterationContext) -> ModeGateAnnotation {
        let metric_note = ctx
            .metric_value
            .map(|value| format!("Current metric {:.4}.", value))
            .unwrap_or_else(|| "Metric unavailable.".to_string());
        ModeGateAnnotation {
            summary_suffix: Some("Run protocol preserved objective-first optimization.".to_string()),
            next_input_prefix: (ctx.gate_status == "continue").then(|| {
                format!("{metric_note} Keep the verified state while pursuing the next measurable improvement.")
            }),
            iteration_note: Some(format!(
                "iter {} => decision={} verify_passed={}",
                ctx.iteration, ctx.decision, ctx.verify_passed
            )),
        }
    }

    fn record_iteration(&mut self, ctx: &ModeIterationContext) {
        self.entries.push(WorkflowModeArtifactEntry {
            iteration: Some(ctx.iteration),
            key: format!("run-iter-{}", ctx.iteration),
            status: ctx.gate_status.clone(),
            title: "Objective iteration".to_string(),
            detail: format!(
                "Decision {} with verify_passed={} and metric={:?}",
                ctx.decision, ctx.verify_passed, ctx.metric_value
            ),
            evidence: vec![format!("objective_satisfied={}", ctx.objective_satisfied)],
        });
    }

    fn export_artifacts(&self) -> Vec<WorkflowModeArtifact> {
        vec![WorkflowModeArtifact {
            name: "objective-log".to_string(),
            description: "Objective-first iteration registry for run mode.".to_string(),
            entries: self.entries.clone(),
        }]
    }

    fn finalize_report(
        &self,
        ctx: &ModeFinalizeContext,
        iteration_notes: &[String],
    ) -> WorkflowModeReport {
        WorkflowModeReport {
            mode: self.mode().to_string(),
            protocol: self.protocol_name().to_string(),
            config_notes: self.config_notes(),
            iteration_notes: iteration_notes.to_vec(),
            final_notes: vec![format!(
                "Run protocol finished after {} iterations with decision {:?}.",
                ctx.iterations_completed, ctx.final_decision
            )],
        }
    }
}

struct VerifyModeProtocol {
    goal: Option<String>,
    criteria_count: usize,
    entries: Vec<WorkflowModeArtifactEntry>,
}

impl VerifyModeProtocol {
    fn new(config: &IterativeWorkflowConfig) -> Self {
        Self {
            goal: config
                .objective
                .as_ref()
                .map(|objective| objective.goal.clone()),
            criteria_count: config
                .verifier
                .as_ref()
                .map(|verifier| verifier.criteria.len())
                .unwrap_or(0),
            entries: Vec::new(),
        }
    }
}

impl WorkflowModeProtocol for VerifyModeProtocol {
    fn mode(&self) -> &'static str {
        "verify"
    }

    fn protocol_name(&self) -> &'static str {
        "candidate-admission-and-selection"
    }

    fn config_notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if let Some(goal) = self.goal.as_ref() {
            notes.push(format!("Compare candidates against objective: {goal}"));
        }
        notes.push(format!(
            "Verifier criteria configured: {}",
            self.criteria_count
        ));
        notes
    }

    fn iteration_brief(&self, iteration: u32) -> Option<String> {
        Some(format!(
            "Iteration {iteration}: generate a candidate that is not only correct, but auditable enough to win later verifier comparison."
        ))
    }

    fn annotate_gate(&self, ctx: &ModeIterationContext) -> ModeGateAnnotation {
        ModeGateAnnotation {
            summary_suffix: Some(
                "Verify protocol tracks viable candidates for later selection.".to_string(),
            ),
            next_input_prefix: (ctx.gate_status == "continue").then(|| {
                "Preserve strong verification evidence and keep the result comparable against prior candidates.".to_string()
            }),
            iteration_note: Some(format!(
                "iter {} => verify decision={} verify_passed={}",
                ctx.iteration, ctx.decision, ctx.verify_passed
            )),
        }
    }

    fn record_iteration(&mut self, ctx: &ModeIterationContext) {
        let status = if matches!(ctx.decision.as_str(), "keep" | "stop-satisfied") {
            "candidate"
        } else {
            ctx.gate_status.as_str()
        };
        self.entries.push(WorkflowModeArtifactEntry {
            iteration: Some(ctx.iteration),
            key: format!("cand-{:03}", ctx.iteration),
            status: status.to_string(),
            title: "Verifier candidate".to_string(),
            detail: format!(
                "Decision {} with verify_passed={} and metric={:?}",
                ctx.decision, ctx.verify_passed, ctx.metric_value
            ),
            evidence: vec![format!("objective_satisfied={}", ctx.objective_satisfied)],
        });
    }

    fn export_artifacts(&self) -> Vec<WorkflowModeArtifact> {
        vec![WorkflowModeArtifact {
            name: "candidate-registry".to_string(),
            description: "Candidate admission registry for verify mode.".to_string(),
            entries: self.entries.clone(),
        }]
    }

    fn finalize_report(
        &self,
        ctx: &ModeFinalizeContext,
        iteration_notes: &[String],
    ) -> WorkflowModeReport {
        WorkflowModeReport {
            mode: self.mode().to_string(),
            protocol: self.protocol_name().to_string(),
            config_notes: self.config_notes(),
            iteration_notes: iteration_notes.to_vec(),
            final_notes: vec![format!(
                "Verify protocol finished after {} iterations with decision {:?}.",
                ctx.iterations_completed, ctx.final_decision
            )],
        }
    }
}

struct PlanModeProtocol {
    entries: Vec<WorkflowModeArtifactEntry>,
}

impl PlanModeProtocol {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl WorkflowModeProtocol for PlanModeProtocol {
    fn mode(&self) -> &'static str {
        "plan"
    }

    fn protocol_name(&self) -> &'static str {
        "planner-handoff"
    }

    fn config_notes(&self) -> Vec<String> {
        vec![
            "Plan mode delegates final delivery to Prometheus-style planning workflows."
                .to_string(),
        ]
    }

    fn iteration_brief(&self, iteration: u32) -> Option<String> {
        Some(format!(
            "Iteration {iteration}: preserve planning-only behavior and capture unresolved decisions explicitly."
        ))
    }

    fn annotate_gate(&self, ctx: &ModeIterationContext) -> ModeGateAnnotation {
        ModeGateAnnotation {
            summary_suffix: Some(
                "Plan protocol kept the workflow in planner-only mode.".to_string(),
            ),
            next_input_prefix: (ctx.gate_status == "continue").then(|| {
                "Record assumptions, defaults, and open decisions before the next planning round."
                    .to_string()
            }),
            iteration_note: Some(format!(
                "iter {} => planner decision={} objective_satisfied={}",
                ctx.iteration, ctx.decision, ctx.objective_satisfied
            )),
        }
    }

    fn record_iteration(&mut self, ctx: &ModeIterationContext) {
        self.entries.push(WorkflowModeArtifactEntry {
            iteration: Some(ctx.iteration),
            key: format!("plan-iter-{}", ctx.iteration),
            status: ctx.gate_status.clone(),
            title: "Planning checkpoint".to_string(),
            detail: format!(
                "Decision {} with objective_satisfied={}",
                ctx.decision, ctx.objective_satisfied
            ),
            evidence: vec!["planner-only".to_string()],
        });
    }

    fn export_artifacts(&self) -> Vec<WorkflowModeArtifact> {
        vec![WorkflowModeArtifact {
            name: "planning-log".to_string(),
            description: "Planner-only checkpoint registry.".to_string(),
            entries: self.entries.clone(),
        }]
    }

    fn finalize_report(
        &self,
        ctx: &ModeFinalizeContext,
        iteration_notes: &[String],
    ) -> WorkflowModeReport {
        WorkflowModeReport {
            mode: self.mode().to_string(),
            protocol: self.protocol_name().to_string(),
            config_notes: self.config_notes(),
            iteration_notes: iteration_notes.to_vec(),
            final_notes: vec![format!(
                "Plan protocol ended after {} iterations with decision {:?}.",
                ctx.iterations_completed, ctx.final_decision
            )],
        }
    }
}

struct SecurityModeProtocol {
    notes: Vec<String>,
    coverage_targets: Vec<String>,
    required_evidence: Vec<String>,
    registry: ModeArtifactRegistry,
}

impl SecurityModeProtocol {
    fn new(config: Option<&SecurityConfig>) -> Self {
        let mut notes = Vec::new();
        let mut coverage_targets = Vec::new();
        let mut required_evidence = Vec::new();

        if let Some(config) = config {
            if !config.coverage_targets.is_empty() {
                coverage_targets = config
                    .coverage_targets
                    .iter()
                    .map(|target| format!("{target:?}"))
                    .collect();
                notes.push(format!("Coverage targets: {:?}", config.coverage_targets));
            }
            if let Some(severity) = config.fail_on_severity {
                notes.push(format!("Fail on severity: {:?}", severity));
            }
            if !config.required_evidence.is_empty() {
                required_evidence = config
                    .required_evidence
                    .iter()
                    .map(|item| format!("{item:?}"))
                    .collect();
                notes.push(format!("Required evidence: {:?}", config.required_evidence));
            }
            if config.diff_mode == Some(true) {
                notes.push("Diff mode enabled.".to_string());
            }
            if config.auto_fix == Some(true) {
                notes.push("Auto-fix allowed for validated findings.".to_string());
            }
        }

        Self {
            notes,
            coverage_targets,
            required_evidence,
            registry: ModeArtifactRegistry::new(
                "finding-registry",
                "Security finding registry capturing evidence, attack surface, and severity posture.",
            ),
        }
    }
}

impl WorkflowModeProtocol for SecurityModeProtocol {
    fn mode(&self) -> &'static str {
        "security"
    }

    fn protocol_name(&self) -> &'static str {
        "stride-owasp-finding-registry"
    }

    fn config_notes(&self) -> Vec<String> {
        self.notes.clone()
    }

    fn iteration_brief(&self, iteration: u32) -> Option<String> {
        Some(format!(
            "Iteration {iteration}: preserve attack-surface coverage, severity rationale, and evidence traceability."
        ))
    }

    fn annotate_gate(&self, ctx: &ModeIterationContext) -> ModeGateAnnotation {
        ModeGateAnnotation {
            summary_suffix: Some(
                "Security protocol requires evidence-backed findings and explicit severity framing."
                    .to_string(),
            ),
            next_input_prefix: (ctx.gate_status == "continue").then(|| {
                "Before the next round, update the finding registry with file/line evidence, attack scenario, and severity justification.".to_string()
            }),
            iteration_note: Some(format!(
                "iter {} => security decision={} guard_passed={:?}",
                ctx.iteration, ctx.decision, ctx.guard_passed
            )),
        }
    }

    fn record_iteration(&mut self, ctx: &ModeIterationContext) {
        let coverage_targets = if self.coverage_targets.is_empty() {
            "default-scope".to_string()
        } else {
            self.coverage_targets.join(", ")
        };
        let required_evidence = if self.required_evidence.is_empty() {
            "manual-review".to_string()
        } else {
            self.required_evidence.join(", ")
        };
        let status = security_status(ctx);

        self.registry.upsert(
            "active-finding",
            Some(ctx.iteration),
            status,
            "Active security finding",
            format!(
                "Decision {} at iteration {} with verify_passed={} and guard_passed={:?}.",
                ctx.decision, ctx.iteration, ctx.verify_passed, ctx.guard_passed
            ),
            vec![
                "attack-scenario".to_string(),
                "severity-justification".to_string(),
                bool_evidence("verify_passed", Some(ctx.verify_passed)),
                bool_evidence("guard_passed", ctx.guard_passed),
            ],
        );
        self.registry.upsert(
            "coverage-review",
            Some(ctx.iteration),
            status,
            "Coverage and evidence review",
            format!(
                "Coverage targets [{}]; required evidence [{}].",
                coverage_targets, required_evidence
            ),
            vec![
                "file-line".to_string(),
                format!("coverage-targets={coverage_targets}"),
                format!("required-evidence={required_evidence}"),
            ],
        );
        merge_external_artifacts(
            &mut self.registry,
            &ctx.structured_artifacts,
            "finding-registry",
        );
    }

    fn export_artifacts(&self) -> Vec<WorkflowModeArtifact> {
        vec![self.registry.export()]
    }

    fn finalize_report(
        &self,
        ctx: &ModeFinalizeContext,
        iteration_notes: &[String],
    ) -> WorkflowModeReport {
        WorkflowModeReport {
            mode: self.mode().to_string(),
            protocol: self.protocol_name().to_string(),
            config_notes: self.config_notes(),
            iteration_notes: iteration_notes.to_vec(),
            final_notes: vec![
                format!(
                    "Security protocol completed after {} iterations; objective_satisfied={}.",
                    ctx.iterations_completed, ctx.objective_satisfied
                ),
                format!(
                    "Finding registry status: {}.",
                    self.registry.status_summary()
                ),
            ],
        }
    }
}

struct DebugModeProtocol {
    notes: Vec<String>,
    symptom: Option<String>,
    registry: ModeArtifactRegistry,
}

impl DebugModeProtocol {
    fn new(config: Option<&DebugConfig>) -> Self {
        let mut notes = Vec::new();
        let mut symptom = None;
        if let Some(config) = config {
            symptom = Some(config.symptom.clone());
            notes.push(format!("Symptom: {}", config.symptom));
            if let Some(command) = config.repro_command.as_ref() {
                notes.push(format!("Repro command: {}", command.command));
            }
            if let Some(limit) = config.max_hypotheses {
                notes.push(format!("Max hypotheses: {limit}"));
            }
            if !config.required_evidence.is_empty() {
                notes.push(format!("Required evidence: {:?}", config.required_evidence));
            }
        }
        Self {
            notes,
            symptom,
            registry: ModeArtifactRegistry::new(
                "hypothesis-log",
                "Debug hypothesis registry with repro and experiment evidence.",
            ),
        }
    }
}

impl WorkflowModeProtocol for DebugModeProtocol {
    fn mode(&self) -> &'static str {
        "debug"
    }

    fn protocol_name(&self) -> &'static str {
        "hypothesis-registry"
    }

    fn config_notes(&self) -> Vec<String> {
        self.notes.clone()
    }

    fn iteration_brief(&self, iteration: u32) -> Option<String> {
        Some(format!(
            "Iteration {iteration}: update the hypothesis registry and preserve repro evidence before changing the code path."
        ))
    }

    fn annotate_gate(&self, ctx: &ModeIterationContext) -> ModeGateAnnotation {
        ModeGateAnnotation {
            summary_suffix: Some(
                "Debug protocol expects confirmed or disproven hypotheses for each round."
                    .to_string(),
            ),
            next_input_prefix: (ctx.gate_status == "continue").then(|| {
                "Start the next round by naming the active hypothesis, the repro signal, and the experiment you will use to confirm or disprove it.".to_string()
            }),
            iteration_note: Some(format!(
                "iter {} => debug decision={} metric={:?}",
                ctx.iteration, ctx.decision, ctx.metric_value
            )),
        }
    }

    fn record_iteration(&mut self, ctx: &ModeIterationContext) {
        let symptom = self
            .symptom
            .clone()
            .unwrap_or_else(|| "unspecified symptom".to_string());
        let status = debug_status(ctx);

        self.registry.upsert(
            "primary-hypothesis",
            Some(ctx.iteration),
            status,
            "Primary hypothesis",
            format!(
                "Symptom '{}' remained under investigation at iteration {} with decision {}.",
                symptom, ctx.iteration, ctx.decision
            ),
            vec![
                "hypothesis".to_string(),
                "experiment".to_string(),
                bool_evidence("verify_passed", Some(ctx.verify_passed)),
            ],
        );
        self.registry.upsert(
            "repro-signal",
            Some(ctx.iteration),
            status,
            "Repro signal",
            format!(
                "Iteration {} metric {:?}; objective_satisfied={}.",
                ctx.iteration, ctx.metric_value, ctx.objective_satisfied
            ),
            vec![
                "repro-signal".to_string(),
                bool_evidence("objective_satisfied", Some(ctx.objective_satisfied)),
            ],
        );
        merge_external_artifacts(
            &mut self.registry,
            &ctx.structured_artifacts,
            "hypothesis-log",
        );
    }

    fn export_artifacts(&self) -> Vec<WorkflowModeArtifact> {
        vec![self.registry.export()]
    }

    fn finalize_report(
        &self,
        ctx: &ModeFinalizeContext,
        iteration_notes: &[String],
    ) -> WorkflowModeReport {
        WorkflowModeReport {
            mode: self.mode().to_string(),
            protocol: self.protocol_name().to_string(),
            config_notes: self.config_notes(),
            iteration_notes: iteration_notes.to_vec(),
            final_notes: vec![
                format!(
                    "Debug protocol completed after {} iterations with decision {:?}.",
                    ctx.iterations_completed, ctx.final_decision
                ),
                format!(
                    "Hypothesis registry status: {}.",
                    self.registry.status_summary()
                ),
            ],
        }
    }
}

struct FixModeProtocol {
    notes: Vec<String>,
    categories: Vec<String>,
    registry: ModeArtifactRegistry,
}

impl FixModeProtocol {
    fn new(config: Option<&FixConfig>) -> Self {
        let mut notes = Vec::new();
        let mut categories = Vec::new();
        if let Some(config) = config {
            if let Some(target) = config.target {
                notes.push(format!("Fix target: {:?}", target));
            }
            if !config.categories.is_empty() {
                categories = config
                    .categories
                    .iter()
                    .map(|category| format!("{category:?}"))
                    .collect();
                notes.push(format!("Fix categories: {:?}", config.categories));
            }
            if config.from_debug == Some(true) {
                notes.push(format!(
                    "Derived from debug run: {}",
                    config.debug_run_ref.as_deref().unwrap_or("<missing>")
                ));
            }
            if config.stop_on_zero == Some(true) {
                notes.push("Stop when broken-state count reaches zero.".to_string());
            }
        }
        Self {
            notes,
            categories,
            registry: ModeArtifactRegistry::new(
                "repair-log",
                "Fix mode repair ordering and remaining broken-state registry.",
            ),
        }
    }
}

impl WorkflowModeProtocol for FixModeProtocol {
    fn mode(&self) -> &'static str {
        "fix"
    }

    fn protocol_name(&self) -> &'static str {
        "broken-state-repair-ordering"
    }

    fn config_notes(&self) -> Vec<String> {
        self.notes.clone()
    }

    fn iteration_brief(&self, iteration: u32) -> Option<String> {
        Some(format!(
            "Iteration {iteration}: reduce the remaining broken states in priority order without reopening repaired paths."
        ))
    }

    fn annotate_gate(&self, ctx: &ModeIterationContext) -> ModeGateAnnotation {
        ModeGateAnnotation {
            summary_suffix: Some(
                "Fix protocol tracks remaining broken states and repair ordering.".to_string(),
            ),
            next_input_prefix: (ctx.gate_status == "continue").then(|| {
                "Before the next round, name the remaining broken state, why it is still open, and the next repair step.".to_string()
            }),
            iteration_note: Some(format!(
                "iter {} => fix decision={} objective_satisfied={}",
                ctx.iteration, ctx.decision, ctx.objective_satisfied
            )),
        }
    }

    fn record_iteration(&mut self, ctx: &ModeIterationContext) {
        let categories = if self.categories.is_empty() {
            "unspecified".to_string()
        } else {
            self.categories.join(", ")
        };
        let status = fix_status(ctx);

        self.registry.upsert(
            "primary-broken-state",
            Some(ctx.iteration),
            status,
            "Primary broken state",
            format!(
                "Iteration {} decision {} across categories [{}].",
                ctx.iteration, ctx.decision, categories
            ),
            vec![
                "remaining-broken-state".to_string(),
                "next-repair-step".to_string(),
                bool_evidence("verify_passed", Some(ctx.verify_passed)),
            ],
        );
        self.registry.upsert(
            "regression-guard",
            Some(ctx.iteration),
            status,
            "Regression guard",
            format!(
                "Repair safety at iteration {} with objective_satisfied={}.",
                ctx.iteration, ctx.objective_satisfied
            ),
            vec![
                bool_evidence("objective_satisfied", Some(ctx.objective_satisfied)),
                format!("categories={categories}"),
            ],
        );
        merge_external_artifacts(&mut self.registry, &ctx.structured_artifacts, "repair-log");
    }

    fn export_artifacts(&self) -> Vec<WorkflowModeArtifact> {
        vec![self.registry.export()]
    }

    fn finalize_report(
        &self,
        ctx: &ModeFinalizeContext,
        iteration_notes: &[String],
    ) -> WorkflowModeReport {
        WorkflowModeReport {
            mode: self.mode().to_string(),
            protocol: self.protocol_name().to_string(),
            config_notes: self.config_notes(),
            iteration_notes: iteration_notes.to_vec(),
            final_notes: vec![
                format!(
                    "Fix protocol completed after {} iterations with decision {:?}.",
                    ctx.iterations_completed, ctx.final_decision
                ),
                format!(
                    "Repair registry status: {}.",
                    self.registry.status_summary()
                ),
            ],
        }
    }
}

struct ShipModeProtocol {
    notes: Vec<String>,
    auto_approve: bool,
    rollback_enabled: bool,
    monitor_enabled: bool,
    checklist: ModeArtifactRegistry,
}

impl ShipModeProtocol {
    fn new(config: Option<&ShipConfig>) -> Self {
        let mut notes = Vec::new();
        let mut auto_approve = false;
        let mut rollback_enabled = false;
        let mut monitor_enabled = false;

        if let Some(config) = config {
            notes.push(format!("Ship type: {:?}", config.ship_type));
            if config.dry_run == Some(true) {
                notes.push("Dry-run gate enabled.".to_string());
            }
            if config.auto_approve == Some(true) {
                auto_approve = true;
                notes.push("Auto-approve allowed when approvals pass.".to_string());
            }
            if config.rollback_enabled == Some(true) {
                rollback_enabled = true;
                notes.push("Rollback is enabled.".to_string());
            }
            if let Some(duration) = config.monitor_duration_ms {
                monitor_enabled = true;
                notes.push(format!("Monitor duration: {} ms", duration));
            }
            if let Some(command) = config.monitor_command.as_ref() {
                monitor_enabled = true;
                notes.push(format!("Monitor command: {}", command.command));
            }
        }

        let mut checklist = ModeArtifactRegistry::new(
            "ship-checklist",
            "Ship readiness checklist covering dry-run, approval, rollback, and monitoring.",
        );
        checklist.upsert(
            "dry-run",
            None,
            "pending",
            "Dry-run",
            "Waiting for a successful dry-run verification.".to_string(),
            vec!["dry-run".to_string()],
        );
        checklist.upsert(
            "approval",
            None,
            "pending",
            "Approval",
            "Waiting for approval posture to pass.".to_string(),
            vec!["approval".to_string()],
        );
        checklist.upsert(
            "rollback",
            None,
            "pending",
            "Rollback",
            "Waiting for rollback readiness evidence.".to_string(),
            vec!["rollback".to_string()],
        );
        checklist.upsert(
            "monitor",
            None,
            "pending",
            "Monitor",
            "Waiting for post-ship monitoring readiness.".to_string(),
            vec!["monitor".to_string()],
        );

        Self {
            notes,
            auto_approve,
            rollback_enabled,
            monitor_enabled,
            checklist,
        }
    }

    fn checklist_status(&self, item: &str, ctx: &ModeIterationContext) -> &'static str {
        match item {
            "dry-run" => {
                if ctx.verify_passed {
                    "passed"
                } else {
                    "blocked"
                }
            }
            "approval" => {
                if ctx.objective_satisfied
                    || ctx.gate_status == "done"
                    || (self.auto_approve && ctx.verify_passed)
                {
                    "passed"
                } else if ctx.gate_status == "blocked" {
                    "blocked"
                } else {
                    "pending"
                }
            }
            "rollback" => {
                if !self.rollback_enabled || ctx.verify_passed {
                    "passed"
                } else if ctx.gate_status == "blocked" {
                    "blocked"
                } else {
                    "pending"
                }
            }
            "monitor" => {
                if !self.monitor_enabled || ctx.objective_satisfied || ctx.gate_status == "done" {
                    "passed"
                } else if ctx.gate_status == "blocked" {
                    "blocked"
                } else {
                    "pending"
                }
            }
            _ => "pending",
        }
    }
}

impl WorkflowModeProtocol for ShipModeProtocol {
    fn mode(&self) -> &'static str {
        "ship"
    }

    fn protocol_name(&self) -> &'static str {
        "checklist-dryrun-approval-rollback-monitor"
    }

    fn config_notes(&self) -> Vec<String> {
        self.notes.clone()
    }

    fn iteration_brief(&self, iteration: u32) -> Option<String> {
        Some(format!(
            "Iteration {iteration}: preserve deployment safety gates, rollback readiness, and monitoring evidence."
        ))
    }

    fn annotate_gate(&self, ctx: &ModeIterationContext) -> ModeGateAnnotation {
        ModeGateAnnotation {
            summary_suffix: Some(
                "Ship protocol requires explicit dry-run, approval, rollback, and monitor posture."
                    .to_string(),
            ),
            next_input_prefix: (ctx.gate_status == "continue").then(|| {
                "Before the next round, restate the dry-run outcome, approval state, rollback readiness, and monitor plan.".to_string()
            }),
            iteration_note: Some(format!(
                "iter {} => ship decision={} verify_passed={}",
                ctx.iteration, ctx.decision, ctx.verify_passed
            )),
        }
    }

    fn record_iteration(&mut self, ctx: &ModeIterationContext) {
        self.checklist.upsert(
            "dry-run",
            Some(ctx.iteration),
            self.checklist_status("dry-run", ctx),
            "Dry-run",
            format!(
                "Iteration {} verify_passed={}.",
                ctx.iteration, ctx.verify_passed
            ),
            vec![bool_evidence("verify_passed", Some(ctx.verify_passed))],
        );
        self.checklist.upsert(
            "approval",
            Some(ctx.iteration),
            self.checklist_status("approval", ctx),
            "Approval",
            format!(
                "Decision {} with auto_approve={}.",
                ctx.decision, self.auto_approve
            ),
            vec![bool_evidence(
                "objective_satisfied",
                Some(ctx.objective_satisfied),
            )],
        );
        self.checklist.upsert(
            "rollback",
            Some(ctx.iteration),
            self.checklist_status("rollback", ctx),
            "Rollback",
            format!(
                "Rollback enabled={} at iteration {}.",
                self.rollback_enabled, ctx.iteration
            ),
            vec![format!("rollback_enabled={}", self.rollback_enabled)],
        );
        self.checklist.upsert(
            "monitor",
            Some(ctx.iteration),
            self.checklist_status("monitor", ctx),
            "Monitor",
            format!(
                "Monitor enabled={} with gate_status={}.",
                self.monitor_enabled, ctx.gate_status
            ),
            vec![format!("monitor_enabled={}", self.monitor_enabled)],
        );
        merge_external_artifacts(
            &mut self.checklist,
            &ctx.structured_artifacts,
            "ship-checklist",
        );
    }

    fn export_artifacts(&self) -> Vec<WorkflowModeArtifact> {
        vec![self.checklist.export()]
    }

    fn finalize_report(
        &self,
        ctx: &ModeFinalizeContext,
        iteration_notes: &[String],
    ) -> WorkflowModeReport {
        WorkflowModeReport {
            mode: self.mode().to_string(),
            protocol: self.protocol_name().to_string(),
            config_notes: self.config_notes(),
            iteration_notes: iteration_notes.to_vec(),
            final_notes: vec![
                format!(
                    "Ship protocol completed after {} iterations with decision {:?}.",
                    ctx.iterations_completed, ctx.final_decision
                ),
                format!(
                    "Ship checklist status: {}.",
                    self.checklist.status_summary()
                ),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iterative_workflow::{IterativeWorkflowKind, WorkflowDescriptor};
    use std::collections::HashMap;

    fn base_config(mode: IterativeWorkflowMode) -> IterativeWorkflowConfig {
        IterativeWorkflowConfig {
            workflow: WorkflowDescriptor {
                kind: IterativeWorkflowKind::Autoresearch,
                mode,
            },
            objective: None,
            iteration_policy: None,
            decision_policy: None,
            workspace_policy: None,
            artifacts: None,
            approval_policy: None,
            security: None,
            debug: None,
            fix: None,
            verifier: None,
            ship: None,
        }
    }

    fn iteration(
        iteration: u32,
        decision: &str,
        gate_status: &str,
        objective_satisfied: bool,
        metric_value: Option<f64>,
        verify_passed: bool,
        guard_passed: Option<bool>,
    ) -> ModeIterationContext {
        ModeIterationContext {
            iteration,
            decision: decision.to_string(),
            gate_status: gate_status.to_string(),
            objective_satisfied,
            metric_value,
            verify_passed,
            guard_passed,
            structured_artifacts: Vec::new(),
        }
    }

    fn find_entry<'a>(
        artifact: &'a WorkflowModeArtifact,
        key: &str,
    ) -> &'a WorkflowModeArtifactEntry {
        artifact
            .entries
            .iter()
            .find(|entry| entry.key == key)
            .expect("artifact entry should exist")
    }

    #[test]
    fn security_protocol_updates_existing_finding_registry_entries() {
        let mut config = base_config(IterativeWorkflowMode::Security);
        config.security = Some(SecurityConfig {
            coverage_targets: vec![crate::iterative_workflow::SecurityCoverageTarget::Stride],
            fail_on_severity: Some(crate::iterative_workflow::SeverityLevel::High),
            diff_mode: Some(true),
            auto_fix: Some(false),
            required_evidence: vec![
                crate::iterative_workflow::SecurityEvidenceRequirement::FileLine,
            ],
        });
        let mut protocol = mode_protocol_for(&config);
        protocol.record_iteration(&iteration(
            1,
            "keep",
            "continue",
            false,
            Some(1.0),
            true,
            Some(true),
        ));
        protocol.record_iteration(&iteration(
            2,
            "stop-satisfied",
            "done",
            true,
            Some(2.0),
            true,
            Some(true),
        ));

        let artifacts = protocol.export_artifacts();
        assert_eq!(artifacts[0].name, "finding-registry");
        assert_eq!(artifacts[0].entries.len(), 2);
        assert_eq!(
            find_entry(&artifacts[0], "active-finding").status,
            "verified"
        );
        assert_eq!(
            find_entry(&artifacts[0], "coverage-review").iteration,
            Some(2)
        );
    }

    #[test]
    fn structured_artifact_metadata_is_extracted_from_json_fence() {
        let mut output = OrchestratorOutput {
            content: "Security review complete.\n```json\n{\"workflowModeArtifacts\":[{\"name\":\"finding-registry\",\"description\":\"Security findings\",\"entries\":[{\"iteration\":1,\"key\":\"active-finding\",\"status\":\"verified\",\"title\":\"SQL injection\",\"detail\":\"Confirmed and bounded.\",\"evidence\":[\"file-line\",\"attack-scenario\"]}]}]}\n```".to_string(),
            steps: 1,
            tool_calls_count: 0,
            metadata: HashMap::new(),
            finish_reason: crate::runtime::events::FinishReason::EndTurn,
        };
        let mut exec_ctx = ExecutionContext {
            session_id: "s".to_string(),
            workdir: "/tmp".to_string(),
            agent_name: "test".to_string(),
            metadata: HashMap::new(),
        };
        exec_ctx
            .metadata
            .insert("workflow_mode".to_string(), json!("security"));

        attach_mode_artifacts_metadata(&mut output, &exec_ctx);

        let artifacts = mode_artifacts_from_metadata(&output.metadata);
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].name, "finding-registry");
        assert_eq!(artifacts[0].entries[0].key, "active-finding");
        assert_eq!(artifacts[0].entries[0].status, "verified");
    }

    #[test]
    fn security_protocol_prefers_structured_artifact_updates_when_present() {
        let mut config = base_config(IterativeWorkflowMode::Security);
        config.security = Some(SecurityConfig {
            coverage_targets: vec![crate::iterative_workflow::SecurityCoverageTarget::Stride],
            fail_on_severity: None,
            diff_mode: Some(false),
            auto_fix: Some(false),
            required_evidence: Vec::new(),
        });
        let mut protocol = mode_protocol_for(&config);
        let mut ctx = iteration(1, "keep", "continue", false, Some(1.0), true, Some(true));
        ctx.structured_artifacts = vec![WorkflowModeArtifact {
            name: "finding-registry".to_string(),
            description: "Security findings".to_string(),
            entries: vec![WorkflowModeArtifactEntry {
                iteration: Some(1),
                key: "active-finding".to_string(),
                status: "verified".to_string(),
                title: "Structured finding".to_string(),
                detail: "Imported from scheduler structured fields.".to_string(),
                evidence: vec!["structured".to_string()],
            }],
        }];

        protocol.record_iteration(&ctx);

        let artifacts = protocol.export_artifacts();
        let active = find_entry(&artifacts[0], "active-finding");
        assert_eq!(active.status, "verified");
        assert_eq!(active.title, "Structured finding");
        assert!(active.evidence.contains(&"structured".to_string()));
    }

    #[test]
    fn debug_protocol_tracks_hypothesis_state_transitions() {
        let mut config = base_config(IterativeWorkflowMode::Debug);
        config.debug = Some(DebugConfig {
            symptom: "panic in parser".to_string(),
            repro_command: None,
            min_severity: None,
            required_evidence: Vec::new(),
            max_hypotheses: Some(4),
        });
        let mut protocol = mode_protocol_for(&config);
        protocol.record_iteration(&iteration(
            1,
            "discard",
            "continue",
            false,
            Some(1.0),
            false,
            None,
        ));
        protocol.record_iteration(&iteration(
            2,
            "stop-satisfied",
            "done",
            true,
            Some(0.0),
            true,
            None,
        ));

        let artifacts = protocol.export_artifacts();
        assert_eq!(artifacts[0].name, "hypothesis-log");
        assert_eq!(artifacts[0].entries.len(), 2);
        assert_eq!(
            find_entry(&artifacts[0], "primary-hypothesis").status,
            "confirmed"
        );
        assert_eq!(find_entry(&artifacts[0], "repro-signal").iteration, Some(2));
    }

    #[test]
    fn fix_protocol_tracks_repair_registry_state_transitions() {
        let mut config = base_config(IterativeWorkflowMode::Fix);
        config.fix = Some(FixConfig {
            target: Some(crate::iterative_workflow::FixTarget::Explicit),
            categories: vec![crate::iterative_workflow::FixCategory::Runtime],
            from_debug: None,
            debug_run_ref: None,
            stop_on_zero: Some(true),
        });
        let mut protocol = mode_protocol_for(&config);
        protocol.record_iteration(&iteration(
            1,
            "keep",
            "continue",
            false,
            Some(1.0),
            true,
            None,
        ));
        protocol.record_iteration(&iteration(
            2,
            "stop-satisfied",
            "done",
            true,
            Some(2.0),
            true,
            None,
        ));

        let artifacts = protocol.export_artifacts();
        assert_eq!(artifacts[0].name, "repair-log");
        assert_eq!(artifacts[0].entries.len(), 2);
        assert_eq!(
            find_entry(&artifacts[0], "primary-broken-state").status,
            "resolved"
        );
        assert!(find_entry(&artifacts[0], "regression-guard")
            .evidence
            .contains(&"categories=Runtime".to_string()));
    }

    #[test]
    fn ship_protocol_tracks_checklist_items_in_place() {
        let mut config = base_config(IterativeWorkflowMode::Ship);
        config.ship = Some(ShipConfig {
            ship_type: crate::iterative_workflow::ShipType::Deployment,
            dry_run: Some(true),
            auto_approve: Some(false),
            rollback_enabled: Some(true),
            monitor_duration_ms: Some(1000),
            monitor_command: None,
        });
        let mut protocol = mode_protocol_for(&config);
        protocol.record_iteration(&iteration(
            1,
            "discard",
            "continue",
            false,
            Some(1.0),
            false,
            None,
        ));
        protocol.record_iteration(&iteration(
            2,
            "stop-satisfied",
            "done",
            true,
            Some(2.0),
            true,
            None,
        ));

        let artifacts = protocol.export_artifacts();
        assert_eq!(artifacts[0].name, "ship-checklist");
        assert_eq!(artifacts[0].entries.len(), 4);
        assert_eq!(find_entry(&artifacts[0], "dry-run").status, "passed");
        assert_eq!(find_entry(&artifacts[0], "approval").status, "passed");
        assert_eq!(find_entry(&artifacts[0], "rollback").status, "passed");
        assert_eq!(find_entry(&artifacts[0], "monitor").status, "passed");
    }
}
