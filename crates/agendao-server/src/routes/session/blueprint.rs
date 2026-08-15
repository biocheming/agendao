use agendao_api::{
    RejectSessionBlueprintResponse, SessionBlueprintView, SetSessionBlueprintRequest,
};
use axum::extract::{Path, State};
use axum::Json;
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::error::{ApiError, Result};
use crate::scheduler_runner::{
    selection_source_name, validate_user_blueprint, BLUEPRINT_FINGERPRINT_METADATA_KEY,
    BLUEPRINT_LOCK_METADATA_KEY, REJECTED_BLUEPRINTS_METADATA_KEY, SELECTION_SOURCE_METADATA_KEY,
};
use crate::ServerState;

use super::session_crud::persist_session_if_enabled;

pub(super) async fn get_session_blueprint(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionBlueprintView>> {
    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    let metadata = &session.record().metadata;
    let blueprint = metadata
        .get(BLUEPRINT_LOCK_METADATA_KEY)
        .cloned()
        .ok_or_else(|| ApiError::NotFound("session has no effective Blueprint".to_string()))?;
    let blueprint = serde_json::from_value(blueprint)
        .map_err(|error| ApiError::InternalError(format!("invalid Blueprint metadata: {error}")))?;
    let fingerprint = metadata
        .get(BLUEPRINT_FINGERPRINT_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ApiError::InternalError("Blueprint fingerprint is missing".to_string()))?
        .to_string();
    let selection_source = metadata
        .get(SELECTION_SOURCE_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ApiError::InternalError("Blueprint selection source is missing".to_string())
        })?
        .to_string();
    Ok(Json(SessionBlueprintView {
        blueprint,
        fingerprint,
        selection_source,
    }))
}

pub(super) async fn set_session_blueprint(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(request): Json<SetSessionBlueprintRequest>,
) -> Result<Json<SessionBlueprintView>> {
    let validated = validate_user_blueprint(&state, request.blueprint)
        .await
        .map_err(ApiError::BadRequest)?;
    let view = SessionBlueprintView {
        blueprint: validated.blueprint().clone(),
        fingerprint: validated.fingerprint().to_string(),
        selection_source: selection_source_name(
            agendao_orchestrator::selector::SelectionSource::User,
        )
        .to_string(),
    };
    {
        let mut sessions = state.sessions.lock().await;
        let session = sessions
            .get_mut(&id)
            .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
        session.insert_metadata(
            BLUEPRINT_LOCK_METADATA_KEY,
            serde_json::to_value(&view.blueprint)
                .map_err(|error| ApiError::InternalError(error.to_string()))?,
        );
        session.insert_metadata(
            BLUEPRINT_FINGERPRINT_METADATA_KEY,
            serde_json::json!(&view.fingerprint),
        );
        session.insert_metadata(
            SELECTION_SOURCE_METADATA_KEY,
            serde_json::json!(&view.selection_source),
        );
    }
    persist_session_if_enabled(&state, &id).await;
    Ok(Json(view))
}

pub(super) async fn reject_session_blueprint(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<RejectSessionBlueprintResponse>> {
    let rejected_fingerprint = {
        let mut sessions = state.sessions.lock().await;
        let session = sessions
            .get_mut(&id)
            .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
        let source = session
            .record()
            .metadata
            .get(SELECTION_SOURCE_METADATA_KEY)
            .and_then(serde_json::Value::as_str);
        if source != Some("planner") {
            return Err(ApiError::BadRequest(
                "only an AI-planned Blueprint can be rejected".to_string(),
            ));
        }
        let fingerprint = session
            .record()
            .metadata
            .get(BLUEPRINT_FINGERPRINT_METADATA_KEY)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ApiError::InternalError("Blueprint fingerprint is missing".to_string()))?
            .to_string();
        let mut rejected = session
            .record()
            .metadata
            .get(REJECTED_BLUEPRINTS_METADATA_KEY)
            .cloned()
            .map(serde_json::from_value::<BTreeSet<String>>)
            .transpose()
            .map_err(|error| {
                ApiError::InternalError(format!("invalid rejected Blueprint metadata: {error}"))
            })?
            .unwrap_or_default();
        rejected.insert(fingerprint.clone());
        session.insert_metadata(
            REJECTED_BLUEPRINTS_METADATA_KEY,
            serde_json::to_value(rejected)
                .map_err(|error| ApiError::InternalError(error.to_string()))?,
        );
        session.remove_metadata(BLUEPRINT_LOCK_METADATA_KEY);
        session.remove_metadata(BLUEPRINT_FINGERPRINT_METADATA_KEY);
        session.remove_metadata(SELECTION_SOURCE_METADATA_KEY);
        fingerprint
    };
    persist_session_if_enabled(&state, &id).await;
    Ok(Json(RejectSessionBlueprintResponse {
        rejected_fingerprint,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_orchestrator::blueprint::{
        AgentId, AgentNode, BlueprintName, BlueprintSchemaVersion, EndNode, ExecutionLimits,
        NodeId, NodeSpec, OutputContract, OutputFormat, ResultSource, SchedulerBlueprint,
    };
    use std::collections::BTreeMap;

    fn valid_user_blueprint() -> SchedulerBlueprint {
        SchedulerBlueprint {
            schema: BlueprintSchemaVersion::V1,
            name: BlueprintName::from("user-edited"),
            entry: NodeId::from("execute"),
            nodes: BTreeMap::from([
                (
                    NodeId::from("execute"),
                    NodeSpec::Agent(AgentNode {
                        agent: AgentId::from("build"),
                        skills: BTreeSet::new(),
                        tools: BTreeSet::new(),
                        required_model_capabilities: BTreeSet::new(),
                        max_steps: 4,
                        next: NodeId::from("done"),
                    }),
                ),
                (
                    NodeId::from("done"),
                    NodeSpec::End(EndNode {
                        result: ResultSource::LastNode,
                    }),
                ),
            ]),
            limits: ExecutionLimits {
                max_model_calls: 8,
                max_tool_calls: 8,
                max_total_tokens: 16_384,
                max_wall_time_ms: 60_000,
                max_parallelism: 1,
                max_graph_nodes: 4,
                max_graph_depth: 4,
                max_loop_iterations: 1,
                max_agent_steps: 4,
            },
            output: OutputContract {
                format: OutputFormat::Markdown,
                include_usage: true,
                include_artifact_refs: true,
            },
        }
    }

    #[tokio::test]
    async fn editing_and_inspecting_blueprint_uses_canonical_validator() {
        let state = Arc::new(ServerState::new());
        let session = agendao_session::Session::new("project", ".");
        let id = session.id.clone();
        state.sessions.lock().await.update(session);

        let blueprint = valid_user_blueprint();
        let Json(saved) = set_session_blueprint(
            State(state.clone()),
            Path(id.clone()),
            Json(SetSessionBlueprintRequest {
                blueprint: blueprint.clone(),
            }),
        )
        .await
        .expect("save validated Blueprint");
        assert_eq!(saved.blueprint, blueprint);
        assert_eq!(saved.selection_source, "user");
        assert_eq!(saved.fingerprint.len(), 64);

        let Json(inspected) = get_session_blueprint(State(state.clone()), Path(id.clone()))
            .await
            .expect("inspect saved Blueprint");
        assert_eq!(inspected, saved);

        let mut invalid = valid_user_blueprint();
        let NodeSpec::Agent(agent) = invalid
            .nodes
            .get_mut(&NodeId::from("execute"))
            .expect("agent node")
        else {
            panic!("expected agent node");
        };
        agent.agent = AgentId::from("missing-agent");
        let error = set_session_blueprint(
            State(state.clone()),
            Path(id.clone()),
            Json(SetSessionBlueprintRequest { blueprint: invalid }),
        )
        .await
        .expect_err("invalid Blueprint must be rejected");
        assert!(matches!(error, ApiError::BadRequest(_)));

        let Json(after_rejection) = get_session_blueprint(State(state), Path(id))
            .await
            .expect("existing Blueprint remains intact");
        assert_eq!(after_rejection, saved);
    }

    #[tokio::test]
    async fn rejecting_planner_blueprint_clears_lock_and_records_fingerprint() {
        let state = Arc::new(ServerState::new());
        let mut session = agendao_session::Session::new("project", ".");
        let id = session.id.clone();
        session.insert_metadata(
            BLUEPRINT_LOCK_METADATA_KEY,
            serde_json::json!({"schema": "v1"}),
        );
        session.insert_metadata(
            BLUEPRINT_FINGERPRINT_METADATA_KEY,
            serde_json::json!("fingerprint-1"),
        );
        session.insert_metadata(SELECTION_SOURCE_METADATA_KEY, serde_json::json!("planner"));
        state.sessions.lock().await.update(session);

        let Json(response) = reject_session_blueprint(State(state.clone()), Path(id.clone()))
            .await
            .expect("reject planner Blueprint");
        assert_eq!(response.rejected_fingerprint, "fingerprint-1");

        let sessions = state.sessions.lock().await;
        let metadata = &sessions.get(&id).unwrap().record().metadata;
        assert!(!metadata.contains_key(BLUEPRINT_LOCK_METADATA_KEY));
        assert_eq!(
            serde_json::from_value::<BTreeSet<String>>(
                metadata[REJECTED_BLUEPRINTS_METADATA_KEY].clone()
            )
            .unwrap(),
            BTreeSet::from(["fingerprint-1".to_string()])
        );
    }
}
