use frontend_state_types::{
    CliFrontendProjection, CliMcpServerStatus, CliModelCatalogEntry, CliVisibleTranscript,
    CliSessionTokenStats,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CliRecentSessionInfo {
    title: Option<String>,
    model_label: Option<String>,
    preset_label: Option<String>,
}

#[derive(Clone)]
enum CliActiveAbortHandle {
    /// Server-side execution — abort via HTTP POST.
    Server {
        api_client: Arc<CliApiClient>,
        session_id: String,
    },
}

enum CliDispatchInput {
    Line(String),
    Eof,
}

#[derive(Debug, Clone, Default)]
struct CliSchedulerResolution {
    defaults: Option<SchedulerRequestDefaults>,
    profile_model: Option<(String, String)>,
}

fn resolve_scheduler_profile_config(
    config: &Config,
    requested_scheduler_profile: Option<&str>,
) -> anyhow::Result<Option<(String, SchedulerProfileConfig)>> {
    let requested = requested_scheduler_profile
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let scheduler_path = config
        .scheduler_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(name) = requested {
        if name == AUTO_SCHEDULER_PROFILE_NAME {
            return Ok(Some((name.to_string(), scheduler_auto_profile_config())));
        }

        if let Ok(preset) = SchedulerPresetKind::from_str(name) {
            return Ok(Some((
                name.to_string(),
                SchedulerProfileConfig {
                    orchestrator: Some(preset.as_str().to_string()),
                    ..Default::default()
                },
            )));
        }

        let path = scheduler_path.ok_or_else(|| {
            anyhow::anyhow!(
                "Scheduler profile could not be resolved: `{}`. No scheduler config is configured.",
                name
            )
        })?;
        let scheduler_config = SchedulerConfig::load_from_file(path).map_err(|error| {
            anyhow::anyhow!(
                "Scheduler profile could not be resolved: `{}`. Failed to load scheduler config: {}",
                name,
                error
            )
        })?;
        let profile = scheduler_config.profile(name).map_err(|error| {
            anyhow::anyhow!(
                "Scheduler profile could not be resolved: `{}`. {}",
                name,
                error
            )
        })?;
        return Ok(Some((name.to_string(), profile.clone())));
    }

    if let Some(path) = scheduler_path {
        let scheduler_config = match SchedulerConfig::load_from_file(path) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!(path = %path, %error, "failed to load scheduler config");
                return Ok(None);
            }
        };

        if let Some(name) = scheduler_config.default_profile_key() {
            if let Ok(profile) = scheduler_config.profile(name) {
                return Ok(Some((name.to_string(), profile.clone())));
            }
        }
        return Ok(None);
    }

    Ok(None)
}

fn resolve_scheduler_runtime(
    config: &Config,
    requested_scheduler_profile: Option<&str>,
) -> anyhow::Result<CliSchedulerResolution> {
    let Some((profile_name, profile)) =
        resolve_scheduler_profile_config(config, requested_scheduler_profile)?
    else {
        return Ok(CliSchedulerResolution::default());
    };

    let defaults = match scheduler_plan_from_profile(Some(profile_name.clone()), &profile) {
        Ok(plan) => Some(scheduler_request_defaults_from_plan(&plan)),
        Err(error) => {
            if requested_scheduler_profile
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some()
            {
                return Err(anyhow::anyhow!(
                    "Scheduler profile could not be resolved: `{}`. Failed to build profile plan: {}",
                    profile_name,
                    error
                ));
            }
            tracing::warn!(
                profile = %profile_name,
                %error,
                "failed to build default scheduler profile plan"
            );
            None
        }
    };
    let profile_model = profile
        .model
        .as_ref()
        .map(|model| (model.provider_id.clone(), model.model_id.clone()));

    Ok(CliSchedulerResolution {
        defaults,
        profile_model,
    })
}

async fn build_cli_execution_runtime(
    input: CliRuntimeBuildInput<'_>,
) -> anyhow::Result<CliExecutionRuntime> {
    let CliRuntimeBuildInput {
        config,
        agent_registry,
        selection,
        working_dir,
    } = input;
    let observed_topology = Arc::new(Mutex::new(CliObservedExecutionTopology::default()));
    let frontend_projection = Arc::new(Mutex::new(CliFrontendProjection::default()));
    let scheduler_stage_snapshots = Arc::new(Mutex::new(HashMap::new()));
    let scheduler_resolution =
        resolve_scheduler_runtime(config, selection.requested_scheduler_profile.as_deref())?;
    let scheduler_defaults = scheduler_resolution.defaults.clone();
    let scheduler_profile_name = scheduler_defaults
        .as_ref()
        .and_then(|defaults| defaults.profile_name.clone());
    let scheduler_root_agent = scheduler_defaults
        .as_ref()
        .and_then(|defaults| defaults.root_agent_name.clone());
    let agent_name = resolve_requested_agent_name(
        config,
        selection.requested_agent.as_deref(),
        scheduler_defaults.as_ref(),
    );

    let mut agent_info = agent_registry
        .get(&agent_name)
        .cloned()
        .unwrap_or_else(AgentInfo::build);

    if let Some(ref model_id) = selection.model {
        let provider_id = selection
            .provider
            .clone()
            .unwrap_or_else(|| "openai".to_string());
        agent_info = agent_info.with_model(model_id.clone(), provider_id);
    } else if let Some((provider_id, model_id)) = scheduler_resolution.profile_model.clone() {
        agent_info = agent_info.with_model(model_id, provider_id);
    }

    let resolved_model_label = agent_info
        .model
        .as_ref()
        .map(|m| format!("{}/{}", m.provider_id, m.model_id))
        .unwrap_or_else(|| "auto".to_string());
    if let Ok(mut projection) = frontend_projection.lock() {
        projection.current_model_label = Some(resolved_model_label.clone());
    }

    // Shared spinner guard slot — closures capture this; process_message_with_mode
    // swaps in the real spinner's guard each cycle.
    let spinner_guard: Arc<std::sync::Mutex<SpinnerGuard>> =
        Arc::new(std::sync::Mutex::new(SpinnerGuard::noop()));
    let prompt_session_slot: Arc<std::sync::Mutex<Option<Arc<PromptSession>>>> =
        Arc::new(std::sync::Mutex::new(None));

    tracing::info!(
        requested_agent = ?selection.requested_agent,
        requested_scheduler_profile = ?selection.requested_scheduler_profile,
        resolved_agent = %agent_name,
        scheduler_profile = ?scheduler_profile_name,
        scheduler_root_agent = ?scheduler_root_agent,
        resolved_model = %resolved_model_label,
        "resolved cli runtime execution configuration"
    );

    Ok(CliExecutionRuntime {
        resolved_agent_name: agent_name,
        scheduler_profile_name,
        resolved_model_label,
        working_dir,
        observed_topology,
        frontend_projection,
        scheduler_stage_snapshots,
        terminal_surface: None,
        prompt_chrome: None,
        prompt_session: None,
        prompt_session_slot,
        queued_inputs: Arc::new(AsyncMutex::new(VecDeque::new())),
        busy_flag: Arc::new(AtomicBool::new(false)),
        exit_requested: Arc::new(AtomicBool::new(false)),
        active_abort: Arc::new(AsyncMutex::new(None)),
        recovery_base_prompt: None,
        spinner_guard,
        api_client: None,
        server_session_id: None,
        related_session_ids: Arc::new(Mutex::new(BTreeSet::new())),
        root_session_transcript: Arc::new(Mutex::new(CliVisibleTranscript::default())),
        attached_session_transcripts: Arc::new(Mutex::new(HashMap::new())),
        stream_accumulators: Arc::new(Mutex::new(HashMap::new())),
        render_states: Arc::new(Mutex::new(HashMap::new())),
        focused_session_id: Arc::new(Mutex::new(None)),
        show_thinking: Arc::new(AtomicBool::new(selection.show_thinking)),
    })
}

fn cli_available_presets(config: &Config) -> Vec<String> {
    let mut names = BTreeSet::new();
    names.insert(AUTO_SCHEDULER_PROFILE_NAME.to_string());
    for preset in SchedulerPresetKind::public_presets() {
        names.insert(preset.as_str().to_string());
    }

    if let Some(path) = config
        .scheduler_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Ok(scheduler_config) = SchedulerConfig::load_from_file(path) {
            for name in scheduler_config.profiles.keys() {
                names.insert(name.clone());
            }
        }
    }

    names.into_iter().collect()
}

fn cli_list_presets(
    config: &Config,
    active_profile: Option<&str>,
    runtime: Option<&CliExecutionRuntime>,
) {
    let style = CliStyle::detect();
    let lines = cli_available_presets(config)
        .into_iter()
        .map(|preset| {
            let active = if active_profile == Some(preset.as_str()) {
                format!(" {}", style.bold_green("← active"))
            } else {
                String::new()
            };
            format!("{preset}{active}")
        })
        .collect::<Vec<_>>();
    let _ = print_cli_list_on_surface(runtime, "Available Presets", None, &lines, &style);
}

fn cli_mode_label(runtime: &CliExecutionRuntime) -> String {
    match runtime.scheduler_profile_name.as_deref() {
        Some(profile) => format!("Preset {}", profile),
        None => format!("Agent {}", runtime.resolved_agent_name),
    }
}

fn cli_session_hint_string(
    session: &crate::api_client::SessionListItem,
    key: &str,
) -> Option<String> {
    let hints = session.hints.as_ref()?;
    let value = match key {
        "current_model" => hints.current_model.as_deref(),
        "model_provider" => hints.model_provider.as_deref(),
        "model_id" => hints.model_id.as_deref(),
        "scheduler_profile" => hints.scheduler_profile.as_deref(),
        "agent" => hints.agent.as_deref(),
        _ => None,
    }?;

    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn cli_recent_session_info_for_directory(
    sessions: &[crate::api_client::SessionListItem],
    current_dir: &Path,
) -> Option<CliRecentSessionInfo> {
    let current_dir = current_dir.display().to_string();
    let session = sessions
        .iter()
        .filter(|session| session.directory == current_dir)
        .max_by_key(|session| session.time.updated)
        .or_else(|| sessions.iter().max_by_key(|session| session.time.updated))?;

    let model_label = cli_session_hint_string(session, "current_model").or_else(|| {
        cli_session_hint_string(session, "model_provider")
            .zip(cli_session_hint_string(session, "model_id"))
            .map(|(provider, model)| format!("{provider}/{model}"))
    });
    let preset_label = cli_session_hint_string(session, "scheduler_profile").or_else(|| {
        cli_session_hint_string(session, "agent").map(|agent| format!("agent:{agent}"))
    });
    let title = (!session.title.trim().is_empty()).then(|| session.title.trim().to_string());

    Some(CliRecentSessionInfo {
        title,
        model_label,
        preset_label,
    })
}

async fn cli_load_recent_session_info(
    api_client: &CliApiClient,
    current_dir: &Path,
) -> Option<CliRecentSessionInfo> {
    let sessions = api_client.list_sessions(None, Some(20)).await.ok()?;
    cli_recent_session_info_for_directory(&sessions, current_dir)
}

fn cli_render_startup_banner(style: &CliStyle, recent: Option<&CliRecentSessionInfo>) -> String {
    let mut out = String::new();
    out.push_str("\r\n");

    for (idx, line) in logo_lines("").into_iter().enumerate() {
        let rendered = if idx == 0 {
            style.bold_rgb(&line, 94, 196, 255)
        } else {
            style.rgb(&line, 145, 167, 196)
        };
        out.push_str(&rendered);
        out.push_str("\r\n");
    }

    out.push_str(&style.dim(&format!(
        "{APP_SHORT_NAME} {APP_VERSION_DATE} · {APP_TAGLINE}"
    )));
    out.push_str("\r\n");

    if let Some(recent) = recent {
        if let Some(title) = recent.title.as_deref() {
            out.push_str(&style.bold("Last session: "));
            out.push_str(title);
            out.push_str("\r\n");
        }
        out.push_str(&style.bold("Last model: "));
        out.push_str(recent.model_label.as_deref().unwrap_or("—"));
        out.push_str("\r\n");
        out.push_str(&style.bold("Last preset: "));
        out.push_str(recent.preset_label.as_deref().unwrap_or("—"));
        out.push_str("\r\n");
    }

    out.push_str("\r\n");
    out
}

#[cfg(test)]
mod frontend_state_tests {
    use super::resolve_scheduler_runtime;
    use rocode_config::Config;

    #[test]
    fn explicit_unknown_scheduler_profile_fails_instead_of_silent_fallback() {
        let error = resolve_scheduler_runtime(&Config::default(), Some("missing"))
            .expect_err("explicitly requested unknown scheduler profile should fail");
        let message = error.to_string();

        assert!(message.contains("Scheduler profile could not be resolved"));
        assert!(message.contains("missing"));
    }

    #[test]
    fn no_requested_scheduler_profile_keeps_default_cli_behavior() {
        let resolution = resolve_scheduler_runtime(&Config::default(), None)
            .expect("missing scheduler config without explicit request should not fail");

        assert!(resolution.defaults.is_none());
        assert!(resolution.profile_model.is_none());
    }

    #[test]
    fn explicit_auto_scheduler_profile_resolves_to_router_defaults() {
        let resolution = resolve_scheduler_runtime(&Config::default(), Some("auto"))
            .expect("built-in auto scheduler profile should resolve");
        let defaults = resolution
            .defaults
            .expect("auto should apply scheduler defaults");

        assert_eq!(defaults.profile_name.as_deref(), Some("auto"));
        assert!(defaults.root_agent_name.is_none());
    }
}
