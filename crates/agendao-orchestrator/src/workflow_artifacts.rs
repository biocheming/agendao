use crate::iterative_workflow::{ArtifactFileDefinition, IterativeWorkflowConfig};
use crate::verifier_engine::VerifierPersistentCache;
use crate::{ExecutionContext, OrchestratorError};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_OBJECTIVE_PROFILE: &str = "_default";
const DEFAULT_ITERATION_LOG_FILENAME: &str = "iteration-log.tsv";
const DEFAULT_SUMMARY_FILENAME: &str = "summary.md";
const DEFAULT_MANIFEST_FILENAME: &str = "run-manifest.json";
const DEFAULT_LATEST_FILENAME: &str = "latest-run.json";
const DEFAULT_MODE_ARTIFACTS_FILENAME: &str = "mode-artifacts.json";
const DEFAULT_VERIFIER_CACHE_FILENAME: &str = "pairwise-cache.json";

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WorkflowCommandArtifact {
    pub exit_code: Option<i32>,
    pub passed: bool,
    pub timed_out: bool,
    pub runtime_error: Option<String>,
    pub output_excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct WorkflowCommitRecord {
    pub iteration: u32,
    pub commit_sha: String,
    pub message: String,
    pub decision: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct WorkflowModeReport {
    pub mode: String,
    pub protocol: String,
    pub config_notes: Vec<String>,
    pub iteration_notes: Vec<String>,
    pub final_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct WorkflowModeArtifactEntry {
    pub iteration: Option<u32>,
    pub key: String,
    pub status: String,
    pub title: String,
    pub detail: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct WorkflowModeArtifact {
    pub name: String,
    pub description: String,
    pub entries: Vec<WorkflowModeArtifactEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WorkflowBaselineRecord {
    pub source: String,
    pub value: Option<f64>,
    pub summary: String,
    pub verify: Option<WorkflowCommandArtifact>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WorkflowIterationRecord {
    pub iteration: u32,
    pub phase: String,
    pub decision: String,
    pub gate_status: String,
    pub commit_sha: Option<String>,
    pub metric_value: Option<f64>,
    pub baseline_value: Option<f64>,
    pub delta_from_baseline: Option<f64>,
    pub verify: Option<WorkflowCommandArtifact>,
    pub guard: Option<WorkflowCommandArtifact>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WorkflowRunSummaryRecord {
    pub iterations_completed: u32,
    pub final_iteration: Option<u32>,
    pub baseline_metric: Option<f64>,
    pub best_metric: Option<f64>,
    pub final_metric: Option<f64>,
    pub kept_commits: Vec<WorkflowCommitRecord>,
    pub best_commit: Option<WorkflowCommitRecord>,
    pub squashed_commit: Option<WorkflowCommitRecord>,
    pub final_decision: Option<String>,
    pub final_gate_status: Option<String>,
    pub final_summary: Option<String>,
    pub final_response: Option<String>,
    pub verifier: Option<WorkflowVerifierSummary>,
    pub mode_report: Option<WorkflowModeReport>,
    pub mode_artifacts: Vec<WorkflowModeArtifact>,
    pub objective_satisfied: bool,
    pub cancelled: bool,
    pub exhausted_budget: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct WorkflowVerifierCriterionSummary {
    pub criterion_id: String,
    pub winner_candidate_id: Option<String>,
    pub score_a: Option<String>,
    pub score_b: Option<String>,
    pub explanation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct WorkflowVerifierSummary {
    pub selected_candidate_id: Option<String>,
    pub selected_iteration: Option<u32>,
    pub candidates_considered: u32,
    pub pairwise_comparisons: u32,
    pub judge_calls: u32,
    pub cache_hits: u32,
    pub selection_strategy: String,
    pub used_judge: bool,
    pub used_logprobs: bool,
    pub judge_rationale: Option<String>,
    pub criterion_scores: Vec<WorkflowVerifierCriterionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WorkflowRunManifest {
    pub profile_name: Option<String>,
    pub workflow_kind: String,
    pub workflow_mode: String,
    pub session_id: String,
    pub objective_fingerprint: String,
    pub objective_goal: Option<String>,
    pub run_dir: String,
    pub iterations_completed: u32,
    pub final_iteration: Option<u32>,
    pub baseline_metric: Option<f64>,
    pub best_metric: Option<f64>,
    pub final_metric: Option<f64>,
    pub kept_commits: Vec<WorkflowCommitRecord>,
    pub best_commit: Option<WorkflowCommitRecord>,
    pub squashed_commit: Option<WorkflowCommitRecord>,
    pub final_decision: Option<String>,
    pub final_gate_status: Option<String>,
    pub objective_satisfied: bool,
    pub cancelled: bool,
    pub exhausted_budget: bool,
    pub final_summary: Option<String>,
    pub verifier: Option<WorkflowVerifierSummary>,
    pub mode_report: Option<WorkflowModeReport>,
    pub mode_artifacts: Vec<WorkflowModeArtifact>,
    pub completed_at_unix_ms: u128,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkflowArtifactWriter {
    base_dir: PathBuf,
    run_dir: PathBuf,
    profile_name: Option<String>,
    profile_component: String,
    workflow_kind: String,
    workflow_mode: String,
    session_id: String,
    objective_goal: Option<String>,
    objective_fingerprint: String,
    iteration_log: Option<ArtifactFileConfig>,
    summary: Option<ArtifactFileConfig>,
}

#[derive(Debug, Clone)]
struct ArtifactFileConfig {
    filename: String,
    format: String,
}

impl WorkflowArtifactWriter {
    pub(crate) fn new(
        config: &IterativeWorkflowConfig,
        exec_ctx: &ExecutionContext,
        profile_name: Option<&str>,
    ) -> Result<Self, OrchestratorError> {
        let workdir = Path::new(&exec_ctx.workdir);
        let base_dir = workflow_artifact_root(config, workdir);
        let run_dir = workflow_run_dir(config, workdir, &exec_ctx.session_id);
        let objective_goal = config
            .objective
            .as_ref()
            .map(|objective| objective.goal.clone());
        let objective_fingerprint = objective_fingerprint(config)?;
        let profile_name = profile_name.map(str::to_string);

        Ok(Self {
            base_dir,
            run_dir,
            profile_component: sanitize_profile_component(profile_name.as_deref()),
            profile_name,
            workflow_kind: format!("{:?}", config.workflow.kind).to_ascii_lowercase(),
            workflow_mode: config.workflow.mode.as_str().to_string(),
            session_id: exec_ctx.session_id.clone(),
            objective_goal,
            objective_fingerprint,
            iteration_log: config
                .artifacts
                .as_ref()
                .and_then(|artifacts| artifacts.iteration_log.as_ref())
                .map(|file| ArtifactFileConfig::iteration_log(file)),
            summary: config
                .artifacts
                .as_ref()
                .and_then(|artifacts| artifacts.summary.as_ref())
                .map(|file| ArtifactFileConfig::summary(file)),
        })
    }

    pub(crate) fn record_baseline(
        &self,
        baseline: &WorkflowBaselineRecord,
    ) -> Result<(), OrchestratorError> {
        let Some(file) = self.iteration_log.as_ref() else {
            return Ok(());
        };
        let record = WorkflowIterationRecord {
            iteration: 0,
            phase: "baseline".to_string(),
            decision: baseline.source.clone(),
            gate_status: "baseline".to_string(),
            commit_sha: None,
            metric_value: baseline.value,
            baseline_value: baseline.value,
            delta_from_baseline: Some(0.0),
            verify: baseline.verify.clone(),
            guard: None,
            summary: baseline.summary.clone(),
        };
        self.append_iteration_record(file, &record)
    }

    pub(crate) fn append_iteration(
        &self,
        record: &WorkflowIterationRecord,
    ) -> Result<(), OrchestratorError> {
        let Some(file) = self.iteration_log.as_ref() else {
            return Ok(());
        };
        self.append_iteration_record(file, record)
    }

    pub(crate) fn write_summary(
        &self,
        summary: &WorkflowRunSummaryRecord,
    ) -> Result<WorkflowRunManifest, OrchestratorError> {
        self.ensure_run_dir()?;

        if let Some(file) = self.summary.as_ref() {
            let path = self.run_dir.join(&file.filename);
            let body = match file.format.as_str() {
                "json" => serde_json::to_string_pretty(summary).map_err(|err| {
                    OrchestratorError::Other(format!(
                        "failed to serialize workflow summary as json: {err}"
                    ))
                })?,
                "markdown" | "md" => self.render_markdown_summary(summary),
                other => {
                    return Err(OrchestratorError::Other(format!(
                        "unsupported workflow summary format '{other}'"
                    )));
                }
            };
            write_string(&path, &body)?;
        }

        let manifest = WorkflowRunManifest {
            profile_name: self.profile_name.clone(),
            workflow_kind: self.workflow_kind.clone(),
            workflow_mode: self.workflow_mode.clone(),
            session_id: self.session_id.clone(),
            objective_fingerprint: self.objective_fingerprint.clone(),
            objective_goal: self.objective_goal.clone(),
            run_dir: self.run_dir.display().to_string(),
            iterations_completed: summary.iterations_completed,
            final_iteration: summary.final_iteration,
            baseline_metric: summary.baseline_metric,
            best_metric: summary.best_metric,
            final_metric: summary.final_metric,
            kept_commits: summary.kept_commits.clone(),
            best_commit: summary.best_commit.clone(),
            squashed_commit: summary.squashed_commit.clone(),
            final_decision: summary.final_decision.clone(),
            final_gate_status: summary.final_gate_status.clone(),
            objective_satisfied: summary.objective_satisfied,
            cancelled: summary.cancelled,
            exhausted_budget: summary.exhausted_budget,
            final_summary: summary.final_summary.clone(),
            verifier: summary.verifier.clone(),
            mode_report: summary.mode_report.clone(),
            mode_artifacts: summary.mode_artifacts.clone(),
            completed_at_unix_ms: now_unix_ms(),
        };

        let manifest_json = serde_json::to_string_pretty(&manifest).map_err(|err| {
            OrchestratorError::Other(format!("failed to serialize workflow run manifest: {err}"))
        })?;
        write_string(
            &self.run_dir.join(DEFAULT_MANIFEST_FILENAME),
            &manifest_json,
        )?;
        let mode_artifacts_json =
            serde_json::to_string_pretty(&summary.mode_artifacts).map_err(|err| {
                OrchestratorError::Other(format!(
                    "failed to serialize workflow mode artifacts: {err}"
                ))
            })?;
        write_string(
            &self.run_dir.join(DEFAULT_MODE_ARTIFACTS_FILENAME),
            &mode_artifacts_json,
        )?;
        write_string(&self.base_dir.join(DEFAULT_LATEST_FILENAME), &manifest_json)?;
        write_string(&self.objective_index_path(), &manifest_json)?;

        Ok(manifest)
    }

    pub(crate) fn read_last_run_manifest(
        &self,
    ) -> Result<Option<WorkflowRunManifest>, OrchestratorError> {
        let path = self.objective_index_path();
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to read workflow objective index '{}': {err}",
                path.display()
            ))
        })?;
        serde_json::from_str::<WorkflowRunManifest>(&content)
            .map(Some)
            .map_err(|err| {
                OrchestratorError::Other(format!(
                    "failed to parse workflow objective index '{}': {err}",
                    path.display()
                ))
            })
    }

    pub(crate) fn read_verifier_cache(
        &self,
    ) -> Result<Option<VerifierPersistentCache>, OrchestratorError> {
        let path = self.verifier_cache_path();
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to read verifier cache '{}': {err}",
                path.display()
            ))
        })?;
        serde_json::from_str::<VerifierPersistentCache>(&content)
            .map(Some)
            .map_err(|err| {
                OrchestratorError::Other(format!(
                    "failed to parse verifier cache '{}': {err}",
                    path.display()
                ))
            })
    }

    pub(crate) fn write_verifier_cache(
        &self,
        cache: &VerifierPersistentCache,
    ) -> Result<(), OrchestratorError> {
        let path = self.verifier_cache_path();
        let content = serde_json::to_string_pretty(cache).map_err(|err| {
            OrchestratorError::Other(format!("failed to serialize verifier cache: {err}"))
        })?;
        write_string(&path, &content)
    }

    fn append_iteration_record(
        &self,
        file: &ArtifactFileConfig,
        record: &WorkflowIterationRecord,
    ) -> Result<(), OrchestratorError> {
        self.ensure_run_dir()?;
        let path = self.run_dir.join(&file.filename);
        match file.format.as_str() {
            "tsv" => self.append_tsv_record(&path, record),
            "jsonl" => self.append_jsonl_record(&path, record),
            other => Err(OrchestratorError::Other(format!(
                "unsupported workflow iteration log format '{other}'"
            ))),
        }
    }

    fn append_tsv_record(
        &self,
        path: &Path,
        record: &WorkflowIterationRecord,
    ) -> Result<(), OrchestratorError> {
        let is_new = !path.exists();
        let mut file = open_append(path)?;
        if is_new {
            writeln!(
                file,
                "phase\titeration\tdecision\tgate_status\tcommit\tmetric\tbaseline\tdelta\tverify_exit_code\tverify_passed\tguard_exit_code\tguard_passed\tsummary"
            )
            .map_err(io_err(path))?;
        }
        writeln!(
            file,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            escape_tsv(&record.phase),
            record.iteration,
            escape_tsv(&record.decision),
            escape_tsv(&record.gate_status),
            record.commit_sha.as_deref().unwrap_or("-"),
            render_number(record.metric_value),
            render_number(record.baseline_value),
            render_number(record.delta_from_baseline),
            render_opt_i32(record.verify.as_ref().and_then(|command| command.exit_code)),
            render_bool(record.verify.as_ref().map(|command| command.passed)),
            render_opt_i32(record.guard.as_ref().and_then(|command| command.exit_code)),
            render_bool(record.guard.as_ref().map(|command| command.passed)),
            escape_tsv(&record.summary),
        )
        .map_err(io_err(path))
    }

    fn append_jsonl_record(
        &self,
        path: &Path,
        record: &WorkflowIterationRecord,
    ) -> Result<(), OrchestratorError> {
        let mut file = open_append(path)?;
        let line = serde_json::to_string(record).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to serialize workflow iteration record '{}': {err}",
                path.display()
            ))
        })?;
        writeln!(file, "{line}").map_err(io_err(path))
    }

    fn render_markdown_summary(&self, summary: &WorkflowRunSummaryRecord) -> String {
        let completion = if summary.cancelled {
            "Cancelled"
        } else if summary.objective_satisfied {
            "Objective satisfied"
        } else if summary.exhausted_budget {
            "Iteration budget exhausted"
        } else {
            "Completed"
        };
        let mut lines = vec![
            "# Workflow Summary".to_string(),
            String::new(),
            format!(
                "- Profile: {}",
                self.profile_name
                    .as_deref()
                    .unwrap_or(DEFAULT_OBJECTIVE_PROFILE)
            ),
            format!("- Mode: {}", self.workflow_mode),
            format!("- Session: {}", self.session_id),
            format!("- Completion: {completion}"),
            format!("- Iterations completed: {}", summary.iterations_completed),
            format!("- Best metric: {}", render_number(summary.best_metric)),
            format!("- Final metric: {}", render_number(summary.final_metric)),
            format!("- Kept commits: {}", summary.kept_commits.len()),
        ];
        if let Some(commit) = summary.best_commit.as_ref() {
            lines.push(format!(
                "- Best commit: {} ({})",
                commit.commit_sha, commit.message
            ));
        }
        if let Some(commit) = summary.squashed_commit.as_ref() {
            lines.push(format!(
                "- Squashed commit: {} ({})",
                commit.commit_sha, commit.message
            ));
        }
        if let Some(decision) = summary.final_decision.as_deref() {
            lines.push(format!("- Final decision: {decision}"));
        }
        if let Some(status) = summary.final_gate_status.as_deref() {
            lines.push(format!("- Gate status: {status}"));
        }
        if let Some(text) = summary.final_summary.as_deref() {
            lines.push(String::new());
            lines.push("## Summary".to_string());
            lines.push(text.trim().to_string());
        }
        if let Some(text) = summary.final_response.as_deref() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                lines.push(String::new());
                lines.push("## Final Response".to_string());
                lines.push(trimmed.to_string());
            }
        }
        if let Some(verifier) = summary.verifier.as_ref() {
            lines.push(String::new());
            lines.push("## Verifier Summary".to_string());
            lines.push(format!(
                "- Selected candidate: {}",
                verifier.selected_candidate_id.as_deref().unwrap_or("none")
            ));
            lines.push(format!(
                "- Selected iteration: {}",
                verifier
                    .selected_iteration
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_string())
            ));
            lines.push(format!(
                "- Candidates considered: {}",
                verifier.candidates_considered
            ));
            lines.push(format!(
                "- Pairwise comparisons: {}",
                verifier.pairwise_comparisons
            ));
            lines.push(format!("- Judge calls: {}", verifier.judge_calls));
            lines.push(format!("- Cache hits: {}", verifier.cache_hits));
            lines.push(format!(
                "- Selection strategy: {}",
                verifier.selection_strategy
            ));
            lines.push(format!("- Used judge: {}", verifier.used_judge));
            lines.push(format!("- Used logprobs: {}", verifier.used_logprobs));
            if let Some(rationale) = verifier.judge_rationale.as_deref() {
                let trimmed = rationale.trim();
                if !trimmed.is_empty() {
                    lines.push(format!("- Judge rationale: {trimmed}"));
                }
            }
            if !verifier.criterion_scores.is_empty() {
                lines.push("- Criterion scores:".to_string());
                for score in &verifier.criterion_scores {
                    let winner = score
                        .winner_candidate_id
                        .as_deref()
                        .unwrap_or("unspecified");
                    let score_pair = match (score.score_a.as_deref(), score.score_b.as_deref()) {
                        (Some(score_a), Some(score_b)) => format!(" ({score_a}/{score_b})"),
                        _ => String::new(),
                    };
                    let explanation = score
                        .explanation
                        .as_deref()
                        .map(|text| format!(": {}", text.trim()))
                        .unwrap_or_default();
                    lines.push(format!(
                        "  - {} -> {}{}{}",
                        score.criterion_id, winner, score_pair, explanation
                    ));
                }
            }
        }
        if !summary.kept_commits.is_empty() {
            lines.push(String::new());
            lines.push("## Git Memory".to_string());
            for commit in &summary.kept_commits {
                lines.push(format!(
                    "- iter {}: {} {}",
                    commit.iteration, commit.commit_sha, commit.message
                ));
            }
        }
        if let Some(report) = summary.mode_report.as_ref() {
            lines.push(String::new());
            lines.push(format!("## Mode Protocol ({})", report.mode));
            lines.push(format!("- Protocol: {}", report.protocol));
            for note in &report.config_notes {
                lines.push(format!("- {note}"));
            }
            if !report.iteration_notes.is_empty() {
                lines.push(String::new());
                lines.push("### Iteration Notes".to_string());
                for note in &report.iteration_notes {
                    lines.push(format!("- {note}"));
                }
            }
            if !report.final_notes.is_empty() {
                lines.push(String::new());
                lines.push("### Final Notes".to_string());
                for note in &report.final_notes {
                    lines.push(format!("- {note}"));
                }
            }
        }
        if !summary.mode_artifacts.is_empty() {
            lines.push(String::new());
            lines.push("## Mode Artifacts".to_string());
            for artifact in &summary.mode_artifacts {
                lines.push(format!("### {}", artifact.name));
                lines.push(artifact.description.clone());
                for entry in &artifact.entries {
                    let iteration = entry
                        .iteration
                        .map(|value| format!("iter {value}"))
                        .unwrap_or_else(|| "final".to_string());
                    let evidence = if entry.evidence.is_empty() {
                        String::new()
                    } else {
                        format!(" Evidence: {}.", entry.evidence.join("; "))
                    };
                    lines.push(format!(
                        "- [{}] {} {}: {}.{}",
                        entry.status, iteration, entry.title, entry.detail, evidence
                    ));
                }
            }
        }
        lines.join("\n")
    }

    fn ensure_run_dir(&self) -> Result<(), OrchestratorError> {
        fs::create_dir_all(&self.run_dir).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to create workflow artifact directory '{}': {err}",
                self.run_dir.display()
            ))
        })?;
        Ok(())
    }

    fn objective_index_path(&self) -> PathBuf {
        self.base_dir
            .join("objectives")
            .join(&self.profile_component)
            .join(format!("{}.json", self.objective_fingerprint))
    }

    fn verifier_cache_path(&self) -> PathBuf {
        self.base_dir
            .join("verifier-cache")
            .join(&self.profile_component)
            .join(&self.objective_fingerprint)
            .join(DEFAULT_VERIFIER_CACHE_FILENAME)
    }
}

impl ArtifactFileConfig {
    fn iteration_log(file: &ArtifactFileDefinition) -> Self {
        let format = file
            .format
            .as_deref()
            .unwrap_or("tsv")
            .trim()
            .to_ascii_lowercase();
        let filename = file.filename.clone().unwrap_or_else(|| {
            if format == "jsonl" {
                "iteration-log.jsonl".to_string()
            } else {
                DEFAULT_ITERATION_LOG_FILENAME.to_string()
            }
        });
        Self { filename, format }
    }

    fn summary(file: &ArtifactFileDefinition) -> Self {
        let format = file
            .format
            .as_deref()
            .unwrap_or("markdown")
            .trim()
            .to_ascii_lowercase();
        let filename = file.filename.clone().unwrap_or_else(|| {
            if format == "json" {
                "summary.json".to_string()
            } else {
                DEFAULT_SUMMARY_FILENAME.to_string()
            }
        });
        Self { filename, format }
    }
}

pub(crate) fn workflow_artifact_root(config: &IterativeWorkflowConfig, workdir: &Path) -> PathBuf {
    config
        .artifacts
        .as_ref()
        .and_then(|artifacts| artifacts.root_dir.as_deref())
        .map(|root| resolve_path(root, workdir))
        .unwrap_or_else(|| workdir.join(".agendao").join("autoresearch"))
}

pub(crate) fn workflow_run_dir(
    config: &IterativeWorkflowConfig,
    workdir: &Path,
    session_id: &str,
) -> PathBuf {
    let base = workflow_artifact_root(config, workdir);
    config
        .artifacts
        .as_ref()
        .and_then(|artifacts| artifacts.run_dir.as_deref())
        .map(|run_dir| resolve_path(run_dir, &base))
        .unwrap_or_else(|| base.join(session_id))
}

fn objective_fingerprint(config: &IterativeWorkflowConfig) -> Result<String, OrchestratorError> {
    let objective = config.objective.as_ref().ok_or_else(|| {
        OrchestratorError::Other(
            "workflow objective is required to compute objective fingerprint".to_string(),
        )
    })?;
    let payload = serde_json::to_string(objective).map_err(|err| {
        OrchestratorError::Other(format!(
            "failed to serialize workflow objective for fingerprinting: {err}"
        ))
    })?;
    let mut hasher = DefaultHasher::new();
    payload.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

fn sanitize_profile_component(value: Option<&str>) -> String {
    let raw = value.unwrap_or(DEFAULT_OBJECTIVE_PROFILE).trim();
    if raw.is_empty() {
        return DEFAULT_OBJECTIVE_PROFILE.to_string();
    }
    raw.chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '-',
        })
        .collect()
}

fn resolve_path(value: &str, base: &Path) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn write_string(path: &Path, content: &str) -> Result<(), OrchestratorError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to create workflow artifact parent '{}': {err}",
                parent.display()
            ))
        })?;
    }
    fs::write(path, content).map_err(|err| {
        OrchestratorError::Other(format!(
            "failed to write workflow artifact '{}': {err}",
            path.display()
        ))
    })
}

fn open_append(path: &Path) -> Result<std::fs::File, OrchestratorError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to create workflow artifact parent '{}': {err}",
                parent.display()
            ))
        })?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| {
            OrchestratorError::Other(format!(
                "failed to open workflow artifact '{}' for append: {err}",
                path.display()
            ))
        })
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> OrchestratorError + '_ {
    move |err| {
        OrchestratorError::Other(format!(
            "failed to update workflow artifact '{}': {err}",
            path.display()
        ))
    }
}

fn render_number(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "-".to_string())
}

fn render_opt_i32(value: Option<i32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn render_bool(value: Option<bool>) -> String {
    value
        .map(|value| if value { "true" } else { "false" }.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn escape_tsv(value: &str) -> String {
    value
        .replace('\t', " ")
        .replace('\n', " ")
        .replace('\r', " ")
        .trim()
        .to_string()
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iterative_workflow::{
        ArtifactDefinition, IterationMode, IterationPolicyDefinition, IterativeWorkflowKind,
        IterativeWorkflowMode, WorkflowDescriptor,
    };
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn workflow_config_with_artifacts() -> IterativeWorkflowConfig {
        IterativeWorkflowConfig {
            workflow: WorkflowDescriptor {
                kind: IterativeWorkflowKind::Autoresearch,
                mode: IterativeWorkflowMode::Run,
            },
            objective: Some(crate::iterative_workflow::ObjectiveDefinition {
                goal: "Improve the score".to_string(),
                scope: crate::iterative_workflow::ScopeDefinition {
                    include: vec!["src/**".to_string()],
                    exclude: Vec::new(),
                },
                direction: crate::iterative_workflow::ObjectiveDirection::HigherIsBetter,
                metric: crate::iterative_workflow::MetricDefinition {
                    kind: crate::iterative_workflow::MetricKind::ExitCode,
                    pattern: None,
                    count_pattern: None,
                    json_path: None,
                    unit: None,
                },
                verify: crate::iterative_workflow::CommandDefinition {
                    command: "cargo test".to_string(),
                    timeout_ms: None,
                    env: HashMap::new(),
                    working_directory: None,
                },
                guard: None,
                satisfied_when: None,
            }),
            iteration_policy: Some(IterationPolicyDefinition {
                mode: IterationMode::Bounded,
                max_iterations: Some(3),
                stop_conditions: Vec::new(),
                stuck_threshold: None,
                progress_report_every: None,
            }),
            decision_policy: Some(crate::iterative_workflow::DecisionPolicyDefinition {
                baseline_strategy: None,
                baseline_value: None,
                keep_conditions: Vec::new(),
                discard_conditions: Vec::new(),
                rework_policy: None,
                crash_retry_policy: None,
                simplicity_override: None,
            }),
            workspace_policy: Some(crate::iterative_workflow::WorkspacePolicyDefinition {
                mutation_mode: None,
                protected_paths: Vec::new(),
                snapshot_strategy: crate::iterative_workflow::SnapshotStrategy::PatchFile,
                commit_policy: None,
            }),
            artifacts: Some(ArtifactDefinition {
                root_dir: None,
                run_dir: None,
                iteration_log: Some(ArtifactFileDefinition {
                    format: Some("tsv".to_string()),
                    filename: Some("iterations.tsv".to_string()),
                }),
                summary: Some(ArtifactFileDefinition {
                    format: Some("json".to_string()),
                    filename: Some("summary.json".to_string()),
                }),
            }),
            approval_policy: None,
            security: None,
            debug: None,
            fix: None,
            verifier: None,
            ship: None,
        }
    }

    fn temp_exec_ctx() -> ExecutionContext {
        let suffix = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "agendao-workflow-artifacts-{}-{}-{suffix}",
            std::process::id(),
            now_unix_ms()
        ));
        fs::create_dir_all(&root).expect("temp root should create");
        ExecutionContext {
            session_id: "artifact-session".to_string(),
            workdir: root.display().to_string(),
            agent_name: "hephaestus".to_string(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn workflow_run_dir_defaults_under_agendao_autoresearch() {
        let exec_ctx = temp_exec_ctx();
        let config = workflow_config_with_artifacts();
        let run_dir = workflow_run_dir(&config, Path::new(&exec_ctx.workdir), &exec_ctx.session_id);
        assert!(run_dir.ends_with(Path::new(".agendao/autoresearch/artifact-session")));
        fs::remove_dir_all(exec_ctx.workdir).expect("temp workdir should clean up");
    }

    #[test]
    fn workflow_artifact_writer_persists_indexed_manifest() {
        let exec_ctx = temp_exec_ctx();
        let config = workflow_config_with_artifacts();
        let writer = WorkflowArtifactWriter::new(&config, &exec_ctx, Some("autoresearch-run"))
            .expect("writer should construct");

        let manifest = writer
            .write_summary(&WorkflowRunSummaryRecord {
                iterations_completed: 2,
                final_iteration: Some(2),
                baseline_metric: Some(0.0),
                best_metric: Some(1.0),
                final_metric: Some(1.0),
                kept_commits: vec![WorkflowCommitRecord {
                    iteration: 1,
                    commit_sha: "abc123".to_string(),
                    message: "kept iteration".to_string(),
                    decision: "keep".to_string(),
                    summary: "kept iteration".to_string(),
                }],
                best_commit: Some(WorkflowCommitRecord {
                    iteration: 1,
                    commit_sha: "abc123".to_string(),
                    message: "kept iteration".to_string(),
                    decision: "keep".to_string(),
                    summary: "kept iteration".to_string(),
                }),
                squashed_commit: None,
                final_decision: Some("stop-satisfied".to_string()),
                final_gate_status: Some("done".to_string()),
                final_summary: Some("objective satisfied".to_string()),
                final_response: None,
                verifier: None,
                mode_report: None,
                mode_artifacts: Vec::new(),
                objective_satisfied: true,
                cancelled: false,
                exhausted_budget: false,
            })
            .expect("summary should persist");

        let indexed = writer
            .read_last_run_manifest()
            .expect("manifest should load")
            .expect("manifest should exist");
        assert_eq!(
            indexed.objective_fingerprint,
            manifest.objective_fingerprint
        );
        assert_eq!(indexed.best_metric, Some(1.0));

        fs::remove_dir_all(exec_ctx.workdir).expect("temp workdir should clean up");
    }
}
