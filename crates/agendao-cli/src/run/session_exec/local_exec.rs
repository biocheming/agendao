use std::path::Path;
use std::sync::Arc;

use crate::run::local_server_bridge;

use super::message_io::{build_prompt_message, print_assistant_messages};

/// Options controlling a single local CLI prompt run (session selection +
/// prompt routing fields), grouped so `run_cli_prompt_local` stays readable.
pub(in crate::run) struct LocalPromptRequest<'a> {
    pub command: Option<&'a str>,
    pub continue_last: bool,
    pub session: Option<&'a str>,
    pub fork: bool,
    pub model: Option<&'a str>,
    pub agent: Option<&'a str>,
    pub variant: Option<&'a str>,
    pub title: Option<&'a str>,
    pub directory: &'a str,
}

pub(in crate::run) async fn run_cli_prompt_local(
    state: &Arc<local_server_bridge::CliLocalServerState>,
    input: &str,
    request: LocalPromptRequest<'_>,
) -> anyhow::Result<()> {
    let LocalPromptRequest {
        command,
        continue_last,
        session,
        fork,
        model,
        agent,
        variant,
        title,
        directory,
    } = request;
    let session_id =
        resolve_local_session(state, continue_last, session, fork, title, directory).await?;

    let message = build_prompt_message(input, command);
    local_server_bridge::local_prompt(
        Arc::clone(state),
        &session_id,
        agendao_client::PromptRequest {
            message: Some(message),
            parts: None,
            idempotency_key: None,
            ingress_source: Some("cli".to_string()),
            agent: agent.map(|s| s.to_string()),
            scheduler_profile: None,
            model: model.map(|s| s.to_string()),
            variant: variant.map(|s| s.to_string()),
            command: command.map(|s| s.to_string()),
            arguments: None,
            source_origin: Some(agendao_types::MessageSourceOrigin::Operator),
            source_surface: Some(agendao_types::MessageSourceSurface::Cli),
        },
    )
    .await?;

    let messages = local_server_bridge::local_list_messages(Arc::clone(state), &session_id).await?;
    print_assistant_messages(&messages);
    Ok(())
}

async fn resolve_local_session(
    state: &Arc<local_server_bridge::CliLocalServerState>,
    continue_last: bool,
    session: Option<&str>,
    fork: bool,
    title: Option<&str>,
    directory: &str,
) -> anyhow::Result<String> {
    let base_id = if let Some(session_id) = session {
        Some(session_id.to_string())
    } else if continue_last {
        local_server_bridge::local_list_sessions(Arc::clone(state), None, Some(100))
            .await?
            .into_iter()
            .find(|s| s.parent_id.is_none() && s.directory == directory)
            .map(|s| s.id)
    } else {
        None
    };

    if let Some(base_id) = base_id {
        if fork {
            let forked =
                local_server_bridge::local_fork_session(Arc::clone(state), &base_id).await?;
            return Ok(forked.id);
        }
        return Ok(base_id);
    }

    let created = local_server_bridge::local_create_session(
        Arc::clone(state),
        agendao_client::CreateSessionRequest {
            scheduler_profile: None,
            directory: Some(directory.to_string()),
            project_id: None,
            title: title.map(|s| s.to_string()),
        },
    )
    .await?;
    Ok(created.id)
}

pub(in crate::run) fn cli_session_directory(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}
