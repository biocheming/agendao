use agendao_api::{
    ProvisionExternalAdapterSessionRequest, ProvisionExternalAdapterSessionResponse,
};
use agendao_config::{Config, ExternalAdapterEntryConfig};
use agendao_provider::AuthManager;
use agendao_session::prompt::{
    external_adapter_event_to_ingress_turn, ExternalAdapterIngressMappingError, IngressTurnEnvelope,
};
use agendao_session::Session;
use agendao_storage::{
    ExternalAdapterReplayInsertOutcome, ExternalAdapterReplayRecord,
    ExternalAdapterReplayRepository,
};
use agendao_types::{
    ConfigPolicyValidationEffect, ConfigPolicyValidationItem, ConfigPolicyValidationOwner,
    ConfigPolicyValidationScope, ConfigPolicyValidationScopeKind, ConfigPolicyValidationSeverity,
    ExternalAdapterEvent, ExternalAdapterResolvedBinding, ExternalAdapterSource,
    ExternalAdapterValidationError,
};
use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;

use super::session::prompt::{
    load_verified_external_adapter_binding, persist_verified_external_adapter_binding,
    session_prompt_with_verified_ingress, SessionPromptRequest, VerifiedSessionIngress,
};
use super::session::{create_session_from_spec, session_to_info, CreateSessionSpec};
use crate::{ApiError, Result, ServerState};

pub(crate) fn external_adapter_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route(
            "/session/provision",
            post(provision_external_adapter_session),
        )
        .route("/generic-webhook/parse", post(parse_generic_webhook))
        .route("/generic-webhook/verify", post(verify_generic_webhook))
        .route("/generic-webhook/run", post(run_generic_webhook))
}

pub(crate) fn collect_external_adapter_config_validation(
    config: &Config,
) -> Vec<ConfigPolicyValidationItem> {
    let Some(external) = config.external_adapter.as_ref() else {
        return Vec::new();
    };

    let mut adapter_ids = external.adapters.keys().cloned().collect::<Vec<_>>();
    adapter_ids.sort();

    let mut reports = Vec::new();
    for adapter_id in adapter_ids {
        let Some(adapter) = external.adapters.get(&adapter_id) else {
            continue;
        };
        if let Some(item) = external_adapter_missing_secret_ref_item(&adapter_id, adapter) {
            reports.push(item);
        }
        if let Some(item) = external_adapter_missing_default_workspace_item(&adapter_id, adapter) {
            reports.push(item);
        }
    }
    reports
}

fn external_adapter_missing_secret_ref_item(
    adapter_id: &str,
    adapter: &ExternalAdapterEntryConfig,
) -> Option<ConfigPolicyValidationItem> {
    if !generic_webhook_adapter_enabled(adapter) {
        return None;
    }

    if adapter
        .secret_ref
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return None;
    }

    let adapter_id = adapter_id.trim();
    let subject_id = (!adapter_id.is_empty()).then(|| adapter_id.to_string());
    let path = if let Some(subject_id) = subject_id.as_deref() {
        format!("external_adapter.adapters.{subject_id}.secret_ref")
    } else {
        "external_adapter.adapters".to_string()
    };
    let label = if adapter_id.is_empty() {
        "<unknown>"
    } else {
        adapter_id
    };

    Some(ConfigPolicyValidationItem {
        owner: ConfigPolicyValidationOwner::ExternalAdapter,
        scope: ConfigPolicyValidationScope {
            kind: ConfigPolicyValidationScopeKind::ExternalAdapter,
            subject_id,
        },
        path,
        severity: ConfigPolicyValidationSeverity::Error,
        effect: ConfigPolicyValidationEffect::FailClosedRequestGate,
        code: "external_adapter_missing_secret_ref".to_string(),
        message: format!(
            "External adapter `{label}` is configured for generic-webhook but does not declare secret_ref."
        ),
        fallback: None,
    })
}

fn external_adapter_missing_default_workspace_item(
    adapter_id: &str,
    adapter: &ExternalAdapterEntryConfig,
) -> Option<ConfigPolicyValidationItem> {
    if !generic_webhook_adapter_enabled(adapter) || adapter.allow_session_run != Some(true) {
        return None;
    }

    if adapter
        .default_workspace
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return None;
    }

    let adapter_id = adapter_id.trim();
    let subject_id = (!adapter_id.is_empty()).then(|| adapter_id.to_string());
    let path = if let Some(subject_id) = subject_id.as_deref() {
        format!("external_adapter.adapters.{subject_id}.default_workspace")
    } else {
        "external_adapter.adapters".to_string()
    };
    let label = if adapter_id.is_empty() {
        "<unknown>"
    } else {
        adapter_id
    };

    Some(ConfigPolicyValidationItem {
        owner: ConfigPolicyValidationOwner::ExternalAdapter,
        scope: ConfigPolicyValidationScope {
            kind: ConfigPolicyValidationScopeKind::ExternalAdapter,
            subject_id,
        },
        path,
        severity: ConfigPolicyValidationSeverity::Error,
        effect: ConfigPolicyValidationEffect::FailClosedRequestGate,
        code: "external_adapter_missing_default_workspace".to_string(),
        message: format!(
            "External adapter `{label}` allows session runs but does not declare default_workspace for owner-local session provisioning."
        ),
        fallback: None,
    })
}

async fn parse_generic_webhook(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<GenericWebhookParseRequest>,
) -> Result<Json<GenericWebhookParseResponse>> {
    Ok(Json(parse_generic_webhook_request(
        request,
        state.external_adapter_replay_repo.is_some(),
    )?))
}

async fn verify_generic_webhook(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<GenericWebhookParseRequest>,
) -> Result<Json<GenericWebhookParseResponse>> {
    let config = state.config_store.config();
    let replay_repo = state.external_adapter_replay_repo.as_deref();
    Ok(Json(
        verify_generic_webhook_request(request, &config, &state.auth_manager, replay_repo).await?,
    ))
}

async fn run_generic_webhook(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(request): Json<GenericWebhookParseRequest>,
) -> Result<Json<GenericWebhookRunResponse>> {
    let verified_run = verify_generic_webhook_for_session_run(request, state.clone()).await?;

    let Json(session) = session_prompt_with_verified_ingress(
        state,
        headers,
        verified_run.session_id,
        verified_run.prompt_request,
        verified_run.verified_ingress,
    )
    .await?;

    Ok(Json(GenericWebhookRunResponse {
        verification: verified_run.verification,
        session,
    }))
}

async fn provision_external_adapter_session(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<ProvisionExternalAdapterSessionRequest>,
) -> Result<Json<ProvisionExternalAdapterSessionResponse>> {
    let provisioned = provision_generic_webhook_session(state, request).await?;
    Ok(Json(provisioned))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenericWebhookParseRequest {
    pub event: ExternalAdapterEvent,
    pub binding: ExternalAdapterResolvedBinding,
    pub replay_guard: GenericWebhookReplayGuardRef,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct GenericWebhookReplayGuardRef {
    pub timestamp_ms: i64,
    pub nonce: String,
    pub signature: String,
}

impl GenericWebhookReplayGuardRef {
    fn validate(&self) -> Result<()> {
        if self.timestamp_ms <= 0 {
            return Err(ApiError::BadRequest(
                "replay_guard.timestamp_ms must be positive".to_string(),
            ));
        }
        require_non_blank("replay_guard.nonce", &self.nonce)?;
        require_non_blank("replay_guard.signature", &self.signature)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
struct GenericWebhookParseResponse {
    pub adapter: String,
    pub source: ExternalAdapterSource,
    pub dry_run: bool,
    pub would_enqueue: bool,
    pub binding: ExternalAdapterResolvedBinding,
    pub replay_guard: GenericWebhookReplayGuardInspection,
    pub authority: GenericWebhookAuthorityInspection,
    pub ingress: IngressTurnEnvelope,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct GenericWebhookRunResponse {
    pub verification: GenericWebhookParseResponse,
    pub session: serde_json::Value,
}

#[derive(Debug)]
struct VerifiedGenericWebhookRun {
    pub verification: GenericWebhookParseResponse,
    pub session_id: String,
    pub prompt_request: SessionPromptRequest,
    pub verified_ingress: VerifiedSessionIngress,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct GenericWebhookReplayGuardInspection {
    pub present: bool,
    pub verified: bool,
    pub nonce: String,
    pub timestamp_ms: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct GenericWebhookAuthorityInspection {
    pub config_owner: String,
    pub secret_owner: String,
    pub replay_store: String,
}

fn parse_generic_webhook_request(
    request: GenericWebhookParseRequest,
    replay_store_available: bool,
) -> Result<GenericWebhookParseResponse> {
    if request.event.source != ExternalAdapterSource::GenericWebhook {
        return Err(ApiError::BadRequest(format!(
            "generic webhook endpoint requires event.source `{}`",
            ExternalAdapterSource::GenericWebhook.as_str()
        )));
    }

    request
        .event
        .validate()
        .map_err(external_adapter_validation_error)?;
    request
        .binding
        .validate()
        .map_err(external_adapter_validation_error)?;
    request.replay_guard.validate()?;

    let ingress =
        external_adapter_event_to_ingress_turn(request.binding.session_id.trim(), &request.event)
            .map_err(external_adapter_ingress_mapping_error)?;

    Ok(GenericWebhookParseResponse {
        adapter: request.event.adapter_id.trim().to_string(),
        source: request.event.source,
        dry_run: true,
        would_enqueue: false,
        binding: ExternalAdapterResolvedBinding {
            session_id: request.binding.session_id.trim().to_string(),
            actor_id: request.binding.actor_id.trim().to_string(),
            workspace_id: request.binding.workspace_id.trim().to_string(),
            route_policy_id: request
                .binding
                .route_policy_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        },
        replay_guard: GenericWebhookReplayGuardInspection {
            present: true,
            verified: false,
            nonce: request.replay_guard.nonce.trim().to_string(),
            timestamp_ms: request.replay_guard.timestamp_ms,
            status: "parse_only_not_verified".to_string(),
        },
        authority: GenericWebhookAuthorityInspection {
            config_owner: "agendao-config.external_adapter".to_string(),
            secret_owner: "AuthManager via external_adapter.adapters[*].secret_ref".to_string(),
            replay_store: if replay_store_available {
                "agendao-storage.external_adapter_replay".to_string()
            } else {
                "unavailable: storage repository not configured".to_string()
            },
        },
        ingress,
        warnings: vec![
            "generic webhook parse-only skeleton did not verify the signature".to_string(),
            "generic webhook parse-only skeleton did not enqueue a session turn".to_string(),
            "generic webhook parse-only skeleton did not record replay state".to_string(),
        ],
    })
}

async fn verify_generic_webhook_request(
    request: GenericWebhookParseRequest,
    config: &Config,
    auth_manager: &AuthManager,
    replay_repo: Option<&ExternalAdapterReplayRepository>,
) -> Result<GenericWebhookParseResponse> {
    let replay_repo = replay_repo.ok_or_else(|| {
        ApiError::BadRequest(
            "external adapter replay store is required before accepting webhook events".to_string(),
        )
    })?;
    let replay_config = config
        .external_adapter
        .as_ref()
        .and_then(|external| external.replay.as_ref());
    let adapter = configured_generic_webhook_adapter(&config, &request.event)?;
    enforce_binding_policy(adapter, &request.binding)?;
    let secret_ref = adapter
        .secret_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "external adapter `{}` requires secret_ref",
                request.event.adapter_id.trim()
            ))
        })?;
    let secret = auth_manager.get_api_key(secret_ref).await.ok_or_else(|| {
        ApiError::BadRequest(format!(
            "external adapter secret `{}` is not available in AuthManager",
            secret_ref
        ))
    })?;

    verify_replay_signature(&request, &secret, replay_config)?;
    apply_replay_retention(replay_repo, replay_config).await?;

    let mut response = parse_generic_webhook_request(request.clone(), true)?;
    let replay_record = replay_record_from_request_and_response(&request, &response);
    match replay_repo.record(&replay_record).await.map_err(|error| {
        ApiError::InternalError(format!(
            "failed to record external adapter replay state: {error}"
        ))
    })? {
        ExternalAdapterReplayInsertOutcome::Inserted => {}
        ExternalAdapterReplayInsertOutcome::Duplicate => {
            return Err(ApiError::BadRequest(
                "duplicate external adapter event or replay nonce".to_string(),
            ));
        }
    }

    response.replay_guard.verified = true;
    response.replay_guard.status = "verified_recorded".to_string();
    response.authority.replay_store =
        "agendao-storage.external_adapter_replay:recorded".to_string();
    response.warnings = vec![
        "generic webhook verified and recorded replay state".to_string(),
        "generic webhook verify gate did not enqueue a session turn".to_string(),
    ];
    Ok(response)
}

async fn verify_generic_webhook_for_session_run(
    request: GenericWebhookParseRequest,
    state: Arc<ServerState>,
) -> Result<VerifiedGenericWebhookRun> {
    let config = state.config_store.config();
    let adapter = configured_generic_webhook_adapter(&config, &request.event)?;
    enforce_binding_policy(adapter, &request.binding)?;
    enforce_session_run_allowed(adapter, request.event.adapter_id.trim())?;
    let session =
        load_session_for_external_adapter_run(state.as_ref(), request.binding.session_id.trim())
            .await?;
    enforce_session_binding_authority(&session, &request.binding)?;

    let mut verification = verify_generic_webhook_request(
        request,
        &config,
        &state.auth_manager,
        state.external_adapter_replay_repo.as_deref(),
    )
    .await?;
    let session_id = verification.binding.session_id.clone();
    let prompt_request = SessionPromptRequest::from_verified_ingress(&verification.ingress);
    let verified_ingress = VerifiedSessionIngress {
        ingress: verification.ingress.clone(),
        external_adapter_binding: Some(verification.binding.clone()),
    };

    verification.dry_run = false;
    verification.would_enqueue = true;
    verification.warnings = vec![
        "generic webhook verified and recorded replay state".to_string(),
        "generic webhook run gate called the shared session runtime entrypoint".to_string(),
    ];

    Ok(VerifiedGenericWebhookRun {
        verification,
        session_id,
        prompt_request,
        verified_ingress,
    })
}

fn generic_webhook_adapter_enabled(adapter: &ExternalAdapterEntryConfig) -> bool {
    if adapter.enabled == Some(false) {
        return false;
    }

    adapter
        .source
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        == Some(ExternalAdapterSource::GenericWebhook.as_str())
}

fn configured_generic_webhook_adapter<'a>(
    config: &'a Config,
    event: &ExternalAdapterEvent,
) -> Result<&'a ExternalAdapterEntryConfig> {
    configured_external_adapter_for_source(config, event.adapter_id.trim(), event.source)
}

fn configured_external_adapter_for_source<'a>(
    config: &'a Config,
    adapter_id: &str,
    source: ExternalAdapterSource,
) -> Result<&'a ExternalAdapterEntryConfig> {
    if source != ExternalAdapterSource::GenericWebhook {
        return Err(ApiError::BadRequest(format!(
            "generic webhook endpoint requires event.source `{}`",
            ExternalAdapterSource::GenericWebhook.as_str()
        )));
    }
    let adapter = config
        .external_adapter
        .as_ref()
        .and_then(|external| external.adapters.get(adapter_id))
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "external adapter `{}` is not configured",
                adapter_id
            ))
        })?;

    if adapter.enabled == Some(false) {
        return Err(ApiError::BadRequest(format!(
            "external adapter `{}` is disabled",
            adapter_id
        )));
    }
    if adapter
        .source
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        != Some(source.as_str())
    {
        return Err(ApiError::BadRequest(format!(
            "external adapter `{}` is not configured for `{}`",
            adapter_id,
            source.as_str()
        )));
    }

    Ok(adapter)
}

fn enforce_binding_policy(
    adapter: &ExternalAdapterEntryConfig,
    binding: &ExternalAdapterResolvedBinding,
) -> Result<()> {
    binding
        .validate()
        .map_err(external_adapter_validation_error)?;

    if !adapter.allowed_workspaces.is_empty()
        && !adapter
            .allowed_workspaces
            .iter()
            .any(|workspace| workspace.trim() == binding.workspace_id.trim())
    {
        return Err(ApiError::BadRequest(format!(
            "workspace `{}` is not allowed for this external adapter",
            binding.workspace_id.trim()
        )));
    }

    if let Some(route_policy_id) = adapter
        .route_policy_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if binding
            .route_policy_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            != Some(route_policy_id)
        {
            return Err(ApiError::BadRequest(format!(
                "route_policy_id `{}` is required for this external adapter",
                route_policy_id
            )));
        }
    }

    Ok(())
}

fn enforce_session_run_allowed(
    adapter: &ExternalAdapterEntryConfig,
    adapter_id: &str,
) -> Result<()> {
    if adapter.allow_session_run == Some(true) {
        return Ok(());
    }

    Err(ApiError::BadRequest(format!(
        "external adapter `{}` is not allowed to run session turns",
        adapter_id
    )))
}

async fn load_session_for_external_adapter_run(
    state: &ServerState,
    session_id: &str,
) -> Result<Session> {
    let sessions = state.sessions.lock().await;
    sessions
        .get(session_id)
        .cloned()
        .ok_or_else(|| ApiError::SessionNotFound(session_id.to_string()))
}

fn enforce_session_binding_authority(
    session: &Session,
    binding: &ExternalAdapterResolvedBinding,
) -> Result<()> {
    let binding = normalized_binding(binding);
    if let Some(existing) = load_verified_external_adapter_binding(session) {
        if normalized_binding(&existing) != binding {
            return Err(ApiError::BadRequest(format!(
                "session `{}` is already bound to a different external adapter actor/workspace",
                session.record().id
            )));
        }
        return Ok(());
    }

    Err(ApiError::BadRequest(format!(
        "session `{}` is not provisioned for external adapter binding; provision it through the owner-local external-adapter session path first",
        session.record().id
    )))
}

fn normalized_binding(binding: &ExternalAdapterResolvedBinding) -> ExternalAdapterResolvedBinding {
    ExternalAdapterResolvedBinding {
        session_id: binding.session_id.trim().to_string(),
        actor_id: binding.actor_id.trim().to_string(),
        workspace_id: binding.workspace_id.trim().to_string(),
        route_policy_id: binding
            .route_policy_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
    }
}

fn verify_replay_signature(
    request: &GenericWebhookParseRequest,
    secret: &str,
    replay_config: Option<&agendao_config::ExternalAdapterReplayConfig>,
) -> Result<()> {
    request.replay_guard.validate()?;
    if let Some(window_seconds) = replay_config
        .and_then(|replay| replay.nonce_window_seconds)
        .filter(|window| *window > 0)
    {
        let now_ms = Utc::now().timestamp_millis();
        let window_ms = i64::try_from(window_seconds.saturating_mul(1000)).unwrap_or(i64::MAX);
        let age_ms = now_ms
            .saturating_sub(request.replay_guard.timestamp_ms)
            .abs();
        if age_ms > window_ms {
            return Err(ApiError::BadRequest(
                "external adapter webhook timestamp is outside the configured replay window"
                    .to_string(),
            ));
        }
    }
    let expected = generic_webhook_signature(secret, request)?;
    let supplied = normalize_signature(&request.replay_guard.signature);
    if expected != supplied {
        return Err(ApiError::BadRequest(
            "invalid external adapter webhook signature".to_string(),
        ));
    }
    Ok(())
}

async fn apply_replay_retention(
    replay_repo: &ExternalAdapterReplayRepository,
    replay_config: Option<&agendao_config::ExternalAdapterReplayConfig>,
) -> Result<()> {
    let Some(retention_seconds) = replay_config
        .and_then(|replay| replay.retention_seconds)
        .filter(|retention| *retention > 0)
    else {
        return Ok(());
    };

    let retention_ms = i64::try_from(retention_seconds.saturating_mul(1000)).unwrap_or(i64::MAX);
    let cutoff_ms = Utc::now().timestamp_millis().saturating_sub(retention_ms);
    replay_repo
        .prune_recorded_before(cutoff_ms)
        .await
        .map_err(|error| {
            ApiError::InternalError(format!(
                "failed to prune external adapter replay state: {error}"
            ))
        })?;
    Ok(())
}

fn generic_webhook_signature(secret: &str, request: &GenericWebhookParseRequest) -> Result<String> {
    let event = serde_json::to_string(&request.event).map_err(|error| {
        ApiError::BadRequest(format!(
            "failed to serialize external adapter event: {error}"
        ))
    })?;
    let binding = serde_json::to_string(&request.binding).map_err(|error| {
        ApiError::BadRequest(format!(
            "failed to serialize external adapter binding: {error}"
        ))
    })?;
    let payload = format!(
        "{}.{}.{}.{}",
        request.replay_guard.timestamp_ms,
        request.replay_guard.nonce.trim(),
        event,
        binding
    );
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|error| {
        ApiError::BadRequest(format!(
            "failed to initialize external adapter signature: {error}"
        ))
    })?;
    mac.update(payload.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn normalize_signature(signature: &str) -> String {
    signature
        .trim()
        .strip_prefix("sha256=")
        .unwrap_or_else(|| signature.trim())
        .to_ascii_lowercase()
}

fn replay_record_from_request_and_response(
    request: &GenericWebhookParseRequest,
    response: &GenericWebhookParseResponse,
) -> ExternalAdapterReplayRecord {
    ExternalAdapterReplayRecord {
        adapter_id: response.adapter.clone(),
        source: response.source.as_str().to_string(),
        external_event_id: response
            .ingress
            .external_adapter
            .as_ref()
            .map(|external| external.external_event_id.clone())
            .unwrap_or_default(),
        idempotency_key: response.ingress.idempotency_key.clone().unwrap_or_default(),
        external_user_id: response
            .ingress
            .external_adapter
            .as_ref()
            .map(|external| external.external_user_id.clone())
            .unwrap_or_default(),
        external_conversation_id: response
            .ingress
            .external_adapter
            .as_ref()
            .map(|external| external.external_conversation_id.clone())
            .unwrap_or_default(),
        session_id: response.binding.session_id.clone(),
        actor_id: response.binding.actor_id.clone(),
        workspace_id: response.binding.workspace_id.clone(),
        nonce: response.replay_guard.nonce.clone(),
        signature_hash: sha256_hex(request.replay_guard.signature.trim().as_bytes()),
        received_at_ms: response.ingress.received_at_ms,
        recorded_at_ms: Utc::now().timestamp_millis(),
        status: "verified".to_string(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn external_adapter_validation_error(error: ExternalAdapterValidationError) -> ApiError {
    ApiError::BadRequest(error.to_string())
}

fn external_adapter_ingress_mapping_error(error: ExternalAdapterIngressMappingError) -> ApiError {
    ApiError::BadRequest(error.to_string())
}

fn require_non_blank(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(ApiError::BadRequest(format!("{field} is required")))
    } else {
        Ok(())
    }
}

async fn provision_generic_webhook_session(
    state: Arc<ServerState>,
    request: ProvisionExternalAdapterSessionRequest,
) -> Result<ProvisionExternalAdapterSessionResponse> {
    let config = state.config_store.config();
    let adapter_id = request.adapter_id.trim();
    require_non_blank("adapter_id", adapter_id)?;
    require_non_blank("actor_id", &request.actor_id)?;

    let adapter = configured_generic_webhook_adapter_by_id(&config, adapter_id)?;
    enforce_session_run_allowed(adapter, adapter_id)?;

    let workspace_id = request
        .workspace_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            adapter
                .default_workspace
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .ok_or_else(|| {
            ApiError::BadRequest(
                "external adapter session provisioning requires workspace_id or adapter default_workspace"
                    .to_string(),
            )
        })?;

    let preflight_binding = normalized_binding(&ExternalAdapterResolvedBinding {
        session_id: "__provision__".to_string(),
        actor_id: request.actor_id.clone(),
        workspace_id: workspace_id.clone(),
        route_policy_id: request.route_policy_id.clone(),
    });
    enforce_binding_policy(adapter, &preflight_binding)?;

    let session = create_session_from_spec(
        &state,
        CreateSessionSpec {
            scheduler_profile: request.scheduler_profile.clone(),
            directory: request.directory.clone(),
            project_id: request.project_id.clone(),
            title: request.title.clone(),
        },
    )
    .await?;

    let binding = normalized_binding(&ExternalAdapterResolvedBinding {
        session_id: session.record().id.clone(),
        actor_id: request.actor_id,
        workspace_id,
        route_policy_id: request.route_policy_id,
    });

    let session = {
        let mut sessions = state.sessions.lock().await;
        let Some(mut session) = sessions.get(&binding.session_id).cloned() else {
            return Err(ApiError::SessionNotFound(binding.session_id.clone()));
        };
        persist_verified_external_adapter_binding(&mut session, &binding);
        sessions.update(session.clone());
        session
    };
    state.sync_sessions_to_storage().await.map_err(|error| {
        ApiError::InternalError(format!(
            "failed to persist provisioned external adapter session: {error}"
        ))
    })?;

    Ok(ProvisionExternalAdapterSessionResponse {
        adapter: adapter_id.to_string(),
        source: ExternalAdapterSource::GenericWebhook,
        binding,
        session: session_to_info(&session),
    })
}

fn configured_generic_webhook_adapter_by_id<'a>(
    config: &'a Config,
    adapter_id: &str,
) -> Result<&'a ExternalAdapterEntryConfig> {
    configured_external_adapter_for_source(
        config,
        adapter_id,
        ExternalAdapterSource::GenericWebhook,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_config::{ConfigStore, ExternalAdapterConfig, ExternalAdapterEntryConfig};
    use agendao_provider::{AuthInfo, AuthManager};
    use agendao_storage::Database;
    use agendao_types::{ExternalAdapterAttachmentRef, ExternalAdapterReplyTarget};
    use std::collections::HashMap;

    fn sample_request() -> GenericWebhookParseRequest {
        GenericWebhookParseRequest {
            event: ExternalAdapterEvent {
                adapter_id: "generic".to_string(),
                source: ExternalAdapterSource::GenericWebhook,
                external_event_id: "evt_1".to_string(),
                external_user_id: "user_1".to_string(),
                external_conversation_id: "chat_1".to_string(),
                external_thread_id: None,
                received_at_ms: 1_714_000_000_000,
                text: "hello".to_string(),
                attachments: vec![ExternalAdapterAttachmentRef {
                    id: "file_1".to_string(),
                    kind: "image".to_string(),
                    uri: "agendao://external/generic/file_1".to_string(),
                }],
                idempotency_key: None,
                reply_target: Some(ExternalAdapterReplyTarget {
                    target_type: "chat".to_string(),
                    target_id: "chat_1".to_string(),
                    thread_id: None,
                }),
                raw_event_ref: None,
            },
            binding: ExternalAdapterResolvedBinding {
                session_id: "ses_1".to_string(),
                actor_id: "actor_1".to_string(),
                workspace_id: "ws_1".to_string(),
                route_policy_id: Some("default".to_string()),
            },
            replay_guard: GenericWebhookReplayGuardRef {
                timestamp_ms: 1_714_000_000_000,
                nonce: "nonce_1".to_string(),
                signature: "sig_1".to_string(),
            },
        }
    }

    fn configured_external_adapter() -> Config {
        configured_external_adapter_with_session_run(false)
    }

    fn configured_external_adapter_with_session_run(allow_session_run: bool) -> Config {
        Config {
            external_adapter: Some(ExternalAdapterConfig {
                adapters: HashMap::from([(
                    "generic".to_string(),
                    ExternalAdapterEntryConfig {
                        enabled: Some(true),
                        source: Some("generic-webhook".to_string()),
                        secret_ref: Some("external-adapter:generic".to_string()),
                        default_workspace: Some("ws_1".to_string()),
                        route_policy_id: Some("default".to_string()),
                        allow_session_run: Some(allow_session_run),
                        allowed_workspaces: vec!["ws_1".to_string()],
                    },
                )]),
                replay: None,
            }),
            ..Default::default()
        }
    }

    async fn configured_auth_manager() -> AuthManager {
        let auth = AuthManager::new();
        auth.set(
            "external-adapter:generic",
            AuthInfo::Api {
                key: "webhook-secret".to_string(),
            },
        )
        .await;
        auth
    }

    fn request_for_session(session_id: &str) -> GenericWebhookParseRequest {
        let mut request = sample_request();
        request.binding.session_id = session_id.to_string();
        request
    }

    async fn configured_session_run_state(
        config: Config,
    ) -> (
        Arc<ServerState>,
        Arc<ExternalAdapterReplayRepository>,
        String,
    ) {
        let db = Database::in_memory().await.unwrap();
        let repo = Arc::new(ExternalAdapterReplayRepository::new(db.pool().clone()));
        let mut state = ServerState::new();
        state.config_store = Arc::new(ConfigStore::new(config));
        state.external_adapter_replay_repo = Some(repo.clone());
        state
            .auth_manager
            .set(
                "external-adapter:generic",
                AuthInfo::Api {
                    key: "webhook-secret".to_string(),
                },
            )
            .await;
        let session_id = {
            let mut sessions = state.sessions.lock().await;
            sessions.create("default", ".").id.clone()
        };
        (Arc::new(state), repo, session_id)
    }

    async fn provisioned_session_run_state(
        config: Config,
    ) -> (
        Arc<ServerState>,
        Arc<ExternalAdapterReplayRepository>,
        String,
    ) {
        let (state, repo, _) = configured_session_run_state(config).await;
        let provisioned = provision_generic_webhook_session(
            state.clone(),
            ProvisionExternalAdapterSessionRequest {
                adapter_id: "generic".to_string(),
                actor_id: "actor_1".to_string(),
                workspace_id: None,
                route_policy_id: Some("default".to_string()),
                scheduler_profile: None,
                directory: None,
                project_id: None,
                title: None,
            },
        )
        .await
        .expect("session should provision");
        (state, repo, provisioned.binding.session_id)
    }

    fn sign_request(request: &mut GenericWebhookParseRequest) {
        request.replay_guard.signature = generic_webhook_signature("webhook-secret", request)
            .expect("test signature should be generated");
    }

    #[test]
    fn parse_generic_webhook_is_dry_run_only() {
        let response = parse_generic_webhook_request(sample_request(), true).unwrap();

        assert!(response.dry_run);
        assert!(!response.would_enqueue);
        assert_eq!(response.ingress.session_id, "ses_1");
        assert_eq!(response.ingress.user_intent_text, "hello");
        assert_eq!(response.replay_guard.status, "parse_only_not_verified");
        assert_eq!(
            response.authority.replay_store,
            "agendao-storage.external_adapter_replay"
        );
    }

    #[test]
    fn parse_generic_webhook_requires_generic_webhook_source() {
        let mut request = sample_request();
        request.event.source = ExternalAdapterSource::Cron;

        let error = parse_generic_webhook_request(request, true).unwrap_err();

        assert!(matches!(error, ApiError::BadRequest(_)));
    }

    #[test]
    fn parse_generic_webhook_requires_resolved_binding() {
        let mut request = sample_request();
        request.binding.session_id = " ".to_string();

        let error = parse_generic_webhook_request(request, true).unwrap_err();

        assert!(matches!(error, ApiError::BadRequest(_)));
    }

    #[test]
    fn parse_generic_webhook_requires_replay_guard_signature_material() {
        let mut request = sample_request();
        request.replay_guard.signature = " ".to_string();

        let error = parse_generic_webhook_request(request, true).unwrap_err();

        assert!(matches!(error, ApiError::BadRequest(_)));
    }

    #[test]
    fn parse_generic_webhook_rejects_unknown_request_fields() {
        let payload = serde_json::json!({
            "event": {
                "adapter_id": "generic",
                "source": "generic-webhook",
                "external_event_id": "evt_1",
                "external_user_id": "user_1",
                "external_conversation_id": "chat_1",
                "received_at_ms": 1,
                "text": "hello"
            },
            "binding": {
                "session_id": "ses_1",
                "actor_id": "actor_1",
                "workspace_id": "ws_1"
            },
            "replay_guard": {
                "timestamp_ms": 1,
                "nonce": "nonce_1",
                "signature": "sig_1"
            },
            "execute_now": true
        });

        let result = serde_json::from_value::<GenericWebhookParseRequest>(payload);

        assert!(result.is_err());
    }

    #[test]
    fn parse_generic_webhook_keeps_external_metadata_out_of_shadow_text() {
        let response = parse_generic_webhook_request(sample_request(), false).unwrap();

        assert_eq!(response.ingress.user_intent_text, "hello");
        assert!(!response.ingress.user_intent_text.contains("user_1"));
        assert!(!response.ingress.user_intent_text.contains("sig_1"));
        assert_eq!(response.ingress.attachments.len(), 1);
        assert_eq!(
            response.authority.replay_store,
            "unavailable: storage repository not configured"
        );
    }

    #[test]
    fn session_run_gate_is_fail_closed() {
        let adapter = ExternalAdapterEntryConfig {
            allow_session_run: None,
            ..Default::default()
        };

        let error = enforce_session_run_allowed(&adapter, "generic").unwrap_err();

        assert!(matches!(error, ApiError::BadRequest(_)));

        let adapter = ExternalAdapterEntryConfig {
            allow_session_run: Some(true),
            ..Default::default()
        };
        assert!(enforce_session_run_allowed(&adapter, "generic").is_ok());
    }

    #[test]
    fn run_prompt_request_is_derived_from_verified_ingress_only() {
        let response = parse_generic_webhook_request(sample_request(), true).unwrap();

        let prompt_request = SessionPromptRequest::from_verified_ingress(&response.ingress);

        assert_eq!(prompt_request.message.as_deref(), Some("hello"));
        assert_eq!(
            prompt_request.idempotency_key.as_deref(),
            Some("external:generic:generic-webhook:evt_1")
        );
        assert!(prompt_request.parts.is_none());
        assert!(prompt_request.ingress_source.is_none());
        assert!(prompt_request.scheduler_profile.is_none());
        assert!(prompt_request.agent.is_none());
    }

    #[test]
    fn external_adapter_validation_reports_missing_default_workspace_for_session_run() {
        let reports = collect_external_adapter_config_validation(&Config {
            external_adapter: Some(ExternalAdapterConfig {
                adapters: HashMap::from([(
                    "generic".to_string(),
                    ExternalAdapterEntryConfig {
                        enabled: Some(true),
                        source: Some("generic-webhook".to_string()),
                        secret_ref: Some("external-adapter:generic".to_string()),
                        allow_session_run: Some(true),
                        ..Default::default()
                    },
                )]),
                replay: None,
            }),
            ..Default::default()
        });

        let item = reports
            .iter()
            .find(|item| item.code == "external_adapter_missing_default_workspace")
            .expect("missing default workspace item");
        assert_eq!(
            item.path,
            "external_adapter.adapters.generic.default_workspace"
        );
    }

    #[tokio::test]
    async fn verify_generic_webhook_uses_config_auth_and_replay_store() {
        let db = Database::in_memory().await.unwrap();
        let repo = ExternalAdapterReplayRepository::new(db.pool().clone());
        let config = configured_external_adapter();
        let auth = configured_auth_manager().await;
        let mut request = sample_request();
        sign_request(&mut request);

        let response = verify_generic_webhook_request(request, &config, &auth, Some(&repo))
            .await
            .unwrap();

        assert!(response.dry_run);
        assert!(!response.would_enqueue);
        assert!(response.replay_guard.verified);
        assert_eq!(response.replay_guard.status, "verified_recorded");
        assert_eq!(
            response.authority.replay_store,
            "agendao-storage.external_adapter_replay:recorded"
        );

        let stored = repo
            .get_by_event("generic", "generic-webhook", "evt_1")
            .await
            .unwrap()
            .expect("verified request should record replay state");
        assert_eq!(stored.session_id, "ses_1");
        assert_eq!(stored.actor_id, "actor_1");
        assert_eq!(stored.workspace_id, "ws_1");
        assert_eq!(stored.status, "verified");
        assert_ne!(stored.signature_hash, "sig_1");
    }

    #[tokio::test]
    async fn run_generic_webhook_gate_rejects_before_replay_record() {
        let config = configured_external_adapter_with_session_run(false);
        let (state, repo, session_id) = configured_session_run_state(config).await;
        let mut request = request_for_session(&session_id);
        sign_request(&mut request);

        let error = verify_generic_webhook_for_session_run(request, state)
            .await
            .unwrap_err();

        assert!(matches!(error, ApiError::BadRequest(_)));
        assert!(repo
            .get_by_event("generic", "generic-webhook", "evt_1")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn run_generic_webhook_records_replay_before_runtime_input() {
        let config = configured_external_adapter_with_session_run(true);
        let (state, repo, session_id) = provisioned_session_run_state(config).await;
        let mut request = request_for_session(&session_id);
        sign_request(&mut request);

        let run = verify_generic_webhook_for_session_run(request, state)
            .await
            .unwrap();

        assert!(!run.verification.dry_run);
        assert!(run.verification.would_enqueue);
        assert_eq!(run.session_id, session_id);
        assert_eq!(run.prompt_request.message.as_deref(), Some("hello"));
        assert_eq!(
            run.prompt_request.idempotency_key.as_deref(),
            Some("external:generic:generic-webhook:evt_1")
        );
        assert_eq!(run.verified_ingress.ingress.session_id, session_id);
        assert!(run.verified_ingress.ingress.external_adapter.is_some());
        assert_eq!(
            run.verified_ingress
                .external_adapter_binding
                .as_ref()
                .map(|binding| binding.workspace_id.as_str()),
            Some("ws_1")
        );

        let stored = repo
            .get_by_event("generic", "generic-webhook", "evt_1")
            .await
            .unwrap()
            .expect("run verification should record replay before runtime input is used");
        assert_eq!(stored.status, "verified");
        assert_eq!(stored.session_id, session_id);
    }

    #[tokio::test]
    async fn provision_generic_webhook_session_creates_bound_session() {
        let config = configured_external_adapter_with_session_run(true);
        let (state, _, _) = configured_session_run_state(config).await;

        let response = provision_generic_webhook_session(
            state.clone(),
            ProvisionExternalAdapterSessionRequest {
                adapter_id: "generic".to_string(),
                actor_id: "actor_1".to_string(),
                workspace_id: None,
                route_policy_id: Some("default".to_string()),
                scheduler_profile: None,
                directory: None,
                project_id: None,
                title: Some("Webhook Session".to_string()),
            },
        )
        .await
        .unwrap();

        assert_eq!(response.adapter, "generic");
        assert_eq!(response.binding.actor_id, "actor_1");
        assert_eq!(response.binding.workspace_id, "ws_1");
        assert_eq!(response.session.title, "Webhook Session");

        let sessions = state.sessions.lock().await;
        let session = sessions
            .get(&response.binding.session_id)
            .expect("provisioned session should exist");
        assert_eq!(
            load_verified_external_adapter_binding(session)
                .as_ref()
                .map(|binding| binding.workspace_id.as_str()),
            Some("ws_1")
        );
    }

    #[tokio::test]
    async fn run_generic_webhook_rejects_unprovisioned_session() {
        let config = configured_external_adapter_with_session_run(true);
        let (state, repo, session_id) = configured_session_run_state(config).await;
        let mut request = request_for_session(&session_id);
        sign_request(&mut request);

        let error = verify_generic_webhook_for_session_run(request, state)
            .await
            .unwrap_err();

        assert!(matches!(error, ApiError::BadRequest(_)));
        assert!(repo
            .get_by_event("generic", "generic-webhook", "evt_1")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn run_generic_webhook_rejects_conflicting_existing_binding() {
        let config = configured_external_adapter_with_session_run(true);
        let (state, repo, session_id) = provisioned_session_run_state(config).await;
        {
            let mut sessions = state.sessions.lock().await;
            let mut session = sessions
                .get(&session_id)
                .cloned()
                .expect("session should exist");
            session.insert_metadata(
                "verified_external_adapter_binding".to_string(),
                serde_json::json!({
                    "session_id": session_id,
                    "actor_id": "actor_existing",
                    "workspace_id": "ws_1",
                    "route_policy_id": "default"
                }),
            );
            sessions.update(session);
        }
        let mut request = request_for_session(&session_id);
        sign_request(&mut request);

        let error = verify_generic_webhook_for_session_run(request, state)
            .await
            .unwrap_err();

        assert!(matches!(error, ApiError::BadRequest(_)));
        assert!(repo
            .get_by_event("generic", "generic-webhook", "evt_1")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn verify_generic_webhook_rejects_invalid_signature() {
        let db = Database::in_memory().await.unwrap();
        let repo = ExternalAdapterReplayRepository::new(db.pool().clone());
        let config = configured_external_adapter();
        let auth = configured_auth_manager().await;
        let mut request = sample_request();
        request.replay_guard.signature = "sha256:not-the-signature".to_string();

        let error = verify_generic_webhook_request(request, &config, &auth, Some(&repo))
            .await
            .unwrap_err();

        assert!(matches!(error, ApiError::BadRequest(_)));
        assert!(repo
            .get_by_event("generic", "generic-webhook", "evt_1")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn verify_generic_webhook_rejects_duplicate_replay() {
        let db = Database::in_memory().await.unwrap();
        let repo = ExternalAdapterReplayRepository::new(db.pool().clone());
        let config = configured_external_adapter();
        let auth = configured_auth_manager().await;
        let mut request = sample_request();
        sign_request(&mut request);

        verify_generic_webhook_request(request.clone(), &config, &auth, Some(&repo))
            .await
            .unwrap();
        let error = verify_generic_webhook_request(request, &config, &auth, Some(&repo))
            .await
            .unwrap_err();

        assert!(matches!(error, ApiError::BadRequest(_)));
    }

    #[tokio::test]
    async fn verify_generic_webhook_rejects_unconfigured_or_disallowed_binding() {
        let db = Database::in_memory().await.unwrap();
        let repo = ExternalAdapterReplayRepository::new(db.pool().clone());
        let config = configured_external_adapter();
        let auth = configured_auth_manager().await;
        let mut request = sample_request();
        request.binding.workspace_id = "other_ws".to_string();
        sign_request(&mut request);

        let error = verify_generic_webhook_request(request, &config, &auth, Some(&repo))
            .await
            .unwrap_err();

        assert!(matches!(error, ApiError::BadRequest(_)));
    }

    #[tokio::test]
    async fn verify_generic_webhook_rejects_timestamp_outside_nonce_window() {
        let db = Database::in_memory().await.unwrap();
        let repo = ExternalAdapterReplayRepository::new(db.pool().clone());
        let mut config = configured_external_adapter();
        config
            .external_adapter
            .as_mut()
            .expect("external adapter config")
            .replay = Some(agendao_config::ExternalAdapterReplayConfig {
            retention_seconds: None,
            nonce_window_seconds: Some(60),
        });
        let auth = configured_auth_manager().await;
        let mut request = sample_request();
        request.replay_guard.timestamp_ms = 1;
        sign_request(&mut request);

        let error = verify_generic_webhook_request(request, &config, &auth, Some(&repo))
            .await
            .unwrap_err();

        assert!(matches!(error, ApiError::BadRequest(_)));
    }

    #[tokio::test]
    async fn verify_generic_webhook_prunes_old_replay_rows_using_retention() {
        let db = Database::in_memory().await.unwrap();
        let repo = ExternalAdapterReplayRepository::new(db.pool().clone());
        let mut config = configured_external_adapter();
        config
            .external_adapter
            .as_mut()
            .expect("external adapter config")
            .replay = Some(agendao_config::ExternalAdapterReplayConfig {
            retention_seconds: Some(60),
            nonce_window_seconds: None,
        });
        let auth = configured_auth_manager().await;

        repo.record(&ExternalAdapterReplayRecord {
            adapter_id: "generic".to_string(),
            source: "generic-webhook".to_string(),
            external_event_id: "evt_old".to_string(),
            idempotency_key: "external:generic:generic-webhook:evt_old".to_string(),
            external_user_id: "user_1".to_string(),
            external_conversation_id: "chat_1".to_string(),
            session_id: "ses_old".to_string(),
            actor_id: "actor_old".to_string(),
            workspace_id: "ws_1".to_string(),
            nonce: "nonce_old".to_string(),
            signature_hash: "sha256:old".to_string(),
            received_at_ms: 1,
            recorded_at_ms: Utc::now().timestamp_millis() - 120_000,
            status: "verified".to_string(),
        })
        .await
        .unwrap();

        let mut request = sample_request();
        sign_request(&mut request);
        verify_generic_webhook_request(request, &config, &auth, Some(&repo))
            .await
            .unwrap();

        assert!(repo
            .get_by_event("generic", "generic-webhook", "evt_old")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn verify_generic_webhook_requires_auth_manager_secret() {
        let db = Database::in_memory().await.unwrap();
        let repo = ExternalAdapterReplayRepository::new(db.pool().clone());
        let config = configured_external_adapter();
        let auth = AuthManager::new();
        let mut request = sample_request();
        sign_request(&mut request);

        let error = verify_generic_webhook_request(request, &config, &auth, Some(&repo))
            .await
            .unwrap_err();

        assert!(matches!(error, ApiError::BadRequest(_)));
    }
}
