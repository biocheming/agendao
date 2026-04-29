use async_trait::async_trait;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::OnceLock;

use rocode_agent::{AgentInfo, AgentMode, AgentRegistry};
use rocode_command::output_blocks::{MessageBlock, MessageRole as OutputMessageRole, OutputBlock};
use rocode_config::{Config as AppConfig, SkillTreeNodeConfig};
use rocode_execution_types::{CompiledExecutionRequest, ExecutionRequestContext};
use rocode_orchestrator::output_metadata::output_usage;
use rocode_orchestrator::{
    resolve_skill_markdown_repo, scheduler_orchestrator_from_plan, scheduler_plan_from_profile,
    scheduler_request_defaults_from_file, scheduler_request_defaults_from_plan,
    stage_policy_available_tools, stage_policy_from_label, AgentResolver, AvailableAgentMeta,
    AvailableCategoryMeta, ExecutionContext as OrchestratorExecutionContext,
    ModelRef as OrchestratorModelRef, ModelResolver, Orchestrator, OrchestratorContext,
    OrchestratorError, SchedulerConfig, SchedulerPresetKind, SchedulerProfileConfig,
    SchedulerRequestDefaults, SkillTreeNode, SkillTreeRequestPlan, SkillTreeTruncationStrategy,
    ToolExecError as OrchestratorToolExecError, ToolExecutor as OrchestratorToolExecutor,
    ToolOutput as OrchestratorToolOutput, ToolRunner,
};
use tokio_util::sync::CancellationToken;

use crate::request_options::{resolve_compiled_execution_request, ExecutionResolutionContext};
use crate::routes::skill_catalog::enrich_scheduler_plan_skills;
use crate::runtime_control::SessionRunStatus;
use crate::session_runtime::events::{
    broadcast_session_updated, emit_output_block_via_hook, server_output_block_hook,
};
use crate::session_runtime::{
    assistant_visible_text, ensure_default_session_title,
    finalize_active_scheduler_stage_cancelled, first_user_message_text,
    visible_assistant_text_from_orchestrator_output, ModelPricing, SessionSchedulerLifecycleHook,
};
use crate::{ApiError, Result, ServerState};
use rocode_session::prompt::{
    auto_compact_session_with_focus_if_needed, OutputBlockEvent, OutputBlockHook,
};
use rocode_session::{MessageRole, PartType as SessionPartType, SessionMessage};

use super::super::permission::request_permission;
use super::super::tui::request_question_answers;
use super::autoresearch_target::{
    AutoresearchProfileOverrideRecord, AUTORESEARCH_PROFILE_NAME,
    AUTORESEARCH_PROFILE_OVERRIDE_METADATA_KEY,
};
use super::cancel::is_scheduler_cancellation_error;
use super::messages::resolve_provider_and_model;
use super::prompt::{
    build_scheduler_session_context_packet, create_scheduler_user_message,
    merge_scheduler_prompt_with_memory, move_scheduler_final_answer_after_stage_messages,
    resolve_prompt_memory_context, SchedulerUserMessageContext,
    SCHEDULER_SESSION_CONTEXT_METADATA_KEY, SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY,
};
use super::session_crud::{resolved_session_directory, set_session_run_status};
use super::telemetry::persist_session_telemetry_metadata;

use super::cancel::abort_session_execution;

const BUILTIN_AUTORESEARCH_SCHEDULER_JSONC: &str =
    include_str!("../../../assets/autoresearch.scheduler.jsonc");
const SCHEDULER_CONTEXT_HYDRATE_TOOL: &str = "scheduler_context_hydrate";
const SCHEDULER_CONTEXT_PACKET_VERSION: u64 = 1;
const SCHEDULER_CONTEXT_HYDRATE_DEFAULT_MESSAGE_LIMIT: usize = 2_000;
const SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGE_LIMIT: usize = 8_000;
const SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGES: usize = 12;

fn to_orchestrator_skill_tree(node: &SkillTreeNodeConfig) -> SkillTreeNode {
    SkillTreeNode {
        node_id: node.node_id.clone(),
        markdown_path: node.markdown_path.clone(),
        children: node
            .children
            .iter()
            .map(to_orchestrator_skill_tree)
            .collect(),
    }
}

fn builtin_autoresearch_scheduler_config() -> Option<SchedulerConfig> {
    static CONFIG: OnceLock<Option<SchedulerConfig>> = OnceLock::new();

    CONFIG
        .get_or_init(|| {
            let mut config = match SchedulerConfig::load_from_str(
                BUILTIN_AUTORESEARCH_SCHEDULER_JSONC,
            ) {
                Ok(config) => config,
                Err(error) => {
                    tracing::warn!(%error, "failed to load built-in autoresearch scheduler config");
                    return None;
                }
            };
            let base_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
            if let Err(error) = config.resolve_agent_tree_paths(base_dir.as_path()) {
                tracing::warn!(%error, "failed to resolve built-in autoresearch agent trees");
                return None;
            }
            if let Err(error) = config.resolve_workflow_paths(base_dir.as_path()) {
                tracing::warn!(%error, "failed to resolve built-in autoresearch workflow paths");
                return None;
            }
            Some(config)
        })
        .clone()
}

fn resolve_bundled_scheduler_request_defaults(
    requested_profile: Option<&str>,
) -> Option<SchedulerRequestDefaults> {
    let profile_name = requested_profile
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let scheduler_config = builtin_autoresearch_scheduler_config()?;
    let profile = scheduler_config.profile(profile_name).ok()?;
    let plan = scheduler_plan_from_profile(Some(profile_name.to_string()), profile).ok()?;
    Some(scheduler_request_defaults_from_plan(&plan))
}

fn resolve_bundled_scheduler_profile_config(
    requested_profile: Option<&str>,
) -> Option<(String, SchedulerProfileConfig)> {
    let profile_name = requested_profile
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let scheduler_config = builtin_autoresearch_scheduler_config()?;
    let profile = scheduler_config.profile(profile_name).ok()?.clone();
    Some((profile_name.to_string(), profile))
}

fn resolve_builtin_scheduler_request_defaults(
    requested_profile: Option<&str>,
) -> Option<SchedulerRequestDefaults> {
    let profile_name = requested_profile
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let preset = SchedulerPresetKind::from_str(profile_name).ok()?;
    let profile = SchedulerProfileConfig {
        orchestrator: Some(preset.as_str().to_string()),
        ..Default::default()
    };
    let plan = scheduler_plan_from_profile(Some(profile_name.to_string()), &profile).ok()?;
    Some(scheduler_request_defaults_from_plan(&plan))
}

fn normalized_requested_scheduler_profile<'a>(
    requested_profile: Option<&'a str>,
) -> Option<&'a str> {
    requested_profile
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn resolve_scheduler_request_defaults(
    config: &AppConfig,
    requested_profile: Option<&str>,
) -> Option<SchedulerRequestDefaults> {
    if let Some(defaults) = resolve_builtin_scheduler_request_defaults(requested_profile) {
        return Some(defaults);
    }
    let scheduler_path = config
        .scheduler_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(profile_name) = requested_profile
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(scheduler_path) = scheduler_path {
            let scheduler_config = match SchedulerConfig::load_from_file(scheduler_path) {
                Ok(config) => config,
                Err(error) => {
                    tracing::warn!(path = %scheduler_path, %error, "failed to load scheduler config");
                    return None;
                }
            };
            let profile = match scheduler_config.profile(profile_name) {
                Ok(profile) => profile,
                Err(error) => {
                    tracing::warn!(path = %scheduler_path, profile = %profile_name, %error, "failed to resolve requested scheduler profile");
                    return None;
                }
            };
            let plan = match scheduler_plan_from_profile(Some(profile_name.to_string()), profile) {
                Ok(plan) => plan,
                Err(error) => {
                    tracing::warn!(path = %scheduler_path, profile = %profile_name, %error, "failed to build requested scheduler profile plan");
                    return None;
                }
            };
            return Some(scheduler_request_defaults_from_plan(&plan));
        }

        return resolve_bundled_scheduler_request_defaults(Some(profile_name));
    }

    let scheduler_path = scheduler_path?;

    match scheduler_request_defaults_from_file(scheduler_path) {
        Ok(defaults) => Some(defaults),
        Err(error) => {
            tracing::warn!(path = %scheduler_path, %error, "failed to load scheduler request defaults");
            None
        }
    }
}

pub(crate) fn resolve_scheduler_request_defaults_validated(
    config: &AppConfig,
    requested_profile: Option<&str>,
) -> Result<Option<SchedulerRequestDefaults>> {
    let Some(profile_name) = normalized_requested_scheduler_profile(requested_profile) else {
        return Ok(resolve_scheduler_request_defaults(config, None));
    };

    if let Some(defaults) = resolve_builtin_scheduler_request_defaults(Some(profile_name)) {
        return Ok(Some(defaults));
    }

    let scheduler_path = config
        .scheduler_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(scheduler_path) = scheduler_path {
        let scheduler_config = SchedulerConfig::load_from_file(scheduler_path).map_err(|error| {
            tracing::warn!(
                path = %scheduler_path,
                profile = %profile_name,
                %error,
                "failed to load scheduler config for requested scheduler profile"
            );
            ApiError::BadRequest(format!(
                "Scheduler profile could not be resolved: `{}`. Failed to load scheduler config: {}",
                profile_name, error
            ))
        })?;

        let profile = scheduler_config.profile(profile_name).map_err(|error| {
            tracing::warn!(
                path = %scheduler_path,
                profile = %profile_name,
                %error,
                "failed to resolve requested scheduler profile"
            );
            ApiError::BadRequest(format!(
                "Scheduler profile could not be resolved: `{}`. {}",
                profile_name, error
            ))
        })?;

        let plan =
            scheduler_plan_from_profile(Some(profile_name.to_string()), profile).map_err(|error| {
                tracing::warn!(
                    path = %scheduler_path,
                    profile = %profile_name,
                    %error,
                    "failed to build requested scheduler profile plan"
                );
                ApiError::BadRequest(format!(
                    "Scheduler profile could not be resolved: `{}`. Failed to build profile plan: {}",
                    profile_name, error
                ))
            })?;

        return Ok(Some(scheduler_request_defaults_from_plan(&plan)));
    }

    if let Some(defaults) = resolve_bundled_scheduler_request_defaults(Some(profile_name)) {
        return Ok(Some(defaults));
    }

    Err(ApiError::BadRequest(format!(
        "Scheduler profile could not be resolved: `{}`. No scheduler config is configured.",
        profile_name
    )))
}

pub(super) fn scheduler_system_prompt_preview(
    profile_name: &str,
    profile: &SchedulerProfileConfig,
) -> String {
    let orchestrator = profile.orchestrator.as_deref().unwrap_or(profile_name);
    SchedulerPresetKind::from_str(orchestrator)
        .ok()
        .map(|preset| preset.definition().system_prompt_preview().to_string())
        .unwrap_or_else(|| {
            format!(
                "You are the `{profile_name}` scheduler profile.
Bias: follow its configured stages and orchestration contract faithfully.
Boundary: preserve the profile's execution constraints and role semantics."
            )
        })
}

pub(super) fn scheduler_mode_kind(profile_name: &str) -> &'static str {
    if SchedulerPresetKind::from_str(profile_name).is_ok() {
        "preset"
    } else {
        "profile"
    }
}

pub(crate) struct PromptRequestConfigInput<'a> {
    pub state: &'a Arc<ServerState>,
    pub config: &'a AppConfig,
    pub session_id: &'a str,
    pub requested_agent: Option<&'a str>,
    pub requested_scheduler_profile: Option<&'a str>,
    pub scheduler_profile_override: Option<(String, SchedulerProfileConfig)>,
    pub request_model: Option<&'a str>,
    pub request_variant: Option<&'a str>,
    pub route: &'static str,
}

fn scheduler_request_defaults_from_override(
    profile_name: &str,
    profile: &SchedulerProfileConfig,
) -> Option<SchedulerRequestDefaults> {
    let plan = scheduler_plan_from_profile(Some(profile_name.to_string()), profile).ok()?;
    Some(scheduler_request_defaults_from_plan(&plan))
}

async fn resolve_session_scheduler_profile_override(
    state: &Arc<ServerState>,
    session_id: &str,
    requested_scheduler_profile: Option<&str>,
) -> Option<(String, SchedulerProfileConfig)> {
    if requested_scheduler_profile != Some(AUTORESEARCH_PROFILE_NAME) {
        return None;
    }

    let sessions = state.sessions.lock().await;
    let session = sessions.get(session_id)?;
    let metadata = session
        .record()
        .metadata
        .get(AUTORESEARCH_PROFILE_OVERRIDE_METADATA_KEY)?
        .clone();
    let record = serde_json::from_value::<AutoresearchProfileOverrideRecord>(metadata).ok()?;
    Some((record.profile_name, record.profile))
}

pub(super) fn resolve_scheduler_profile_config(
    config: &AppConfig,
    requested_profile: Option<&str>,
) -> Option<(String, SchedulerProfileConfig)> {
    let profile_name = requested_profile
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    if let Ok(preset) = SchedulerPresetKind::from_str(profile_name) {
        return Some((
            profile_name.to_string(),
            SchedulerProfileConfig {
                orchestrator: Some(preset.as_str().to_string()),
                ..Default::default()
            },
        ));
    }

    let scheduler_path = config
        .scheduler_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(scheduler_path) = scheduler_path {
        let scheduler_config = match SchedulerConfig::load_from_file(scheduler_path) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!(path = %scheduler_path, %error, "failed to load scheduler profile config");
                return None;
            }
        };
        let profile = match scheduler_config.profile(profile_name) {
            Ok(profile) => profile.clone(),
            Err(error) => {
                tracing::warn!(path = %scheduler_path, profile = %profile_name, %error, "failed to resolve scheduler profile config");
                return None;
            }
        };
        return Some((profile_name.to_string(), profile));
    }

    resolve_bundled_scheduler_profile_config(Some(profile_name))
}

#[derive(Clone)]
pub(super) struct SchedulerAgentResolver {
    pub(super) registry: Arc<AgentRegistry>,
}

impl AgentResolver for SchedulerAgentResolver {
    fn resolve(&self, name: &str) -> Option<rocode_orchestrator::AgentDescriptor> {
        self.registry
            .get(name)
            .map(to_orchestrator_agent_descriptor)
    }
}

fn to_orchestrator_agent_descriptor(info: &AgentInfo) -> rocode_orchestrator::AgentDescriptor {
    rocode_orchestrator::AgentDescriptor {
        name: info.name.clone(),
        system_prompt: info.system_prompt.clone(),
        model: info
            .model
            .as_ref()
            .map(|model| rocode_orchestrator::ModelRef {
                provider_id: model.provider_id.clone(),
                model_id: model.model_id.clone(),
            }),
        max_steps: info.max_steps,
        temperature: info.temperature,
        allowed_tools: info.allowed_tools.clone(),
    }
}

pub(crate) fn to_task_agent_info(info: &AgentInfo) -> rocode_tool::TaskAgentInfo {
    rocode_tool::TaskAgentInfo {
        name: info.name.clone(),
        model: info.model.as_ref().map(|m| rocode_tool::TaskAgentModel {
            provider_id: m.provider_id.clone(),
            model_id: m.model_id.clone(),
        }),
        can_use_task: info.is_tool_allowed("task"),
        steps: info.max_steps,
        execution: Some(ExecutionRequestContext {
            provider_id: info.model.as_ref().map(|m| m.provider_id.clone()),
            model_id: info.model.as_ref().map(|m| m.model_id.clone()),
            max_tokens: info.max_tokens,
            temperature: info.temperature,
            top_p: info.top_p,
            variant: info.variant.clone(),
            provider_options: (!info.options.is_empty()).then_some(info.options.clone()),
        }),
        max_tokens: info.max_tokens,
        temperature: info.temperature,
        top_p: info.top_p,
        variant: info.variant.clone(),
    }
}

#[derive(Clone)]
pub(super) struct SessionSchedulerModelResolver {
    pub(super) state: Arc<ServerState>,
    pub(super) fallback_provider_id: String,
    pub(super) fallback_model_id: String,
    pub(super) fallback_request: CompiledExecutionRequest,
}

#[async_trait]
impl ModelResolver for SessionSchedulerModelResolver {
    async fn chat_stream(
        &self,
        model: Option<&OrchestratorModelRef>,
        messages: Vec<rocode_provider::Message>,
        tools: Vec<rocode_provider::ToolDefinition>,
        exec_ctx: &OrchestratorExecutionContext,
    ) -> std::result::Result<rocode_provider::StreamResult, OrchestratorError> {
        let (provider_id, model_id) = model
            .map(|model| (model.provider_id.clone(), model.model_id.clone()))
            .unwrap_or_else(|| {
                (
                    self.fallback_provider_id.clone(),
                    self.fallback_model_id.clone(),
                )
            });

        let provider = {
            let providers = self.state.providers.read().await;
            providers
                .get_provider(&provider_id)
                .map_err(|error| OrchestratorError::ModelError(error.to_string()))?
        };

        let mut request = self
            .fallback_request
            .with_model(model_id)
            .to_chat_request(messages, tools, true);
        if exec_ctx
            .metadata
            .get("workflow_verifier_use_logprobs")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            request.provider_options = Some(merge_verifier_logprob_options(
                request.provider_options.take(),
                exec_ctx
                    .metadata
                    .get("workflow_verifier_top_logprobs")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|value| u8::try_from(value).ok())
                    .unwrap_or(20),
            ));
        }
        provider
            .chat_stream(request)
            .await
            .map_err(|error| OrchestratorError::ModelError(error.to_string()))
    }
}

fn merge_verifier_logprob_options(
    provider_options: Option<std::collections::HashMap<String, serde_json::Value>>,
    top_logprobs: u8,
) -> std::collections::HashMap<String, serde_json::Value> {
    let mut options = provider_options.unwrap_or_default();
    let top_logprobs = top_logprobs.clamp(1, 20);

    options.insert("logprobs".to_string(), serde_json::json!(top_logprobs));
    for key in ["openai", "responses"] {
        let mut nested = options
            .get(key)
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        nested.insert("logprobs".to_string(), serde_json::json!(top_logprobs));
        options.insert(key.to_string(), serde_json::Value::Object(nested));
    }

    options
}

#[derive(Clone)]
pub(super) struct SessionSchedulerToolExecutor {
    pub(super) state: Arc<ServerState>,
    pub(super) session_id: String,
    pub(super) message_id: String,
    pub(super) directory: String,
    pub(super) abort_token: CancellationToken,
    pub(super) current_model: Option<String>,
    pub(super) tool_runtime_config: rocode_tool::ToolRuntimeConfig,
    pub(super) agent_registry: Arc<AgentRegistry>,
}

#[derive(Clone)]
pub(super) struct SchedulerRunCancelToken {
    pub(super) token: CancellationToken,
}

impl rocode_orchestrator::runtime::events::CancelToken for SchedulerRunCancelToken {
    fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

impl SessionSchedulerToolExecutor {
    async fn build_tool_context(
        &self,
        exec_ctx: &OrchestratorExecutionContext,
    ) -> rocode_tool::ToolContext {
        let mut base_ctx = rocode_tool::ToolContext::new(
            self.session_id.clone(),
            self.message_id.clone(),
            self.directory.clone(),
        )
        .with_agent(exec_ctx.agent_name.clone())
        .with_abort(self.abort_token.clone())
        .with_config_store(self.state.config_store.clone())
        .with_tool_runtime_config(self.tool_runtime_config.clone())
        .with_registry(self.state.tool_registry.clone())
        .with_get_last_model({
            let current_model = self.current_model.clone();
            move |_session_id| {
                let current_model = current_model.clone();
                async move { Ok(current_model.clone()) }
            }
        })
        .with_get_agent_info({
            let agent_registry = self.agent_registry.clone();
            move |name| {
                let agent_registry = agent_registry.clone();
                async move { Ok(agent_registry.get(&name).map(to_task_agent_info)) }
            }
        })
        .with_ask_question({
            let state = self.state.clone();
            let session_id = self.session_id.clone();
            move |questions| {
                let state = state.clone();
                let session_id = session_id.clone();
                async move { request_question_answers(state, session_id, questions).await }
            }
        })
        .with_ask({
            let state = self.state.clone();
            let session_id = self.session_id.clone();
            move |request| {
                let state = state.clone();
                let session_id = session_id.clone();
                async move { request_permission(state, session_id, request).await }
            }
        })
        .with_resolve_category({
            let category_registry = self.state.category_registry.clone();
            move |category| {
                let registry = category_registry.clone();
                async move {
                    Ok(registry
                        .resolve(&category)
                        .map(|def| rocode_tool::TaskCategoryInfo {
                            name: category,
                            description: def.description.clone(),
                            model: def.model.as_ref().map(|m| rocode_tool::TaskAgentModel {
                                provider_id: m.provider_id.clone(),
                                model_id: m.model_id.clone(),
                            }),
                            prompt_suffix: def.prompt_suffix.clone(),
                            variant: def.variant.clone(),
                        }))
                }
            }
        })
        .with_create_subsession(|agent, _title, _model, _disabled_tools| async move {
            Ok(format!("scheduler_task_{}_{}", agent, uuid::Uuid::new_v4()))
        })
        .with_prompt_subsession(|_session_id, _prompt| async move {
            Err(rocode_tool::ToolError::ExecutionError(
                "The scheduler execution environment does not support subagent sessions (task/task_flow). \
                 Use 'agent' execution mode instead of 'scheduler' for workflows that require subagents."
                    .to_string(),
            ))
        });
        base_ctx.call_id = exec_ctx
            .metadata
            .get("call_id")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        base_ctx.extra = exec_ctx.metadata.clone();
        let inventory = self.state.tool_registry.list_ids().await;
        let available_tools = base_ctx
            .extra
            .get("scheduler_stage_tool_policy")
            .and_then(|value| value.as_str())
            .and_then(stage_policy_from_label)
            .map(|policy| stage_policy_available_tools(policy, &inventory))
            .unwrap_or_else(|| {
                inventory
                    .iter()
                    .map(|tool| tool.to_ascii_lowercase())
                    .collect()
            });
        let mut available_tool_ids = available_tools.into_iter().collect::<Vec<_>>();
        available_tool_ids.sort();
        let mut available_toolsets =
            rocode_skill::infer_toolsets_from_tools(available_tool_ids.iter().map(String::as_str))
                .into_iter()
                .collect::<Vec<_>>();
        available_toolsets.sort();
        base_ctx.extra.insert(
            "available_tool_ids".to_string(),
            serde_json::json!(available_tool_ids),
        );
        base_ctx.extra.insert(
            "available_toolsets".to_string(),
            serde_json::json!(available_toolsets),
        );
        Self::with_agent_task_publish_bus(base_ctx, self.state.clone())
    }

    /// Wire `publish_bus` to route `agent_task.*` events to
    /// [`RuntimeControlRegistry`] so spawned agent tasks appear in the
    /// execution topology with correct parent links.
    fn with_agent_task_publish_bus(
        ctx: rocode_tool::ToolContext,
        state: Arc<ServerState>,
    ) -> rocode_tool::ToolContext {
        let session_id = ctx.session_id.clone();
        ctx.with_publish_bus(move |event_type, properties| {
            let state = state.clone();
            let session_id = session_id.clone();
            async move {
                match event_type.as_str() {
                    "agent_task.registered" => {
                        let task_id = properties["task_id"].as_str().unwrap_or_default();
                        let agent_name = properties["agent_name"].as_str().unwrap_or_default();
                        let parent_tool_call_id = properties["parent_tool_call_id"].as_str().map(
                            crate::runtime_control::RuntimeControlRegistry::tool_call_execution_id,
                        );
                        // Resolve stage_id from the parent execution's record.
                        let stage_id = if let Some(ref pid) = parent_tool_call_id {
                            state.runtime_telemetry.resolve_stage_id(pid).await
                        } else {
                            None
                        };
                        state
                            .runtime_telemetry
                            .register_agent_task(
                                task_id,
                                &session_id,
                                agent_name,
                                parent_tool_call_id,
                                stage_id.clone(),
                            )
                            .await;
                        // Update agent counts on the stage message.
                        if let Some(ref sid) = stage_id {
                            update_stage_agent_counts(&state, &session_id, sid).await;
                        }
                    }
                    "agent_task.completed" => {
                        let task_id = properties["task_id"].as_str().unwrap_or_default();
                        // Resolve stage_id before finishing (record still exists).
                        let exec_id =
                            crate::runtime_control::RuntimeControlRegistry::agent_task_execution_id(
                                task_id,
                            );
                        let stage_id = state.runtime_telemetry.resolve_stage_id(&exec_id).await;
                        state.runtime_telemetry.finish_agent_task(task_id).await;
                        // Update agent counts on the stage message.
                        if let Some(ref sid) = stage_id {
                            update_stage_agent_counts(&state, &session_id, sid).await;
                        }
                    }
                    _ => {}
                }
            }
        })
    }

    async fn hydrate_scheduler_context(
        &self,
        arguments: serde_json::Value,
        exec_ctx: &OrchestratorExecutionContext,
    ) -> std::result::Result<OrchestratorToolOutput, OrchestratorToolExecError> {
        let requested_ids = scheduler_context_hydrate_message_ids(&arguments)?;
        let allowed_ids = scheduler_context_allowed_message_ids(exec_ctx);
        if allowed_ids.is_empty() {
            return Err(OrchestratorToolExecError::InvalidArguments(
                "scheduler continuity packet is unavailable; no hydration anchors are authorized"
                    .to_string(),
            ));
        }
        let per_message_limit = scheduler_context_hydrate_message_limit(&arguments);
        let session = {
            let sessions = self.state.sessions.lock().await;
            sessions.get(&self.session_id).cloned()
        }
        .ok_or_else(|| {
            OrchestratorToolExecError::ExecutionError("session is no longer available".to_string())
        })?;

        let mut hydrated = Vec::new();
        let mut hydrated_ids = Vec::new();
        let mut rejected = Vec::new();
        let mut missing = Vec::new();
        for message_id in requested_ids {
            if !allowed_ids.contains(&message_id) {
                rejected.push(message_id);
                continue;
            }
            let Some(message) = session.get_message(&message_id) else {
                missing.push(message_id);
                continue;
            };
            if let Some(rendered) =
                render_scheduler_context_hydrated_message(message, per_message_limit)
            {
                hydrated.push(rendered);
                hydrated_ids.push(message_id);
            } else {
                missing.push(message_id);
            }
        }

        let mut sections = vec![
            "## Scheduler Context Hydration\nHydrated exact same-session sources authorized by the scheduler continuity packet."
                .to_string(),
        ];
        if !hydrated.is_empty() {
            sections.push(format!("## Hydrated Messages\n{}", hydrated.join("\n")));
        }
        if !rejected.is_empty() {
            sections.push(format!(
                "## Rejected Message IDs\n{}",
                rejected
                    .iter()
                    .map(|id| format!("- `{id}`: not present in scheduler continuity anchors"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if !missing.is_empty() {
            sections.push(format!(
                "## Missing Message IDs\n{}",
                missing
                    .iter()
                    .map(|id| format!("- `{id}`: not found or no hydratable text"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        Ok(OrchestratorToolOutput {
            output: sections.join("\n\n"),
            is_error: false,
            title: Some("Scheduler context hydrated".to_string()),
            metadata: Some(serde_json::json!({
                "hydrated_count": hydrated.len(),
                "rejected_count": rejected.len(),
                "missing_count": missing.len(),
                "hydrated_message_ids": hydrated_ids,
                "rejected_message_ids": rejected,
                "missing_message_ids": missing,
                "max_chars_per_message": per_message_limit,
            })),
        })
    }
}

fn scheduler_context_hydrate_message_ids(
    arguments: &serde_json::Value,
) -> std::result::Result<Vec<String>, OrchestratorToolExecError> {
    let Some(values) = arguments
        .get("message_ids")
        .and_then(|value| value.as_array())
    else {
        return Err(OrchestratorToolExecError::InvalidArguments(
            "message_ids must be an array of scheduler continuity message ids".to_string(),
        ));
    };
    if values.is_empty() {
        return Err(OrchestratorToolExecError::InvalidArguments(
            "message_ids must not be empty".to_string(),
        ));
    }
    if values.len() > SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGES {
        return Err(OrchestratorToolExecError::InvalidArguments(format!(
            "message_ids must contain at most {SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGES} ids"
        )));
    }
    let mut ids = Vec::new();
    for value in values {
        let Some(id) = value.as_str().map(str::trim).filter(|id| !id.is_empty()) else {
            return Err(OrchestratorToolExecError::InvalidArguments(
                "message_ids must only contain non-empty strings".to_string(),
            ));
        };
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

fn scheduler_context_hydrate_message_limit(arguments: &serde_json::Value) -> usize {
    arguments
        .get("max_chars_per_message")
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
        .unwrap_or(SCHEDULER_CONTEXT_HYDRATE_DEFAULT_MESSAGE_LIMIT)
        .clamp(1, SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGE_LIMIT)
}

fn scheduler_context_allowed_message_ids(exec_ctx: &OrchestratorExecutionContext) -> Vec<String> {
    let Some(packet) = exec_ctx
        .metadata
        .get(SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY)
    else {
        return Vec::new();
    };
    if packet.get("version").and_then(|value| value.as_u64())
        != Some(SCHEDULER_CONTEXT_PACKET_VERSION)
    {
        return Vec::new();
    }
    let mut ids = packet
        .get("exact_recent_tail")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|anchor| anchor.get("message_id").and_then(|value| value.as_str()))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if let Some(id) = packet
        .get("latest_compaction_summary")
        .and_then(|value| value.get("message_id"))
        .and_then(|value| value.as_str())
    {
        ids.push(id.to_string());
    }
    ids.sort();
    ids.dedup();
    ids
}

fn render_scheduler_context_hydrated_message(
    message: &SessionMessage,
    per_message_limit: usize,
) -> Option<String> {
    let text = scheduler_context_hydratable_text(message)?;
    let text = truncate_scheduler_context_hydration(&text, per_message_limit);
    Some(format!(
        "- {} `{}`:\n{}",
        scheduler_context_role_label(&message.role),
        message.id,
        indent_scheduler_context_hydration(&text)
    ))
}

fn scheduler_context_hydratable_text(message: &SessionMessage) -> Option<String> {
    let mut parts = Vec::new();
    let text = message.get_text();
    let text = text.trim();
    if !text.is_empty() {
        parts.push(text.to_string());
    }
    for part in &message.parts {
        if let SessionPartType::Compaction { summary } = &part.part_type {
            let summary = summary.trim();
            if !summary.is_empty() {
                parts.push(format!("[compaction summary]\n{summary}"));
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

fn scheduler_context_role_label(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
        MessageRole::Tool => "tool",
    }
}

fn indent_scheduler_context_hydration(text: &str) -> String {
    text.lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_scheduler_context_hydration(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut truncated = value
        .chars()
        .take(limit.saturating_sub(24))
        .collect::<String>();
    truncated.push_str("\n...[truncated]...");
    truncated
}

fn scheduler_context_hydrate_tool_definition() -> rocode_provider::ToolDefinition {
    rocode_provider::ToolDefinition {
        name: SCHEDULER_CONTEXT_HYDRATE_TOOL.to_string(),
        description: Some(
            "Hydrate exact same-session messages identified by Scheduler Continuity Source Anchors. Use only when the current task needs prior context that is truncated, summarized, or ambiguous."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "required": ["message_ids"],
            "properties": {
                "message_ids": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGES,
                    "items": {"type": "string"},
                    "description": "Message ids from the Scheduler Continuity Source Anchors."
                },
                "max_chars_per_message": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGE_LIMIT,
                    "description": "Maximum characters to return per hydrated message."
                }
            },
            "additionalProperties": false
        }),
    }
}

/// Update `scheduler_stage_done_agent_count` and `scheduler_stage_total_agent_count`
/// in the stage's session message metadata so all three frontends can display agent progress.
async fn update_stage_agent_counts(
    state: &crate::server::ServerState,
    session_id: &str,
    stage_id: &str,
) {
    let (done, total) = state.runtime_telemetry.count_stage_agents(stage_id).await;
    let mut sessions = state.sessions.lock().await;
    let Some(mut session) = sessions.get(session_id).cloned() else {
        return;
    };
    // The stage_id is also used as the message_id for the stage message.
    let mut message_snapshot = None;
    if let Some(message) = session.get_message_mut(stage_id) {
        message.metadata.insert(
            "scheduler_stage_done_agent_count".to_string(),
            serde_json::json!(done),
        );
        message.metadata.insert(
            "scheduler_stage_total_agent_count".to_string(),
            serde_json::json!(total),
        );
        message_snapshot = Some(message.clone());
    }
    session.touch();
    sessions.update(session);
    drop(sessions);

    if let Some(message) = message_snapshot.as_ref() {
        let _ = state
            .runtime_telemetry
            .refresh_stage_summary_from_message(session_id, message)
            .await;
    }
}

#[async_trait]
impl OrchestratorToolExecutor for SessionSchedulerToolExecutor {
    async fn execute(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
        exec_ctx: &OrchestratorExecutionContext,
    ) -> std::result::Result<OrchestratorToolOutput, OrchestratorToolExecError> {
        if tool_name == SCHEDULER_CONTEXT_HYDRATE_TOOL {
            return self.hydrate_scheduler_context(arguments, exec_ctx).await;
        }
        let ctx = self.build_tool_context(exec_ctx).await;
        let result = self
            .state
            .tool_registry
            .execute(tool_name, arguments, ctx)
            .await
            .map_err(|error| match error {
                rocode_tool::ToolError::InvalidArguments(message) => {
                    OrchestratorToolExecError::InvalidArguments(message)
                }
                rocode_tool::ToolError::PermissionDenied(message) => {
                    OrchestratorToolExecError::PermissionDenied(message)
                }
                rocode_tool::ToolError::Cancelled => {
                    OrchestratorToolExecError::ExecutionError("cancelled".to_string())
                }
                other => OrchestratorToolExecError::ExecutionError(other.to_string()),
            })?;
        Ok(OrchestratorToolOutput {
            output: result.output,
            is_error: false,
            title: if result.title.is_empty() {
                None
            } else {
                Some(result.title)
            },
            metadata: if result.metadata.is_empty() {
                None
            } else {
                Some(serde_json::to_value(result.metadata).unwrap_or(serde_json::Value::Null))
            },
        })
    }

    async fn list_ids(&self) -> Vec<String> {
        let mut ids = self.state.tool_registry.list_ids().await;
        if !ids.iter().any(|id| id == SCHEDULER_CONTEXT_HYDRATE_TOOL) {
            ids.push(SCHEDULER_CONTEXT_HYDRATE_TOOL.to_string());
        }
        ids
    }

    async fn list_definitions(
        &self,
        _exec_ctx: &OrchestratorExecutionContext,
    ) -> Vec<rocode_provider::ToolDefinition> {
        let mut tools: Vec<rocode_provider::ToolDefinition> = self
            .state
            .tool_registry
            .list_schemas()
            .await
            .into_iter()
            .map(|schema| rocode_provider::ToolDefinition {
                name: schema.name,
                description: Some(schema.description),
                parameters: schema.parameters,
            })
            .collect();
        tools.push(scheduler_context_hydrate_tool_definition());
        rocode_session::prioritize_tool_definitions(&mut tools);
        tools
    }
}

pub(crate) fn resolve_config_default_agent_name(config: &AppConfig) -> String {
    config
        .default_agent
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("build")
        .to_string()
}

pub(crate) fn resolve_request_skill_tree_plan(
    config: &AppConfig,
    scheduler_defaults: Option<&SchedulerRequestDefaults>,
) -> Option<SkillTreeRequestPlan> {
    if let Some(plan) = scheduler_defaults.and_then(|defaults| defaults.skill_tree_plan.clone()) {
        return Some(plan);
    }

    let skill_tree = config.composition.as_ref()?.skill_tree.as_ref()?;
    if matches!(skill_tree.enabled, Some(false)) {
        return None;
    }

    let root = skill_tree.root.as_ref()?;
    let root = to_orchestrator_skill_tree(root);
    let markdown_repo = resolve_skill_markdown_repo(&config.skill_paths);
    let truncation_strategy = skill_tree
        .truncation_strategy
        .as_deref()
        .and_then(SkillTreeTruncationStrategy::from_label);
    if skill_tree.truncation_strategy.is_some() && truncation_strategy.is_none() {
        tracing::warn!(
            strategy = skill_tree
                .truncation_strategy
                .as_deref()
                .unwrap_or_default(),
            "unknown skill tree truncation strategy; using default head-tail"
        );
    }

    match SkillTreeRequestPlan::from_tree_with_options(
        &root,
        &markdown_repo,
        skill_tree.separator.as_deref(),
        skill_tree.token_budget,
        truncation_strategy,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            tracing::warn!(%error, "failed to build request skill tree plan");
            None
        }
    }
}

pub(crate) struct ResolvedPromptRequestConfig {
    pub scheduler_applied: bool,
    pub scheduler_profile_name: Option<String>,
    pub scheduler_profile_config: Option<SchedulerProfileConfig>,
    pub scheduler_root_agent: Option<String>,
    pub scheduler_skill_tree_applied: bool,
    pub request_skill_tree_plan: Option<SkillTreeRequestPlan>,
    pub resolved_agent: Option<AgentInfo>,
    pub provider: Arc<dyn rocode_provider::Provider>,
    pub provider_id: String,
    pub model_id: String,
    pub agent_system_prompt: Option<String>,
    pub compiled_request: CompiledExecutionRequest,
}

pub(super) fn resolve_request_model_inputs(
    scheduler_applied: bool,
    agent_model: Option<&str>,
    scheduler_profile: Option<&SchedulerProfileConfig>,
    request_model: Option<&str>,
    config_model: Option<&str>,
) -> (Option<String>, Option<String>, Option<String>) {
    if scheduler_applied {
        if let Some(agent_model) = agent_model {
            return (None, Some(agent_model.to_string()), None);
        }

        if let Some(model) = scheduler_profile.and_then(|profile| profile.model.as_ref()) {
            return (
                None,
                Some(model.model_id.clone()),
                Some(model.provider_id.clone()),
            );
        }

        return (
            request_model.map(str::to_string),
            config_model.map(str::to_string),
            None,
        );
    }

    (
        request_model.map(str::to_string),
        agent_model.or(config_model).map(str::to_string),
        None,
    )
}

fn build_execution_resolution_context(
    session_id: &str,
    provider_id: &str,
    model_id: &str,
    request_variant: Option<&str>,
    resolved_agent: Option<&AgentInfo>,
) -> ExecutionResolutionContext {
    ExecutionResolutionContext {
        session_id: session_id.to_string(),
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
        max_tokens: resolved_agent.and_then(|agent| agent.max_tokens),
        temperature: resolved_agent.and_then(|agent| agent.temperature),
        top_p: resolved_agent.and_then(|agent| agent.top_p),
        variant: request_variant
            .map(str::to_string)
            .or_else(|| resolved_agent.and_then(|agent| agent.variant.clone())),
    }
}

pub(crate) async fn resolve_prompt_request_config(
    input: PromptRequestConfigInput<'_>,
) -> Result<ResolvedPromptRequestConfig> {
    let PromptRequestConfigInput {
        state,
        config,
        session_id,
        requested_agent,
        requested_scheduler_profile,
        scheduler_profile_override,
        request_model,
        request_variant,
        route,
    } = input;

    let scheduler_profile_override = if let Some(profile_override) = scheduler_profile_override {
        Some(profile_override)
    } else {
        resolve_session_scheduler_profile_override(state, session_id, requested_scheduler_profile)
            .await
    };
    let scheduler_defaults =
        if let Some((profile_name, profile)) = scheduler_profile_override.as_ref() {
            scheduler_request_defaults_from_override(profile_name, profile)
        } else {
            resolve_scheduler_request_defaults_validated(config, requested_scheduler_profile)?
        };
    let scheduler_applied = scheduler_defaults.is_some();
    let scheduler_profile_name = scheduler_profile_override
        .as_ref()
        .map(|(profile_name, _)| profile_name.clone())
        .or_else(|| {
            scheduler_defaults
                .as_ref()
                .and_then(|defaults| defaults.profile_name.clone())
        });
    let scheduler_root_agent = scheduler_defaults
        .as_ref()
        .and_then(|defaults| defaults.root_agent_name.clone());
    let scheduler_skill_tree_applied = scheduler_defaults
        .as_ref()
        .and_then(|defaults| defaults.skill_tree_plan.as_ref())
        .is_some();
    let scheduler_agent_name = if requested_agent.is_none() {
        scheduler_root_agent.clone()
    } else {
        None
    };
    let fallback_agent_name =
        if requested_agent.is_none() && scheduler_agent_name.is_none() && !scheduler_applied {
            Some(resolve_config_default_agent_name(config))
        } else {
            None
        };

    let agent_registry = AgentRegistry::from_config(config);
    let selected_agent_name = requested_agent
        .or(scheduler_agent_name.as_deref())
        .or(fallback_agent_name.as_deref());
    let resolved_agent = selected_agent_name.and_then(|name| agent_registry.get(name).cloned());
    if requested_agent.is_some() && resolved_agent.is_none() {
        tracing::warn!(
            route,
            requested_agent = ?requested_agent,
            scheduler_agent = ?scheduler_agent_name,
            fallback_agent = ?fallback_agent_name,
            "requested agent not found in registry; proceeding without agent-specific overrides"
        );
    } else if scheduler_agent_name.is_some() && resolved_agent.is_none() {
        tracing::warn!(
            route,
            scheduler_agent = ?scheduler_agent_name,
            "scheduler root agent not found in registry; proceeding without agent-specific overrides"
        );
    }

    let scheduler_profile_config = scheduler_profile_override
        .as_ref()
        .map(|(_, profile)| profile.clone())
        .or_else(|| {
            scheduler_profile_name
                .as_deref()
                .and_then(|profile_name| {
                    resolve_scheduler_profile_config(config, Some(profile_name))
                })
                .map(|(_, profile)| profile)
        });
    let scheduler_profile_model = scheduler_profile_config
        .as_ref()
        .and_then(|profile| profile.model.as_ref())
        .map(|model| format!("{}/{}", model.provider_id, model.model_id));
    let agent_model = resolved_agent
        .as_ref()
        .and_then(|agent| agent.model.as_ref())
        .map(|model| format!("{}/{}", model.provider_id, model.model_id));
    let (request_model_input, config_model_input, config_provider_input) =
        resolve_request_model_inputs(
            scheduler_applied,
            agent_model.as_deref(),
            scheduler_profile_config.as_ref(),
            request_model,
            config.model.as_deref(),
        );
    let (provider, provider_id, model_id) = resolve_provider_and_model(
        state,
        request_model_input.as_deref(),
        config_model_input.as_deref(),
        config_provider_input.as_deref(),
    )
    .await?;

    let request_skill_tree_plan =
        resolve_request_skill_tree_plan(config, scheduler_defaults.as_ref());
    let mut agent_system_prompt = resolved_agent
        .as_ref()
        .and_then(|agent| agent.resolved_system_prompt());
    if let Some(plan) = request_skill_tree_plan.as_ref() {
        agent_system_prompt = plan.compose_system_prompt(agent_system_prompt.as_deref());
    }

    let compiled_request = resolve_compiled_execution_request(
        config,
        &build_execution_resolution_context(
            session_id,
            &provider_id,
            &model_id,
            request_variant,
            resolved_agent.as_ref(),
        ),
    )
    .await;
    tracing::info!(
        route,
        requested_agent = ?requested_agent,
        scheduler_agent = ?scheduler_agent_name,
        scheduler_applied,
        scheduler_profile = ?scheduler_profile_name,
        scheduler_root_agent = ?scheduler_root_agent,
        scheduler_skill_tree_applied,
        request_skill_tree_applied = request_skill_tree_plan.is_some(),
        fallback_agent = ?fallback_agent_name,
        resolved_agent = ?resolved_agent.as_ref().map(|agent| agent.name.as_str()),
        agent_model = ?agent_model,
        scheduler_profile_model = ?scheduler_profile_model,
        request_model_input = ?request_model_input,
        config_model_input = ?config_model_input,
        config_provider_input = ?config_provider_input,
        system_prompt_applied = agent_system_prompt.is_some(),
        "resolved request prompt agent configuration"
    );

    Ok(ResolvedPromptRequestConfig {
        scheduler_applied,
        scheduler_profile_name,
        scheduler_profile_config,
        scheduler_root_agent,
        scheduler_skill_tree_applied,
        request_skill_tree_plan,
        resolved_agent,
        provider,
        provider_id,
        model_id,
        agent_system_prompt,
        compiled_request,
    })
}

pub(crate) fn apply_skill_tree_telemetry_metadata(
    metadata: &mut std::collections::HashMap<String, serde_json::Value>,
    plan: Option<&SkillTreeRequestPlan>,
) {
    let Some(plan) = plan else {
        return;
    };
    metadata.insert(
        "scheduler_stage_estimated_context_tokens".to_string(),
        serde_json::json!(plan.estimated_tokens() as u64),
    );
    if let Some(token_budget) = plan.token_budget {
        metadata.insert(
            "scheduler_stage_skill_tree_budget".to_string(),
            serde_json::json!(token_budget as u64),
        );
    }
    metadata.insert(
        "scheduler_stage_skill_tree_truncation_strategy".to_string(),
        serde_json::json!(plan.truncation_strategy.as_label()),
    );
    metadata.insert(
        "scheduler_stage_skill_tree_truncated".to_string(),
        serde_json::json!(plan.is_truncated()),
    );
}

fn maybe_auto_compact_scheduler_session(
    session: &mut rocode_session::Session,
    provider: &dyn rocode_provider::Provider,
    model_id: &str,
    max_output_tokens: Option<u64>,
    config_store: Option<&rocode_config::ConfigStore>,
    live_context_tokens: Option<u64>,
    focus: Option<&str>,
    phase: &str,
) -> bool {
    let Some(summary) = auto_compact_session_with_focus_if_needed(
        session,
        provider,
        model_id,
        max_output_tokens,
        config_store,
        live_context_tokens,
        focus,
    ) else {
        return false;
    };

    if let Some(message) = session.messages_mut().last_mut() {
        message.metadata.insert(
            "context_compaction_phase".to_string(),
            serde_json::json!(phase),
        );
        message.metadata.insert(
            "context_compaction_notice".to_string(),
            serde_json::json!("Context compacted"),
        );
    }

    tracing::info!(phase, summary, "scheduler context compacted");
    true
}

#[derive(Debug, Clone)]
pub struct LocalSchedulerPromptRequest {
    pub session_id: Option<String>,
    pub directory: String,
    pub prompt_text: String,
    pub display_prompt_text: String,
    pub scheduler_profile: String,
    pub model: Option<String>,
    pub variant: Option<String>,
}

async fn resolve_local_scheduler_prompt_parts(
    prompt_text: &str,
    directory: &str,
    config: &AppConfig,
) -> Vec<rocode_session::prompt::PartInput> {
    let known_agents = AgentRegistry::from_config(config)
        .list_all()
        .into_iter()
        .map(|agent| agent.name.clone())
        .collect::<Vec<_>>();
    rocode_session::resolve_prompt_parts(prompt_text, Path::new(directory), &known_agents).await
}

#[derive(Debug, Clone, Default)]
pub struct LocalSchedulerPromptOutcome {
    pub session_id: String,
    pub assistant_text: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub context_tokens: u64,
    pub cancelled: bool,
}

pub async fn run_local_scheduler_prompt(
    state: Arc<ServerState>,
    req: LocalSchedulerPromptRequest,
    output_hook: Option<OutputBlockHook>,
) -> anyhow::Result<LocalSchedulerPromptOutcome> {
    let output_hook = output_hook.or_else(|| Some(server_output_block_hook(state.clone())));
    let config = state.config_store.config();
    let session_id = {
        let mut sessions = state.sessions.lock().await;
        match req
            .session_id
            .as_deref()
            .and_then(|id| sessions.get(id).cloned())
        {
            Some(existing) => existing.id.clone(),
            None => sessions
                .create(
                    "rocode-cli",
                    resolved_session_directory(&req.directory, &state.project_root()),
                )
                .id
                .clone(),
        }
    };
    let request_config = resolve_prompt_request_config(PromptRequestConfigInput {
        state: &state,
        config: &config,
        session_id: &session_id,
        requested_agent: None,
        requested_scheduler_profile: Some(req.scheduler_profile.as_str()),
        scheduler_profile_override: None,
        request_model: req.model.as_deref(),
        request_variant: req.variant.as_deref(),
        route: "cli-local",
    })
    .await
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let profile_name = request_config
        .scheduler_profile_name
        .clone()
        .ok_or_else(|| anyhow::anyhow!("scheduler profile was not resolved"))?;
    let request_skill_tree_plan = request_config.request_skill_tree_plan.clone();
    let mut profile_config = request_config
        .scheduler_profile_config
        .clone()
        .or_else(|| {
            resolve_scheduler_profile_config(&config, Some(&profile_name))
                .map(|(_, profile)| profile)
        })
        .ok_or_else(|| anyhow::anyhow!("scheduler profile config not found: {}", profile_name))?;

    let mut session = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("failed to initialize local scheduler session"))?
    };
    let normalized_directory =
        resolved_session_directory(session.record().directory.as_str(), &state.project_root());
    if session.record().directory != normalized_directory {
        session.set_directory(normalized_directory);
    }

    let scheduler_applied = request_config.scheduler_applied;
    let scheduler_root_agent = request_config.scheduler_root_agent.clone();
    let scheduler_skill_tree_applied = request_config.scheduler_skill_tree_applied;
    let provider = request_config.provider.clone();
    let provider_id = request_config.provider_id.clone();
    let model_id = request_config.model_id.clone();
    let fallback_request = resolve_compiled_execution_request(
        &config,
        &ExecutionResolutionContext {
            session_id: session_id.clone(),
            provider_id: provider_id.clone(),
            model_id: model_id.clone(),
            variant: req.variant.clone(),
            ..Default::default()
        },
    )
    .await;

    set_session_run_status(&state, &session_id, SessionRunStatus::Busy).await;

    session.insert_metadata("model_provider", serde_json::json!(&provider_id));
    session.insert_metadata("model_id", serde_json::json!(&model_id));
    session.insert_metadata("scheduler_applied", serde_json::json!(scheduler_applied));
    session.insert_metadata(
        "scheduler_skill_tree_applied",
        serde_json::json!(scheduler_skill_tree_applied),
    );
    session.insert_metadata("scheduler_profile", serde_json::json!(profile_name.clone()));
    if let Some(root_agent) = scheduler_root_agent.as_deref() {
        session.insert_metadata("scheduler_root_agent", serde_json::json!(root_agent));
    } else {
        session.remove_metadata("scheduler_root_agent");
    }

    let pre_compact_live_tokens = request_skill_tree_plan
        .as_ref()
        .map(|plan| plan.estimated_tokens() as u64);
    if maybe_auto_compact_scheduler_session(
        &mut session,
        provider.as_ref(),
        &model_id,
        request_config.compiled_request.max_tokens,
        Some(state.config_store.as_ref()),
        pre_compact_live_tokens,
        Some(&req.prompt_text),
        "scheduler.pre_run",
    ) {
        let mut sessions = state.sessions.lock().await;
        sessions.update(session.clone());
    }

    let (memory_frozen_snapshot_block, _memory_prefetch_packet, memory_prefetch_block) =
        resolve_prompt_memory_context(&state, &mut session, &req.prompt_text).await;
    let scheduler_session_context_packet = build_scheduler_session_context_packet(&session);
    let scheduler_session_context_block = scheduler_session_context_packet
        .as_ref()
        .map(|packet| packet.render());
    let scheduler_execution_prompt = merge_scheduler_prompt_with_memory(
        &req.prompt_text,
        memory_frozen_snapshot_block.as_deref(),
        memory_prefetch_block.as_deref(),
    );

    let mode_kind = scheduler_mode_kind(&profile_name);
    let resolved_system_prompt = scheduler_system_prompt_preview(&profile_name, &profile_config);
    let prompt_parts = resolve_local_scheduler_prompt_parts(
        &req.prompt_text,
        session.record().directory.as_str(),
        &config,
    )
    .await;
    let scheduler_input = rocode_session::PromptInput {
        session_id: session_id.clone(),
        message_id: None,
        model: None,
        agent: None,
        no_reply: false,
        system: None,
        variant: req.variant.clone(),
        parts: prompt_parts,
        tools: None,
    };
    let user_message_id = create_scheduler_user_message(
        state.prompt_runner.as_ref(),
        &mut session,
        &scheduler_input,
        SchedulerUserMessageContext {
            display_prompt_text: &req.display_prompt_text,
            resolved_user_prompt: &req.prompt_text,
            profile_name: &profile_name,
            mode_kind,
            resolved_system_prompt: &resolved_system_prompt,
            recovery: None,
        },
    )
    .await
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let assistant_message_id = session.add_assistant_message().id.clone();

    if session.is_default_title() {
        if let Some(first_text) = first_user_message_text(&session) {
            let immediate = rocode_session::generate_session_title(&first_text);
            if !immediate.is_empty() && immediate != "New Session" {
                session.set_auto_title(immediate);
            }
        }
    }

    {
        let mut sessions = state.sessions.lock().await;
        sessions.update(session.clone());
    }

    let agent_registry = Arc::new(AgentRegistry::from_config(&config));
    if profile_config.available_agents.is_empty() {
        profile_config.available_agents = agent_registry
            .list()
            .iter()
            .filter(|a| !a.hidden && matches!(a.mode, AgentMode::Subagent | AgentMode::All))
            .map(|a| AvailableAgentMeta {
                name: a.name.clone(),
                description: a.description.clone().unwrap_or_default(),
                mode: match a.mode {
                    AgentMode::Primary => "primary".to_string(),
                    AgentMode::Subagent => "subagent".to_string(),
                    AgentMode::All => "all".to_string(),
                },
                cost: if a.name == "oracle" {
                    "EXPENSIVE".to_string()
                } else {
                    "CHEAP".to_string()
                },
            })
            .collect();
    }
    if profile_config.available_categories.is_empty() {
        profile_config.available_categories = state
            .category_registry
            .category_descriptions()
            .into_iter()
            .map(|(name, description)| AvailableCategoryMeta { name, description })
            .collect();
    }

    let current_model = Some(format!("{}:{}", provider_id, model_id));
    let scheduler_abort_token = CancellationToken::new();
    state
        .runtime_telemetry
        .register_scheduler_run(
            &session_id,
            scheduler_abort_token.clone(),
            Some(profile_name.clone()),
        )
        .await;
    let tool_executor: Arc<dyn OrchestratorToolExecutor> = Arc::new(SessionSchedulerToolExecutor {
        state: state.clone(),
        session_id: session_id.clone(),
        message_id: assistant_message_id.clone(),
        directory: session.record().directory.clone(),
        abort_token: scheduler_abort_token.clone(),
        current_model,
        tool_runtime_config: rocode_tool::ToolRuntimeConfig::from_config(&config),
        agent_registry: agent_registry.clone(),
    });
    let tool_runner = ToolRunner::new(tool_executor.clone());
    let model_resolver: Arc<dyn ModelResolver> = Arc::new(SessionSchedulerModelResolver {
        state: state.clone(),
        fallback_provider_id: provider_id.clone(),
        fallback_model_id: model_id.clone(),
        fallback_request: fallback_request.clone(),
    });
    let mut exec_metadata = std::collections::HashMap::from([
        (
            "message_id".to_string(),
            serde_json::json!(assistant_message_id.clone()),
        ),
        (
            "user_message_id".to_string(),
            serde_json::json!(user_message_id.clone()),
        ),
        (
            "scheduler_profile".to_string(),
            serde_json::json!(profile_name.clone()),
        ),
    ]);
    if let Some(session_context) = scheduler_session_context_block.as_deref() {
        exec_metadata.insert(
            SCHEDULER_SESSION_CONTEXT_METADATA_KEY.to_string(),
            serde_json::json!(session_context),
        );
    }
    if let Some(session_context_packet) = scheduler_session_context_packet.as_ref() {
        exec_metadata.insert(
            SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY.to_string(),
            session_context_packet.metadata_value(),
        );
    }
    apply_skill_tree_telemetry_metadata(&mut exec_metadata, request_skill_tree_plan.as_ref());
    let exec_ctx = OrchestratorExecutionContext {
        session_id: session_id.clone(),
        workdir: session.record().directory.clone(),
        agent_name: profile_name.clone(),
        metadata: exec_metadata,
    };
    let model_pricing = {
        let providers = state.providers.read().await;
        providers
            .find_model(&model_id)
            .map(|(_, info)| ModelPricing::from_model_info(&info))
    };
    let lifecycle_hook = Arc::new(
        SessionSchedulerLifecycleHook::new(state.clone(), session_id.clone(), profile_name.clone())
            .with_model_pricing(model_pricing)
            .with_output_hook(output_hook.clone()),
    );
    let ctx = OrchestratorContext {
        agent_resolver: Arc::new(SchedulerAgentResolver {
            registry: agent_registry.clone(),
        }),
        model_resolver,
        tool_executor,
        lifecycle_hook,
        cancel_token: Arc::new(SchedulerRunCancelToken {
            token: scheduler_abort_token.clone(),
        }),
        exec_ctx,
    };
    let mut plan = scheduler_plan_from_profile(Some(profile_name.clone()), &profile_config)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    enrich_scheduler_plan_skills(&state, &mut plan).await?;

    let orchestrator_result = scheduler_orchestrator_from_plan(plan, tool_runner)
        .execute(&scheduler_execution_prompt, &ctx)
        .await;
    state
        .runtime_telemetry
        .finish_scheduler_run(&session_id)
        .await;

    session = {
        let sessions = state.sessions.lock().await;
        sessions
            .get(&session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("scheduler session vanished"))?
    };

    let mut prompt_tokens = 0;
    let mut completion_tokens = 0;
    let mut context_tokens = 0;
    let mut cancelled = false;
    if let Some(assistant) = session.get_message_mut(&assistant_message_id) {
        assistant.metadata.insert(
            "model_provider".to_string(),
            serde_json::json!(&provider_id),
        );
        assistant
            .metadata
            .insert("model_id".to_string(), serde_json::json!(&model_id));
        assistant.metadata.insert(
            "scheduler_profile".to_string(),
            serde_json::json!(profile_name.clone()),
        );
        assistant.metadata.insert(
            "resolved_scheduler_profile".to_string(),
            serde_json::json!(profile_name.clone()),
        );
        assistant.metadata.insert(
            "resolved_execution_mode_kind".to_string(),
            serde_json::json!(mode_kind),
        );
        assistant
            .metadata
            .insert("mode".to_string(), serde_json::json!(profile_name.clone()));
        assistant.metadata.insert(
            "scheduler_applied".to_string(),
            serde_json::json!(scheduler_applied),
        );
        match orchestrator_result {
            Ok(output) => {
                cancelled = output.is_cancelled();
                if cancelled {
                    let _ = finalize_active_scheduler_stage_cancelled(&state, &session_id).await;
                    assistant.finish = Some("cancelled".to_string());
                    assistant
                        .metadata
                        .insert("finish_reason".to_string(), serde_json::json!("cancelled"));
                } else {
                    assistant.finish = Some("stop".to_string());
                }
                assistant.metadata.insert(
                    "scheduler_steps".to_string(),
                    serde_json::json!(output.steps),
                );
                assistant.metadata.insert(
                    "scheduler_tool_calls".to_string(),
                    serde_json::json!(output.tool_calls_count),
                );
                if let Some(usage) = output_usage(&output.metadata) {
                    prompt_tokens = usage.prompt_tokens;
                    context_tokens = usage.context_tokens.max(usage.prompt_tokens);
                    completion_tokens = usage.completion_tokens;
                    let cost = model_pricing
                        .map(|p| {
                            p.compute(
                                usage.prompt_tokens,
                                usage.completion_tokens,
                                usage.cache_read_tokens,
                                usage.cache_write_tokens,
                            )
                        })
                        .unwrap_or(0.0);
                    assistant.usage = Some(rocode_session::MessageUsage {
                        input_tokens: usage.prompt_tokens,
                        output_tokens: usage.completion_tokens,
                        reasoning_tokens: usage.reasoning_tokens,
                        cache_read_tokens: usage.cache_read_tokens,
                        cache_write_tokens: usage.cache_write_tokens,
                        context_tokens: usage.context_tokens.max(usage.prompt_tokens),
                        total_cost: cost,
                    });
                }
                assistant.add_text(visible_assistant_text_from_orchestrator_output(
                    &output.content,
                ));
            }
            Err(error) => {
                cancelled = is_scheduler_cancellation_error(&error);
                if cancelled {
                    let _ = finalize_active_scheduler_stage_cancelled(&state, &session_id).await;
                    assistant.finish = Some("cancelled".to_string());
                    assistant
                        .metadata
                        .insert("finish_reason".to_string(), serde_json::json!("cancelled"));
                    assistant.add_text("Scheduler cancelled.");
                } else {
                    assistant.finish = Some("error".to_string());
                    assistant
                        .metadata
                        .insert("error".to_string(), serde_json::json!(error.to_string()));
                    assistant.add_text(format!("Scheduler error: {}", error));
                }
            }
        }
    }

    move_scheduler_final_answer_after_stage_messages(&mut session, &assistant_message_id);
    ensure_default_session_title(&mut session, provider.clone(), &model_id).await;
    let assistant_text = session
        .get_message(&assistant_message_id)
        .map(assistant_visible_text)
        .unwrap_or_default();

    maybe_auto_compact_scheduler_session(
        &mut session,
        provider.as_ref(),
        &model_id,
        request_config.compiled_request.max_tokens,
        Some(state.config_store.as_ref()),
        (context_tokens > 0)
            .then_some(context_tokens)
            .or_else(|| (prompt_tokens > 0).then_some(prompt_tokens)),
        Some(&req.prompt_text),
        "scheduler.post_run",
    );

    let _ = state
        .runtime_telemetry
        .record_session_usage(
            &session_id,
            Some(&assistant_message_id),
            session.get_usage(),
        )
        .await;
    persist_session_telemetry_metadata(&state, &mut session).await;
    {
        let mut sessions = state.sessions.lock().await;
        sessions.update(session.clone());
    }
    broadcast_session_updated(state.as_ref(), session_id.clone(), "prompt.completed");
    set_session_run_status(&state, &session_id, SessionRunStatus::Idle).await;

    if let Some(output_hook) = output_hook {
        if !assistant_text.trim().is_empty() {
            emit_output_block_via_hook(
                Some(&output_hook),
                OutputBlockEvent {
                    session_id: session_id.clone(),
                    block: OutputBlock::Message(MessageBlock::full(
                        OutputMessageRole::Assistant,
                        assistant_text.clone(),
                    )),
                    id: Some(assistant_message_id.clone()),
                },
            )
            .await;
        }
    }

    Ok(LocalSchedulerPromptOutcome {
        session_id,
        assistant_text,
        prompt_tokens,
        completion_tokens,
        context_tokens,
        cancelled,
    })
}

pub async fn abort_local_session_execution(
    state: Arc<ServerState>,
    session_id: &str,
    scheduler_stage_only: bool,
) -> serde_json::Value {
    abort_session_execution(&state, session_id, scheduler_stage_only).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn verifier_logprob_options_merge_top_level_and_responses_options() {
        let options = merge_verifier_logprob_options(
            Some(HashMap::from([(
                "responses".to_string(),
                serde_json::json!({ "reasoningEffort": "low" }),
            )])),
            20,
        );

        assert_eq!(options["logprobs"].as_u64(), Some(20));
        assert_eq!(options["responses"]["logprobs"].as_u64(), Some(20));
        assert_eq!(
            options["responses"]["reasoningEffort"].as_str(),
            Some("low")
        );
        assert_eq!(options["openai"]["logprobs"].as_u64(), Some(20));
    }

    #[test]
    fn scheduler_context_hydrate_only_allows_packet_anchors() {
        let exec_ctx = OrchestratorExecutionContext {
            session_id: "session".to_string(),
            workdir: "/tmp".to_string(),
            agent_name: "sisyphus".to_string(),
            metadata: HashMap::from([(
                SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY.to_string(),
                serde_json::json!({
                    "version": 1,
                    "exact_recent_tail": [
                        {"message_id": "msg_user", "role": "user"},
                        {"message_id": "msg_assistant", "role": "assistant"}
                    ],
                    "latest_compaction_summary": {"message_id": "msg_compaction"}
                }),
            )]),
        };

        let allowed = scheduler_context_allowed_message_ids(&exec_ctx);

        assert_eq!(
            allowed,
            vec![
                "msg_assistant".to_string(),
                "msg_compaction".to_string(),
                "msg_user".to_string()
            ]
        );
    }

    #[test]
    fn scheduler_context_hydrate_rejects_unknown_packet_version() {
        let exec_ctx = OrchestratorExecutionContext {
            session_id: "session".to_string(),
            workdir: "/tmp".to_string(),
            agent_name: "sisyphus".to_string(),
            metadata: HashMap::from([(
                SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY.to_string(),
                serde_json::json!({
                    "version": 99,
                    "exact_recent_tail": [
                        {"message_id": "msg_user", "role": "user"}
                    ]
                }),
            )]),
        };

        assert!(scheduler_context_allowed_message_ids(&exec_ctx).is_empty());
    }

    #[test]
    fn scheduler_context_hydrate_arguments_validate_and_dedupe_ids() {
        let ids = scheduler_context_hydrate_message_ids(&serde_json::json!({
            "message_ids": ["msg_1", "msg_1", "msg_2"]
        }))
        .expect("valid message ids should parse");

        assert_eq!(ids, vec!["msg_1".to_string(), "msg_2".to_string()]);
        assert!(scheduler_context_hydrate_message_ids(&serde_json::json!({
            "message_ids": []
        }))
        .is_err());
        assert_eq!(
            scheduler_context_hydrate_message_limit(&serde_json::json!({
                "max_chars_per_message": 99_999
            })),
            SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGE_LIMIT
        );
    }

    #[test]
    fn scheduler_context_hydrate_renders_text_and_compaction_parts() {
        let mut message = SessionMessage::assistant("session");
        message.id = "msg_compaction".to_string();
        message.add_text("visible text");
        message.parts.push(rocode_session::MessagePart {
            id: "part_compaction".to_string(),
            part_type: SessionPartType::Compaction {
                summary: "older findings".to_string(),
            },
            created_at: chrono::Utc::now(),
            message_id: Some(message.id.clone()),
        });

        let rendered = render_scheduler_context_hydrated_message(&message, 4_000)
            .expect("message should hydrate");

        assert!(rendered.contains("assistant `msg_compaction`"));
        assert!(rendered.contains("visible text"));
        assert!(rendered.contains("[compaction summary]"));
        assert!(rendered.contains("older findings"));
    }

    #[tokio::test]
    async fn local_scheduler_prompt_parts_resolve_file_references() {
        let temp_dir =
            std::env::temp_dir().join(format!("rocode-local-scheduler-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let file_path = temp_dir.join("note.txt");
        std::fs::write(&file_path, "hello").expect("temp file should be written");

        let parts = resolve_local_scheduler_prompt_parts(
            "Inspect @note.txt",
            temp_dir.to_str().expect("temp path should be utf-8"),
            &AppConfig::default(),
        )
        .await;

        assert!(matches!(
            &parts[0],
            rocode_session::prompt::PartInput::Text { text } if text == "Inspect @note.txt"
        ));
        assert!(parts.iter().any(|part| matches!(
            part,
            rocode_session::prompt::PartInput::File { filename, mime, .. }
            if filename.as_deref() == Some("note.txt")
                && mime.as_deref() == Some("text/plain")
        )));

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir(&temp_dir);
    }
}
