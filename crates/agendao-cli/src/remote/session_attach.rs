use agendao_config::schema::ShareMode;
use agendao_runtime_context::ResolvedWorkspaceContext;
use serde::Deserialize;

use super::{parse_http_json, server_url};
use crate::util::parse_bool_env;

#[derive(Debug, Deserialize)]
struct RemoteSessionInfo {
    id: String,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    directory: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteShareInfo {
    url: String,
}

fn remote_show_thinking_from_context(context: &ResolvedWorkspaceContext) -> Option<bool> {
    context
        .config
        .ui_preferences
        .as_ref()
        .and_then(|ui| ui.show_thinking)
}

async fn fetch_remote_workspace_context(
    client: &reqwest::Client,
    base_url: &str,
) -> anyhow::Result<ResolvedWorkspaceContext> {
    let context_endpoint = server_url(base_url, "/workspace/context");
    parse_http_json(client.get(context_endpoint).send().await?).await
}

pub(super) async fn resolve_remote_session(
    client: &reqwest::Client,
    base_url: &str,
    continue_last: bool,
    session: Option<String>,
    fork: bool,
    title: Option<String>,
    directory: Option<String>,
) -> anyhow::Result<String> {
    let base_id = if let Some(session_id) = session {
        Some(session_id)
    } else if continue_last {
        let list_endpoint = server_url(base_url, "/session?roots=true&limit=100");
        let sessions: Vec<RemoteSessionInfo> =
            parse_http_json(client.get(list_endpoint).send().await?).await?;
        let directory = directory
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        sessions
            .into_iter()
            .find(|s| {
                s.parent_id.is_none()
                    && directory
                        .map(|dir| s.directory.as_deref() == Some(dir))
                        .unwrap_or(true)
            })
            .map(|s| s.id)
    } else {
        None
    };

    if let Some(base_id) = base_id {
        if fork {
            let fork_endpoint = server_url(base_url, &format!("/session/{}/fork", base_id));
            let forked: RemoteSessionInfo = parse_http_json(
                client
                    .post(fork_endpoint)
                    .json(&serde_json::json!({ "message_id": null }))
                    .send()
                    .await?,
            )
            .await?;
            return Ok(forked.id);
        }
        return Ok(base_id);
    }

    let create_endpoint = server_url(base_url, "/session");
    let created: RemoteSessionInfo = parse_http_json(
        client
            .post(create_endpoint)
            .json(&serde_json::json!({
                "title": title,
                "directory": directory
            }))
            .send()
            .await?,
    )
    .await?;
    Ok(created.id)
}

pub(super) async fn maybe_share_remote_session(
    client: &reqwest::Client,
    base_url: &str,
    session_id: &str,
    share_requested: bool,
) -> anyhow::Result<()> {
    let auto_share_env = std::env::var("AGENDAO_AUTO_SHARE")
        .ok()
        .map(|v| parse_bool_env(&v))
        .unwrap_or(false);
    let context = fetch_remote_workspace_context(client, base_url).await?;
    let config_auto = matches!(context.config.share, Some(ShareMode::Auto));

    if !(share_requested || auto_share_env || config_auto) {
        return Ok(());
    }

    let share_endpoint = server_url(base_url, &format!("/session/{}/share", session_id));
    let shared: RemoteShareInfo =
        parse_http_json(client.post(share_endpoint).send().await?).await?;
    println!("~  {}", shared.url);
    Ok(())
}

pub(super) async fn refresh_show_thinking_from_context(
    client: &reqwest::Client,
    base_url: &str,
) -> Option<bool> {
    fetch_remote_workspace_context(client, base_url)
        .await
        .ok()
        .and_then(|context| remote_show_thinking_from_context(&context))
}
