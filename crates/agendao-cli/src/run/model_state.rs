use std::sync::Arc;

use crate::api_client::CliApiClient;
use crate::run::local_dispatch;
use crate::run::local_server_bridge;

pub(super) async fn cli_save_recent_model_ref(
    local_server: &Option<Arc<local_server_bridge::CliLocalServerState>>,
    transport: &Option<Arc<agendao_client::FrontendTransport>>,
    api_client: &CliApiClient,
    model_ref: &str,
) {
    let Some((provider, model)) = model_ref.split_once('/') else {
        return;
    };
    let provider = provider.trim();
    let model = model.trim();
    if provider.is_empty() || model.is_empty() {
        return;
    }
    let mut recent = local_dispatch::get_recent_models(local_server, transport, api_client)
        .await
        .unwrap_or_default();
    recent.retain(|entry| {
        !(entry.provider.eq_ignore_ascii_case(provider) && entry.model.eq_ignore_ascii_case(model))
    });
    recent.insert(
        0,
        agendao_state::RecentModelEntry {
            provider: provider.to_string(),
            model: model.to_string(),
        },
    );
    recent.truncate(agendao_state::MAX_RECENT_MODELS);
    if let Err(error) =
        local_dispatch::put_recent_models(local_server, transport, api_client, &recent).await
    {
        tracing::warn!(%error, "failed to persist CLI recent model");
    }
}

pub(super) fn cli_resolve_show_thinking(
    explicit_flag: bool,
    config: Option<&agendao_config::Config>,
    fallback: bool,
) -> bool {
    if explicit_flag {
        return true;
    }

    config
        .and_then(|cfg| cfg.ui_preferences.as_ref())
        .and_then(|ui| ui.show_thinking)
        .unwrap_or(fallback)
}
