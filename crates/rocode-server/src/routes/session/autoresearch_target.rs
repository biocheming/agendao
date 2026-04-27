use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use rocode_config::Config as AppConfig;
use rocode_orchestrator::{
    AttemptPolicy, BaselineStrategy, CommandDefinition, DecisionPolicyDefinition, DiscardCondition,
    IterationMode, IterationPolicyDefinition, IterativeWorkflowConfig, IterativeWorkflowKind,
    IterativeWorkflowMode, IterativeWorkflowSource, KeepCondition, MetricDefinition, MetricKind,
    ObjectiveDefinition, ObjectiveDirection, SchedulerProfileConfig, ScopeDefinition,
    SkillTreeRequestPlan, SnapshotStrategy, WorkflowDescriptor, WorkspacePolicyDefinition,
};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

use super::scheduler::resolve_scheduler_profile_config;

pub(crate) const AUTORESEARCH_PROFILE_NAME: &str = "autoresearch-run";
pub(crate) const AUTORESEARCH_PROFILE_OVERRIDE_METADATA_KEY: &str = "autoresearch_profile_override";

const REQUIRED_SECTION_ORDER: [&str; 7] = [
    "Goal",
    "Scope",
    "Exclude",
    "Metric",
    "Iteration Policy",
    "Decision Policy",
    "Context Markdown",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutoresearchProfileOverrideRecord {
    pub profile_name: String,
    pub target_name: String,
    pub target_description: String,
    pub skill_root: String,
    pub profile: SchedulerProfileConfig,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedAutoresearchCommand {
    pub raw_arguments_for_execution: String,
    pub raw_arguments_for_pending: String,
    pub scheduler_profile_name: String,
    pub profile_override: Option<AutoresearchProfileOverrideRecord>,
}

#[derive(Debug, Clone)]
struct WorkspaceAutoresearchTarget {
    name: String,
    description: String,
    skill_root: PathBuf,
    skill_markdown: PathBuf,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct AutoresearchTargetError {
    message: String,
}

impl AutoresearchTargetError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct AutoresearchMetricTemplate {
    direction: AutoresearchMetricDirection,
    kind: AutoresearchMetricKind,
    pattern: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum AutoresearchMetricDirection {
    HigherIsBetter,
    LowerIsBetter,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum AutoresearchMetricKind {
    NumericExtract,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct AutoresearchIterationPolicyTemplate {
    mode: AutoresearchIterationMode,
    max_iterations: u32,
    stuck_threshold: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum AutoresearchIterationMode {
    Bounded,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct AutoresearchDecisionPolicyTemplate {
    baseline_strategy: AutoresearchBaselineStrategy,
    keep_conditions: Vec<AutoresearchKeepCondition>,
    discard_conditions: Vec<AutoresearchDiscardCondition>,
    crash_retry_max_attempts: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum AutoresearchBaselineStrategy {
    CaptureBeforeFirstIteration,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum AutoresearchKeepCondition {
    MetricImproved,
    VerifyPassed,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum AutoresearchDiscardCondition {
    MetricRegressed,
    MetricUnchanged,
    VerifyFailed,
}

pub(crate) fn resolve_autoresearch_command(
    config: &AppConfig,
    workspace_root: &str,
    raw_arguments: &str,
) -> Result<ResolvedAutoresearchCommand, AutoresearchTargetError> {
    let workspace_root = Path::new(workspace_root);
    let targets = discover_workspace_targets(workspace_root);
    let trimmed = raw_arguments.trim();
    let explicit = parse_explicit_target_request(trimmed, &targets);

    if let Some(explicit) = explicit {
        let matches = targets
            .iter()
            .filter(|target| target.name == explicit.target_name)
            .cloned()
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return Err(AutoresearchTargetError::new(format!(
                "Autoresearch target '{}' was not found in this workspace.\nRun /autoresearch only when a default target exists or only one target is available.",
                explicit.target_name
            )));
        }
        if matches.len() > 1 {
            return Err(ambiguous_target_error(&targets));
        }
        let target = &matches[0];
        let override_record = compile_workspace_target(config, workspace_root, target)?;
        return Ok(ResolvedAutoresearchCommand {
            raw_arguments_for_execution: explicit.remainder.to_string(),
            raw_arguments_for_pending: trimmed.to_string(),
            scheduler_profile_name: AUTORESEARCH_PROFILE_NAME.to_string(),
            profile_override: Some(override_record),
        });
    }

    match targets.len() {
        0 => Ok(ResolvedAutoresearchCommand {
            raw_arguments_for_execution: trimmed.to_string(),
            raw_arguments_for_pending: trimmed.to_string(),
            scheduler_profile_name: AUTORESEARCH_PROFILE_NAME.to_string(),
            profile_override: None,
        }),
        1 => {
            let target = &targets[0];
            let override_record = compile_workspace_target(config, workspace_root, target)?;
            Ok(ResolvedAutoresearchCommand {
                raw_arguments_for_execution: trimmed.to_string(),
                raw_arguments_for_pending: trimmed.to_string(),
                scheduler_profile_name: AUTORESEARCH_PROFILE_NAME.to_string(),
                profile_override: Some(override_record),
            })
        }
        _ => Err(ambiguous_target_error(&targets)),
    }
}

fn ambiguous_target_error(targets: &[WorkspaceAutoresearchTarget]) -> AutoresearchTargetError {
    let mut names = targets
        .iter()
        .map(|target| target.name.clone())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();

    let mut lines = vec![
        "/autoresearch is ambiguous in this workspace.".to_string(),
        "Available autoresearch skills:".to_string(),
    ];
    lines.extend(names.iter().map(|name| format!("- {}", name)));
    if let Some(first) = names.first() {
        lines.push(String::new());
        lines.push("Run:".to_string());
        lines.push(format!("  /autoresearch {}", first));
    }
    AutoresearchTargetError::new(lines.join("\n"))
}

fn discover_workspace_targets(workspace_root: &Path) -> Vec<WorkspaceAutoresearchTarget> {
    let skills_root = workspace_root.join(".rocode").join("skills");
    let Ok(entries) = fs::read_dir(&skills_root) else {
        return Vec::new();
    };

    let mut targets = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_markdown = path.join("SKILL.md");
        if !skill_markdown.is_file() {
            continue;
        }
        let Ok(content) = fs::read_to_string(&skill_markdown) else {
            continue;
        };
        let Ok((frontmatter, _)) = split_frontmatter(&content) else {
            continue;
        };
        let Some(candidate) =
            parse_workspace_target_frontmatter(&frontmatter, &path, &skill_markdown)
        else {
            continue;
        };
        targets.push(candidate);
    }

    targets.sort_by(|left, right| left.name.cmp(&right.name));
    targets
}

fn parse_workspace_target_frontmatter(
    yaml: &str,
    skill_root: &Path,
    skill_markdown: &Path,
) -> Option<WorkspaceAutoresearchTarget> {
    let parsed = serde_yaml::from_str::<Value>(yaml).ok()?;
    let mapping = parsed.as_mapping()?;
    let name = mapping
        .get(Value::String("name".to_string()))?
        .as_str()?
        .trim()
        .to_string();
    let description = mapping
        .get(Value::String("description".to_string()))?
        .as_str()?
        .trim()
        .to_string();
    let kind = mapping
        .get(Value::String("kind".to_string()))?
        .as_str()?
        .trim();

    if name.is_empty() || description.is_empty() || kind != "autoresearch" {
        return None;
    }

    Some(WorkspaceAutoresearchTarget {
        name,
        description,
        skill_root: skill_root.to_path_buf(),
        skill_markdown: skill_markdown.to_path_buf(),
    })
}

fn compile_workspace_target(
    config: &AppConfig,
    workspace_root: &Path,
    target: &WorkspaceAutoresearchTarget,
) -> Result<AutoresearchProfileOverrideRecord, AutoresearchTargetError> {
    let content = fs::read_to_string(&target.skill_markdown).map_err(|error| {
        compile_error(&target.name, format!("failed to read SKILL.md: {}", error))
    })?;
    let (frontmatter, body) = split_frontmatter(&content)
        .map_err(|_| invalid_skill(&target.name, "missing frontmatter."))?;
    let document = parse_autoresearch_document(&target.name, &frontmatter, &body)?;
    if !target
        .skill_root
        .join("scripts")
        .join("verify.sh")
        .is_file()
    {
        return Err(invalid_skill(
            &target.name,
            "scripts/verify.sh is required.",
        ));
    }
    let profile = compile_profile(config, workspace_root, target, &document)?;
    let skill_root =
        relative_workspace_path(workspace_root, &target.skill_root).ok_or_else(|| {
            compile_error(
                &target.name,
                "skill root must stay inside the active workspace".to_string(),
            )
        })?;

    Ok(AutoresearchProfileOverrideRecord {
        profile_name: AUTORESEARCH_PROFILE_NAME.to_string(),
        target_name: document.name,
        target_description: target.description.clone(),
        skill_root,
        profile,
    })
}

fn compile_profile(
    config: &AppConfig,
    workspace_root: &Path,
    target: &WorkspaceAutoresearchTarget,
    document: &ParsedAutoresearchDocument,
) -> Result<SchedulerProfileConfig, AutoresearchTargetError> {
    let mut profile = resolve_scheduler_profile_config(config, Some(AUTORESEARCH_PROFILE_NAME))
        .map(|(_, profile)| profile)
        .ok_or_else(|| {
            compile_error(
                &target.name,
                format!(
                    "scheduler profile '{}' could not be resolved.",
                    AUTORESEARCH_PROFILE_NAME
                ),
            )
        })?;

    let base_workflow = profile.workflow().cloned();
    let verify_path = format!(
        "{}/scripts/verify.sh",
        relative_workspace_path(workspace_root, &target.skill_root).ok_or_else(|| {
            compile_error(
                &target.name,
                "skill root must stay inside the active workspace".to_string(),
            )
        })?
    );
    let guard_path = target
        .skill_root
        .join("scripts")
        .join("guard.sh")
        .is_file()
        .then(|| {
            format!(
                "{}/scripts/guard.sh",
                relative_workspace_path(workspace_root, &target.skill_root).unwrap_or_default()
            )
        });

    let auto_exclude = format!(
        "{}/**",
        relative_workspace_path(workspace_root, &target.skill_root).unwrap_or_default()
    );
    let mut exclude = document.exclude.clone();
    if !exclude.iter().any(|value| value == &auto_exclude) {
        exclude.push(auto_exclude);
    }

    let verify = compile_script_command(
        format!("bash {}", verify_path),
        base_workflow.as_ref().and_then(|workflow| {
            workflow
                .objective
                .as_ref()
                .map(|objective| &objective.verify)
        }),
    );
    let guard = guard_path.as_ref().map(|path| {
        compile_script_command(
            format!("bash {}", path),
            base_workflow.as_ref().and_then(|workflow| {
                workflow
                    .objective
                    .as_ref()
                    .and_then(|objective| objective.guard.as_ref())
            }),
        )
    });

    let workflow = IterativeWorkflowConfig {
        workflow: WorkflowDescriptor {
            kind: IterativeWorkflowKind::Autoresearch,
            mode: IterativeWorkflowMode::Run,
        },
        objective: Some(ObjectiveDefinition {
            goal: document.goal.clone(),
            scope: ScopeDefinition {
                include: document.scope.clone(),
                exclude,
            },
            direction: match document.metric.direction {
                AutoresearchMetricDirection::HigherIsBetter => ObjectiveDirection::HigherIsBetter,
                AutoresearchMetricDirection::LowerIsBetter => ObjectiveDirection::LowerIsBetter,
            },
            metric: MetricDefinition {
                kind: match document.metric.kind {
                    AutoresearchMetricKind::NumericExtract => MetricKind::NumericExtract,
                },
                pattern: Some(document.metric.pattern.clone()),
                count_pattern: None,
                json_path: None,
                unit: None,
            },
            verify,
            guard,
            satisfied_when: None,
        }),
        iteration_policy: Some(IterationPolicyDefinition {
            mode: match document.iteration_policy.mode {
                AutoresearchIterationMode::Bounded => IterationMode::Bounded,
            },
            max_iterations: Some(document.iteration_policy.max_iterations),
            stop_conditions: Vec::new(),
            stuck_threshold: Some(document.iteration_policy.stuck_threshold),
            progress_report_every: base_workflow
                .as_ref()
                .and_then(|workflow| workflow.iteration_policy.as_ref())
                .and_then(|policy| policy.progress_report_every),
        }),
        decision_policy: Some(DecisionPolicyDefinition {
            baseline_strategy: Some(match document.decision_policy.baseline_strategy {
                AutoresearchBaselineStrategy::CaptureBeforeFirstIteration => {
                    BaselineStrategy::CaptureBeforeFirstIteration
                }
            }),
            baseline_value: None,
            keep_conditions: document
                .decision_policy
                .keep_conditions
                .iter()
                .map(|condition| match condition {
                    AutoresearchKeepCondition::MetricImproved => KeepCondition::MetricImproved,
                    AutoresearchKeepCondition::VerifyPassed => KeepCondition::VerifyPassed,
                })
                .collect(),
            discard_conditions: document
                .decision_policy
                .discard_conditions
                .iter()
                .map(|condition| match condition {
                    AutoresearchDiscardCondition::MetricRegressed => {
                        DiscardCondition::MetricRegressed
                    }
                    AutoresearchDiscardCondition::MetricUnchanged => {
                        DiscardCondition::MetricUnchanged
                    }
                    AutoresearchDiscardCondition::VerifyFailed => DiscardCondition::VerifyFailed,
                })
                .collect(),
            rework_policy: base_workflow
                .as_ref()
                .and_then(|workflow| workflow.decision_policy.as_ref())
                .and_then(|policy| policy.rework_policy.clone()),
            crash_retry_policy: Some(AttemptPolicy {
                max_attempts: Some(document.decision_policy.crash_retry_max_attempts),
            }),
            simplicity_override: base_workflow
                .as_ref()
                .and_then(|workflow| workflow.decision_policy.as_ref())
                .and_then(|policy| policy.simplicity_override.clone()),
        }),
        workspace_policy: Some(WorkspacePolicyDefinition {
            mutation_mode: base_workflow
                .as_ref()
                .and_then(|workflow| workflow.workspace_policy.as_ref())
                .and_then(|policy| policy.mutation_mode),
            protected_paths: base_workflow
                .as_ref()
                .and_then(|workflow| workflow.workspace_policy.as_ref())
                .map(|policy| policy.protected_paths.clone())
                .unwrap_or_default(),
            snapshot_strategy: SnapshotStrategy::WorktreeFork,
            commit_policy: base_workflow
                .as_ref()
                .and_then(|workflow| workflow.workspace_policy.as_ref())
                .and_then(|policy| policy.commit_policy.clone()),
        }),
        artifacts: base_workflow
            .as_ref()
            .and_then(|workflow| workflow.artifacts.clone()),
        approval_policy: base_workflow
            .as_ref()
            .and_then(|workflow| workflow.approval_policy.clone()),
        security: None,
        debug: None,
        fix: None,
        verifier: None,
        ship: None,
    };

    profile.description = Some(document.description.clone());
    profile.workflow = Some(IterativeWorkflowSource::Inline(workflow));
    profile.skill_tree = Some(compile_skill_tree_plan(
        profile.skill_tree.as_ref(),
        &document.context_markdown,
    ));

    Ok(profile)
}

fn compile_skill_tree_plan(
    base: Option<&SkillTreeRequestPlan>,
    context_markdown: &str,
) -> SkillTreeRequestPlan {
    let mut plan = base.cloned().unwrap_or(SkillTreeRequestPlan {
        context_markdown: String::new(),
        token_budget: None,
        truncation_strategy: Default::default(),
    });
    plan.context_markdown = context_markdown.trim().to_string();
    plan
}

fn compile_script_command(command: String, base: Option<&CommandDefinition>) -> CommandDefinition {
    let mut compiled = base.cloned().unwrap_or(CommandDefinition {
        command: String::new(),
        timeout_ms: Some(1_200_000),
        env: HashMap::new(),
        working_directory: None,
    });
    compiled.command = command;
    compiled.working_directory = None;
    compiled
}

fn split_frontmatter(content: &str) -> Result<(String, String), AutoresearchTargetError> {
    let normalized = content.replace("\r\n", "\n");
    let mut lines = normalized.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Err(AutoresearchTargetError::new("missing frontmatter"));
    }

    let mut frontmatter = Vec::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        frontmatter.push(line.to_string());
    }
    if !closed {
        return Err(AutoresearchTargetError::new("missing frontmatter"));
    }

    let body = lines.collect::<Vec<_>>().join("\n");
    Ok((frontmatter.join("\n"), body))
}

#[derive(Debug)]
struct ParsedAutoresearchDocument {
    name: String,
    description: String,
    goal: String,
    scope: Vec<String>,
    exclude: Vec<String>,
    metric: AutoresearchMetricTemplate,
    iteration_policy: AutoresearchIterationPolicyTemplate,
    decision_policy: AutoresearchDecisionPolicyTemplate,
    context_markdown: String,
}

fn parse_autoresearch_document(
    target_name: &str,
    frontmatter_yaml: &str,
    body: &str,
) -> Result<ParsedAutoresearchDocument, AutoresearchTargetError> {
    let mapping = parse_frontmatter_mapping(target_name, frontmatter_yaml)?;
    let name = frontmatter_required_string(target_name, &mapping, "name")?;
    let description = frontmatter_required_string(target_name, &mapping, "description")?;
    let kind = frontmatter_required_string(target_name, &mapping, "kind")?;
    if kind != "autoresearch" {
        return Err(invalid_skill(
            target_name,
            "frontmatter.kind must be 'autoresearch'.",
        ));
    }
    let version = mapping
        .get(Value::String("version".to_string()))
        .and_then(frontmatter_version_value);
    if version != Some(1) {
        return Err(invalid_skill(target_name, "frontmatter.version must be 1."));
    }

    let sections = parse_sections(target_name, body)?;
    let goal = parse_plain_text_section(target_name, &sections[0])?;
    let scope = parse_markdown_list_section(target_name, &sections[1])?;
    let exclude = parse_markdown_list_section(target_name, &sections[2])?;
    let metric = parse_yaml_section::<AutoresearchMetricTemplate>(target_name, &sections[3])?;
    let iteration_policy =
        parse_yaml_section::<AutoresearchIterationPolicyTemplate>(target_name, &sections[4])?;
    let decision_policy =
        parse_yaml_section::<AutoresearchDecisionPolicyTemplate>(target_name, &sections[5])?;
    let context_markdown = parse_context_markdown_section(target_name, &sections[6])?;

    Ok(ParsedAutoresearchDocument {
        name,
        description,
        goal,
        scope,
        exclude,
        metric,
        iteration_policy,
        decision_policy,
        context_markdown,
    })
}

fn parse_frontmatter_mapping(
    target_name: &str,
    yaml: &str,
) -> Result<Mapping, AutoresearchTargetError> {
    let value = serde_yaml::from_str::<Value>(yaml).map_err(|error| {
        invalid_skill(
            target_name,
            format!("failed to parse frontmatter yaml: {}.", error),
        )
    })?;
    value.as_mapping().cloned().ok_or_else(|| {
        invalid_skill(
            target_name,
            "frontmatter must be a yaml mapping.".to_string(),
        )
    })
}

fn frontmatter_required_string(
    target_name: &str,
    mapping: &Mapping,
    key: &str,
) -> Result<String, AutoresearchTargetError> {
    mapping
        .get(Value::String(key.to_string()))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| invalid_skill(target_name, format!("frontmatter.{} is required.", key)))
}

fn frontmatter_version_value(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_str()
            .and_then(|raw| raw.trim().parse::<i64>().ok())
    })
}

#[derive(Debug, Clone)]
struct ParsedSection {
    title: String,
    content: String,
}

fn parse_sections(
    target_name: &str,
    body: &str,
) -> Result<Vec<ParsedSection>, AutoresearchTargetError> {
    let normalized = body.replace("\r\n", "\n");
    let mut current_title: Option<String> = None;
    let mut current_lines = Vec::new();
    let mut sections = Vec::new();

    for line in normalized.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            if let Some(previous_title) = current_title.take() {
                sections.push(ParsedSection {
                    title: previous_title,
                    content: current_lines.join("\n"),
                });
                current_lines.clear();
            }
            current_title = Some(title.trim().to_string());
            continue;
        }

        if current_title.is_some() {
            current_lines.push(line.to_string());
        }
    }

    if let Some(title) = current_title.take() {
        sections.push(ParsedSection {
            title,
            content: current_lines.join("\n"),
        });
    }

    for required in REQUIRED_SECTION_ORDER {
        if !sections.iter().any(|section| section.title == required) {
            return Err(invalid_skill(
                target_name,
                format!("missing required section '{}'.", required),
            ));
        }
    }

    let titles = sections
        .iter()
        .map(|section| section.title.as_str())
        .collect::<Vec<_>>();
    if titles != REQUIRED_SECTION_ORDER {
        return Err(invalid_skill(
            target_name,
            "section order does not match the autoresearch template.",
        ));
    }

    Ok(sections)
}

fn parse_plain_text_section(
    target_name: &str,
    section: &ParsedSection,
) -> Result<String, AutoresearchTargetError> {
    let trimmed = section.content.trim();
    if trimmed.is_empty() {
        return Err(invalid_skill(
            target_name,
            format!("section '{}' must not be empty.", section.title),
        ));
    }
    Ok(trimmed.to_string())
}

fn parse_markdown_list_section(
    target_name: &str,
    section: &ParsedSection,
) -> Result<Vec<String>, AutoresearchTargetError> {
    let mut items = Vec::new();
    for line in section.content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(item) = trimmed.strip_prefix("- ") else {
            return Err(invalid_skill(
                target_name,
                format!(
                    "section '{}' must be a markdown list using '- ' items.",
                    section.title
                ),
            ));
        };
        let item = item.trim();
        if item.is_empty() {
            return Err(invalid_skill(
                target_name,
                format!(
                    "section '{}' must be a markdown list using '- ' items.",
                    section.title
                ),
            ));
        }
        items.push(item.to_string());
    }

    if items.is_empty() {
        return Err(invalid_skill(
            target_name,
            format!(
                "section '{}' must be a markdown list using '- ' items.",
                section.title
            ),
        ));
    }

    Ok(items)
}

fn parse_yaml_section<T: serde::de::DeserializeOwned>(
    target_name: &str,
    section: &ParsedSection,
) -> Result<T, AutoresearchTargetError> {
    let yaml = extract_yaml_code_block(target_name, section)?;
    serde_yaml::from_str::<T>(&yaml).map_err(|error| {
        invalid_skill(
            target_name,
            format!(
                "failed to parse yaml in section '{}': {}.",
                section.title, error
            ),
        )
    })
}

fn extract_yaml_code_block(
    target_name: &str,
    section: &ParsedSection,
) -> Result<String, AutoresearchTargetError> {
    let mut lines = section.content.lines().peekable();
    while lines.peek().is_some_and(|line| line.trim().is_empty()) {
        lines.next();
    }

    let Some(opening) = lines.next() else {
        return Err(invalid_skill(
            target_name,
            format!(
                "section '{}' must contain a single yaml code block.",
                section.title
            ),
        ));
    };
    if opening.trim() != "```yaml" {
        return Err(invalid_skill(
            target_name,
            format!(
                "section '{}' must contain a single yaml code block.",
                section.title
            ),
        ));
    }

    let mut yaml_lines = Vec::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim() == "```" {
            closed = true;
            break;
        }
        yaml_lines.push(line.to_string());
    }
    if !closed {
        return Err(invalid_skill(
            target_name,
            format!(
                "section '{}' must contain a single yaml code block.",
                section.title
            ),
        ));
    }

    for line in lines {
        if !line.trim().is_empty() {
            return Err(invalid_skill(
                target_name,
                format!(
                    "section '{}' must contain a single yaml code block.",
                    section.title
                ),
            ));
        }
    }

    Ok(yaml_lines.join("\n"))
}

fn parse_context_markdown_section(
    target_name: &str,
    section: &ParsedSection,
) -> Result<String, AutoresearchTargetError> {
    let trimmed = section.content.trim();
    if trimmed.is_empty() {
        return Err(invalid_skill(
            target_name,
            "section 'Context Markdown' must not be empty.",
        ));
    }
    if trimmed.starts_with("```") {
        return Err(invalid_skill(
            target_name,
            "section 'Context Markdown' must be plain markdown text, not a code block.",
        ));
    }
    Ok(trimmed.to_string())
}

fn parse_explicit_target_request<'a>(
    raw_arguments: &'a str,
    targets: &[WorkspaceAutoresearchTarget],
) -> Option<ExplicitTargetRequest<'a>> {
    let trimmed = raw_arguments.trim();
    if trimmed.is_empty() {
        return None;
    }

    let token_end = trimmed
        .char_indices()
        .find_map(|(index, ch)| ch.is_whitespace().then_some(index))
        .unwrap_or(trimmed.len());
    let candidate = &trimmed[..token_end];
    if !looks_like_target_name(candidate) {
        return None;
    }
    let remainder = trimmed[token_end..].trim_start();
    let matches_target = targets.iter().any(|target| target.name == candidate);
    if matches_target || remainder.is_empty() || remainder.starts_with("--") {
        return Some(ExplicitTargetRequest {
            target_name: candidate,
            remainder,
        });
    }

    None
}

struct ExplicitTargetRequest<'a> {
    target_name: &'a str,
    remainder: &'a str,
}

fn looks_like_target_name(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with("--")
        && !value.contains(':')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

fn relative_workspace_path(workspace_root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(workspace_root).ok()?;
    let rendered = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/");
    (!rendered.is_empty()).then_some(rendered)
}

fn invalid_skill(target_name: &str, detail: impl Into<String>) -> AutoresearchTargetError {
    AutoresearchTargetError::new(format!(
        "Autoresearch skill '{}' is invalid: {}",
        target_name,
        detail.into()
    ))
}

fn compile_error(target_name: &str, detail: impl Into<String>) -> AutoresearchTargetError {
    AutoresearchTargetError::new(format!(
        "Failed to compile autoresearch target '{}'.\n{}",
        target_name,
        detail.into()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_skill(root: &Path, dir_name: &str, skill_markdown: &str, verify: bool, guard: bool) {
        let skill_root = root.join(".rocode").join("skills").join(dir_name);
        fs::create_dir_all(skill_root.join("scripts")).unwrap();
        fs::write(skill_root.join("SKILL.md"), skill_markdown).unwrap();
        if verify {
            fs::write(
                skill_root.join("scripts").join("verify.sh"),
                "#!/usr/bin/env bash\n",
            )
            .unwrap();
        }
        if guard {
            fs::write(
                skill_root.join("scripts").join("guard.sh"),
                "#!/usr/bin/env bash\n",
            )
            .unwrap();
        }
    }

    fn valid_skill(name: &str) -> String {
        format!(
            r#"---
name: {name}
description: Improve regression score without breaking guarded builds.
kind: autoresearch
version: 1
---

> Warning
> This file is machine-parsed by ROCode for autoresearch.

## Goal
Improve the curated regression score.

## Scope
- crates/**
- Cargo.toml

## Exclude
- target/**

## Metric
```yaml
direction: higher-is-better
kind: numeric-extract
pattern: score=([0-9]+)
```

## Iteration Policy
```yaml
mode: bounded
max_iterations: 6
stuck_threshold: 2
```

## Decision Policy
```yaml
baseline_strategy: capture-before-first-iteration
keep_conditions:
  - metric-improved
  - verify-passed
discard_conditions:
  - metric-regressed
  - metric-unchanged
  - verify-failed
crash_retry_max_attempts: 2
```

## Context Markdown
Operate as an evidence-backed autoresearch loop.
"#
        )
    }

    #[test]
    fn resolve_autoresearch_command_falls_back_to_builtin_when_workspace_has_no_targets() {
        let dir = tempdir().unwrap();
        let resolved =
            resolve_autoresearch_command(&AppConfig::default(), &dir.path().to_string_lossy(), "")
                .unwrap();

        assert_eq!(resolved.scheduler_profile_name, AUTORESEARCH_PROFILE_NAME);
        assert!(resolved.profile_override.is_none());
    }

    #[test]
    fn resolve_autoresearch_command_resolves_unique_workspace_target() {
        let dir = tempdir().unwrap();
        write_skill(
            dir.path(),
            "coverage-lift",
            &valid_skill("coverage-lift"),
            true,
            false,
        );

        let resolved =
            resolve_autoresearch_command(&AppConfig::default(), &dir.path().to_string_lossy(), "")
                .unwrap();

        let override_record = resolved.profile_override.expect("workspace override");
        assert_eq!(override_record.target_name, "coverage-lift");
        assert_eq!(
            override_record
                .profile
                .workflow()
                .and_then(|workflow| workflow.objective.as_ref())
                .map(|objective| objective.verify.command.as_str()),
            Some("bash .rocode/skills/coverage-lift/scripts/verify.sh")
        );
        assert!(override_record
            .profile
            .workflow()
            .and_then(|workflow| workflow.objective.as_ref())
            .map(|objective| objective
                .scope
                .exclude
                .contains(&".rocode/skills/coverage-lift/**".to_string()))
            .unwrap_or(false));
    }

    #[test]
    fn resolve_autoresearch_command_errors_when_workspace_targets_are_ambiguous() {
        let dir = tempdir().unwrap();
        write_skill(
            dir.path(),
            "coverage-lift",
            &valid_skill("coverage-lift"),
            true,
            false,
        );
        write_skill(
            dir.path(),
            "flaky-fixer",
            &valid_skill("flaky-fixer"),
            true,
            false,
        );

        let error =
            resolve_autoresearch_command(&AppConfig::default(), &dir.path().to_string_lossy(), "")
                .unwrap_err();

        assert!(error
            .to_string()
            .contains("/autoresearch is ambiguous in this workspace."));
        assert!(error.to_string().contains("- coverage-lift"));
        assert!(error.to_string().contains("- flaky-fixer"));
    }

    #[test]
    fn resolve_autoresearch_command_errors_when_explicit_target_does_not_exist() {
        let dir = tempdir().unwrap();
        let error = resolve_autoresearch_command(
            &AppConfig::default(),
            &dir.path().to_string_lossy(),
            "coverage-lift",
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("Autoresearch target 'coverage-lift' was not found"));
    }

    #[test]
    fn resolve_autoresearch_command_preserves_pending_raw_arguments_with_target() {
        let dir = tempdir().unwrap();
        write_skill(
            dir.path(),
            "coverage-lift",
            &valid_skill("coverage-lift"),
            true,
            false,
        );

        let resolved = resolve_autoresearch_command(
            &AppConfig::default(),
            &dir.path().to_string_lossy(),
            "coverage-lift --goal \"narrow it\"",
        )
        .unwrap();

        assert_eq!(
            resolved.raw_arguments_for_pending,
            "coverage-lift --goal \"narrow it\""
        );
        assert_eq!(resolved.raw_arguments_for_execution, "--goal \"narrow it\"");
    }

    #[test]
    fn resolve_autoresearch_command_does_not_treat_multiword_free_text_as_target() {
        let dir = tempdir().unwrap();
        let resolved = resolve_autoresearch_command(
            &AppConfig::default(),
            &dir.path().to_string_lossy(),
            "improve coverage",
        )
        .unwrap();

        assert_eq!(resolved.raw_arguments_for_execution, "improve coverage");
        assert!(resolved.profile_override.is_none());
    }

    #[test]
    fn resolve_autoresearch_command_errors_on_invalid_version() {
        let dir = tempdir().unwrap();
        write_skill(
            dir.path(),
            "coverage-lift",
            &valid_skill("coverage-lift").replace("version: 1", "version: 2"),
            true,
            false,
        );

        let error =
            resolve_autoresearch_command(&AppConfig::default(), &dir.path().to_string_lossy(), "")
                .unwrap_err();

        assert!(error.to_string().contains("frontmatter.version must be 1."));
    }

    #[test]
    fn resolve_autoresearch_command_errors_when_verify_script_is_missing() {
        let dir = tempdir().unwrap();
        write_skill(
            dir.path(),
            "coverage-lift",
            &valid_skill("coverage-lift"),
            false,
            false,
        );

        let error =
            resolve_autoresearch_command(&AppConfig::default(), &dir.path().to_string_lossy(), "")
                .unwrap_err();

        assert!(error.to_string().contains("scripts/verify.sh is required."));
    }

    #[test]
    fn resolve_autoresearch_command_allows_missing_guard_script() {
        let dir = tempdir().unwrap();
        write_skill(
            dir.path(),
            "coverage-lift",
            &valid_skill("coverage-lift"),
            true,
            false,
        );

        let resolved =
            resolve_autoresearch_command(&AppConfig::default(), &dir.path().to_string_lossy(), "")
                .unwrap();
        let workflow = resolved
            .profile_override
            .expect("workspace override")
            .profile
            .workflow()
            .cloned()
            .expect("workflow");

        assert!(workflow
            .objective
            .as_ref()
            .and_then(|objective| objective.guard.as_ref())
            .is_none());
    }
}
