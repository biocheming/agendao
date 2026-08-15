//! GET `/tool/catalog`：Settings→Tools 读面。
//!
//! 与 skill catalog 的关键差异：disabled tools 仍列出（`disabled` 标记），
//! 否则 UI 无法提供 re-enable 入口。实现上从**未过滤**的全量 registry
//! （`create_default_registry_with_config(None)`）出发，按当前 config 的
//! `disabled_tools` 逐条计算 disabled/protected 标记；live registry
//! （`ServerState::tool_registry`）在构建时已经删掉了 disabled tools，
//! 不能作为列表源。

use std::sync::Arc;

use axum::{extract::State, Json};

use crate::{Result, ServerState};

pub(crate) async fn list_tool_entries(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<agendao_api::ToolListEntry>>> {
    Ok(Json(build_tool_list_entries(&state).await))
}

/// 全量 tool 列表 + 每条的 family/protected/disabled 标记（按 id 排序）。
/// `local_list_tools`（TUI local-direct）与 HTTP 路由共用同一权威。
pub(crate) async fn build_tool_list_entries(
    state: &Arc<ServerState>,
) -> Vec<agendao_api::ToolListEntry> {
    let config = state.config_store.config();
    let disabled = &config.disabled_tools;
    let registry = agendao_tool::create_default_registry_with_config(None).await;

    let mut entries: Vec<agendao_api::ToolListEntry> = registry
        .list()
        .await
        .into_iter()
        .map(|tool| {
            let id = tool.id().to_string();
            let family = tool.catalog_metadata().and_then(|catalog| catalog.family);
            let is_disabled = agendao_config::matching::matching_disabled_pattern(disabled, &id)
                .is_some()
                || family
                    .as_deref()
                    .and_then(|family| {
                        agendao_config::matching::matching_disabled_pattern(disabled, family)
                    })
                    .is_some();
            agendao_api::ToolListEntry {
                protected: agendao_tool::is_protected_facade_tool(&id),
                disabled: is_disabled,
                id,
                description: tool.description().to_string(),
                family,
            }
        })
        .collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries
}
