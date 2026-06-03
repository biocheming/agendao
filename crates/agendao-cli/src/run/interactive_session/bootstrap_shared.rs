use super::*;

pub(super) struct InteractiveSessionBootstrap {
    pub(super) working_dir: PathBuf,
    pub(super) config: Config,
    pub(super) command_registry: CommandRegistry,
    pub(super) provider_registry: Arc<ProviderRegistry>,
    pub(super) agent_registry: Arc<AgentRegistry>,
    pub(super) api_client: Arc<CliApiClient>,
    pub(super) recent_session_info: Option<CliRecentSessionInfo>,
    pub(super) selection: CliRunSelection,
    pub(super) runtime: CliExecutionRuntime,
    pub(super) repl_style: CliStyle,
    pub(super) server_url: String,
    pub(super) server_session_id: String,
    pub(super) local_state: Option<Arc<agendao_server::ServerState>>,
    pub(super) transport: Option<Arc<agendao_client::FrontendTransport>>,
}

pub(super) async fn bootstrap_interactive_session(
    model: Option<String>,
    provider: Option<String>,
    requested_agent: Option<String>,
    requested_scheduler_profile: Option<String>,
    thinking_requested: bool,
    port_override: Option<u16>,
    working_dir: PathBuf,
    runtime_context: &FrontendRuntimeContext,
    local: bool,
    unix_socket: Option<String>,
) -> anyhow::Result<InteractiveSessionBootstrap> {
    let working_dir = working_dir.canonicalize().unwrap_or(working_dir);
    let config = load_config(&working_dir)?;
    let command_registry = CommandRegistry::new();

    let local_state: Option<Arc<agendao_server::ServerState>> = if local {
        eprintln!("Starting CLI interactive session in Direct (in-process) mode");
        Some(Arc::new(
            agendao_server::ServerState::new_with_storage_for_url_in_workspace(
                "http://127.0.0.1:0".to_string(),
                working_dir.clone(),
            )
            .await?,
        ))
    } else {
        None
    };

    let discovery_socket_path = unix_socket.clone();
    let server_discovery_handle = if local {
        None
    } else {
        let ctx = runtime_context.clone();
        let wd = working_dir.clone();
        Some(tokio::spawn(async move {
            ctx.discover_or_start_server_with_request(crate::ServerDiscoveryRequest {
                port_override,
                cwd: Some(wd),
                unix_socket_path: discovery_socket_path,
            })
            .await
        }))
    };

    let provider_registry = Arc::new(setup_providers_for_dir(&config, &working_dir).await?);

    if provider_registry.list().is_empty() {
        eprintln!("Error: No API keys configured.");
        println!("Set one of the following environment variables:");
        eprintln!("  - ANTHROPIC_API_KEY");
        eprintln!("  - OPENAI_API_KEY");
        eprintln!("  - OPENROUTER_API_KEY");
        eprintln!("  - GOOGLE_API_KEY");
        eprintln!("  - MISTRAL_API_KEY");
        eprintln!("  - GROQ_API_KEY");
        eprintln!("  - XAI_API_KEY");
        eprintln!("  - DEEPSEEK_API_KEY");
        eprintln!("  - COHERE_API_KEY");
        eprintln!("  - TOGETHER_API_KEY");
        eprintln!("  - PERPLEXITY_API_KEY");
        eprintln!("  - CEREBRAS_API_KEY");
        eprintln!("  - DEEPINFRA_API_KEY");
        eprintln!("  - VERCEL_API_KEY");
        eprintln!("  - GITLAB_TOKEN");
        eprintln!("  - GITHUB_COPILOT_TOKEN");
        eprintln!("  - GOOGLE_VERTEX_API_KEY + GOOGLE_VERTEX_PROJECT_ID + GOOGLE_VERTEX_LOCATION");
        std::process::exit(1);
    }

    let agent_registry = Arc::new(AgentRegistry::from_config(&config));
    let server_url = if let Some(handle) = server_discovery_handle {
        handle.await??
    } else {
        "http://127.0.0.1:0".to_string()
    };
    let api_client = Arc::new(CliApiClient::new(server_url.clone()));
    let transport = if local {
        None
    } else if let Some(socket_path) = unix_socket.as_deref() {
        agendao_client::transport::TransportSelector::new(
            Some(socket_path.to_string()),
            server_url.clone(),
            None,
        )
        .select_unix_required()
        .await
        .map(Arc::new)
        .map(Some)?
    } else {
        None
    };
    let server_context =
        crate::local_dispatch::get_workspace_context(&local_state, &transport, &api_client)
            .await
            .ok();
    let recent_session_info =
        cli_load_recent_session_info(&local_state, &transport, &api_client, &working_dir).await;
    let explicit_model_requested = model.is_some();
    let (recent_model, recent_provider) = if explicit_model_requested {
        (None, None)
    } else {
        server_context
            .as_ref()
            .and_then(|context| context.recent_models.first())
            .map(|entry| (Some(entry.model.clone()), Some(entry.provider.clone())))
            .unwrap_or((None, None))
    };

    let (carry_model, carry_provider) = recent_session_info
        .as_ref()
        .and_then(|info| info.model_label.clone())
        .map(|label| {
            let (p, m) = parse_model_and_provider(Some(label));
            (m, p)
        })
        .unwrap_or((None, None));

    let carry_preset = recent_session_info
        .as_ref()
        .and_then(|info| info.preset_label.as_deref())
        .and_then(|label| {
            if label.starts_with("agent:") {
                None
            } else {
                Some(label.to_string())
            }
        });

    let selection = CliRunSelection {
        model: model.or(recent_model).or(carry_model),
        provider: provider.or(recent_provider).or(carry_provider),
        requested_agent,
        requested_scheduler_profile: requested_scheduler_profile.or(carry_preset),
        show_thinking: cli_resolve_show_thinking(
            thinking_requested,
            server_context.as_ref().map(|context| &context.config),
            false,
        ),
    };

    let mut runtime = build_cli_execution_runtime(CliRuntimeBuildInput {
        config: &config,
        agent_registry: agent_registry.clone(),
        selection: &selection,
        working_dir: working_dir.clone(),
    })
    .await?;
    cli_save_recent_model_ref(
        &local_state,
        &transport,
        &api_client,
        &runtime.resolved_model_label,
    )
    .await;
    let repl_style = CliStyle::detect();

    let session_info = if let Some(ref state) = local_state {
        agendao_server::local_create_session(
            Arc::clone(state),
            agendao_client::CreateSessionRequest {
                scheduler_profile: selection.requested_scheduler_profile.clone(),
                directory: Some(cli_session_directory(&working_dir)),
                project_id: None,
                title: None,
            },
        )
        .await?
    } else {
        api_client
            .create_session(
                selection.requested_scheduler_profile.clone(),
                Some(cli_session_directory(&working_dir)),
            )
            .await?
    };
    let server_session_id = session_info.id.clone();
    runtime.api_client = Some(api_client.clone());
    cli_set_root_server_session(&mut runtime, server_session_id.clone());

    Ok(InteractiveSessionBootstrap {
        working_dir,
        config,
        command_registry,
        provider_registry,
        agent_registry,
        api_client,
        recent_session_info,
        selection,
        runtime,
        repl_style,
        server_url,
        server_session_id,
        local_state,
        transport,
    })
}
