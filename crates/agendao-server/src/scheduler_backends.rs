use agendao_execution_types::CompiledExecutionRequest;
use agendao_orchestrator::agent_loop::{
    AgentObservationContext, ToolBackend, ToolCall, ToolExecution,
};
use agendao_orchestrator::blueprint::EvaluatorId;
use agendao_orchestrator::context::NodeResult;
use agendao_orchestrator::engine::{Evaluation, EvaluationOutcome, EvaluatorBackend};
use agendao_orchestrator::selector::{PlannerBackend, PlannerDecision, PlannerInput};
use agendao_provider::{Content, Message, Provider};
use agendao_tool::{ToolContext, ToolRegistry};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::scheduler_cache::BoundedLruCache;

const MAX_PLANNER_CATALOG_CACHE_ENTRIES: usize = 32;
const MAX_PLANNER_CATALOG_CACHE_BYTES: usize = 1024 * 1024;

type PlannerCatalogCache = BoundedLruCache<String, Arc<str>>;

static PLANNER_CATALOG_CACHE: OnceLock<Mutex<PlannerCatalogCache>> = OnceLock::new();
static PLANNER_CATALOG_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static PLANNER_CATALOG_CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
static PLANNER_CATALOG_CACHE_EVICTIONS: AtomicU64 = AtomicU64::new(0);

pub struct RegistryToolBackend {
    registry: Arc<ToolRegistry>,
    context: ToolContext,
}

impl RegistryToolBackend {
    pub fn new(registry: Arc<ToolRegistry>, context: ToolContext) -> Self {
        Self { registry, context }
    }
}

#[async_trait]
impl ToolBackend for RegistryToolBackend {
    async fn execute(
        &self,
        _context: &AgentObservationContext<'_>,
        call: &ToolCall,
    ) -> Result<ToolExecution, String> {
        let mut context = self.context.clone();
        context.call_id = Some(call.id.clone());
        self.registry
            .execute(call.tool.as_str(), call.arguments.clone(), context)
            .await
            .map(|result| ToolExecution {
                output: result.output,
                title: (!result.title.is_empty()).then_some(result.title),
                metadata: (!result.metadata.is_empty())
                    .then(|| serde_json::to_value(result.metadata).unwrap_or_default()),
                is_error: false,
            })
            .map_err(|error| error.to_string())
    }
}

pub struct ModelEvaluatorBackend {
    provider: Arc<dyn Provider>,
    request_defaults: CompiledExecutionRequest,
    prompts: BTreeMap<EvaluatorId, String>,
}

pub struct ModelPlannerBackend {
    provider: Arc<dyn Provider>,
    request_defaults: CompiledExecutionRequest,
}

impl ModelPlannerBackend {
    pub fn new(provider: Arc<dyn Provider>, request_defaults: CompiledExecutionRequest) -> Self {
        Self {
            provider,
            request_defaults,
        }
    }
}

#[async_trait]
impl PlannerBackend for ModelPlannerBackend {
    async fn plan(&self, input: PlannerInput) -> Result<PlannerDecision, String> {
        let prompt = planner_prompt_json(&input)?;
        let response = self
            .provider
            .chat(self.request_defaults.to_chat_request_with_system(
                vec![Message::user(prompt)],
                Vec::new(),
                Some(false),
                Some(
                    "Select an AgenDao scheduler. Return exactly one JSON object matching the requested typed decision. Use only catalog IDs, stay within policy limits, and never add prose or markdown fences."
                        .to_string(),
                ),
            ))
            .await
            .map_err(|error| error.to_string())?;
        let content = response
            .choices
            .first()
            .and_then(|choice| match &choice.message.content {
                Content::Text(text) => Some(text.as_str()),
                Content::Parts(parts) => parts.iter().find_map(|part| part.text.as_deref()),
            })
            .ok_or_else(|| "planner returned no decision".to_string())?;
        serde_json::from_str(content.trim())
            .map_err(|error| format!("invalid planner decision: {error}"))
    }
}

fn planner_prompt_json(input: &PlannerInput) -> Result<String, String> {
    let catalog = cached_planner_catalog_json(input)?;
    let goal = serde_json::to_string(&input.goal).map_err(|error| error.to_string())?;
    let workspace =
        serde_json::to_string(&input.workspace_summary).map_err(|error| error.to_string())?;
    let revision =
        serde_json::to_string(&input.catalog_revision).map_err(|error| error.to_string())?;
    let fingerprint =
        serde_json::to_string(&input.catalog_fingerprint).map_err(|error| error.to_string())?;
    let policy = serde_json::to_string(&input.policy).map_err(|error| error.to_string())?;
    let rejected = serde_json::to_string(&input.rejected_blueprint_fingerprints)
        .map_err(|error| error.to_string())?;
    Ok(format!(
        "{{\"goal\":{goal},\"workspace_summary\":{workspace},\"catalog_revision\":{revision},\"catalog_fingerprint\":{fingerprint},\"catalog\":{catalog},\"policy\":{policy},\"rejected_blueprint_fingerprints\":{rejected}}}"
    ))
}

fn cached_planner_catalog_json(input: &PlannerInput) -> Result<Arc<str>, String> {
    let cache = PLANNER_CATALOG_CACHE.get_or_init(|| {
        Mutex::new(PlannerCatalogCache::new(
            MAX_PLANNER_CATALOG_CACHE_ENTRIES,
            MAX_PLANNER_CATALOG_CACHE_BYTES,
        ))
    });
    {
        let mut cache = cache
            .lock()
            .map_err(|_| "Planner catalog cache is poisoned".to_string())?;
        if let Some(json) = cache.get(&input.catalog_fingerprint) {
            let hits = PLANNER_CATALOG_CACHE_HITS.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::debug!(
                hits,
                misses = PLANNER_CATALOG_CACHE_MISSES.load(Ordering::Relaxed),
                evictions = PLANNER_CATALOG_CACHE_EVICTIONS.load(Ordering::Relaxed),
                "Planner catalog cache hit"
            );
            return Ok(json);
        }
    }

    PLANNER_CATALOG_CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    let json: Arc<str> =
        Arc::from(serde_json::to_string(&input.catalog).map_err(|error| error.to_string())?);
    let cache_bytes = input.catalog_fingerprint.len().saturating_add(json.len());
    let evictions = cache
        .lock()
        .map_err(|_| "Planner catalog cache is poisoned".to_string())?
        .insert(input.catalog_fingerprint.clone(), json.clone(), cache_bytes);
    PLANNER_CATALOG_CACHE_EVICTIONS.fetch_add(evictions as u64, Ordering::Relaxed);
    tracing::debug!(
        hits = PLANNER_CATALOG_CACHE_HITS.load(Ordering::Relaxed),
        misses = PLANNER_CATALOG_CACHE_MISSES.load(Ordering::Relaxed),
        evictions = PLANNER_CATALOG_CACHE_EVICTIONS.load(Ordering::Relaxed),
        "Planner catalog cache miss"
    );
    Ok(json)
}

impl ModelEvaluatorBackend {
    pub fn new(
        provider: Arc<dyn Provider>,
        request_defaults: CompiledExecutionRequest,
        prompts: BTreeMap<EvaluatorId, String>,
    ) -> Self {
        Self {
            provider,
            request_defaults,
            prompts,
        }
    }
}

#[async_trait]
impl EvaluatorBackend for ModelEvaluatorBackend {
    async fn evaluate(
        &self,
        evaluator: &EvaluatorId,
        candidate: &NodeResult,
    ) -> Result<EvaluationOutcome, String> {
        let prompt = self
            .prompts
            .get(evaluator)
            .ok_or_else(|| format!("unknown evaluator '{}'", evaluator.as_str()))?;
        let response = self
            .provider
            .chat(self.request_defaults.to_chat_request_with_system(
                vec![Message::user(format!(
                    "{prompt}\n\nCandidate:\n{}",
                    candidate.output.as_deref().unwrap_or(&candidate.summary)
                ))],
                Vec::new(),
                Some(false),
                Some("Return exactly PASS, FAIL, or INDETERMINATE.".to_string()),
            ))
            .await
            .map_err(|error| error.to_string())?;
        let text = response
            .choices
            .first()
            .and_then(|choice| match &choice.message.content {
                Content::Text(text) => Some(text.as_str()),
                Content::Parts(parts) => parts.iter().find_map(|part| part.text.as_deref()),
            })
            .unwrap_or_default()
            .trim()
            .to_ascii_uppercase();
        let evaluation = match text.as_str() {
            "PASS" => Evaluation::Pass,
            "FAIL" => Evaluation::Fail,
            _ => Evaluation::Indeterminate,
        };
        let usage = response.usage.map_or_else(Default::default, |usage| {
            agendao_orchestrator::context::Usage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                cache_read_tokens: usage.cache_read_input_tokens.unwrap_or_default(),
                cache_miss_tokens: usage.cache_miss_input_tokens.unwrap_or_default(),
                cache_write_tokens: usage.cache_creation_input_tokens.unwrap_or_default(),
                ..Default::default()
            }
        });
        Ok(EvaluationOutcome { evaluation, usage })
    }
}
