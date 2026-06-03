//! Shared event-loop helpers for compact and rich interactive modes.
//! Renamed from compact_legacy_sse.rs (P1 cleanup) — the "legacy" label
//! was misleading; these functions remain the active main-path event loop.
use super::*;

pub(super) async fn wait_for_rich_input(
    runtime: &mut CliExecutionRuntime,
    config: &Config,
    agent_registry: &AgentRegistry,
    api_client: &Arc<CliApiClient>,
    local_state: &Option<Arc<agendao_server::ServerState>>,
    dispatch_rx: &mut mpsc::UnboundedReceiver<CliDispatchInput>,
    sse_rx: &mut mpsc::UnboundedReceiver<CliServerEvent>,
    repl_style: &CliStyle,
) -> anyhow::Result<Option<String>> {
    loop {
        tokio::select! {
            dispatch = dispatch_rx.recv() => {
                match dispatch {
                    Some(CliDispatchInput::Line(line)) => return Ok(Some(line)),
                    Some(CliDispatchInput::ModeCycle { reverse }) => {
                        cli_cycle_prompt_mode(runtime, config, agent_registry, reverse);
                    }
                    Some(CliDispatchInput::Eof) | None => return Ok(None),
                }
            }
            sse_event = sse_rx.recv() => {
                if let Some(event) = sse_event {
                    handle_interactive_event(runtime, api_client, local_state, event, repl_style).await;
                }
            }
        }
    }
}

pub(super) async fn drain_available_events(
    runtime: &CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    local_state: &Option<Arc<agendao_server::ServerState>>,
    sse_rx: &mut mpsc::UnboundedReceiver<CliServerEvent>,
    repl_style: &CliStyle,
) {
    super::prompt_shared::drain_sse_events(sse_rx, |event| {
        handle_interactive_event(runtime, api_client, local_state, event, repl_style)
    })
    .await;
}

async fn handle_interactive_event(
    runtime: &CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    local_state: &Option<Arc<agendao_server::ServerState>>,
    event: CliServerEvent,
    repl_style: &CliStyle,
) {
    match event {
        CliServerEvent::ConfigUpdated => {
            cli_handle_config_updated_from_sse(runtime, api_client).await;
        }
        CliServerEvent::QuestionCreated {
            request_id,
            session_id: _,
            questions_json,
        } => {
            handle_question_from_sse(runtime, api_client, local_state, &request_id, &questions_json).await;
        }
        CliServerEvent::PermissionRequested {
            session_id,
            permission_id,
            info_json,
        } => {
            if cli_tracks_related_session(runtime, &session_id) {
                handle_permission_from_sse(runtime, api_client, local_state, &permission_id, &info_json).await;
            }
        }
        other => {
            handle_sse_event(runtime, other, repl_style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_agent::AgentRegistry;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn wait_for_rich_input_applies_mode_cycle_before_returning_line() {
        let config = Config::default();
        let agent_registry = Arc::new(AgentRegistry::from_config(&config));
        let modes = cli_prompt_modes(&config, agent_registry.as_ref());
        assert!(modes.len() >= 2, "expected at least two execution modes");

        let initial_agent = modes
            .iter()
            .find_map(|mode| match mode {
                CliPromptModeEntry::Agent(agent) => Some(agent.clone()),
                CliPromptModeEntry::Preset(_) => None,
            })
            .expect("expected at least one agent mode");

        let mut runtime = build_cli_execution_runtime(CliRuntimeBuildInput {
            config: &config,
            agent_registry: agent_registry.clone(),
            selection: &CliRunSelection {
                requested_agent: Some(initial_agent),
                ..CliRunSelection::default()
            },
            working_dir: std::env::current_dir().expect("cwd"),
        })
        .await
        .expect("build runtime");

        match &modes[0] {
            CliPromptModeEntry::Agent(agent) => {
                runtime.resolved_agent_name = agent.clone();
                runtime.scheduler_profile_name = None;
            }
            CliPromptModeEntry::Preset(profile) => {
                runtime.scheduler_profile_name = Some(profile.clone());
            }
        }

        let api_client = Arc::new(CliApiClient::new("http://127.0.0.1:0".to_string()));
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel();
        let (_sse_tx, mut sse_rx) = mpsc::unbounded_channel();
        dispatch_tx
            .send(CliDispatchInput::ModeCycle { reverse: false })
            .expect("send mode cycle");
        dispatch_tx
            .send(CliDispatchInput::Line("hello".to_string()))
            .expect("send line");

        let line = wait_for_rich_input(
            &mut runtime,
            &config,
            agent_registry.as_ref(),
            &api_client,
            &None,
            &mut dispatch_rx,
            &mut sse_rx,
            &CliStyle::plain(),
        )
        .await
        .expect("wait for rich input");

        assert_eq!(line.as_deref(), Some("hello"));
        match &modes[1] {
            CliPromptModeEntry::Agent(agent) => {
                assert_eq!(runtime.resolved_agent_name, *agent);
                assert!(runtime.scheduler_profile_name.is_none());
            }
            CliPromptModeEntry::Preset(profile) => {
                assert_eq!(
                    runtime.scheduler_profile_name.as_deref(),
                    Some(profile.as_str())
                );
            }
        }
    }
}
