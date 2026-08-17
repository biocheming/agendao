use super::message_io::{build_prompt_message, print_json_prompt_result};
use crate::cli::RunOutputFormat;

/// Session-selection fields for the Unix-socket transport prompt path,
/// mirroring `LocalPromptRequest` so `--session/--continue/--fork/--title`
/// behave the same in both modes.
pub(in crate::run) struct TransportPromptRequest<'a> {
    pub command: Option<&'a str>,
    pub continue_last: bool,
    pub session: Option<&'a str>,
    pub fork: bool,
    pub model: Option<&'a str>,
    pub agent: Option<&'a str>,
    pub scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
    pub variant: Option<&'a str>,
    pub title: Option<&'a str>,
    pub directory: &'a str,
    pub format: RunOutputFormat,
}

pub(in crate::run) async fn run_cli_prompt_transport(
    transport: &agendao_client::FrontendTransport,
    input: &str,
    request: TransportPromptRequest<'_>,
) -> anyhow::Result<()> {
    let session_id =
        resolve_transport_session(transport, &request, request.title, request.directory).await?;
    let message = build_prompt_message(input, request.command);
    let response = transport
        .prompt(
            &session_id,
            &message,
            agendao_client::transport::PromptOptions {
                agent_id: request.agent.map(|s| s.to_string()),
                scheduler: request.scheduler.clone(),
                model: request.model.map(|s| s.to_string()),
                variant: request.variant.map(|s| s.to_string()),
                source_origin: Some(agendao_types::MessageSourceOrigin::Operator),
                source_surface: Some(agendao_types::MessageSourceSurface::Cli),
                command: request.command.map(|s| s.to_string()),
                ..Default::default()
            },
        )
        .await?;
    match request.format {
        RunOutputFormat::Json => print_json_prompt_result(&session_id, &response.text),
        RunOutputFormat::Default => println!("{}", response.text),
    }
    Ok(())
}

async fn resolve_transport_session(
    transport: &agendao_client::FrontendTransport,
    request: &TransportPromptRequest<'_>,
    title: Option<&str>,
    directory: &str,
) -> anyhow::Result<String> {
    let base_id = if let Some(session_id) = request.session {
        Some(session_id.to_string())
    } else if request.continue_last {
        transport
            .list_sessions()
            .await?
            .into_iter()
            .find(|s| s.parent_id.is_none() && s.directory == directory)
            .map(|s| s.id)
    } else {
        None
    };

    if let Some(base_id) = base_id {
        if request.fork {
            let forked = transport.fork_session(&base_id, None).await?;
            return Ok(forked.id);
        }
        return Ok(base_id);
    }

    let created = transport
        .create_session(agendao_api::CreateSessionRequest {
            scheduler: None,
            directory: Some(directory.to_string()),
            project_id: None,
            title: title.map(|s| s.to_string()),
        })
        .await?;
    Ok(created.id)
}
