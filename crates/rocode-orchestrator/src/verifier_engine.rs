use crate::iterative_workflow::{
    ObjectiveDirection, VerifierConfig, VerifierCriterionAggregation, VerifierCriterionDefinition,
    VerifierSelectionStrategy, VerifierTraceFormat,
};
use crate::runtime::events::CancelToken;
use crate::traits::{LifecycleHook, ModelResolver};
use crate::types::ExecutionContext;
use crate::verifier_trace::VerifierTraceSnapshot;
use crate::workflow_artifacts::{
    WorkflowModeArtifact, WorkflowModeArtifactEntry, WorkflowVerifierCriterionSummary,
    WorkflowVerifierSummary,
};
use crate::OrchestratorError;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;

const VERIFIER_METRIC_EPSILON: f64 = 1e-9;

#[derive(Debug, Clone)]
pub(crate) struct VerifierCandidate {
    pub candidate_id: String,
    pub source_iteration: u32,
    pub fingerprint: String,
    pub output_content: String,
    pub metric_value: Option<f64>,
    pub trace: VerifierTraceSnapshot,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct VerifierCriterionScore {
    pub criterion_id: String,
    pub winner_candidate_id: Option<String>,
    pub score_a: Option<f64>,
    pub score_b: Option<f64>,
    pub explanation: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct VerifierPairwiseRecord {
    pub comparison_key: String,
    pub incumbent_id: String,
    pub challenger_id: String,
    pub winner_id: String,
    pub strategy: VerifierSelectionStrategy,
    pub repetitions: u32,
    pub used_judge: bool,
    pub used_logprobs: bool,
    pub cached: bool,
    pub score_a_mean: Option<f64>,
    pub score_b_mean: Option<f64>,
    pub scores: Vec<VerifierCriterionScore>,
    pub score_job_keys: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct VerifierScoreJob {
    pub job_key: String,
    pub objective_fingerprint: String,
    pub incumbent_id: String,
    pub incumbent_fingerprint: String,
    pub challenger_id: String,
    pub challenger_fingerprint: String,
    pub criterion_id: String,
    pub criterion_fingerprint: String,
    pub repetition: u32,
    pub requested_logprobs: bool,
    pub requested_top_logprobs: u32,
    pub trace_hash: String,
    pub trace_format: String,
    pub model_provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum VerifierScoreFallbackKind {
    None,
    TextTag,
    JsonVerdict,
    Metric,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum VerifierLogprobStatus {
    NotRequested,
    RequestedUsable,
    RequestedMissingProviderMetadata,
    RequestedMissingLogprobsField,
    RequestedEmptyLogprobs,
    RequestedUnusableScoreTokenLogprobs,
}

impl Default for VerifierLogprobStatus {
    fn default() -> Self {
        Self::NotRequested
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VerifierScoreJobResult {
    pub criterion_id: String,
    pub repetition: u32,
    pub score_a: f64,
    pub score_b: f64,
    #[serde(default)]
    pub requested_logprobs: bool,
    pub used_logprobs: bool,
    #[serde(default)]
    pub logprob_status: VerifierLogprobStatus,
    #[serde(default)]
    pub model_provider_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub latency_ms: Option<u128>,
    #[serde(default)]
    pub error: Option<String>,
    pub fallback_kind: VerifierScoreFallbackKind,
    pub raw_text_excerpt: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct VerifierScoreJobRecord {
    pub job: VerifierScoreJob,
    pub cached: bool,
    pub result: VerifierScoreJobResult,
}

#[derive(Debug, Clone)]
pub(crate) struct VerifierRuntimeState {
    pub candidates: Vec<VerifierCandidate>,
    pub selected_candidate_id: Option<String>,
    pub used_judge: bool,
    pub used_logprobs: bool,
    pub pairwise_comparisons: u32,
    pub judge_calls: u32,
    pub cache_hits: u32,
    pub last_judge_rationale: Option<String>,
    pub last_judge_scores: Vec<VerifierCriterionScore>,
    pairwise_cache: HashMap<String, VerifierSelectionOutcome>,
    score_job_cache: HashMap<String, VerifierScoreJobResult>,
    pairwise_records: Vec<VerifierPairwiseRecord>,
    score_job_records: Vec<VerifierScoreJobRecord>,
    round_robin_win_counts: HashMap<String, f64>,
}

impl VerifierRuntimeState {
    pub(crate) fn new() -> Self {
        Self {
            candidates: Vec::new(),
            selected_candidate_id: None,
            used_judge: false,
            used_logprobs: false,
            pairwise_comparisons: 0,
            judge_calls: 0,
            cache_hits: 0,
            last_judge_rationale: None,
            last_judge_scores: Vec::new(),
            pairwise_cache: HashMap::new(),
            score_job_cache: HashMap::new(),
            pairwise_records: Vec::new(),
            score_job_records: Vec::new(),
            round_robin_win_counts: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct VerifierPersistentCache {
    version: u32,
    #[serde(default)]
    entries: HashMap<String, VerifierSelectionOutcome>,
    #[serde(default)]
    score_jobs: HashMap<String, VerifierScoreJobResult>,
}

pub(crate) struct VerifierEngine {
    verifier: VerifierConfig,
    objective_goal: String,
    objective_direction: ObjectiveDirection,
    model_resolver: Option<Arc<dyn ModelResolver>>,
    lifecycle_hook: Option<Arc<dyn LifecycleHook>>,
    cancel_token: Option<Arc<dyn CancelToken>>,
    exec_ctx: Option<ExecutionContext>,
}

impl VerifierEngine {
    pub(crate) fn new(
        verifier: VerifierConfig,
        objective_goal: String,
        objective_direction: ObjectiveDirection,
    ) -> Self {
        Self {
            verifier,
            objective_goal,
            objective_direction,
            model_resolver: None,
            lifecycle_hook: None,
            cancel_token: None,
            exec_ctx: None,
        }
    }

    pub(crate) fn bind_judge(
        &mut self,
        model_resolver: Arc<dyn ModelResolver>,
        lifecycle_hook: Arc<dyn LifecycleHook>,
        cancel_token: Arc<dyn CancelToken>,
        mut exec_ctx: ExecutionContext,
    ) {
        exec_ctx.metadata.insert(
            "workflow_verifier_stage".to_string(),
            json!("candidate-selection"),
        );
        if self.verifier.use_logprobs.unwrap_or(false) {
            exec_ctx
                .metadata
                .insert("workflow_verifier_use_logprobs".to_string(), json!(true));
            exec_ctx.metadata.insert(
                "workflow_verifier_top_logprobs".to_string(),
                json!(self.verifier.granularity.unwrap_or(20).clamp(1, 20)),
            );
        }
        self.model_resolver = Some(model_resolver);
        self.lifecycle_hook = Some(lifecycle_hook);
        self.cancel_token = Some(cancel_token);
        self.exec_ctx = Some(exec_ctx);
    }

    pub(crate) fn record_candidate(
        &self,
        state: &mut VerifierRuntimeState,
        iteration: u32,
        output_content: String,
        metric_value: Option<f64>,
        trace: VerifierTraceSnapshot,
    ) -> String {
        let fingerprint = candidate_fingerprint(&output_content, metric_value, &trace);
        let candidate = VerifierCandidate {
            candidate_id: format!("cand-{iteration:03}"),
            source_iteration: iteration,
            fingerprint,
            output_content,
            metric_value,
            trace,
        };
        let candidate_id = candidate.candidate_id.clone();

        if let Some(existing) = state
            .candidates
            .iter_mut()
            .find(|existing| existing.candidate_id == candidate.candidate_id)
        {
            *existing = candidate;
        } else {
            state.candidates.push(candidate);
        }
        if state.selected_candidate_id.is_none() {
            state.selected_candidate_id = Some(candidate_id.clone());
        }

        candidate_id
    }

    pub(crate) fn import_cache(
        &self,
        state: &mut VerifierRuntimeState,
        cache: VerifierPersistentCache,
    ) {
        state.pairwise_cache = cache.entries;
        state.score_job_cache = cache.score_jobs;
    }

    pub(crate) fn export_cache(&self, state: &VerifierRuntimeState) -> VerifierPersistentCache {
        VerifierPersistentCache {
            version: 1,
            entries: state.pairwise_cache.clone(),
            score_jobs: state.score_job_cache.clone(),
        }
    }

    pub(crate) async fn select_candidate(
        &self,
        state: &mut VerifierRuntimeState,
        latest_candidate_id: &str,
    ) {
        let judge_selection = match self.verifier.selection {
            Some(VerifierSelectionStrategy::RoundRobin) => {
                self.select_round_robin(state, latest_candidate_id).await
            }
            Some(VerifierSelectionStrategy::Tournament) => self.select_tournament(state).await,
            None => None,
        };

        state.selected_candidate_id = judge_selection
            .as_ref()
            .map(|outcome| outcome.winner_id.clone())
            .or_else(|| {
                verifier_metric_selection(
                    &state.candidates,
                    state.selected_candidate_id.as_deref(),
                    latest_candidate_id,
                    self.objective_direction,
                )
            })
            .or_else(|| {
                state
                    .candidates
                    .first()
                    .map(|candidate| candidate.candidate_id.clone())
            });
        if let Some(outcome) = judge_selection {
            state.used_judge |= outcome.used_judge;
            state.used_logprobs |= outcome.used_logprobs;
            state.last_judge_rationale = outcome.rationale;
            state.last_judge_scores = outcome.scores;
        }

        prune_verifier_candidates(
            state,
            self.verifier.max_candidates.unwrap_or(3) as usize,
            self.objective_direction,
        );
    }

    pub(crate) fn selected_candidate<'a>(
        &self,
        state: &'a VerifierRuntimeState,
    ) -> Option<&'a VerifierCandidate> {
        let selected_id = state.selected_candidate_id.as_deref()?;
        state
            .candidates
            .iter()
            .find(|candidate| candidate.candidate_id == selected_id)
            .or_else(|| state.candidates.first())
    }

    pub(crate) fn build_summary(&self, state: &VerifierRuntimeState) -> WorkflowVerifierSummary {
        let selected = self.selected_candidate(state);
        let selection_strategy = self
            .verifier
            .selection
            .map(|strategy| match strategy {
                VerifierSelectionStrategy::RoundRobin => "round-robin",
                VerifierSelectionStrategy::Tournament => "tournament",
            })
            .unwrap_or("metric-fallback")
            .to_string();

        WorkflowVerifierSummary {
            selected_candidate_id: selected.map(|candidate| candidate.candidate_id.clone()),
            selected_iteration: selected.map(|candidate| candidate.source_iteration),
            candidates_considered: state.candidates.len() as u32,
            pairwise_comparisons: state.pairwise_comparisons,
            judge_calls: state.judge_calls,
            cache_hits: state.cache_hits,
            selection_strategy,
            used_judge: state.used_judge,
            used_logprobs: state.used_logprobs,
            judge_rationale: state.last_judge_rationale.clone(),
            criterion_scores: state
                .last_judge_scores
                .iter()
                .map(|score| WorkflowVerifierCriterionSummary {
                    criterion_id: score.criterion_id.clone(),
                    winner_candidate_id: score.winner_candidate_id.clone(),
                    score_a: score.score_a.map(format_verifier_score),
                    score_b: score.score_b.map(format_verifier_score),
                    explanation: score.explanation.clone(),
                })
                .collect(),
        }
    }

    pub(crate) fn export_artifacts(
        &self,
        state: &VerifierRuntimeState,
    ) -> Vec<WorkflowModeArtifact> {
        let matrix_entries = state
            .pairwise_records
            .iter()
            .enumerate()
            .map(|(index, record)| {
                let score_summary = record
                    .scores
                    .iter()
                    .map(|score| {
                        let winner = score
                            .winner_candidate_id
                            .as_deref()
                            .unwrap_or("unspecified");
                        let score_pair = match (score.score_a, score.score_b) {
                            (Some(a), Some(b)) => format!(" score_a={a:.4} score_b={b:.4}"),
                            _ => String::new(),
                        };
                        format!("{} -> {}{}", score.criterion_id, winner, score_pair)
                    })
                    .collect::<Vec<_>>();
                WorkflowModeArtifactEntry {
                    iteration: None,
                    key: format!("pairwise-{:03}", index + 1),
                    status: if record.cached { "cached" } else { "scored" }.to_string(),
                    title: format!("{} vs {}", record.incumbent_id, record.challenger_id),
                    detail: format!(
                        "winner={} strategy={} repetitions={} used_judge={} used_logprobs={} key={}",
                        record.winner_id,
                        selection_strategy_label(record.strategy),
                        record.repetitions,
                        record.used_judge,
                        record.used_logprobs,
                        record.comparison_key
                    ),
                    evidence: {
                        let mut evidence = score_summary;
                        if let (Some(score_a), Some(score_b)) =
                            (record.score_a_mean, record.score_b_mean)
                        {
                            evidence.push(format!(
                                "formula_mean score_a={score_a:.4} score_b={score_b:.4}"
                            ));
                        }
                        if !record.score_job_keys.is_empty() {
                            evidence.push(format!(
                                "score_jobs={}",
                                record.score_job_keys.join(",")
                            ));
                        }
                        evidence
                    },
                }
            })
            .collect::<Vec<_>>();
        let score_job_entries = state
            .score_job_records
            .iter()
            .enumerate()
            .map(|(index, record)| WorkflowModeArtifactEntry {
                iteration: None,
                key: format!("score-job-{:03}", index + 1),
                status: if record.cached { "cached" } else { "scored" }.to_string(),
                title: format!(
                    "{} vs {} / {} / rep {}",
                    record.job.incumbent_id,
                    record.job.challenger_id,
                    record.job.criterion_id,
                    record.job.repetition
                ),
                detail: format!(
                    "score_a={:.4} score_b={:.4} requested_logprobs={} requested_top_logprobs={} used_logprobs={} logprob_status={} fallback={} model={}/{} latency_ms={} error={} key={}",
                    record.result.score_a,
                    record.result.score_b,
                    record.job.requested_logprobs,
                    record.job.requested_top_logprobs,
                    record.result.used_logprobs,
                    logprob_status_label(record.result.logprob_status),
                    score_fallback_label(record.result.fallback_kind),
                    record
                        .result
                        .model_provider_id
                        .as_deref()
                        .unwrap_or(record.job.model_provider_id.as_str()),
                    record
                        .result
                        .model_id
                        .as_deref()
                        .unwrap_or(record.job.model_id.as_str()),
                    record
                        .result
                        .latency_ms
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "n/a".to_string()),
                    record.result.error.as_deref().unwrap_or("none"),
                    record.job.job_key
                ),
                evidence: {
                    let mut evidence = vec![
                        format!(
                            "objective={} trace_hash={} trace_format={}",
                            record.job.objective_fingerprint,
                            record.job.trace_hash,
                            record.job.trace_format
                        ),
                        format!(
                            "candidate_fingerprints {}={} {}={}",
                            record.job.incumbent_id,
                            record.job.incumbent_fingerprint,
                            record.job.challenger_id,
                            record.job.challenger_fingerprint
                        ),
                        format!(
                            "criterion_fingerprint {}={}",
                            record.job.criterion_id,
                            record.job.criterion_fingerprint
                        ),
                    ];
                    evidence.extend(record.result.raw_text_excerpt.iter().cloned());
                    evidence
                },
            })
            .collect::<Vec<_>>();
        let mut round_robin_win_entries = state
            .round_robin_win_counts
            .iter()
            .map(|(candidate_id, wins)| WorkflowModeArtifactEntry {
                iteration: state
                    .candidates
                    .iter()
                    .find(|candidate| candidate.candidate_id == *candidate_id)
                    .map(|candidate| candidate.source_iteration),
                key: candidate_id.clone(),
                status: format!("{wins:.4}"),
                title: format!("Round-robin wins for {candidate_id}"),
                detail: format!("wins={wins:.4}"),
                evidence: Vec::new(),
            })
            .collect::<Vec<_>>();
        round_robin_win_entries.sort_by(|left, right| left.key.cmp(&right.key));

        let selected = self.selected_candidate(state);
        vec![
            WorkflowModeArtifact {
                name: "score-job-matrix".to_string(),
                description:
                    "Verifier canonical score jobs by pair, criterion, repetition, and fallback."
                        .to_string(),
                entries: score_job_entries,
            },
            WorkflowModeArtifact {
                name: "pairwise-score-matrix".to_string(),
                description:
                    "Verifier pairwise comparison matrix with scores, cache status, and winners."
                        .to_string(),
                entries: matrix_entries,
            },
            WorkflowModeArtifact {
                name: "round-robin-win-counts".to_string(),
                description: "Verifier round-robin standings derived from pairwise winners."
                    .to_string(),
                entries: round_robin_win_entries,
            },
            WorkflowModeArtifact {
                name: "selection-report".to_string(),
                description: "Verifier final selection report and cost counters.".to_string(),
                entries: vec![WorkflowModeArtifactEntry {
                    iteration: selected.map(|candidate| candidate.source_iteration),
                    key: "selected-candidate".to_string(),
                    status: selected
                        .map(|candidate| candidate.candidate_id.clone())
                        .unwrap_or_else(|| "none".to_string()),
                    title: "Selected verifier candidate".to_string(),
                    detail: format!(
                        "pairwise_comparisons={} judge_calls={} cache_hits={} used_logprobs={}",
                        state.pairwise_comparisons,
                        state.judge_calls,
                        state.cache_hits,
                        state.used_logprobs
                    ),
                    evidence: state
                        .last_judge_rationale
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>(),
                }],
            },
        ]
    }

    fn pairwise_cache_key(
        &self,
        strategy: VerifierSelectionStrategy,
        repetitions: u32,
        incumbent: &VerifierCandidate,
        challenger: &VerifierCandidate,
    ) -> String {
        format!(
            "v2|model={}/{}|mode={}|strategy={}|reps={}|goal={}|trace={}|criteria={}|a={}|b={}",
            self.verifier.model.provider_id,
            self.verifier.model.model_id,
            if self.verifier.use_logprobs.unwrap_or(false) {
                "logprobs"
            } else {
                "json"
            },
            selection_strategy_label(strategy),
            repetitions,
            text_fingerprint(&self.objective_goal),
            trace_format_label(self.verifier.trace_format),
            criteria_fingerprint(&self.verifier.criteria),
            incumbent.fingerprint,
            challenger.fingerprint
        )
    }

    fn score_job_cache_key(
        &self,
        strategy: VerifierSelectionStrategy,
        incumbent: &VerifierCandidate,
        challenger: &VerifierCandidate,
        criterion: &VerifierCriterionDefinition,
        repetition: u32,
    ) -> String {
        format!(
            "v2|model={}/{}|mode=logprob-score-job|top_logprobs={}|strategy={}|goal={}|trace_format={}|trace={}|criterion={}|rep={}|a={}|b={}",
            self.verifier.model.provider_id,
            self.verifier.model.model_id,
            self.verifier.granularity.unwrap_or(20).clamp(1, 20),
            selection_strategy_label(strategy),
            text_fingerprint(&self.objective_goal),
            trace_format_label(self.verifier.trace_format),
            trace_pair_fingerprint(incumbent, challenger, self.verifier.trace_format),
            criteria_fingerprint(std::slice::from_ref(criterion)),
            repetition,
            incumbent.fingerprint,
            challenger.fingerprint
        )
    }

    fn score_job(
        &self,
        strategy: VerifierSelectionStrategy,
        incumbent: &VerifierCandidate,
        challenger: &VerifierCandidate,
        criterion: &VerifierCriterionDefinition,
        repetition: u32,
    ) -> VerifierScoreJob {
        let criterion_fingerprint = criteria_fingerprint(std::slice::from_ref(criterion));
        let trace_hash = trace_pair_fingerprint(incumbent, challenger, self.verifier.trace_format);
        VerifierScoreJob {
            job_key: self
                .score_job_cache_key(strategy, incumbent, challenger, criterion, repetition),
            objective_fingerprint: text_fingerprint(&self.objective_goal),
            incumbent_id: incumbent.candidate_id.clone(),
            incumbent_fingerprint: incumbent.fingerprint.clone(),
            challenger_id: challenger.candidate_id.clone(),
            challenger_fingerprint: challenger.fingerprint.clone(),
            criterion_id: criterion.id.clone(),
            criterion_fingerprint,
            repetition,
            requested_logprobs: self.verifier.use_logprobs.unwrap_or(false),
            requested_top_logprobs: self.verifier.granularity.unwrap_or(20).clamp(1, 20),
            trace_hash,
            trace_format: trace_format_label(self.verifier.trace_format).to_string(),
            model_provider_id: self.verifier.model.provider_id.clone(),
            model_id: self.verifier.model.model_id.clone(),
        }
    }

    pub(crate) fn judge_notes(&self, state: &VerifierRuntimeState) -> Vec<String> {
        let mut notes = Vec::new();
        if let Some(rationale) = state.last_judge_rationale.as_deref() {
            let trimmed = rationale.trim();
            if !trimmed.is_empty() {
                notes.push(format!("Verifier rationale: {trimmed}"));
            }
        }
        if !state.last_judge_scores.is_empty() {
            let criteria = state
                .last_judge_scores
                .iter()
                .map(|score| {
                    let winner = score
                        .winner_candidate_id
                        .as_deref()
                        .unwrap_or("unspecified");
                    let score_pair = match (score.score_a, score.score_b) {
                        (Some(score_a), Some(score_b)) => format!(" ({score_a:.1}/{score_b:.1})"),
                        _ => String::new(),
                    };
                    let explanation = score
                        .explanation
                        .as_deref()
                        .map(|text| format!(": {}", text.trim()))
                        .unwrap_or_default();
                    format!(
                        "{} -> {}{}{}",
                        score.criterion_id, winner, score_pair, explanation
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            notes.push(format!(
                "Verifier produced {} criterion score entries in the last judge decision: {}.",
                state.last_judge_scores.len(),
                criteria
            ));
        }
        notes
    }

    async fn select_round_robin(
        &self,
        state: &mut VerifierRuntimeState,
        _latest_candidate_id: &str,
    ) -> Option<VerifierSelectionOutcome> {
        let candidates = state.candidates.clone();
        if candidates.len() < 2 {
            return None;
        }

        let mut ordered = candidates;
        ordered.sort_by_key(|candidate| candidate.source_iteration);

        let mut wins = ordered
            .iter()
            .map(|candidate| (candidate.candidate_id.clone(), 0.0f64))
            .collect::<HashMap<_, _>>();
        let mut used_judge = false;
        let mut used_logprobs = false;
        let mut last_outcome = None;

        for left_index in 0..ordered.len() {
            for right_index in (left_index + 1)..ordered.len() {
                let left = ordered[left_index].clone();
                let right = ordered[right_index].clone();
                let winner_id = if let Some(outcome) = self
                    .run_pairwise_majority(
                        state,
                        left.clone(),
                        right.clone(),
                        VerifierSelectionStrategy::RoundRobin,
                    )
                    .await
                {
                    used_judge |= outcome.used_judge;
                    used_logprobs |= outcome.used_logprobs;
                    let winner_id = outcome.winner_id.clone();
                    last_outcome = Some(outcome);
                    winner_id
                } else {
                    best_verifier_candidate(&left, &right, self.objective_direction)
                        .candidate_id
                        .clone()
                };

                if let Some(win_count) = wins.get_mut(&winner_id) {
                    *win_count += 1.0;
                }
            }
        }
        state.round_robin_win_counts = wins.clone();

        let winner = ordered.into_iter().max_by(|left, right| {
            let left_wins = wins.get(&left.candidate_id).copied().unwrap_or_default();
            let right_wins = wins.get(&right.candidate_id).copied().unwrap_or_default();
            left_wins
                .partial_cmp(&right_wins)
                .unwrap_or(Ordering::Equal)
                .then_with(|| compare_verifier_candidates(right, left, self.objective_direction))
        })?;

        Some(VerifierSelectionOutcome {
            winner_id: winner.candidate_id,
            used_judge,
            used_logprobs,
            rationale: last_outcome
                .as_ref()
                .and_then(|outcome| outcome.rationale.clone()),
            scores: last_outcome
                .map(|outcome| outcome.scores)
                .unwrap_or_default(),
        })
    }

    async fn select_tournament(
        &self,
        state: &mut VerifierRuntimeState,
    ) -> Option<VerifierSelectionOutcome> {
        let mut ordered = state.candidates.clone();
        ordered.sort_by_key(|candidate| candidate.source_iteration);
        let initial_champion = state
            .selected_candidate_id
            .as_deref()
            .and_then(|selected_id| {
                ordered
                    .iter()
                    .find(|candidate| candidate.candidate_id == selected_id)
            })
            .cloned()
            .or_else(|| ordered.first().cloned())?;
        let mut champion = initial_champion;
        let mut last_outcome = None;
        let mut used_judge = false;
        let mut used_logprobs = false;

        for challenger in ordered {
            if challenger.candidate_id == champion.candidate_id {
                continue;
            }
            if let Some(outcome) = self
                .run_pairwise_majority(
                    state,
                    champion.clone(),
                    challenger.clone(),
                    VerifierSelectionStrategy::Tournament,
                )
                .await
            {
                used_judge |= outcome.used_judge;
                used_logprobs |= outcome.used_logprobs;
                if outcome.winner_id == challenger.candidate_id {
                    champion = challenger;
                }
                last_outcome = Some(outcome);
            } else {
                champion =
                    best_verifier_candidate(&champion, &challenger, self.objective_direction)
                        .clone();
            }
        }

        Some(VerifierSelectionOutcome {
            winner_id: champion.candidate_id,
            used_judge,
            used_logprobs,
            rationale: last_outcome
                .as_ref()
                .and_then(|outcome| outcome.rationale.clone()),
            scores: last_outcome
                .map(|outcome| outcome.scores)
                .unwrap_or_default(),
        })
    }

    async fn run_pairwise_majority(
        &self,
        state: &mut VerifierRuntimeState,
        incumbent: VerifierCandidate,
        challenger: VerifierCandidate,
        selection_strategy: VerifierSelectionStrategy,
    ) -> Option<VerifierSelectionOutcome> {
        let repetitions = self.verifier.repetitions.unwrap_or(1).max(1);
        if self.verifier.use_logprobs.unwrap_or(false) {
            return self
                .run_pairwise_formula(
                    state,
                    incumbent,
                    challenger,
                    selection_strategy,
                    repetitions,
                )
                .await;
        }
        let cache_key =
            self.pairwise_cache_key(selection_strategy, repetitions, &incumbent, &challenger);
        if let Some(cached) = state.pairwise_cache.get(&cache_key).cloned() {
            state.pairwise_comparisons += 1;
            state.cache_hits += 1;
            state.pairwise_records.push(VerifierPairwiseRecord {
                comparison_key: cache_key,
                incumbent_id: incumbent.candidate_id,
                challenger_id: challenger.candidate_id,
                winner_id: cached.winner_id.clone(),
                strategy: selection_strategy,
                repetitions,
                used_judge: cached.used_judge,
                used_logprobs: cached.used_logprobs,
                cached: true,
                score_a_mean: None,
                score_b_mean: None,
                scores: cached.scores.clone(),
                score_job_keys: Vec::new(),
            });
            return Some(cached);
        }

        let mut incumbent_votes = 0u32;
        let mut challenger_votes = 0u32;
        let mut used_judge = false;
        let mut used_logprobs = false;
        let mut decisions = Vec::new();

        for comparison_index in 1..=repetitions {
            let Some(context) = self.judge_context(
                incumbent.clone(),
                challenger.clone(),
                selection_strategy,
                comparison_index,
                repetitions,
            ) else {
                return None;
            };
            match self.run_judge(context).await {
                Ok(Some(decision)) if decision.winner_id == incumbent.candidate_id => {
                    state.judge_calls += decision.judge_calls;
                    incumbent_votes += 1;
                    used_judge = true;
                    used_logprobs |= decision.used_logprobs;
                    decisions.push(decision);
                }
                Ok(Some(decision)) if decision.winner_id == challenger.candidate_id => {
                    state.judge_calls += decision.judge_calls;
                    challenger_votes += 1;
                    used_judge = true;
                    used_logprobs |= decision.used_logprobs;
                    decisions.push(decision);
                }
                Ok(Some(decision)) => {
                    state.judge_calls += decision.judge_calls;
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "verifier judge failed during pairwise vote; falling back to metric selection for this matchup"
                    );
                }
            }
        }

        if !used_judge {
            return None;
        }

        let aggregated_scores = aggregate_criterion_scores(
            &incumbent.candidate_id,
            &challenger.candidate_id,
            &decisions,
        );
        let winner_id = if let Some(criterion_winner) = aggregate_criterion_winner(
            &incumbent.candidate_id,
            &challenger.candidate_id,
            &self.verifier.criteria,
            &aggregated_scores,
        ) {
            criterion_winner
        } else if challenger_votes > incumbent_votes {
            challenger.candidate_id.clone()
        } else if incumbent_votes > challenger_votes {
            incumbent.candidate_id.clone()
        } else {
            best_verifier_candidate(&incumbent, &challenger, self.objective_direction)
                .candidate_id
                .clone()
        };

        let outcome = VerifierSelectionOutcome {
            winner_id,
            used_judge,
            used_logprobs,
            rationale: decisions
                .last()
                .as_ref()
                .and_then(|decision| decision.rationale.clone()),
            scores: aggregated_scores,
        };
        state.pairwise_comparisons += 1;
        state.pairwise_records.push(VerifierPairwiseRecord {
            comparison_key: cache_key.clone(),
            incumbent_id: incumbent.candidate_id.clone(),
            challenger_id: challenger.candidate_id.clone(),
            winner_id: outcome.winner_id.clone(),
            strategy: selection_strategy,
            repetitions,
            used_judge: outcome.used_judge,
            used_logprobs: outcome.used_logprobs,
            cached: false,
            score_a_mean: None,
            score_b_mean: None,
            scores: outcome.scores.clone(),
            score_job_keys: Vec::new(),
        });
        state.pairwise_cache.insert(cache_key, outcome.clone());
        Some(outcome)
    }

    async fn run_pairwise_formula(
        &self,
        state: &mut VerifierRuntimeState,
        incumbent: VerifierCandidate,
        challenger: VerifierCandidate,
        selection_strategy: VerifierSelectionStrategy,
        repetitions: u32,
    ) -> Option<VerifierSelectionOutcome> {
        let comparison_key =
            self.pairwise_cache_key(selection_strategy, repetitions, &incumbent, &challenger);
        let mut results = Vec::new();
        let mut score_job_keys = Vec::new();
        let mut all_score_jobs_cached = true;

        for repetition in 1..=repetitions {
            for criterion in &self.verifier.criteria {
                let job = self.score_job(
                    selection_strategy,
                    &incumbent,
                    &challenger,
                    criterion,
                    repetition,
                );
                let (result, cached) = if let Some(cached) =
                    state.score_job_cache.get(&job.job_key).cloned()
                {
                    state.cache_hits += 1;
                    (cached, true)
                } else {
                    all_score_jobs_cached = false;
                    state.judge_calls += 1;
                    match self
                        .run_score_job(
                            incumbent.clone(),
                            challenger.clone(),
                            criterion.clone(),
                            repetition,
                            repetitions,
                            selection_strategy,
                            &job,
                        )
                        .await
                    {
                        Ok(result) => {
                            state
                                .score_job_cache
                                .insert(job.job_key.clone(), result.clone());
                            (result, false)
                        }
                        Err(err) => {
                            tracing::warn!(
                                error = %err,
                                criterion = criterion.id,
                                repetition,
                                "verifier score job failed; recording neutral score for deterministic fallback"
                            );
                            let result = VerifierScoreJobResult {
                                criterion_id: criterion.id.clone(),
                                repetition,
                                score_a: 0.5,
                                score_b: 0.5,
                                requested_logprobs: true,
                                used_logprobs: false,
                                logprob_status:
                                    VerifierLogprobStatus::RequestedMissingProviderMetadata,
                                model_provider_id: Some(self.verifier.model.provider_id.clone()),
                                model_id: Some(self.verifier.model.model_id.clone()),
                                latency_ms: None,
                                error: Some(err.to_string()),
                                fallback_kind: VerifierScoreFallbackKind::Error,
                                raw_text_excerpt: Some(err.to_string()),
                            };
                            state
                                .score_job_cache
                                .insert(job.job_key.clone(), result.clone());
                            (result, false)
                        }
                    }
                };

                state.score_job_records.push(VerifierScoreJobRecord {
                    job: job.clone(),
                    cached,
                    result: result.clone(),
                });
                score_job_keys.push(job.job_key);
                results.push(result);
            }
        }

        if results.is_empty() {
            return None;
        }

        let score_a_mean =
            results.iter().map(|result| result.score_a).sum::<f64>() / results.len() as f64;
        let score_b_mean =
            results.iter().map(|result| result.score_b).sum::<f64>() / results.len() as f64;
        let used_logprobs = results.iter().any(|result| result.used_logprobs);
        let winner_id = if score_a_mean > score_b_mean + VERIFIER_METRIC_EPSILON {
            incumbent.candidate_id.clone()
        } else if score_b_mean > score_a_mean + VERIFIER_METRIC_EPSILON {
            challenger.candidate_id.clone()
        } else {
            best_verifier_candidate(&incumbent, &challenger, self.objective_direction)
                .candidate_id
                .clone()
        };
        let aggregated_scores = aggregate_score_job_results(
            &incumbent.candidate_id,
            &challenger.candidate_id,
            &results,
        );
        let outcome = VerifierSelectionOutcome {
            winner_id,
            used_judge: true,
            used_logprobs,
            rationale: Some(if used_logprobs {
                "Verifier used canonical score-job expected reward aggregation.".to_string()
            } else {
                "Verifier used canonical score-job text-tag fallback aggregation.".to_string()
            }),
            scores: aggregated_scores,
        };
        state.pairwise_comparisons += 1;
        state.pairwise_records.push(VerifierPairwiseRecord {
            comparison_key,
            incumbent_id: incumbent.candidate_id,
            challenger_id: challenger.candidate_id,
            winner_id: outcome.winner_id.clone(),
            strategy: selection_strategy,
            repetitions,
            used_judge: true,
            used_logprobs,
            cached: all_score_jobs_cached,
            score_a_mean: Some(score_a_mean),
            score_b_mean: Some(score_b_mean),
            scores: outcome.scores.clone(),
            score_job_keys,
        });
        Some(outcome)
    }

    async fn run_score_job(
        &self,
        incumbent: VerifierCandidate,
        challenger: VerifierCandidate,
        criterion: VerifierCriterionDefinition,
        repetition: u32,
        repetitions: u32,
        selection_strategy: VerifierSelectionStrategy,
        job: &VerifierScoreJob,
    ) -> Result<VerifierScoreJobResult, OrchestratorError> {
        let context = VerifierJudgeContext {
            model_resolver: self.model_resolver.clone().ok_or_else(|| {
                OrchestratorError::Other("verifier judge model resolver is not bound".to_string())
            })?,
            lifecycle_hook: self.lifecycle_hook.clone(),
            cancel_token: self.cancel_token.clone(),
            exec_ctx: self.exec_ctx.clone().ok_or_else(|| {
                OrchestratorError::Other(
                    "verifier judge execution context is not bound".to_string(),
                )
            })?,
            model: self.verifier.model.clone(),
            goal: self.objective_goal.clone(),
            criteria: vec![criterion.clone()],
            trace_format: self.verifier.trace_format,
            selection_strategy,
            comparison_index: repetition,
            comparison_total: repetitions,
            incumbent,
            challenger,
        };
        let messages = build_verifier_logprob_messages(&context, &criterion);
        let mut exec_ctx = context.exec_ctx.clone();
        exec_ctx
            .metadata
            .insert("workflow_verifier_stage".to_string(), json!("score-job"));
        exec_ctx.metadata.insert(
            "workflow_verifier_score_job_key".to_string(),
            json!(job.job_key),
        );
        exec_ctx.metadata.insert(
            "workflow_verifier_criterion_id".to_string(),
            json!(job.criterion_id),
        );
        exec_ctx.metadata.insert(
            "workflow_verifier_repetition".to_string(),
            json!(job.repetition),
        );
        exec_ctx.metadata.insert(
            "workflow_verifier_incumbent_id".to_string(),
            json!(job.incumbent_id),
        );
        exec_ctx.metadata.insert(
            "workflow_verifier_challenger_id".to_string(),
            json!(job.challenger_id),
        );
        exec_ctx.metadata.insert(
            "workflow_verifier_requested_logprobs".to_string(),
            json!(job.requested_logprobs),
        );
        exec_ctx.metadata.insert(
            "workflow_verifier_requested_top_logprobs".to_string(),
            json!(job.requested_top_logprobs),
        );
        if let Some(cancel_token) = context.cancel_token.as_deref() {
            if cancel_token.is_cancelled() {
                return Err(OrchestratorError::Other(format!(
                    "verifier score job '{}' cancelled before model call",
                    job.job_key
                )));
            }
        }
        if let Some(lifecycle_hook) = context.lifecycle_hook.as_ref() {
            lifecycle_hook
                .on_scheduler_stage_start(
                    &exec_ctx.agent_name,
                    "verifier-score-job",
                    job.repetition,
                    None,
                    &exec_ctx,
                )
                .await;
        }
        let started_at = Instant::now();
        let stream_result = context
            .model_resolver
            .chat_stream(Some(&context.model), messages, Vec::new(), &exec_ctx)
            .await;
        let stream = match stream_result {
            Ok(stream) => stream,
            Err(err) => {
                if let Some(lifecycle_hook) = context.lifecycle_hook.as_ref() {
                    lifecycle_hook
                        .on_scheduler_stage_end(
                            &exec_ctx.agent_name,
                            "verifier-score-job",
                            job.repetition,
                            job.repetition,
                            &format!("error={err}"),
                            &exec_ctx,
                        )
                        .await;
                }
                return Err(err);
            }
        };
        let output =
            match collect_stream_output_governed(stream, context.cancel_token.as_deref()).await {
                Ok(output) => output,
                Err(err) => {
                    if let Some(lifecycle_hook) = context.lifecycle_hook.as_ref() {
                        lifecycle_hook
                            .on_scheduler_stage_end(
                                &exec_ctx.agent_name,
                                "verifier-score-job",
                                job.repetition,
                                job.repetition,
                                &format!("error={err}"),
                                &exec_ctx,
                            )
                            .await;
                    }
                    return Err(err);
                }
            };
        let latency_ms = started_at.elapsed().as_millis();
        let score_a = extract_tag_score(&output, "<score_A>");
        let score_b = extract_tag_score(&output, "<score_B>");

        if !score_a.used_logprobs
            && !score_b.used_logprobs
            && !score_a.parsed_from_text
            && !score_b.parsed_from_text
        {
            return Err(OrchestratorError::Other(format!(
                "verifier score job returned no parseable score tags for criterion '{}'",
                criterion.id
            )));
        }

        let used_logprobs = score_a.used_logprobs || score_b.used_logprobs;
        let logprob_status =
            score_job_logprob_status(&output, true, score_a.used_logprobs, score_b.used_logprobs);
        let fallback_kind = if used_logprobs {
            VerifierScoreFallbackKind::None
        } else {
            VerifierScoreFallbackKind::TextTag
        };

        let result = VerifierScoreJobResult {
            criterion_id: criterion.id,
            repetition,
            score_a: score_a.value,
            score_b: score_b.value,
            requested_logprobs: true,
            used_logprobs,
            logprob_status,
            model_provider_id: Some(self.verifier.model.provider_id.clone()),
            model_id: Some(self.verifier.model.model_id.clone()),
            latency_ms: Some(latency_ms),
            error: None,
            fallback_kind,
            raw_text_excerpt: (!output.text.trim().is_empty())
                .then(|| trim_explanation(&output.text, 400)),
        };
        if let Some(lifecycle_hook) = context.lifecycle_hook.as_ref() {
            lifecycle_hook
                .on_scheduler_stage_end(
                    &exec_ctx.agent_name,
                    "verifier-score-job",
                    job.repetition,
                    job.repetition,
                    &format!(
                        "score_a={:.4} score_b={:.4} used_logprobs={} fallback={}",
                        result.score_a,
                        result.score_b,
                        result.used_logprobs,
                        score_fallback_label(result.fallback_kind)
                    ),
                    &exec_ctx,
                )
                .await;
        }
        Ok(result)
    }

    fn judge_context(
        &self,
        incumbent: VerifierCandidate,
        challenger: VerifierCandidate,
        selection_strategy: VerifierSelectionStrategy,
        comparison_index: u32,
        comparison_total: u32,
    ) -> Option<VerifierJudgeContext> {
        Some(VerifierJudgeContext {
            model_resolver: self.model_resolver.clone()?,
            lifecycle_hook: self.lifecycle_hook.clone(),
            cancel_token: self.cancel_token.clone(),
            exec_ctx: self.exec_ctx.clone()?,
            model: self.verifier.model.clone(),
            goal: self.objective_goal.clone(),
            criteria: self.verifier.criteria.clone(),
            trace_format: self.verifier.trace_format,
            selection_strategy,
            comparison_index,
            comparison_total,
            incumbent,
            challenger,
        })
    }

    async fn run_judge(
        &self,
        context: VerifierJudgeContext,
    ) -> Result<Option<VerifierJudgeDecision>, OrchestratorError> {
        if self.verifier.use_logprobs.unwrap_or(false) {
            let decision = self.run_logprob_judge(context).await?;
            if decision.is_some() {
                return Ok(decision);
            }
            return Ok(None);
        }

        let messages = build_verifier_judge_messages(&context);
        let stream = context
            .model_resolver
            .chat_stream(
                Some(&context.model),
                messages,
                Vec::new(),
                &context.exec_ctx,
            )
            .await?;
        let response = collect_stream_output(stream).await?.text;
        let Some(verdict) = parse_verifier_judge_verdict(&response) else {
            tracing::warn!(
                response = response.trim(),
                "verifier judge returned unparseable verdict; falling back to metric selection"
            );
            return Ok(None);
        };

        let winner = verdict.winner.as_str();
        if winner != context.incumbent.candidate_id && winner != context.challenger.candidate_id {
            tracing::warn!(
                winner,
                incumbent = context.incumbent.candidate_id,
                challenger = context.challenger.candidate_id,
                "verifier judge picked an unknown candidate id; falling back to metric selection"
            );
            return Ok(None);
        }

        Ok(Some(VerifierJudgeDecision {
            winner_id: verdict.winner,
            rationale: verdict.rationale,
            used_logprobs: false,
            judge_calls: 1,
            scores: verdict
                .scores
                .unwrap_or_default()
                .into_iter()
                .map(|score| VerifierCriterionScore {
                    criterion_id: score.criterion_id,
                    winner_candidate_id: score.winner,
                    score_a: score.score_a,
                    score_b: score.score_b,
                    explanation: score.explanation,
                })
                .collect(),
        }))
    }

    async fn run_logprob_judge(
        &self,
        context: VerifierJudgeContext,
    ) -> Result<Option<VerifierJudgeDecision>, OrchestratorError> {
        let mut scores = Vec::new();
        let mut used_any_logprobs = false;

        for criterion in &context.criteria {
            let messages = build_verifier_logprob_messages(&context, criterion);
            let stream = context
                .model_resolver
                .chat_stream(
                    Some(&context.model),
                    messages,
                    Vec::new(),
                    &context.exec_ctx,
                )
                .await?;
            let output = collect_stream_output(stream).await?;
            let score_a = extract_tag_score(&output, "<score_A>");
            let score_b = extract_tag_score(&output, "<score_B>");
            used_any_logprobs |= score_a.used_logprobs || score_b.used_logprobs;

            let winner = if score_a.value > score_b.value + VERIFIER_METRIC_EPSILON {
                Some(context.incumbent.candidate_id.clone())
            } else if score_b.value > score_a.value + VERIFIER_METRIC_EPSILON {
                Some(context.challenger.candidate_id.clone())
            } else {
                None
            };
            scores.push(VerifierCriterionScore {
                criterion_id: criterion.id.clone(),
                winner_candidate_id: winner,
                score_a: Some(score_a.value),
                score_b: Some(score_b.value),
                explanation: (!output.text.trim().is_empty())
                    .then(|| trim_explanation(&output.text, 400)),
            });
        }

        if scores.is_empty() {
            return Ok(None);
        }
        let winner_id = aggregate_criterion_winner(
            &context.incumbent.candidate_id,
            &context.challenger.candidate_id,
            &context.criteria,
            &scores,
        )
        .unwrap_or_else(|| {
            best_verifier_candidate(
                &context.incumbent,
                &context.challenger,
                self.objective_direction,
            )
            .candidate_id
            .clone()
        });

        Ok(Some(VerifierJudgeDecision {
            winner_id,
            rationale: Some(if used_any_logprobs {
                "Verifier used score-token logprobs to compute expected reward.".to_string()
            } else {
                "Verifier requested logprobs but fell back to parsed score tags.".to_string()
            }),
            used_logprobs: used_any_logprobs,
            judge_calls: context.criteria.len() as u32,
            scores,
        }))
    }
}

struct VerifierJudgeContext {
    model_resolver: Arc<dyn ModelResolver>,
    lifecycle_hook: Option<Arc<dyn LifecycleHook>>,
    cancel_token: Option<Arc<dyn CancelToken>>,
    exec_ctx: ExecutionContext,
    model: crate::ModelRef,
    goal: String,
    criteria: Vec<VerifierCriterionDefinition>,
    trace_format: Option<VerifierTraceFormat>,
    selection_strategy: VerifierSelectionStrategy,
    comparison_index: u32,
    comparison_total: u32,
    incumbent: VerifierCandidate,
    challenger: VerifierCandidate,
}

#[derive(Debug, Deserialize)]
struct VerifierJudgeVerdict {
    winner: String,
    #[serde(default)]
    rationale: Option<String>,
    #[serde(default)]
    scores: Option<Vec<VerifierJudgeCriterionScore>>,
}

#[derive(Debug, Deserialize)]
struct VerifierJudgeCriterionScore {
    criterion_id: String,
    #[serde(default)]
    winner: Option<String>,
    #[serde(default)]
    score_a: Option<f64>,
    #[serde(default)]
    score_b: Option<f64>,
    #[serde(default)]
    explanation: Option<String>,
}

struct VerifierJudgeDecision {
    winner_id: String,
    rationale: Option<String>,
    used_logprobs: bool,
    judge_calls: u32,
    scores: Vec<VerifierCriterionScore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VerifierSelectionOutcome {
    winner_id: String,
    used_judge: bool,
    used_logprobs: bool,
    rationale: Option<String>,
    scores: Vec<VerifierCriterionScore>,
}

#[derive(Default)]
struct CriterionAggregate {
    score_a_sum: f64,
    score_a_count: u32,
    score_b_sum: f64,
    score_b_count: u32,
    incumbent_votes: u32,
    challenger_votes: u32,
    last_explanation: Option<String>,
}

fn build_verifier_judge_messages(context: &VerifierJudgeContext) -> Vec<rocode_provider::Message> {
    let criteria = context
        .criteria
        .iter()
        .map(|criterion| {
            let weight = criterion.weight.unwrap_or(1.0);
            let aggregation = match criterion
                .aggregation
                .unwrap_or(VerifierCriterionAggregation::ScoreMargin)
            {
                VerifierCriterionAggregation::WinnerVote => "winner-vote",
                VerifierCriterionAggregation::ScoreMargin => "score-margin",
            };
            format!(
                "- {} ({}; weight={weight:.2}; aggregation={}): {}",
                criterion.name, criterion.id, aggregation, criterion.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let trace_format = match context.trace_format {
        Some(VerifierTraceFormat::Compact) => "compact",
        Some(VerifierTraceFormat::Full) => "full",
        None => "compact",
    };
    let selection_strategy = match context.selection_strategy {
        VerifierSelectionStrategy::RoundRobin => "round-robin",
        VerifierSelectionStrategy::Tournament => "tournament",
    };
    let user_prompt = format!(
        "Task goal:\n{}\n\nSelection criteria:\n{}\n\nSelection strategy: {}\nComparison: {}/{}\nTrace format: {}\n\nCandidate A\nid: {}\niteration: {}\nmetric: {}\ntrace:\n{}\n\nCandidate B\nid: {}\niteration: {}\nmetric: {}\ntrace:\n{}\n\nReturn JSON only with this schema:\n{{\"winner\":\"{}|{}\",\"rationale\":\"optional\",\"scores\":[{{\"criterion_id\":\"criterion-id\",\"winner\":\"{}|{}\",\"score_a\":0.0,\"score_b\":0.0,\"explanation\":\"optional\"}}]}}",
        context.goal,
        criteria,
        selection_strategy,
        context.comparison_index,
        context.comparison_total,
        trace_format,
        context.incumbent.candidate_id,
        context.incumbent.source_iteration,
        display_candidate_metric(context.incumbent.metric_value),
        render_candidate_trace(&context.incumbent, context.trace_format),
        context.challenger.candidate_id,
        context.challenger.source_iteration,
        display_candidate_metric(context.challenger.metric_value),
        render_candidate_trace(&context.challenger, context.trace_format),
        context.incumbent.candidate_id,
        context.challenger.candidate_id,
        context.incumbent.candidate_id,
        context.challenger.candidate_id,
    );

    vec![
        rocode_provider::Message::system(
            "You are a strict pairwise verifier judge. Compare the two candidates and return JSON only. Do not add markdown fences or extra commentary.",
        ),
        rocode_provider::Message::user(user_prompt),
    ]
}

fn selection_strategy_label(strategy: VerifierSelectionStrategy) -> &'static str {
    match strategy {
        VerifierSelectionStrategy::RoundRobin => "round-robin",
        VerifierSelectionStrategy::Tournament => "tournament",
    }
}

fn score_fallback_label(fallback: VerifierScoreFallbackKind) -> &'static str {
    match fallback {
        VerifierScoreFallbackKind::None => "none",
        VerifierScoreFallbackKind::TextTag => "text-tag",
        VerifierScoreFallbackKind::JsonVerdict => "json-verdict",
        VerifierScoreFallbackKind::Metric => "metric",
        VerifierScoreFallbackKind::Error => "error",
    }
}

fn logprob_status_label(status: VerifierLogprobStatus) -> &'static str {
    match status {
        VerifierLogprobStatus::NotRequested => "not-requested",
        VerifierLogprobStatus::RequestedUsable => "requested-usable",
        VerifierLogprobStatus::RequestedMissingProviderMetadata => {
            "requested-missing-provider-metadata"
        }
        VerifierLogprobStatus::RequestedMissingLogprobsField => "requested-missing-logprobs-field",
        VerifierLogprobStatus::RequestedEmptyLogprobs => "requested-empty-logprobs",
        VerifierLogprobStatus::RequestedUnusableScoreTokenLogprobs => {
            "requested-unusable-score-token-logprobs"
        }
    }
}

fn candidate_fingerprint(
    output_content: &str,
    metric_value: Option<f64>,
    trace: &VerifierTraceSnapshot,
) -> String {
    let mut hasher = DefaultHasher::new();
    output_content.hash(&mut hasher);
    metric_value.map(f64::to_bits).hash(&mut hasher);
    trace.stable_fingerprint().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn criteria_fingerprint(criteria: &[VerifierCriterionDefinition]) -> String {
    let mut hasher = DefaultHasher::new();
    for criterion in criteria {
        criterion.id.hash(&mut hasher);
        criterion.name.hash(&mut hasher);
        criterion.description.hash(&mut hasher);
        criterion.weight.map(f64::to_bits).hash(&mut hasher);
        criterion.aggregation.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

fn text_fingerprint(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn trace_pair_fingerprint(
    incumbent: &VerifierCandidate,
    challenger: &VerifierCandidate,
    trace_format: Option<VerifierTraceFormat>,
) -> String {
    let mut hasher = DefaultHasher::new();
    trace_format_label(trace_format).hash(&mut hasher);
    incumbent.trace.stable_fingerprint().hash(&mut hasher);
    challenger.trace.stable_fingerprint().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn trace_format_label(trace_format: Option<VerifierTraceFormat>) -> &'static str {
    match trace_format {
        Some(VerifierTraceFormat::Full) => "full",
        Some(VerifierTraceFormat::Compact) | None => "compact",
    }
}

fn build_verifier_logprob_messages(
    context: &VerifierJudgeContext,
    criterion: &VerifierCriterionDefinition,
) -> Vec<rocode_provider::Message> {
    let trace_format = match context.trace_format {
        Some(VerifierTraceFormat::Compact) => "compact",
        Some(VerifierTraceFormat::Full) => "full",
        None => "compact",
    };
    let user_prompt = format!(
        "You are an expert evaluator of AI coding agents. Evaluate the two candidates on ONE criterion only.\n\nTask goal:\n{}\n\nCriterion: {} ({})\n{}\n\nTrace format: {}\n\nCandidate A\nid: {}\niteration: {}\nmetric: {}\ntrace:\n{}\n\nCandidate B\nid: {}\niteration: {}\nmetric: {}\ntrace:\n{}\n\nRating scale: use exactly one letter from A through T. A means clearly and completely succeeded with verified output. T means clearly and completely failed. Intermediate letters monotonically decrease in quality.\n\nThink briefly, then output final score tags exactly as:\n<score_A>A</score_A>\n<score_B>A</score_B>",
        context.goal,
        criterion.name,
        criterion.id,
        criterion.description,
        trace_format,
        context.incumbent.candidate_id,
        context.incumbent.source_iteration,
        display_candidate_metric(context.incumbent.metric_value),
        render_candidate_trace(&context.incumbent, context.trace_format),
        context.challenger.candidate_id,
        context.challenger.source_iteration,
        display_candidate_metric(context.challenger.metric_value),
        render_candidate_trace(&context.challenger, context.trace_format),
    );

    vec![
        rocode_provider::Message::system(
            "You are a strict verifier judge. The score token immediately after each score tag is the only final answer that matters.",
        ),
        rocode_provider::Message::user(user_prompt),
    ]
}

fn aggregate_criterion_scores(
    incumbent_id: &str,
    challenger_id: &str,
    decisions: &[VerifierJudgeDecision],
) -> Vec<VerifierCriterionScore> {
    let mut aggregates: HashMap<String, CriterionAggregate> = HashMap::new();

    for decision in decisions {
        for score in &decision.scores {
            let entry = aggregates.entry(score.criterion_id.clone()).or_default();
            if let Some(score_a) = score.score_a {
                entry.score_a_sum += score_a;
                entry.score_a_count += 1;
            }
            if let Some(score_b) = score.score_b {
                entry.score_b_sum += score_b;
                entry.score_b_count += 1;
            }

            match criterion_entry_winner(incumbent_id, challenger_id, score) {
                Some(candidate_id) if candidate_id == incumbent_id => entry.incumbent_votes += 1,
                Some(candidate_id) if candidate_id == challenger_id => entry.challenger_votes += 1,
                _ => {}
            }

            if let Some(explanation) = score
                .explanation
                .as_ref()
                .filter(|text| !text.trim().is_empty())
            {
                entry.last_explanation = Some(explanation.trim().to_string());
            }
        }
    }

    let mut scores = aggregates
        .into_iter()
        .map(|(criterion_id, aggregate)| {
            let winner_candidate_id = if aggregate.challenger_votes > aggregate.incumbent_votes {
                Some(challenger_id.to_string())
            } else if aggregate.incumbent_votes > aggregate.challenger_votes {
                Some(incumbent_id.to_string())
            } else if let (Some(avg_a), Some(avg_b)) = (
                average_option(aggregate.score_a_sum, aggregate.score_a_count),
                average_option(aggregate.score_b_sum, aggregate.score_b_count),
            ) {
                if (avg_a - avg_b).abs() > VERIFIER_METRIC_EPSILON {
                    Some(if avg_b > avg_a {
                        challenger_id.to_string()
                    } else {
                        incumbent_id.to_string()
                    })
                } else {
                    None
                }
            } else {
                None
            };

            VerifierCriterionScore {
                criterion_id,
                winner_candidate_id,
                score_a: average_option(aggregate.score_a_sum, aggregate.score_a_count),
                score_b: average_option(aggregate.score_b_sum, aggregate.score_b_count),
                explanation: aggregate.last_explanation,
            }
        })
        .collect::<Vec<_>>();
    scores.sort_by(|left, right| left.criterion_id.cmp(&right.criterion_id));
    scores
}

fn aggregate_score_job_results(
    incumbent_id: &str,
    challenger_id: &str,
    results: &[VerifierScoreJobResult],
) -> Vec<VerifierCriterionScore> {
    let mut aggregates: HashMap<String, CriterionAggregate> = HashMap::new();

    for result in results {
        let entry = aggregates.entry(result.criterion_id.clone()).or_default();
        entry.score_a_sum += result.score_a;
        entry.score_a_count += 1;
        entry.score_b_sum += result.score_b;
        entry.score_b_count += 1;
    }

    let mut scores = aggregates
        .into_iter()
        .map(|(criterion_id, aggregate)| {
            let score_a = average_option(aggregate.score_a_sum, aggregate.score_a_count);
            let score_b = average_option(aggregate.score_b_sum, aggregate.score_b_count);
            let winner_candidate_id = match (score_a, score_b) {
                (Some(score_a), Some(score_b)) if score_a > score_b + VERIFIER_METRIC_EPSILON => {
                    Some(incumbent_id.to_string())
                }
                (Some(score_a), Some(score_b)) if score_b > score_a + VERIFIER_METRIC_EPSILON => {
                    Some(challenger_id.to_string())
                }
                _ => None,
            };

            VerifierCriterionScore {
                criterion_id,
                winner_candidate_id,
                score_a,
                score_b,
                explanation: Some(format!(
                    "mean over {} canonical score job(s)",
                    aggregate.score_a_count.max(aggregate.score_b_count)
                )),
            }
        })
        .collect::<Vec<_>>();
    scores.sort_by(|left, right| left.criterion_id.cmp(&right.criterion_id));
    scores
}

fn criterion_entry_winner<'a>(
    incumbent_id: &'a str,
    challenger_id: &'a str,
    score: &'a VerifierCriterionScore,
) -> Option<&'a str> {
    match score.winner_candidate_id.as_deref() {
        Some(candidate_id) if candidate_id == incumbent_id || candidate_id == challenger_id => {
            Some(candidate_id)
        }
        _ => match (score.score_a, score.score_b) {
            (Some(score_a), Some(score_b))
                if (score_a - score_b).abs() > VERIFIER_METRIC_EPSILON =>
            {
                Some(if score_b > score_a {
                    challenger_id
                } else {
                    incumbent_id
                })
            }
            _ => None,
        },
    }
}

fn aggregate_criterion_winner(
    incumbent_id: &str,
    challenger_id: &str,
    criteria: &[VerifierCriterionDefinition],
    scores: &[VerifierCriterionScore],
) -> Option<String> {
    let mut incumbent_support = 0.0f64;
    let mut challenger_support = 0.0f64;

    for score in scores {
        let weight = criterion_weight(criteria, &score.criterion_id);
        match criterion_aggregation(criteria, &score.criterion_id) {
            VerifierCriterionAggregation::WinnerVote => {
                match score.winner_candidate_id.as_deref() {
                    Some(candidate_id) if candidate_id == incumbent_id => {
                        incumbent_support += weight
                    }
                    Some(candidate_id) if candidate_id == challenger_id => {
                        challenger_support += weight
                    }
                    _ => {}
                }
            }
            VerifierCriterionAggregation::ScoreMargin => {
                if let (Some(score_a), Some(score_b)) = (score.score_a, score.score_b) {
                    let weighted_margin = (score_a - score_b).abs() * weight;
                    if score_b > score_a {
                        challenger_support += weighted_margin;
                    } else if score_a > score_b {
                        incumbent_support += weighted_margin;
                    }
                } else {
                    match score.winner_candidate_id.as_deref() {
                        Some(candidate_id) if candidate_id == incumbent_id => {
                            incumbent_support += weight
                        }
                        Some(candidate_id) if candidate_id == challenger_id => {
                            challenger_support += weight
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    if (challenger_support - incumbent_support).abs() > VERIFIER_METRIC_EPSILON {
        Some(if challenger_support > incumbent_support {
            challenger_id.to_string()
        } else {
            incumbent_id.to_string()
        })
    } else {
        None
    }
}

fn criterion_weight(criteria: &[VerifierCriterionDefinition], criterion_id: &str) -> f64 {
    criteria
        .iter()
        .find(|criterion| criterion.id == criterion_id)
        .and_then(|criterion| criterion.weight)
        .unwrap_or(1.0)
}

fn criterion_aggregation(
    criteria: &[VerifierCriterionDefinition],
    criterion_id: &str,
) -> VerifierCriterionAggregation {
    criteria
        .iter()
        .find(|criterion| criterion.id == criterion_id)
        .and_then(|criterion| criterion.aggregation)
        .unwrap_or(VerifierCriterionAggregation::ScoreMargin)
}

fn average_option(sum: f64, count: u32) -> Option<f64> {
    (count > 0).then_some(sum / count as f64)
}

fn format_verifier_score(value: f64) -> String {
    format!("{value:.4}")
}

#[derive(Debug, Default)]
struct CollectedJudgeOutput {
    text: String,
    logprobs: Vec<VerifierLogprobEntry>,
    provider_metadata_seen: bool,
    logprobs_field_seen: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct VerifierLogprobEntry {
    token: String,
    logprob: f64,
    #[serde(default)]
    top_logprobs: Vec<VerifierTopLogprob>,
}

#[derive(Debug, Clone, Deserialize)]
struct VerifierTopLogprob {
    token: String,
    logprob: f64,
}

async fn collect_stream_output(
    mut stream: rocode_provider::StreamResult,
) -> Result<CollectedJudgeOutput, OrchestratorError> {
    collect_stream_output_inner(&mut stream, None).await
}

async fn collect_stream_output_governed(
    mut stream: rocode_provider::StreamResult,
    cancel_token: Option<&dyn CancelToken>,
) -> Result<CollectedJudgeOutput, OrchestratorError> {
    collect_stream_output_inner(&mut stream, cancel_token).await
}

async fn collect_stream_output_inner(
    stream: &mut rocode_provider::StreamResult,
    cancel_token: Option<&dyn CancelToken>,
) -> Result<CollectedJudgeOutput, OrchestratorError> {
    let mut output = CollectedJudgeOutput::default();

    while let Some(event) = stream.next().await {
        if cancel_token.is_some_and(|token| token.is_cancelled()) {
            return Err(OrchestratorError::Other(
                "verifier score job cancelled during stream consumption".to_string(),
            ));
        }
        match event {
            Ok(rocode_provider::StreamEvent::TextDelta(text)) => output.text.push_str(&text),
            Ok(rocode_provider::StreamEvent::FinishStep {
                provider_metadata, ..
            }) => {
                if let Some(metadata) = provider_metadata {
                    output.provider_metadata_seen = true;
                    if metadata.get("logprobs").is_some() {
                        output.logprobs_field_seen = true;
                    }
                    output
                        .logprobs
                        .extend(parse_provider_logprobs(metadata).unwrap_or_default());
                }
            }
            Ok(rocode_provider::StreamEvent::Done) => break,
            Ok(rocode_provider::StreamEvent::Error(error)) => {
                return Err(OrchestratorError::Other(format!(
                    "verifier judge returned stream error: {error}"
                )));
            }
            Err(error) => {
                return Err(OrchestratorError::Other(format!(
                    "verifier judge provider error: {error}"
                )));
            }
            _ => {}
        }
    }

    Ok(output)
}

fn score_job_logprob_status(
    output: &CollectedJudgeOutput,
    requested_logprobs: bool,
    score_a_used_logprobs: bool,
    score_b_used_logprobs: bool,
) -> VerifierLogprobStatus {
    if !requested_logprobs {
        return VerifierLogprobStatus::NotRequested;
    }
    if score_a_used_logprobs || score_b_used_logprobs {
        return VerifierLogprobStatus::RequestedUsable;
    }
    if !output.provider_metadata_seen {
        return VerifierLogprobStatus::RequestedMissingProviderMetadata;
    }
    if !output.logprobs_field_seen {
        return VerifierLogprobStatus::RequestedMissingLogprobsField;
    }
    if output.logprobs.is_empty() {
        return VerifierLogprobStatus::RequestedEmptyLogprobs;
    }
    VerifierLogprobStatus::RequestedUnusableScoreTokenLogprobs
}

fn parse_provider_logprobs(metadata: serde_json::Value) -> Option<Vec<VerifierLogprobEntry>> {
    let value = metadata.get("logprobs")?.clone();
    if let Ok(groups) = serde_json::from_value::<Vec<Vec<VerifierLogprobEntry>>>(value.clone()) {
        return Some(groups.into_iter().flatten().collect());
    }
    serde_json::from_value::<Vec<VerifierLogprobEntry>>(value).ok()
}

#[derive(Debug, Clone, Copy)]
struct ExtractedScore {
    value: f64,
    used_logprobs: bool,
    parsed_from_text: bool,
}

fn extract_tag_score(output: &CollectedJudgeOutput, tag: &str) -> ExtractedScore {
    if let Some(value) = extract_score_from_logprobs(&output.logprobs, tag) {
        return ExtractedScore {
            value,
            used_logprobs: true,
            parsed_from_text: false,
        };
    }
    let text_score = extract_score_from_text(&output.text, tag);
    ExtractedScore {
        value: text_score.unwrap_or(0.5),
        used_logprobs: false,
        parsed_from_text: text_score.is_some(),
    }
}

fn extract_score_from_logprobs(entries: &[VerifierLogprobEntry], tag: &str) -> Option<f64> {
    if entries.is_empty() {
        return None;
    }
    let mut text_so_far = String::new();
    for (index, entry) in entries.iter().enumerate() {
        text_so_far.push_str(&entry.token);
        if text_so_far.trim_end().ends_with(tag) {
            let next = entries.get(index + 1)?;
            return expected_score_from_logprob_entry(next);
        }
    }
    None
}

fn expected_score_from_logprob_entry(entry: &VerifierLogprobEntry) -> Option<f64> {
    let mut probs_by_value: HashMap<u32, f64> = HashMap::new();
    add_score_token_probability(&mut probs_by_value, &entry.token, entry.logprob);
    for candidate in &entry.top_logprobs {
        add_score_token_probability(&mut probs_by_value, &candidate.token, candidate.logprob);
    }
    if probs_by_value.is_empty() {
        return None;
    }
    let total_probability = probs_by_value.values().sum::<f64>();
    if total_probability <= 0.0 {
        return None;
    }
    let expected = probs_by_value
        .into_iter()
        .map(|(value, probability)| value as f64 * probability)
        .sum::<f64>()
        / total_probability;
    Some((expected - 1.0) / 19.0)
}

fn add_score_token_probability(scores: &mut HashMap<u32, f64>, token: &str, logprob: f64) {
    let Some(value) = score_token_value(token) else {
        return;
    };
    let probability = logprob.exp();
    scores
        .entry(value)
        .and_modify(|existing| *existing = existing.max(probability))
        .or_insert(probability);
}

fn score_token_value(token: &str) -> Option<u32> {
    let trimmed = token.trim();
    let mut chars = trimmed.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    if chars.next().is_some() || !('A'..='T').contains(&letter) {
        return None;
    }
    Some(20 - (letter as u32 - 'A' as u32))
}

fn extract_score_from_text(text: &str, tag: &str) -> Option<f64> {
    let tag_name = tag.trim_matches(['<', '>']);
    let open = format!("<{tag_name}>");
    let close = format!("</{tag_name}>");
    let start = text.find(&open)? + open.len();
    let rest = &text[start..];
    let end = rest.find(&close)?;
    let value = score_token_value(&rest[..end])?;
    Some((value as f64 - 1.0) / 19.0)
}

fn trim_explanation(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    trimmed.chars().take(max_chars).collect::<String>()
}

fn parse_verifier_judge_verdict(response: &str) -> Option<VerifierJudgeVerdict> {
    serde_json::from_str(response.trim()).ok().or_else(|| {
        let start = response.find('{')?;
        let end = response.rfind('}')?;
        serde_json::from_str::<VerifierJudgeVerdict>(&response[start..=end]).ok()
    })
}

fn render_candidate_trace(
    candidate: &VerifierCandidate,
    trace_format: Option<VerifierTraceFormat>,
) -> String {
    candidate.trace.render(trace_format)
}

fn display_candidate_metric(metric_value: Option<f64>) -> String {
    metric_value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "n/a".to_string())
}

fn verifier_metric_selection(
    candidates: &[VerifierCandidate],
    selected_candidate_id: Option<&str>,
    latest_candidate_id: &str,
    direction: ObjectiveDirection,
) -> Option<String> {
    let challenger = candidates
        .iter()
        .find(|candidate| candidate.candidate_id == latest_candidate_id)?;
    let incumbent = selected_candidate_id
        .and_then(|selected_id| {
            candidates
                .iter()
                .find(|candidate| candidate.candidate_id == selected_id)
        })
        .or_else(|| candidates.first())?;

    Some(
        best_verifier_candidate(incumbent, challenger, direction)
            .candidate_id
            .clone(),
    )
}

fn best_verifier_candidate<'a>(
    left: &'a VerifierCandidate,
    right: &'a VerifierCandidate,
    direction: ObjectiveDirection,
) -> &'a VerifierCandidate {
    match compare_verifier_candidates(left, right, direction) {
        Ordering::Greater => right,
        _ => left,
    }
}

fn compare_verifier_candidates(
    left: &VerifierCandidate,
    right: &VerifierCandidate,
    direction: ObjectiveDirection,
) -> Ordering {
    match (left.metric_value, right.metric_value) {
        (Some(left_metric), Some(right_metric))
            if (left_metric - right_metric).abs() > VERIFIER_METRIC_EPSILON =>
        {
            let left_is_better = match direction {
                ObjectiveDirection::HigherIsBetter => left_metric > right_metric,
                ObjectiveDirection::LowerIsBetter => left_metric < right_metric,
            };
            if left_is_better {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        _ => right.source_iteration.cmp(&left.source_iteration),
    }
}

fn prune_verifier_candidates(
    state: &mut VerifierRuntimeState,
    max_candidates: usize,
    direction: ObjectiveDirection,
) {
    if state.candidates.len() <= max_candidates {
        return;
    }

    state
        .candidates
        .sort_by(|left, right| compare_verifier_candidates(left, right, direction));

    let selected_id = state.selected_candidate_id.clone();
    if let Some(selected_id) = selected_id.as_deref() {
        if let Some(selected_index) = state
            .candidates
            .iter()
            .position(|candidate| candidate.candidate_id == selected_id)
        {
            if selected_index >= max_candidates {
                let selected = state.candidates.remove(selected_index);
                state.candidates.truncate(max_candidates.saturating_sub(1));
                state.candidates.push(selected);
                return;
            }
        }
    }

    state.candidates.truncate(max_candidates);
    if state
        .selected_candidate_id
        .as_deref()
        .is_some_and(|selected_id| {
            !state
                .candidates
                .iter()
                .any(|candidate| candidate.candidate_id == selected_id)
        })
    {
        state.selected_candidate_id = state
            .candidates
            .first()
            .map(|candidate| candidate.candidate_id.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verifier_trace::{trajectory_fingerprint, VerifierCommandTrace};

    fn candidate_with_repeated_output(repeat: usize) -> VerifierCandidate {
        let output_content = "abcdef".repeat(repeat);
        let verify = VerifierCommandTrace {
            name: "verify",
            exit_code: Some(0),
            passed: true,
            timed_out: false,
            runtime_error: None,
            output: output_content.clone(),
        };
        let trajectory_fingerprint = trajectory_fingerprint(
            "cand-001",
            1,
            &output_content,
            1,
            0,
            &[],
            Some(1.0),
            ObjectiveDirection::HigherIsBetter,
            &verify,
            None,
            &[],
            &[],
        );
        VerifierCandidate {
            candidate_id: "cand-001".to_string(),
            source_iteration: 1,
            fingerprint: format!("test-{repeat}"),
            output_content: output_content.clone(),
            metric_value: Some(1.0),
            trace: VerifierTraceSnapshot {
                candidate_id: "cand-001".to_string(),
                source_iteration: 1,
                trajectory_fingerprint,
                final_response: output_content.clone(),
                execution_steps: 1,
                execution_tool_calls: 0,
                execution_metadata_summary: Vec::new(),
                metric_value: Some(1.0),
                objective_direction: ObjectiveDirection::HigherIsBetter,
                verify,
                guard: None,
                change_summary: Vec::new(),
                artifact_summary: Vec::new(),
            },
        }
    }

    #[test]
    fn render_candidate_trace_uses_compact_and_full_formats() {
        let candidate = candidate_with_repeated_output(1_000);

        let compact = render_candidate_trace(&candidate, Some(VerifierTraceFormat::Compact));
        let full = render_candidate_trace(&candidate, Some(VerifierTraceFormat::Full));

        assert!(compact.contains("format: compact"));
        assert!(compact.contains("verify command"));
        assert!(compact.contains("final_response"));
        assert!(compact.contains("...[truncated middle]..."));
        assert!(full.contains("format: full"));
        assert!(full.contains("verify command"));
        assert!(full.len() > compact.len());
    }

    #[test]
    fn score_token_logprobs_compute_expected_reward() {
        let output = CollectedJudgeOutput {
            text: "<score_A>A</score_A>".to_string(),
            logprobs: vec![
                VerifierLogprobEntry {
                    token: "<score_A>".to_string(),
                    logprob: 0.0,
                    top_logprobs: Vec::new(),
                },
                VerifierLogprobEntry {
                    token: "A".to_string(),
                    logprob: 0.7f64.ln(),
                    top_logprobs: vec![VerifierTopLogprob {
                        token: "T".to_string(),
                        logprob: 0.3f64.ln(),
                    }],
                },
            ],
            provider_metadata_seen: true,
            logprobs_field_seen: true,
        };

        let score = extract_tag_score(&output, "<score_A>");

        assert!(score.used_logprobs);
        assert!(!score.parsed_from_text);
        assert!((score.value - 0.7).abs() < 1e-9);
    }

    #[test]
    fn score_token_extraction_falls_back_to_text_tags() {
        let output = CollectedJudgeOutput {
            text: "analysis\n<score_A>C</score_A>\n<score_B>T</score_B>".to_string(),
            logprobs: Vec::new(),
            provider_metadata_seen: false,
            logprobs_field_seen: false,
        };

        let score_a = extract_tag_score(&output, "<score_A>");
        let score_b = extract_tag_score(&output, "<score_B>");

        assert!(!score_a.used_logprobs);
        assert!(score_a.parsed_from_text);
        assert_eq!(score_a.value, 17.0 / 19.0);
        assert_eq!(score_b.value, 0.0);
    }

    #[test]
    fn logprob_status_reports_provider_capability_shape() {
        let no_metadata = CollectedJudgeOutput {
            text: "<score_A>A</score_A>".to_string(),
            logprobs: Vec::new(),
            provider_metadata_seen: false,
            logprobs_field_seen: false,
        };
        assert_eq!(
            score_job_logprob_status(&no_metadata, true, false, false),
            VerifierLogprobStatus::RequestedMissingProviderMetadata
        );

        let no_logprobs_field = CollectedJudgeOutput {
            text: "<score_A>A</score_A>".to_string(),
            logprobs: Vec::new(),
            provider_metadata_seen: true,
            logprobs_field_seen: false,
        };
        assert_eq!(
            score_job_logprob_status(&no_logprobs_field, true, false, false),
            VerifierLogprobStatus::RequestedMissingLogprobsField
        );

        let empty_logprobs = CollectedJudgeOutput {
            text: "<score_A>A</score_A>".to_string(),
            logprobs: Vec::new(),
            provider_metadata_seen: true,
            logprobs_field_seen: true,
        };
        assert_eq!(
            score_job_logprob_status(&empty_logprobs, true, false, false),
            VerifierLogprobStatus::RequestedEmptyLogprobs
        );
    }

    #[test]
    fn score_job_results_average_by_criterion() {
        let scores = aggregate_score_job_results(
            "cand-001",
            "cand-002",
            &[
                VerifierScoreJobResult {
                    criterion_id: "correctness".to_string(),
                    repetition: 1,
                    score_a: 0.2,
                    score_b: 0.8,
                    requested_logprobs: true,
                    used_logprobs: true,
                    logprob_status: VerifierLogprobStatus::RequestedUsable,
                    model_provider_id: Some("test".to_string()),
                    model_id: Some("judge".to_string()),
                    latency_ms: Some(1),
                    error: None,
                    fallback_kind: VerifierScoreFallbackKind::None,
                    raw_text_excerpt: None,
                },
                VerifierScoreJobResult {
                    criterion_id: "correctness".to_string(),
                    repetition: 2,
                    score_a: 0.8,
                    score_b: 0.2,
                    requested_logprobs: true,
                    used_logprobs: true,
                    logprob_status: VerifierLogprobStatus::RequestedUsable,
                    model_provider_id: Some("test".to_string()),
                    model_id: Some("judge".to_string()),
                    latency_ms: Some(1),
                    error: None,
                    fallback_kind: VerifierScoreFallbackKind::None,
                    raw_text_excerpt: None,
                },
                VerifierScoreJobResult {
                    criterion_id: "safety".to_string(),
                    repetition: 1,
                    score_a: 0.9,
                    score_b: 0.1,
                    requested_logprobs: true,
                    used_logprobs: false,
                    logprob_status: VerifierLogprobStatus::RequestedMissingProviderMetadata,
                    model_provider_id: Some("test".to_string()),
                    model_id: Some("judge".to_string()),
                    latency_ms: Some(1),
                    error: None,
                    fallback_kind: VerifierScoreFallbackKind::TextTag,
                    raw_text_excerpt: None,
                },
            ],
        );

        let correctness = scores
            .iter()
            .find(|score| score.criterion_id == "correctness")
            .expect("correctness aggregate should exist");
        assert_eq!(correctness.winner_candidate_id, None);
        assert!((correctness.score_a.unwrap() - 0.5).abs() < 1e-9);
        assert!((correctness.score_b.unwrap() - 0.5).abs() < 1e-9);

        let safety = scores
            .iter()
            .find(|score| score.criterion_id == "safety")
            .expect("safety aggregate should exist");
        assert_eq!(safety.winner_candidate_id.as_deref(), Some("cand-001"));
    }
}
