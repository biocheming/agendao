pub(super) use super::*;

mod bootstrap_shared;
mod compact;
mod compact_legacy_sse;
mod prompt_shared;
mod rich;
mod stream_shared;

pub(super) async fn run_chat_session(
    model: Option<String>,
    provider: Option<String>,
    requested_agent: Option<String>,
    requested_scheduler_profile: Option<String>,
    thinking_requested: bool,
    interactive_mode: InteractiveCliMode,
    port_override: Option<u16>,
    working_dir: PathBuf,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()> {
    match interactive_mode {
        InteractiveCliMode::Rich => {
            rich::run_chat_session_rich(
                model,
                provider,
                requested_agent,
                requested_scheduler_profile,
                thinking_requested,
                port_override,
                working_dir,
                runtime_context,
            )
            .await
        }
        InteractiveCliMode::Compact => {
            compact::run_chat_session(
                model,
                provider,
                requested_agent,
                requested_scheduler_profile,
                thinking_requested,
                interactive_mode,
                port_override,
                working_dir,
                runtime_context,
            )
            .await
        }
    }
}

pub(super) fn attach_rich_prompt(
    runtime: &mut CliExecutionRuntime,
    repl_style: &CliStyle,
    current_dir: &Path,
    config: &Config,
    provider_registry: &ProviderRegistry,
    agent_registry: &AgentRegistry,
    recent_session_info: Option<&CliRecentSessionInfo>,
    server_model_list: Option<Vec<String>>,
) -> anyhow::Result<mpsc::UnboundedReceiver<CliDispatchInput>> {
    prompt_shared::attach_rich_prompt(
        runtime,
        repl_style,
        current_dir,
        config,
        provider_registry,
        agent_registry,
        recent_session_info,
        server_model_list,
    )
}
