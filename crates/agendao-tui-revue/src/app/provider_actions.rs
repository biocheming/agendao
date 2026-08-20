//! 火/土 — Provider/Model 写入接线:dialog submit → client → server →
//! refresh providers signal。
//!
//! 道纪闭环(土律·第四条·单点权威 + 道纪·第七条·相生):
//!   1. 木:`ProviderEditDialog` / `SettingsEditState`(in-place) / `ModelEditDialog`
//!      收口 form 输入(api_key 含)
//!   2. 火:`submit_provider_edit` / `submit_model_edit` 调 client.api 写入
//!   3. 土:server `ConfigStore.replace_with` + `AuthManager.set` 唯一落盘
//!   4. 金:写后立即 `refresh_providers_into_store` 回灌 store.providers
//!   5. 水:toast Success/Error 沉淀,下一轮 Settings 渲染读 store 同源
//!
//! api_key 永不下发:server `ProviderInfo` 无该字段;TUI 仅在 submit 瞬间
//! 持有明文,提交后 `dialog.close()` → `Input.clear()` 抹除(道纪·第九条·配对销毁)。

use crate::app::AppHandler;
use crate::dialog::{ModelEditMode, ModelEditSubmission};
use crate::store::types::ToastMsgVariant;

/// `GET /provider` 响应 → `/models` 对话框条目。启动 init 与
/// `refresh_providers_into_store` 共用（土律·第四条·单点权威）——provider/model
/// 写入后模型选择器必须同帧看到，否则新加的 model 要重启才出现。
pub(crate) fn model_entries_from_providers(
    all: &[agendao_client::ProviderInfo],
    connected: &std::collections::HashSet<String>,
) -> Vec<crate::dialog::ModelEntry> {
    all.iter()
        .flat_map(|p| {
            let provider_available = connected.contains(&p.id);
            let display_name = p.name.clone();
            let provider_id = p.id.clone();
            p.models.iter().map(move |m| crate::dialog::ModelEntry {
                provider: provider_id.clone(),
                provider_display: display_name.clone(),
                model_id: m.id.clone(),
                display: format!("{} ({})", m.name, display_name),
                variants: vec![],
                available: m.available.unwrap_or(provider_available),
            })
        })
        .collect()
}

/// Provider 写入模式:Add = 注册新 provider,Edit = 改既有 provider。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderEditMode {
    Add,
    Edit,
}

/// Provider 写入载荷(木 → 火的唯一提交契约)。由 ProviderEditDialog（主路径，
/// Settings→Providers 的 a/e 键）或 in-place Settings 编辑态
/// (`SettingsEditState`，鼠标 "+ Add" / `E` 键)组装,经 `submit_provider_edit`
/// 写入 client → server。
pub struct ProviderEditSubmission {
    pub mode: ProviderEditMode,
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub protocol: String,
    /// Edit 模式留空 = 不重置 auth;Add 模式必填。
    pub api_key: String,
}

impl AppHandler {
    /// 拉服务端 provider 全集回灌 `store.providers` + `store.providers_connected`，
    /// 并同帧重建 `/models` 对话框条目（否则添加 model 后 model_select 仍是
    /// 启动快照，要重启才可见）。OpenSettings / 任意写入完成后调用,
    /// **单点回流真相**(土律·第四条)。
    /// 选中态尽量保留:写后若原 selected provider 仍存在则不变;消失则回退到第一项。
    pub(crate) fn refresh_providers_into_store(&mut self) {
        let Some(api) = self.api.as_ref() else { return };
        match api.get_all_providers() {
            Ok(resp) => self.apply_provider_snapshot(resp),
            Err(e) => self.store.push_toast(
                &format!("Failed to refresh providers: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }

    /// provider 快照落库单点:store 信号 + model_select 同帧重建。
    /// `refresh_providers_into_store` 的写路径与测试直调共用。
    pub(crate) fn apply_provider_snapshot(&mut self, resp: agendao_client::FullProviderListResponse) {
        let connected: std::collections::HashSet<String> =
            resp.connected.iter().cloned().collect();
        let prev_sel = self.store.settings_selected_provider.get();
        // 保留原选中态;若已不在列表则退到第一项(优先 connected)。
        let new_sel = prev_sel
            .as_ref()
            .filter(|id| resp.all.iter().any(|p| &p.id == *id))
            .cloned()
            .or_else(|| {
                resp.all
                    .iter()
                    .find(|p| connected.contains(&p.id))
                    .or_else(|| resp.all.first())
                    .map(|p| p.id.clone())
            });
        let entries = model_entries_from_providers(&resp.all, &connected);
        self.store.providers.set(resp.all);
        self.store.providers_connected.set(connected);
        self.store.settings_selected_provider.set(new_sel);
        self.model_select.set_models(entries);
    }

    /// 处理 Provider 添加/编辑 form dialog 的 submit 载荷。
    /// Add:`register_custom_provider`(= connect_provider with base_url/protocol)
    /// Edit:`update_provider`(改 name/base_url/protocol) +(api_key 非空时)
    /// `connect_provider`(改 auth);两步分离让"只改名字不重置 key"自然成立。
    pub(crate) fn submit_provider_edit(&mut self, s: ProviderEditSubmission) {
        let Some(api) = self.api.as_ref() else {
            self.store
                .push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        let res: anyhow::Result<()> = match s.mode {
            ProviderEditMode::Add => {
                // register_custom_provider 内部:POST /provider/connect with base_url/protocol/api_key
                // → server config + auth 双写。api_key 强制必填(Add 模式)。
                api.register_custom_provider(&s.id, &s.base_url, &s.protocol, &s.api_key)
            }
            ProviderEditMode::Edit => {
                // 1) name/base_url/protocol → PUT /provider/{id}(server update_provider)
                let r1 =
                    api.update_provider(&s.id, Some(&s.name), Some(&s.base_url), Some(&s.protocol));
                // 2) api_key 非空 → POST /provider/connect 改 auth(空 = 不改保留原)
                let r2 = if s.api_key.is_empty() {
                    Ok(true)
                } else {
                    // connect_provider 内部带 base_url/protocol 一并 reset auth + config
                    api.connect_provider(
                        &s.id,
                        &s.api_key,
                        Some(s.base_url.clone()),
                        Some(s.protocol.clone()),
                    )
                    .map(|_| true)
                };
                match (r1, r2) {
                    (Ok(_), Ok(_)) => Ok(()),
                    (Err(e), _) | (_, Err(e)) => Err(e),
                }
            }
        };
        match res {
            Ok(()) => {
                self.refresh_providers_into_store();
                let msg = match s.mode {
                    ProviderEditMode::Add => format!("Provider `{}` added", s.id),
                    ProviderEditMode::Edit => format!("Provider `{}` saved", s.id),
                };
                self.store.push_toast(&msg, ToastMsgVariant::Success);
            }
            Err(e) => self
                .store
                .push_toast(&format!("Save failed: {}", e), ToastMsgVariant::Error),
        }
    }

    /// 处理 Model 添加/编辑 form dialog 的 submit 载荷。
    /// 走 `put_provider_model_config`(server PUT /config/provider/{id}/models/{key})。
    /// server 端 PUT 是整体覆写,半空 ModelConfig 会丢字段(cost/reasoning/
    /// temperature 等)。Add 模式构造最小有效 ModelConfig;Edit 模式以
    /// `s.prefill`(open_edit 时 GET 到的原 ModelConfig 全量副本)为基底,
    /// 只覆盖 form 暴露字段(id/name/context/output/effort/timeout/stall),
    /// 其余字段原样保留(土律·第十条·避免半空覆写)。
    pub(crate) fn submit_model_edit(&mut self, s: ModelEditSubmission) {
        let Some(api) = self.api.as_ref() else {
            self.store
                .push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        use agendao_config::{ModelConfig, ModelLimitConfig};
        // Edit 带 prefill 时以原 config 为基底;Add(或 GET 失败无 prefill,
        // 已在打开时 toast 告知)从 Default 起。
        let mut model = s.prefill.clone().unwrap_or_default();
        model.name = Some(s.name.clone());
        model.model = Some(s.model_key.clone());
        // limit 只覆盖 form 暴露的 context/output;input 保留 prefill 原值。
        let prev_input = model.limit.as_ref().and_then(|l| l.input);
        model.limit = Some(ModelLimitConfig {
            context: s.context_window,
            input: prev_input,
            output: s.max_output_tokens,
        });
        // reasoning effort:字段可见(reasoning 模型/Add)时按表单值覆写
        // (`default` → None 清除显式设置);不可见时保留 prefill 原值不误清。
        if s.reasoning_effort_visible {
            model.reasoning_effort = s.reasoning_effort.clone();
        }
        // timeout / stream stall:表单留空 = 清除(None),与 context/output 同口径。
        model.timeout_secs = s.timeout_secs;
        model.stream_stall_timeout_secs = s.stream_stall_timeout_secs;
        let model: ModelConfig = model;
        match api.put_provider_model_config(&s.provider_id, &s.model_key, &model) {
            Ok(_) => {
                self.refresh_providers_into_store();
                let msg = match s.mode {
                    ModelEditMode::Add => format!("Model `{}` added", s.model_key),
                    ModelEditMode::Edit => format!("Model `{}` saved", s.model_key),
                };
                self.store.push_toast(&msg, ToastMsgVariant::Success);
            }
            Err(e) => self
                .store
                .push_toast(&format!("Model save failed: {}", e), ToastMsgVariant::Error),
        }
    }

    /// 删 provider:DELETE /provider/{id}(server 端 config + auth 双删)。
    /// 由 `PendingConfirm::DeleteProvider` 二次确认后调用。
    pub(crate) fn delete_provider_action(&mut self, provider_id: &str) {
        let Some(api) = self.api.as_ref() else { return };
        match api.delete_provider(provider_id) {
            Ok(_) => {
                self.refresh_providers_into_store();
                self.store.push_toast(
                    &format!("Provider `{}` deleted", provider_id),
                    ToastMsgVariant::Success,
                );
            }
            Err(e) => self
                .store
                .push_toast(&format!("Delete failed: {}", e), ToastMsgVariant::Error),
        }
    }

    /// 删 model:DELETE /config/provider/{id}/models/{key}。
    /// 由 `PendingConfirm::DeleteProviderModel` 二次确认后调用。
    pub(crate) fn delete_provider_model_action(&mut self, provider_id: &str, model_key: &str) {
        let Some(api) = self.api.as_ref() else { return };
        match api.delete_provider_model_config(provider_id, model_key) {
            Ok(_) => {
                self.refresh_providers_into_store();
                self.store.push_toast(
                    &format!("Model `{}` deleted", model_key),
                    ToastMsgVariant::Success,
                );
            }
            Err(e) => self.store.push_toast(
                &format!("Model delete failed: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }

    /// in-place Settings 编辑 submit:从 `SettingsEditState` 字段抽明文,
    /// 包装成 `ProviderEditSubmission`,复用既有 `submit_provider_edit` 写入通路,
    /// 完成后 `settings_edit.close()`(道纪·第九条·配对销毁:api_key 明文抹除)。
    ///
    /// 字段校验语义:Add 模式 name/base_url/api_key 必填,
    /// Edit 模式 base_url 必填(name 不可改,api_key 留空 = 不重置 auth)。
    /// 校验未过时静默不写,toast 提示(让用户继续填,不 close)。
    /// 这里**校验失败保留编辑态**,与 dialog 语义略不同——in-place 形态下用户视野
    /// 仍在 Details pane 字段上,close 反而粗暴。
    pub(crate) fn submit_settings_edit(&mut self) {
        use crate::app::settings_edit_state::SettingsEditMode;

        // 1) 抽字段(明文)。Input.text() 返回 buffer 全量,api_key 明文在 password Input
        //    内部驻留——submit 完(下面 close 时)立刻 clear。
        let mode = self.settings_edit.mode;
        let name = self.settings_edit.name_input.text().trim().to_string();
        let base_url = self.settings_edit.base_url_input.text().trim().to_string();
        let protocol = self.settings_edit.protocol_key().to_string();
        let api_key = self.settings_edit.api_key_input.text().to_string();
        let origin_id = self.settings_edit.origin_provider_id.clone();

        // 2) 校验(失败→toast 不 close,让用户继续编辑)。
        match mode {
            SettingsEditMode::Add => {
                if name.is_empty() || base_url.is_empty() || api_key.is_empty() {
                    self.store.push_toast(
                        "Name, Base URL and API Key are required to add a provider",
                        ToastMsgVariant::Error,
                    );
                    return;
                }
            }
            SettingsEditMode::Edit => {
                if base_url.is_empty() {
                    self.store
                        .push_toast("Base URL cannot be empty", ToastMsgVariant::Error);
                    return;
                }
            }
        }

        // 3) Add 模式 id slug:lowercase + 空格→`-`;Edit 模式直接用 origin_id。
        let id = match mode {
            SettingsEditMode::Add => name
                .to_lowercase()
                .chars()
                .map(|c| if c.is_whitespace() { '-' } else { c })
                .collect(),
            SettingsEditMode::Edit => origin_id,
        };

        // 4) 转译到既有 ProviderEditSubmission(火/土 — 复用同一写入通路,土律·第四条)。
        let submission = ProviderEditSubmission {
            mode: match mode {
                SettingsEditMode::Add => ProviderEditMode::Add,
                SettingsEditMode::Edit => ProviderEditMode::Edit,
            },
            id,
            name,
            base_url,
            protocol,
            api_key,
        };

        // 5) 复用 submit_provider_edit:内部已 refresh_providers_into_store + push_toast。
        self.submit_provider_edit(submission);

        // 6) 关编辑态(配对销毁,api_key 明文抹除)。
        self.settings_edit.close();
        self.layout_dirty = true;
    }
}
