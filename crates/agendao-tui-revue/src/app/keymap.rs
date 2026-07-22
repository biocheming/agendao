//! Key/Event routing for `AppHandler` — 火 (event dispatch authority).
//!
//! All methods on `AppHandler` that interpret a key, mouse, or tick event
//! and decide what state to mutate live here. Rendering (金) and event-loop
//! plumbing (土) stay in `super`.
//!
//! The split keeps the `RootView::render` giant from being entangled with
//! the equally-large keymap; both can grow independently without the other
//! dragging the file over the 1500-line cap.

use revue::event::{Event, Key};

use agendao_command::{CommandRegistry, UiActionId};

use crate::app::{AppHandler, Panel};
use crate::app::dispatch_outcome;
use crate::dialog::{
    PermissionRequest, PermissionLifetime,
    QuestionOption, QuestionRequest,
};
use crate::input::{PromptAction, SlashPopup};
use crate::store::app_store::Route;
use crate::store::types::{RunStatus, SettingsFocusPane, ToastMsgVariant};
use crate::telemetry::event_handler::apply_frontend_event;

/// General 行 → toggle action 的单点映射（土律归一）。
/// 键盘（handle_general_body_key 的 Enter/Space/←/→）与鼠标点击行共用；
/// `prev` 仅对多值行（Theme）有意义：键盘 ← = 上一个，其余 = 下一个。
fn general_row_toggle_action(
    row: crate::store::types::GeneralRow,
    prev: bool,
) -> agendao_command::UiActionId {
    use agendao_command::UiActionId;
    use crate::store::types::GeneralRow;
    match row {
        GeneralRow::ShowThinking => UiActionId::ToggleThinking,
        GeneralRow::ShowScrollbar => UiActionId::ToggleScrollbar,
        GeneralRow::ShowHeader => UiActionId::ToggleHeader,
        GeneralRow::ShowTips => UiActionId::ToggleTips,
        GeneralRow::CompactDensity => UiActionId::ToggleDensity,
        // Theme 是多值循环（4 套），←/→ 分方向；Enter/Space/点击 约定=下一个。
        GeneralRow::Theme if prev => UiActionId::AppearancePrev,
        GeneralRow::Theme => UiActionId::AppearanceNext,
    }
}

impl AppHandler {
    // ── Settings 动作单点（鼠标/键盘共用，土律归一）──
    // 以下方法由 keymap 的键路由与鼠标命中共用，任何一处改语义两端同步。

    /// provider 选中统一写路径：设 selected_provider；换 provider 时清 selected_model。
    pub(crate) fn settings_select_provider_by_id(&mut self, id: String) {
        if self.store.settings_selected_provider.get().as_deref() != Some(id.as_str()) {
            self.store.settings_selected_model.set(None);
        }
        self.store.settings_selected_provider.set(Some(id));
    }

    /// provider 选中循环移动（↑/↓/滚轮共用）。
    pub(crate) fn settings_move_provider(&mut self, dir: i32) {
        let providers = self.store.providers.get();
        if providers.is_empty() {
            return;
        }
        let cur = self.store.settings_selected_provider.get();
        let idx = cur
            .as_ref()
            .and_then(|id| providers.iter().position(|p| &p.id == id))
            .unwrap_or(0);
        let n = providers.len() as i32;
        let nxt = (((idx as i32 + dir) % n) + n) % n;
        let new_id = providers[nxt as usize].id.clone();
        self.settings_select_provider_by_id(new_id);
    }

    /// models 选中循环移动（Details 栏 ↑/↓/滚轮共用）。
    pub(crate) fn settings_move_model(&mut self, dir: i32) {
        let providers = self.store.providers.get();
        let sel_provider_id = self.store.settings_selected_provider.get();
        let Some(provider) = sel_provider_id
            .as_ref()
            .and_then(|id| providers.iter().find(|p| &p.id == id))
        else {
            return;
        };
        if provider.models.is_empty() {
            return;
        }
        let model_keys: Vec<&str> = provider.models.iter().map(|m| m.id.as_str()).collect();
        let cur = self.store.settings_selected_model.get();
        let new_key = match cur
            .as_deref()
            .and_then(|k| model_keys.iter().position(|x| *x == k))
        {
            None => model_keys[0].to_string(),
            Some(idx) => {
                let n = model_keys.len() as i32;
                let nxt = (((idx as i32 + dir) % n) + n) % n;
                model_keys[nxt as usize].to_string()
            }
        };
        self.store.settings_selected_model.set(Some(new_key));
    }

    /// MCP 列表选中循环移动（List/Details 两栏回落同一选中，↑/↓/滚轮共用）。
    pub(crate) fn settings_move_mcp(&mut self, dir: i32) {
        let n = self.store.settings_mcp.get().len() as i32;
        if n == 0 {
            return;
        }
        let cur = self.store.settings_mcp_selected.get() as i32;
        let nxt = (((cur + dir) % n) + n) % n;
        self.store.settings_mcp_selected.set(nxt as usize);
    }

    /// Plugins 列表选中循环移动（同 MCP 口径）。
    pub(crate) fn settings_move_plugins(&mut self, dir: i32) {
        let n = self.store.settings_plugins.get().len() as i32;
        if n == 0 {
            return;
        }
        let cur = self.store.settings_plugins_selected.get() as i32;
        let nxt = (((cur + dir) % n) + n) % n;
        self.store.settings_plugins_selected.set(nxt as usize);
    }

    /// `a`（MCP 列表/详情聚焦）：弹 McpEditDialog 新增 server。
    pub(crate) fn settings_open_add_mcp(&mut self) {
        self.mcp_edit_dialog.open_add();
        self.panel = crate::app::Panel::McpEdit;
    }

    /// `e`（MCP 列表/详情聚焦）：弹 McpEditDialog 编辑选中 server（行已含
    /// config 合并字段，open_edit 直接 prefill，无需二次 GET）。
    pub(crate) fn settings_open_edit_mcp(&mut self) {
        let rows = self.store.settings_mcp.get();
        let idx = self.store.settings_mcp_selected.get();
        let Some(row) = rows.get(idx) else { return };
        self.mcp_edit_dialog.open_edit(row);
        self.panel = crate::app::Panel::McpEdit;
    }

    /// `x`（MCP 列表/详情聚焦）：Confirm 删选中 server（DELETE /config/mcp/{key}）。
    pub(crate) fn settings_confirm_delete_mcp(&mut self) {
        let rows = self.store.settings_mcp.get();
        let idx = self.store.settings_mcp_selected.get();
        let Some(row) = rows.get(idx) else { return };
        let name = row.name.clone();
        self.confirm_dialog.ask(
            "Delete MCP Server",
            &format!(
                "Delete MCP server \"{}\"? This removes the config.mcp entry.",
                name,
            ),
            "Delete",
        );
        self.pending_confirm = Some(crate::app::PendingConfirm::DeleteMcp(name));
        self.panel = crate::app::Panel::Confirm;
    }

    /// `a`（Plugins 列表/详情聚焦）：弹 PluginEditDialog 安装 file 类型插件。
    pub(crate) fn settings_open_install_plugin(&mut self) {
        self.plugin_edit_dialog.open_add();
        self.panel = crate::app::Panel::PluginEdit;
    }

    /// `x`/`d`（Plugins 列表/详情聚焦）：managed → Confirm 删 config 条目；
    /// discovered → toast 指引去对应目录删除（删除入口不在 config，诚实标注）。
    pub(crate) fn settings_confirm_delete_plugin(&mut self) {
        let rows = self.store.settings_plugins.get();
        let sel = self
            .store
            .settings_plugins_selected
            .get()
            .min(rows.len().saturating_sub(1));
        let Some(row) = rows.get(sel) else { return };
        if !row.managed {
            self.store.push_toast(
                &format!(
                    "\"{}\" is discovered — delete it from {} (not a config entry)",
                    row.name, row.origin,
                ),
                ToastMsgVariant::Warning,
            );
            return;
        }
        let name = row.name.clone();
        self.confirm_dialog.ask(
            "Delete Plugin",
            &format!(
                "Delete plugin \"{}\"? This removes the config.plugin entry.",
                name,
            ),
            "Delete",
        );
        self.pending_confirm = Some(crate::app::PendingConfirm::DeletePlugin(name));
        self.panel = crate::app::Panel::Confirm;
    }

    /// Skills 列表选中循环移动（同上）。下标是树状展开后的可见行（含类目头），
    /// 光标可停在类目头上（Enter/Space 折叠/展开），简单可靠优先。
    pub(crate) fn settings_move_skills(&mut self, dir: i32) {
        let rows = self.store.settings_skills.get();
        let collapsed = self.store.settings_skills_collapsed.get();
        let n = crate::store::types::flatten_settings_skill_rows(&rows, &collapsed).len() as i32;
        if n == 0 {
            return;
        }
        let cur = self.store.settings_skills_selected.get() as i32;
        let nxt = (((cur + dir) % n) + n) % n;
        self.store.settings_skills_selected.set(nxt as usize);
    }

    /// Enter/Space（Skills 列表类目头行）：折叠/展开该组（session tree 折叠同范式）。
    /// 数据行上调用为 no-op。返回 true = 当前行是类目头（已切换）。
    pub(crate) fn settings_toggle_skill_group(&mut self) -> bool {
        use crate::store::types::{flatten_settings_skill_rows, SettingsSkillLine};
        let rows = self.store.settings_skills.get();
        let collapsed = self.store.settings_skills_collapsed.get();
        let lines = flatten_settings_skill_rows(&rows, &collapsed);
        let sel = self
            .store
            .settings_skills_selected
            .get()
            .min(lines.len().saturating_sub(1));
        let Some(SettingsSkillLine::Category { name, .. }) = lines.get(sel) else {
            return false;
        };
        // 折叠集 key = 小写类目名（与 flatten 匹配口径一致）。
        let key = name.to_ascii_lowercase();
        self.store.settings_skills_collapsed.update(|set| {
            if !set.remove(&key) {
                set.insert(key.clone());
            }
        });
        // 光标停在类目头上：折叠后下标不变仍指该行，天然合法，无需重定位。
        self.layout_dirty = true;
        true
    }

    /// Tools 列表选中循环移动（同 skills）。下标是树状展开后的可见行（含类目头）。
    pub(crate) fn settings_move_tools(&mut self, dir: i32) {
        let rows = self.store.settings_tools.get();
        let collapsed = self.store.settings_tools_collapsed.get();
        let n = crate::store::types::flatten_settings_tool_rows(&rows, &collapsed).len() as i32;
        if n == 0 {
            return;
        }
        let cur = self.store.settings_tools_selected.get() as i32;
        let nxt = (((cur + dir) % n) + n) % n;
        self.store.settings_tools_selected.set(nxt as usize);
    }

    /// Enter/Space（Tools 列表类目头行）：折叠/展开该 family 组。数据行 no-op。
    pub(crate) fn settings_toggle_tool_group(&mut self) -> bool {
        use crate::store::types::{flatten_settings_tool_rows, SettingsToolLine};
        let rows = self.store.settings_tools.get();
        let collapsed = self.store.settings_tools_collapsed.get();
        let lines = flatten_settings_tool_rows(&rows, &collapsed);
        let sel = self
            .store
            .settings_tools_selected
            .get()
            .min(lines.len().saturating_sub(1));
        let Some(SettingsToolLine::Category { name, .. }) = lines.get(sel) else {
            return false;
        };
        let key = name.to_ascii_lowercase();
        self.store.settings_tools_collapsed.update(|set| {
            if !set.remove(&key) {
                set.insert(key.clone());
            }
        });
        self.layout_dirty = true;
        true
    }

    /// `a`（Providers 聚焦）：弹 ProviderEditDialog 新建 provider。
    pub(crate) fn settings_open_add_provider(&mut self) {
        self.provider_edit_dialog.open_add();
        self.panel = crate::app::Panel::ProviderEdit;
    }

    /// `e`（Providers 聚焦）：弹 ProviderEditDialog 编辑选中 provider。
    pub(crate) fn settings_open_edit_provider(&mut self) {
        let providers = self.store.providers.get();
        let sel_id = self.store.settings_selected_provider.get();
        let Some(sel) = sel_id
            .as_ref()
            .and_then(|id| providers.iter().find(|p| &p.id == id))
        else {
            return;
        };
        self.provider_edit_dialog.open_edit(sel);
        self.panel = crate::app::Panel::ProviderEdit;
    }

    /// `a`（Providers 聚焦 / "+ Add provider" 点击）：进入 Add provider 表单。
    pub(crate) fn settings_enter_add_provider(&mut self) {
        self.settings_edit.enter_add();
        self.store.settings_focus_pane.set(SettingsFocusPane::Details);
        self.layout_dirty = true;
    }

    /// `E`（Providers 聚焦）：legacy in-place 编辑（Details pane 内嵌表单）。
    /// 主路径已迁到 ProviderEditDialog（`e`）；此入口保留给习惯内嵌表单的用户，
    /// 两条路径共享同一 `submit_provider_edit` 写入链路。
    pub(crate) fn settings_enter_edit_provider(&mut self) {
        let providers = self.store.providers.get();
        let sel_id = self.store.settings_selected_provider.get();
        let Some(sel) = sel_id
            .as_ref()
            .and_then(|id| providers.iter().find(|p| &p.id == id))
        else {
            return;
        };
        self.settings_edit.enter_edit(sel);
        self.store.settings_focus_pane.set(SettingsFocusPane::Details);
        self.layout_dirty = true;
    }

    /// `m`：为选中 provider 新增 model（弹 ModelEditDialog）。
    pub(crate) fn settings_open_add_model(&mut self) {
        let Some(provider_id) = self.store.settings_selected_provider.get() else {
            return;
        };
        self.model_edit_dialog.open_add(&provider_id);
        self.panel = crate::app::Panel::ModelEdit;
    }

    /// `e`（Details 聚焦 / 行尾 ✎ 点击）：编辑选中 model（GET prefill → 弹窗）。
    pub(crate) fn settings_open_edit_model(&mut self) {
        let providers = self.store.providers.get();
        let sel_provider_id = self.store.settings_selected_provider.get();
        let sel_model_key = self.store.settings_selected_model.get();
        let (Some(provider_id), Some(model_key)) = (sel_provider_id, sel_model_key)
        else {
            return;
        };
        let Some(provider) = providers.iter().find(|p| p.id == provider_id) else {
            return;
        };
        let Some(model_info) = provider.models.iter().find(|m| m.id == model_key) else {
            return;
        };
        let prefill = if let Some(api) = self.api.as_ref() {
            match api.get_provider_model_config(&provider_id, &model_key) {
                Ok(cfg) => Some(cfg),
                Err(e) => {
                    self.store.push_toast(
                        &format!(
                            "Editing without full prefill: {} (cost/reasoning fields will reset on save)",
                            e
                        ),
                        ToastMsgVariant::Info,
                    );
                    None
                }
            }
        } else {
            None
        };
        self.model_edit_dialog.open_edit(&provider_id, model_info);
        if let Some(cfg) = prefill {
            self.model_edit_dialog.set_prefill(cfg);
        }
        self.panel = crate::app::Panel::ModelEdit;
    }

    /// `d`（Providers 聚焦）：Confirm 删 provider。
    pub(crate) fn settings_confirm_delete_provider(&mut self) {
        let providers = self.store.providers.get();
        let sel_id = self.store.settings_selected_provider.get();
        let Some(sel) = sel_id
            .as_ref()
            .and_then(|id| providers.iter().find(|p| &p.id == id))
        else {
            return;
        };
        self.confirm_dialog.ask(
            "Delete Provider",
            &format!(
                "Delete provider \"{}\"? This removes the config entry and stored API key.",
                sel.name,
            ),
            "Delete",
        );
        self.pending_confirm = Some(crate::app::PendingConfirm::DeleteProvider(sel.id.clone()));
        self.panel = crate::app::Panel::Confirm;
    }

    /// `x`/`d`（Skills 列表/详情聚焦）：Confirm 删除可写 catalog skill。
    /// 类目头 / proposal / 不可写 skill 只 toast 说明，不弹确认（诚实标注）。
    pub(crate) fn settings_confirm_delete_skill(&mut self) {
        use crate::store::types::{
            flatten_settings_skill_rows, SettingsSkillLine, SettingsSkillRow,
        };
        let rows = self.store.settings_skills.get();
        let collapsed = self.store.settings_skills_collapsed.get();
        let lines = flatten_settings_skill_rows(&rows, &collapsed);
        let sel = self
            .store
            .settings_skills_selected
            .get()
            .min(lines.len().saturating_sub(1));
        let Some(line) = lines.get(sel) else { return };
        let SettingsSkillLine::Row(src) = line else {
            self.store.push_toast(
                "Category headers only group skills — select a skill row to delete",
                ToastMsgVariant::Info,
            );
            return;
        };
        match &rows[*src] {
            SettingsSkillRow::Catalog {
                name,
                writable,
                location,
                ..
            } => {
                if !writable {
                    self.store.push_toast(
                        &format!(
                            "\"{}\" is read-only (installed outside project .agendao/skills) — remove it from its install source",
                            name
                        ),
                        ToastMsgVariant::Warning,
                    );
                    return;
                }
                let name = name.clone();
                let location = location.clone();
                self.confirm_dialog.ask(
                    "Delete Skill",
                    &format!(
                        "Delete skill \"{}\"? This removes {} from the project.",
                        name, location,
                    ),
                    "Delete",
                );
                self.pending_confirm = Some(crate::app::PendingConfirm::DeleteSkill(name));
                self.panel = crate::app::Panel::Confirm;
            }
            SettingsSkillRow::Proposal { .. } => {
                self.store.push_toast(
                    "Proposals are approved/rejected (a/r), not deleted",
                    ToastMsgVariant::Info,
                );
            }
        }
    }

    /// `d`（Details 聚焦 / 行尾 ✕ 点击）：Confirm 删 model。
    pub(crate) fn settings_confirm_delete_model(&mut self) {
        let providers = self.store.providers.get();
        let sel_provider_id = self.store.settings_selected_provider.get();
        let sel_model_key = self.store.settings_selected_model.get();
        let (Some(provider_id), Some(model_key)) = (sel_provider_id, sel_model_key)
        else {
            return;
        };
        let model_name = providers
            .iter()
            .find(|p| p.id == provider_id)
            .and_then(|p| p.models.iter().find(|m| m.id == model_key))
            .map(|m| m.name.clone())
            .unwrap_or_else(|| model_key.clone());
        self.confirm_dialog.ask(
            "Delete Model",
            &format!(
                "Delete model \"{}\" from provider \"{}\"? This removes the config entry.",
                model_name, provider_id,
            ),
            "Delete",
        );
        self.pending_confirm = Some(crate::app::PendingConfirm::DeleteProviderModel {
            provider_id,
            model_key,
        });
        self.panel = crate::app::Panel::Confirm;
    }

    /// Enabled pill 点击：toggle provider disabled。
    ///
    /// 经 `PUT /provider/{id}/disabled` 单点写入口翻转 config.disabled_providers
    /// （server 端 replace_with 直写,绕开 PATCH merge 空数组不可清除的语义坑）,
    /// 成功后 refresh_providers_into_store 回灌（水律·回流同源）。
    pub(crate) fn settings_toggle_provider_disabled(&mut self) {
        let providers = self.store.providers.get();
        let sel_id = self.store.settings_selected_provider.get();
        let Some(sel) = sel_id
            .as_ref()
            .and_then(|id| providers.iter().find(|p| &p.id == id))
        else {
            return;
        };
        let Some(api) = self.api.as_ref() else {
            self.store
                .push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        let next = !sel.disabled;
        match api.set_provider_disabled(&sel.id, next) {
            Ok(_) => {
                self.store.push_toast(
                    &format!(
                        "Provider {}: {}",
                        sel.name,
                        if next { "disabled" } else { "enabled" }
                    ),
                    ToastMsgVariant::Success,
                );
                self.refresh_providers_into_store();
            }
            Err(e) => self.store.push_toast(
                &format!("Toggle failed: {}", e),
                ToastMsgVariant::Error,
            ),
        }
        self.layout_dirty = true;
    }

    /// `t` / ⚡ 点击：测试选中 provider 的连接（server 只读探测）。
    ///
    /// 阻塞调用（server 侧超时 10s，正常 <1s）——v1 接受短冻；若后续被感知,
    /// 换 dispatch_outcomes 异步回报（与 prompt dispatch 同构）。
    pub(crate) fn settings_test_provider_connection(&mut self) {
        let providers = self.store.providers.get();
        let sel_id = self.store.settings_selected_provider.get();
        let Some(sel) = sel_id
            .as_ref()
            .and_then(|id| providers.iter().find(|p| &p.id == id))
        else {
            return;
        };
        let Some(api) = self.api.as_ref() else {
            self.store
                .push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        match api.test_provider_connection(&sel.id) {
            Ok(outcome) if outcome.ok => {
                self.store.push_toast(
                    &format!(
                        "✓ {}: {} in {}ms",
                        sel.name,
                        outcome.status.unwrap_or(200),
                        outcome.latency_ms
                    ),
                    ToastMsgVariant::Success,
                );
            }
            Ok(outcome) => {
                let detail = outcome.error.unwrap_or_else(|| {
                    outcome
                        .status
                        .map(|s| format!("HTTP {}", s))
                        .unwrap_or_else(|| "unknown error".to_string())
                });
                self.store.push_toast(
                    &format!("✗ {}: {}", sel.name, detail),
                    ToastMsgVariant::Error,
                );
            }
            Err(e) => self.store.push_toast(
                &format!("Test failed: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }
}

impl AppHandler {
    pub(crate) fn handle(&mut self, event: &Event) -> bool {
        match event {
            Event::Tick => {
                // 光标闪烁节拍：相位翻转时强制重绘（blink_visible 半周期 600ms@50ms/tick）。
                self.blink_tick = self.blink_tick.wrapping_add(1);
                let blink_flipped = crate::widget::blink::blink_visible(self.blink_tick)
                    != crate::widget::blink::blink_visible(self.blink_tick.wrapping_sub(1));
                // Reset interrupt confirmation after 5s timeout
                if self.interrupt_pending
                    && self.interrupt_time.elapsed().as_secs() > 5 {
                        self.interrupt_pending = false;
                    }
                // Advance spinner when running
                if matches!(self.active_session.run_status.get(), RunStatus::Running | RunStatus::Sending) {
                    self.spinner_tick = self.spinner_tick.wrapping_add(1);
                }
                // Garbage-collect expired toasts so the Vec doesn't grow
                // unboundedly and so the next redraw paints over an
                // already-empty list. Without this the toast banner
                // visually persists past its expiry because the renderer
                // skips it but the framebuffer never gets dirtied.
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let prev_toast_count = self.store.toasts.get().len();
                self.store.toasts.update(|t| t.retain(|m| m.expires_at == 0 || m.expires_at > now_ms));
                let toasts_changed = self.store.toasts.get().len() != prev_toast_count;
                let events = self.event_bus.drain();
                let mut changed = toasts_changed;

                // ── 水：drain 本地发送回执（dispatch 后台 task 投递）──
                // 与服务端 FrontendEvent 严格分离（见 dispatch_outcome.rs）。
                let outcomes = self.dispatch_outcomes.drain();
                for oc in &outcomes {
                    // 仅处理当前 active_session（单 active 模型；用户切走后的
                    // 陈旧回执忽略，防误改当前会话状态）。
                    let cur_sid = self.active_session.session_id.get();
                    if cur_sid.as_deref() != Some(oc.session_id()) {
                        continue;
                    }
                    match oc {
                        dispatch_outcome::DispatchOutcome::Sent { status, .. } => {
                            if status == "queued" || status == "awaiting_user" {
                                self.active_session.run_status.set(RunStatus::Running);
                            } else if matches!(self.active_session.run_status.get(), RunStatus::Sending) {
                                // 同步整轮回执（"accepted"）：local_prompt 与 HTTP
                                // session_prompt 都是整轮跑完才返回，回执到达即本轮
                                // 已结束。正常路径轮末 SessionRuntimeReplaced(Idle)
                                // 已先复位；这里是兜底——broadcast lagged 丢事件时
                                // run_status 会永卡 Sending，spinner 永转不停。
                                // 只清 Sending：若事件流已推进到 Running/Idle，说明
                                // 状态机比回执更新，不抢（防迟到回执误清新一轮的状态；
                                // 新一轮只可能由本 session 再次 dispatch 发起，其
                                // Running 事件随即到达，即使撞上也会自愈）。
                                self.active_session.run_status.set(RunStatus::Idle);
                            }
                            // status 其他值且事件流已接管：等服务端 FrontendEvent 经
                            // event_bus 驱动状态机，此处不抢。
                            self.title_refresh_pending = true;
                            changed = true;
                        }
                        dispatch_outcome::DispatchOutcome::Failed { user_msg_id, error, .. } => {
                            // 回收乐观消息（生命周期对称：push ↔ remove），不留
                            // "幽灵 user prompt"误导用户以为已发送。
                            self.active_session.remove_user_message(user_msg_id);
                            self.active_session.push_notice(
                                &format!("err-{}", ts_now()),
                                &format!("Failed to send: {}", error),
                            );
                            self.active_session
                                .run_status
                                .set(RunStatus::Error(error.clone()));
                            self.store.push_toast(
                                &format!("Send failed: {}", error),
                                crate::store::types::ToastMsgVariant::Error,
                            );
                            changed = true;
                        }
                    }
                }
                for fe in &events {
                    use agendao_server_core::frontend_events::FrontendEvent;
                    match fe {
                        FrontendEvent::PermissionUpsert { permission, .. } => {
                            // Map tool name to PermissionType
                            let perm_type = match permission.tool.to_lowercase().as_str() {
                                "read" | "readfile" | "read_file" =>
                                    crate::dialog::PermissionType::ReadFile,
                                "write" | "writefile" | "write_file" =>
                                    crate::dialog::PermissionType::WriteFile,
                                "edit" | "editfile" | "edit_file" =>
                                    crate::dialog::PermissionType::Edit,
                                "bash" | "shell" | "execute" | "executecommand" =>
                                    crate::dialog::PermissionType::Bash,
                                "glob" | "globsearch" =>
                                    crate::dialog::PermissionType::Glob,
                                "grep" | "grepsearch" | "search" =>
                                    crate::dialog::PermissionType::Grep,
                                "ls" | "list" | "listdir" | "listdirectory" =>
                                    crate::dialog::PermissionType::List,
                                "network" | "networkrequest" | "http" =>
                                    crate::dialog::PermissionType::NetworkRequest,
                                "webfetch" | "web_fetch" | "fetch" =>
                                    crate::dialog::PermissionType::WebFetch,
                                "websearch" | "web_search" =>
                                    crate::dialog::PermissionType::WebSearch,
                                "task" | "agent" =>
                                    crate::dialog::PermissionType::Task,
                                "codesearch" | "code_search" =>
                                    crate::dialog::PermissionType::CodeSearch,
                                "external" | "externaldirectory" | "external_directory" =>
                                    crate::dialog::PermissionType::ExternalDirectory,
                                _ => crate::dialog::PermissionType::ExecuteCommand,
                            };
                            // Parse supported_lifetimes from server
                            let supported_lifetimes: Vec<PermissionLifetime> = if permission.supported_lifetimes.is_empty() {
                                vec![PermissionLifetime::Once, PermissionLifetime::Turn, PermissionLifetime::Session]
                            } else {
                                permission.supported_lifetimes.iter().filter_map(|s| match s.as_str() {
                                    "once" => Some(PermissionLifetime::Once),
                                    "turn" => Some(PermissionLifetime::Turn),
                                    "session" | "always" => Some(PermissionLifetime::Session),
                                    _ => None,
                                }).collect()
                            };
                            // Extract resource from input JSON
                            let resource = permission.input.as_object()
                                .and_then(|obj| {
                                    obj.get("command").or_else(|| obj.get("path"))
                                        .or_else(|| obj.get("url")).or_else(|| obj.get("pattern"))
                                        .or_else(|| obj.get("query")).or_else(|| obj.get("directory"))
                                })
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let req = PermissionRequest {
                                id: permission.id.clone(),
                                tool: permission.tool.clone(),
                                message: permission.message.clone(),
                                perm_type,
                                supported_lifetimes,
                                permission_class: permission.permission_class.clone(),
                                scope_label: permission.scope_label.clone(),
                                risk_tags: permission.risk_tags.clone(),
                                resource,
                            };
                            self.permission_dialog.add_request(req);
                            changed = true;
                        }
                        FrontendEvent::QuestionUpsert { question, .. } => {
                            // Build options from QuestionItemInfo when available
                            let qtext = question.questions.first().cloned().unwrap_or_default();
                            let opts: Vec<QuestionOption> = if let Some(item) = question.items.first() {
                                item.options.iter().enumerate().map(|(i, o)| QuestionOption {
                                    id: format!("opt_{}", i),
                                    label: o.label.clone(),
                                    description: o.description.clone().unwrap_or_default(),
                                }).collect()
                            } else {
                                // Fallback: flat string options
                                question.options.as_ref().map(|flat_opts| {
                                    flat_opts.iter().enumerate().map(|(i, opt)| {
                                        let label = opt.first().cloned().unwrap_or_default();
                                        QuestionOption {
                                            id: format!("opt_{}", i),
                                            label,
                                            description: String::new(),
                                        }
                                    }).collect()
                                }).unwrap_or_default()
                            };
                            let qr = QuestionRequest {
                                id: question.id.clone(),
                                text: qtext,
                                options: opts,
                            };
                            self.question_dialog.ask(qr);
                            changed = true;
                        }
                        FrontendEvent::PermissionRemoved { permission_id, .. } => {
                            // Server resolved this permission (by this client or another).
                            // Remove it from the queue so the dialog updates.
                            self.permission_dialog.remove_by_id(permission_id);
                            if !self.permission_dialog.visible {
                                self.panel = Panel::None;
                            }
                            changed = true;
                        }
                        FrontendEvent::QuestionRemoved { .. } => { changed = true; }
                        _ => {
                            changed |= apply_frontend_event(fe, &self.active_session).is_some();
                        }
                    }
                }
                // ── Poll todos when running ──
                if matches!(self.active_session.run_status.get(), RunStatus::Running | RunStatus::Sending) {
                    if let Some(ref api) = self.api {
                        if let Some(ref sid) = self.active_session.session_id.get() {
                            if let Ok(todos) = api.get_session_todos(sid) {
                                if !todos.is_empty() {
                                    let items: Vec<crate::store::types::TodoItem> = todos.iter().map(|t| {
                                        crate::store::types::TodoItem {
                                            content: t.content.clone(),
                                            status: match t.status.as_str() {
                                                "completed" | "done" => crate::store::types::TodoStatus::Completed,
                                                "in_progress" => crate::store::types::TodoStatus::InProgress,
                                                "cancelled" | "canceled" => crate::store::types::TodoStatus::Cancelled,
                                                _ => crate::store::types::TodoStatus::Pending,
                                            },
                                        }
                                    }).collect();
                                    self.active_session.push_todo_list("todos", items, None);
                                    changed = true;
                                }
                            }
                        }
                    }
                }

                // ── 消费 title_refresh_pending：一轮结束后刷新一次 title ──
                // dispatch 发 prompt 时置位；Idle 时从权威 get_session 拉取服务端
                // LLM 生成的 title，同步到 header 用的 active_session.title，然后清除。
                // 只在标记位时查一次，避免 Idle 常态下持续轮询数据库。
                if self.title_refresh_pending
                    && matches!(self.active_session.run_status.get(), RunStatus::Idle)
                {
                    self.title_refresh_pending = false;
                    if let Some(ref api) = self.api {
                        if let Some(ref sid) = self.active_session.session_id.get() {
                            if let Ok(info) = api.get_session(sid) {
                                if self.active_session.title.get() != info.title {
                                    self.active_session.title.set(info.title);
                                    changed = true;
                                }
                            }
                        }
                    }
                }

                changed || blink_flipped || matches!(self.active_session.run_status.get(), RunStatus::Running | RunStatus::Sending) || self.interrupt_pending
            }
            Event::Key(key) => {
                // Ctrl+B → toggle sidebar, Ctrl+P → command palette
                if key.ctrl {
                    match key.key {
                        Key::Char('b') => {
                            self.sidebar_visible = !self.sidebar_visible;
                            return true;
                        }
                        Key::Char('p') => {
                            self.slash_popup.open();
                            self.panel = Panel::Slash;
                            return true;
                        }
                        _ => {}
                    }
                }
                // Alt/Shift/Ctrl+Enter → prompt 换行（Enter 裸键保持发送）。
                // Alt+Enter 是主通道：tmux/标准 xterm 下 S/C-Enter 常退化成
                // 普通 Enter 或文本，无法独立送达；Alt+Enter 几乎处处以
                // ESC+CR 可靠到达。S/C-Enter 保留为增强（能收到的终端照用）。
                // 仅在 prompt 是活动输入时接管：panel 打开或 Settings 路由下
                // Enter 变体仍归各自 panel/表单路由（不抢弹窗语义）。
                if key.key == Key::Enter
                    && (key.alt || key.shift || key.ctrl)
                    && self.panel == Panel::None
                    && !matches!(self.store.route.get(), Route::Settings)
                {
                    self.prompt.insert_newline();
                    return true;
                }
                self.handle_key(&key.key)
            }
            Event::Mouse(m) => {
                use revue::event::{MouseEventKind, MouseButton};
                match m.kind {
                    MouseEventKind::ScrollUp => {
                        // Settings 路由：滚轮分发到焦点栏（金律·不再穿透滚背后 session）。
                        if matches!(self.store.route.get(), Route::Settings) {
                            self.settings_wheel(-1)
                        } else {
                            self.active_session.scroll_up();
                            true
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        if matches!(self.store.route.get(), Route::Settings) {
                            self.settings_wheel(1)
                        } else {
                            self.active_session.scroll_down();
                            true
                        }
                    }
                    MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
                        // Horizontal scroll unused for now
                        false
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        // ── Session list dialog scrollbar click ──
                        // Hit-test before the sidebar / transcript
                        // branches so clicking on the dialog's
                        // own scrollbar moves the dialog cursor
                        // rather than toggling a transcript fold
                        // or scrolling the sidebar. Only the
                        // SessionList dialog publishes its
                        // scrollbar geometry right now (see
                        // `app::session_list_scrollbar_slot`).
                        if let Some(sb) = crate::app::session_list_scrollbar_slot().lock().ok().and_then(|g| *g) {
                            let overlay = crate::widget::ScrollbarOverlay::new(
                                (0, 0),
                                sb.area,
                                sb.item_count,
                                sb.visible_rows,
                                // We don't have the in-window
                                // selected index here, but the
                                // hit-test is mostly insensitive to
                                // offset: arrow rows and thumb
                                // position are computed from the
                                // *ratio* of offset/max_offset, and
                                // the default 0 lands at the top of
                                // the track (which is close enough
                                // to "where the user is" for a 1-tick
                                // approximation; the cursor re-paints
                                // immediately after on next frame).
                                0,
                            );
                            if let Some(hit) = overlay.hit_test(m.x, m.y) {
                                if matches!(self.panel, Panel::SessionList) {
                                    match hit {
                                        crate::widget::ScrollbarHit::ArrowUp => {
                                            self.session_list.selected = 0;
                                            return true;
                                        }
                                        crate::widget::ScrollbarHit::ArrowDown => {
                                            self.session_list.selected = sb.item_count.saturating_sub(1) as usize;
                                            return true;
                                        }
                                        crate::widget::ScrollbarHit::PageUp => {
                                            self.session_list.selected = self.session_list.selected.saturating_sub(sb.visible_rows as usize);
                                            return true;
                                        }
                                        crate::widget::ScrollbarHit::PageDown => {
                                            self.session_list.selected =
                                                (self.session_list.selected + sb.visible_rows as usize)
                                                    .min(sb.item_count.saturating_sub(1) as usize);
                                            return true;
                                        }
                                        crate::widget::ScrollbarHit::BeginDrag(drag) => {
                                            // The session list dialog
                                            // doesn't have a "drag"
                                            // surface of its own (it's
                                            // a flat list). Treat the
                                            // drag as "click to scroll":
                                            // on the next Drag event
                                            // we update `selected`
                                            // based on the cursor's
                                            // current y.
                                            self.session_list_scrollbar_drag = Some(drag);
                                            return true;
                                        }
                                    }
                                }
                            }
                        }
                        // ── 弹窗鼠标（overlay 优先于底层命中）──
                        // ModelEdit：框内点击 = 聚焦对应字段（4 行一块，border 内侧）；
                        // 框外点击不 Esc（防误关），仅消费。几何来自 render 发布的 Rect。
                        // Reasoning effort 字段显隐会影响块序，反查走 dialog 同源方法。
                        if self.panel == crate::app::Panel::ModelEdit {
                            if let Some(rect) = self.model_edit_rect {
                                let in_x = m.x > rect.x && m.x + 1 < rect.x + rect.width;
                                let rel = m.y.wrapping_sub(rect.y + 1);
                                let idx = (rel / 4) as usize;
                                if in_x && m.y > rect.y && rel % 4 != 0 {
                                    if let Some(field) = self.model_edit_dialog.field_at_block_index(idx) {
                                        self.model_edit_dialog.set_focus(field);
                                        // rel%4==2 = 输入行（label0/上框1/输入2/下框3）：
                                        // 光标定位到点击字符（文本起点 = 外框1+字段框1 = rect.x+2）。
                                        if rel % 4 == 2 && m.x >= rect.x + 2 {
                                            self.model_edit_dialog
                                                .set_cursor_at(field, (m.x - rect.x - 2) as usize);
                                        }
                                        self.layout_dirty = true;
                                    }
                                }
                                return true;
                            }
                        }
                        // McpEdit：同 ModelEdit 口径（4 字段 × 4 行块）。
                        if self.panel == crate::app::Panel::McpEdit {
                            if let Some(rect) = self.mcp_edit_rect {
                                let in_x = m.x > rect.x && m.x + 1 < rect.x + rect.width;
                                let rel = m.y.wrapping_sub(rect.y + 1);
                                let idx = (rel / 4) as usize;
                                if in_x && m.y > rect.y && rel % 4 != 0 && idx < crate::dialog::McpEditDialog::FIELDS.len() {
                                    let field = crate::dialog::McpEditDialog::FIELDS[idx];
                                    self.mcp_edit_dialog.set_focus(field);
                                    if rel % 4 == 2 && m.x >= rect.x + 2 {
                                        self.mcp_edit_dialog
                                            .set_cursor_at(field, (m.x - rect.x - 2) as usize);
                                    }
                                    self.layout_dirty = true;
                                }
                                return true;
                            }
                        }
                        // PluginEdit：同 ModelEdit 口径（2 字段 × 4 行块）。
                        if self.panel == crate::app::Panel::PluginEdit {
                            if let Some(rect) = self.plugin_edit_rect {
                                let in_x = m.x > rect.x && m.x + 1 < rect.x + rect.width;
                                let rel = m.y.wrapping_sub(rect.y + 1);
                                let idx = (rel / 4) as usize;
                                if in_x && m.y > rect.y && rel % 4 != 0 && idx < crate::dialog::PluginEditDialog::FIELDS.len() {
                                    let field = crate::dialog::PluginEditDialog::FIELDS[idx];
                                    self.plugin_edit_dialog.set_focus(field);
                                    if rel % 4 == 2 && m.x >= rect.x + 2 {
                                        self.plugin_edit_dialog
                                            .set_cursor_at(field, (m.x - rect.x - 2) as usize);
                                    }
                                    self.layout_dirty = true;
                                }
                                return true;
                            }
                        }
                        // ProviderEdit：同 ModelEdit 口径（4 字段 × 4 行块）。
                        if self.panel == crate::app::Panel::ProviderEdit {
                            if let Some(rect) = self.provider_edit_rect {
                                let in_x = m.x > rect.x && m.x + 1 < rect.x + rect.width;
                                let rel = m.y.wrapping_sub(rect.y + 1);
                                let idx = (rel / 4) as usize;
                                if in_x && m.y > rect.y && rel % 4 != 0 && idx < crate::dialog::ProviderEditDialog::FIELDS.len() {
                                    let field = crate::dialog::ProviderEditDialog::FIELDS[idx];
                                    self.provider_edit_dialog.set_focus(field);
                                    if rel % 4 == 2 && m.x >= rect.x + 2 {
                                        self.provider_edit_dialog
                                            .set_cursor_at(field, (m.x - rect.x - 2) as usize);
                                    }
                                    self.layout_dirty = true;
                                }
                                return true;
                            }
                        }
                        // Confirm：hint 行（末行）"y/Enter: {label}  n/Esc: cancel" 两个
                        // 命中区 → 合成键走 handle_key 既有 panel 路由（单点权威）。
                        if self.panel == crate::app::Panel::Confirm {
                            if let Some(rect) = self.confirm_rect {
                                if m.y == rect.y + rect.height.saturating_sub(1) {
                                    let label_w = self.confirm_dialog.confirm_label.chars().count() as u16;
                                    let seg1_w = 8 + label_w; // "y/Enter: " + label
                                    let hint_w = seg1_w + 2 + 13; // + "  " + "n/Esc: cancel"
                                    let start = rect.x + rect.width.saturating_sub(hint_w) / 2;
                                    if m.x >= start && m.x < start + seg1_w {
                                        return self.handle_key(&Key::Enter);
                                    }
                                    if m.x >= start + seg1_w + 2 && m.x < start + hint_w {
                                        return self.handle_key(&Key::Char('n'));
                                    }
                                }
                                return true;
                            }
                        }
                        // ── Transcript scrollbar click ──
                        // Hit-test before anything else: if the click
                        // landed on the scrollbar (▲/▼/thumb/track),
                        // resolve it here and skip the rest of the
                        // click handlers (which would otherwise try
                        // to fold blocks, focus the prompt, etc.).
                        if let Some((sb_area, (content_h, viewport_h))) = self
                            .transcript_scrollbar_area
                            .zip(self.transcript_scrollbar_metrics)
                        {
                            // Convert the store's "rows back from
                            // bottom" semantics to "rows from top"
                            // (= ScrollbarOverlay's offset) for the
                            // hit-test. Without this, the overlay
                            // would think the thumb is at the top
                            // when the user is actually at the
                            // bottom.
                            let max_offset = content_h.saturating_sub(viewport_h);
                            let user_offset = self.active_session.scroll_offset.get().min(max_offset);
                            let scroll_top = max_offset.saturating_sub(user_offset);
                            let overlay = crate::widget::ScrollbarOverlay::new(
                                (0, 0),
                                sb_area,
                                content_h,
                                viewport_h,
                                scroll_top,
                            );
                            if let Some(hit) = overlay.hit_test(m.x, m.y) {
                                match hit {
                                    crate::widget::ScrollbarHit::ArrowUp => {
                                        self.active_session.scroll_offset.set(max_offset);
                                        self.layout_dirty = true;
                                        return true;
                                    }
                                    crate::widget::ScrollbarHit::ArrowDown => {
                                        self.active_session.scroll_offset.set(0);
                                        self.layout_dirty = true;
                                        return true;
                                    }
                                    crate::widget::ScrollbarHit::PageUp => {
                                        self.active_session.scroll_page_up(viewport_h);
                                        self.layout_dirty = true;
                                        return true;
                                    }
                                    crate::widget::ScrollbarHit::PageDown => {
                                        self.active_session.scroll_page_down(viewport_h);
                                        self.layout_dirty = true;
                                        return true;
                                    }
                                    crate::widget::ScrollbarHit::BeginDrag(drag) => {
                                        self.transcript_scrollbar_drag = Some(drag);
                                        return true;
                                    }
                                }
                            }
                        }

                        // ── Sidebar interactions (Home / Session / Settings 均可用) ──
                        // Tab 切换、session tree 导航、⚙ Settings 入口不应被
                        // Route::Session 门控——Home 上也要能点树打开会话(水生木)。
                        if self.sidebar_visible && m.x < crate::app::SIDEBAR_WIDTH {
                            // Tab 符号行 / 下划线行 → 切 telemetry tab
                            if m.y == self.sidebar_tab_y || m.y == self.sidebar_tab_y + 1 {
                                let new_tab = ((m.x / 4) as usize)
                                    .min(crate::telemetry::sidebar::SIDEBAR_TAB_COUNT - 1);
                                if new_tab != self.sidebar_active_tab {
                                    self.sidebar_active_tab = new_tab;
                                    self.layout_dirty = true;
                                    return true;
                                }
                            }
                            // Session tree 行：箭头区（缩进+▶/▼ 2 列）= 纯 toggle 展开/折叠;
                            // 行其余部分 = open_session 并自动展开其子节点（若有）。
                            if let Some(hit) = self
                                .sidebar_nav_hits
                                .iter()
                                .find(|hit| hit.y == m.y)
                            {
                                let (sid, depth, has_children) =
                                    (hit.session_id.clone(), hit.depth, hit.has_children);
                                let arrow_end: u16 = (depth as u16).saturating_mul(2).saturating_add(2);
                                if has_children && m.x < arrow_end {
                                    // toggle：在集合则折叠出集合,不在则展开入集合。
                                    let newly = self.session_tree_expanded.insert(sid.clone());
                                    if !newly {
                                        self.session_tree_expanded.remove(&sid);
                                    }
                                    self.refresh_sidebar_session_tree();
                                    return true;
                                }
                                if has_children {
                                    self.session_tree_expanded.insert(sid.clone());
                                }
                                self.open_session(&sid);
                                self.refresh_sidebar_session_tree();
                                self.layout_dirty = true;
                                return true;
                            }
                            // 底部 ⚙ → OpenSettings
                            if self.terminal_h > 0
                                && m.y + 1 == self.terminal_h
                                && m.x + crate::telemetry::sidebar::SIDEBAR_GEAR_X_FROM_END
                                    >= crate::app::SIDEBAR_WIDTH
                            {
                                self.execute_slash_action(UiActionId::OpenSettings);
                                self.layout_dirty = true;
                                return true;
                            }
                        }

                        // ── Settings 路由鼠标交互（木生火：点击直达选中/编辑/动作）──
                        // 全部命中逻辑收口在 handle_settings_mouse（几何与
                        // screen::settings 常量同源，动作复用键盘写路径）。
                        if matches!(self.store.route.get(), Route::Settings)
                            && self.handle_settings_mouse(m)
                        {
                            return true;
                        }

                        // Click on transcript → toggle fold of the clicked block.
                        // Click on prompt area → focus input.
                        // Click elsewhere → unfocus.
                        if matches!(self.store.route.get(), Route::Session { .. }) {
                            // ── Session header dir 点击：toggle 全路径 tooltip ──
                            // 命中 dir 文本区 → 切换（None→显 working_dir 全路径 / Some→关）；
                            // y==header_y 但点在 dir 外 → 关闭。dir_w=0（非 Session 路由）跳过。
                            // dir 在 header 行（y==header_y=1），落在 transcript 区（y≥3）外，不与 fold 命中冲突。
                            if m.y == self.header_y && self.header_dir_w > 0 {
                                let hit = m.x >= self.header_dir_x
                                    && m.x < self.header_dir_x + self.header_dir_w;
                                if hit {
                                    let next = if self.store.dir_tooltip.get().is_some() {
                                        None
                                    } else {
                                        Some(crate::store::types::DirTooltip {
                                            path: self.store.working_dir.get(),
                                            x: self.header_dir_x,
                                            y: self.header_y + 1, // dir 下方 1 行（header 单行，下方唯一空间）
                                        })
                                    };
                                    self.store.dir_tooltip.set(next);
                                } else if self.store.dir_tooltip.get().is_some() {
                                    // 点 header 非 dir 区（title/badge）→ 收起 tooltip。
                                    self.store.dir_tooltip.set(None);
                                }
                                self.layout_dirty = true;
                                return true;
                            }

                            let ty = m.y;
                            let transcript_y = self.transcript_area_y;
                            let transcript_h = self.transcript_viewport_h;
                            // 排除左侧 sidebar 列：sidebar 显示时其区域点击不应被
                            // transcript 命中消费（阴阳边界——sidebar 区归 sidebar）。即便
                            // sidebar 内容当前无点击行为，也须先排除，避免越权 toggle
                            // transcript fold / 切 cursor（sidebar 默认显示后必需，否则
                            // 点 sidebar 行会误触 transcript）。纯黑合一后 SIDEBAR_WIDTH 列是
                            // VLine 竖线（独立 1 列），命中边界收紧为 m.x > sidebar_w——跳过
                            // sidebar 含竖线列；sidebar 不显示时 sidebar_w=0，m.x>0 仍覆盖整宽
                            //（列 0 是 transcript 左气口，无 fold 命中，无害）。
                            let sidebar_w = if self.sidebar_visible { crate::app::SIDEBAR_WIDTH } else { 0 };
                            if ty >= transcript_y && ty < transcript_y + transcript_h && m.x > sidebar_w {
                                // Click is inside transcript area.
                                // Compute which row in content space was clicked.
                                let msgs = self.active_session.messages.get();
                                // total_h 与渲染同口径（聚合）——原逐块 layout_block 算高
                                // 与聚合渲染错位，是「连续结果区域点不准」的根因：
                                // 屏幕上一个聚合深井被当成 N 个独立块量高，acc 与真实
                                // 屏幕位置对不上，点第 2 行命中第 5 块。
                                let total_h = crate::screen::transcript_total_height(&msgs, self.store.show_thinking.get(), self.store.compact_density.get(), 80);
                                let max_offset = total_h.saturating_sub(transcript_h);
                                let user_offset = self.active_session.scroll_offset.get().min(max_offset);
                                let scroll_top = max_offset.saturating_sub(user_offset);
                                let row_in_content = ty.saturating_sub(transcript_y) + scroll_top;
                                // 视觉单元遍历（与渲染/total_h 同源）：unit.height 量高，
                                // 命中时 row_owners[rel_row] 把屏幕 y 映射到块——整行命中，
                                // 装饰行 None 不 toggle。聚合/单块统一，不认块类型（金律：
                                // 命中触点 1，新增聚合种类零改动）。鼠标命中频率低且必须
                                // row_owners 真实（否则点 ToolResult 子项错块），显式 None
                                // 全量布局。
                                let units = crate::screen::build_render_units(&msgs, None, 0, self.store.show_thinking.get(), None, 80, self.store.compact_density.get());
                                let mut acc: u16 = 0;
                                let mut clicked_idx = None;
                                for unit in &units {
                                    let block_end = acc + unit.height;
                                    if row_in_content < block_end {
                                        let rel_row = row_in_content.saturating_sub(acc) as usize;
                                        clicked_idx = unit.row_owners.get(rel_row)
                                            .copied()
                                            .flatten()
                                            .map(|offset| unit.base_index + offset);
                                        break;
                                    }
                                    acc = block_end + 1; // +1 for gap between blocks
                                }
                                if let Some(idx) = clicked_idx {
                                    // 复用 cursor+toggle 闭环：组折叠时命中段首/ℹ/more 均
                                    // 展开整组，组展开时切该块详情（与 Space 行为一致）。
                                    self.active_session.transcript_cursor.set(Some(idx));
                                    self.active_session.toggle_fold_at_cursor();
                                    self.layout_dirty = true;
                                    return true;
                                }
                            }
                        }
                        // Fall through: click on prompt area or elsewhere
                        self.prompt.handle_click(m.x, m.y);
                        true
                    }
                    MouseEventKind::Down(MouseButton::Right) => {
                        // Right-click — future: context menu
                        true
                    }
                    MouseEventKind::Drag(MouseButton::Left) => {
                        // Active thumb drag on the transcript scrollbar.
                        // Translate the y delta into a new offset and
                        // store it (in "rows back from bottom" form).
                        if let (Some(drag), Some((sb_area, (content_h, viewport_h)))) = (
                            self.transcript_scrollbar_drag,
                            self.transcript_scrollbar_area.zip(self.transcript_scrollbar_metrics),
                        ) {
                            let overlay = crate::widget::ScrollbarOverlay::new(
                                (0, 0),
                                sb_area,
                                content_h,
                                viewport_h,
                                0,
                            );
                            let new_top = overlay.drag_to_offset(drag, m.y);
                            let max_offset = content_h.saturating_sub(viewport_h);
                            let user_offset = max_offset.saturating_sub(new_top);
                            self.active_session.scroll_offset.set(user_offset);
                            self.layout_dirty = true;
                            return true;
                        }
                        // Active thumb drag on the SessionList dialog
                        // scrollbar. Drag the cursor to a new selected
                        // index proportional to the cursor's y in the
                        // track, translating via the same algorithm
                        // the other scrollbars use.
                        if let (Some(drag), Some(sb)) = (
                            self.session_list_scrollbar_drag,
                            crate::app::session_list_scrollbar_slot().lock().ok().and_then(|g| *g),
                        ) {
                            if matches!(self.panel, Panel::SessionList) {
                                let overlay = crate::widget::ScrollbarOverlay::new(
                                    (0, 0),
                                    sb.area,
                                    sb.item_count,
                                    sb.visible_rows,
                                    0,
                                );
                                let new_in_window = overlay.drag_to_offset(drag, m.y);
                                // Clamp to [0, max_offset] and add
                                // back to `start` (which the dialog
                                // itself chooses from the new
                                // selected on next render). The
                                // math here intentionally ignores
                                // `start` because we're setting a
                                // *raw* item index, not a window
                                // position — the dialog's own
                                // start-window algorithm will then
                                // place it sensibly.
                                let target = (new_in_window as usize).min(sb.item_count.saturating_sub(1) as usize);
                                self.session_list.selected = target;
                                self.layout_dirty = true;
                                return true;
                            }
                        }
                        false
                    }
                    MouseEventKind::Up(_) => {
                        // Release any active drag (transcript,
                        // sidebar, or session-list dialog).
                        if self.transcript_scrollbar_drag.take().is_some() {
                            return true;
                        }
                        if self.session_list_scrollbar_drag.take().is_some() {
                            return true;
                        }
                        false
                    }
                    MouseEventKind::Move => false,
                    _ => false,
                }
            }
            Event::Resize(..) => true,
            _ => false,
        }
    }

    fn handle_key(&mut self, key: &Key) -> bool {
        // ── Panel/Overlay routing: delegated to route_panel_key (panel_dispatch.rs) ──
        if self.route_panel_key(key) {
            return true;
        }

        // ── Transcript scrolling + cursor (PageUp/PageDown, Tab, Space) ──
        //
        // Dispatched BEFORE the prompt input so Space/Tab don't get
        // swallowed by PromptInput's catch-all `_ => self.editor.handle_key(key)`
        // arm (prompt_input.rs `handle_key`). Without this re-order, Space is
        // inserted into the input as a literal space character and the
        // fold toggle never fires — which is why the previous test
        // session showed Tab moving the cursor bar to a thinking block
        // but Space leaving the chip folded.
        //
        // Up/Down stay owned by the prompt for history navigation.
        if matches!(self.store.route.get(), Route::Session { .. }) {
            match key {
                Key::PageUp => {
                    self.active_session.scroll_page_up(10);
                    // Scroll changes which block sits at the cursor row,
                    // so the cursor-bar hstack's content shifts even
                    // though heights don't change. A layout rebuild
                    // isn't strictly required, but forcing it is the
                    // simplest way to make the cursor bar land in the
                    // right slot after a multi-row jump.
                    self.layout_dirty = true;
                    return true;
                }
                Key::PageDown => {
                    self.active_session.scroll_page_down(10);
                    self.layout_dirty = true;
                    return true;
                }
                Key::Tab => {
                    // Tab cycles forward through foldable blocks.
                    self.active_session.cursor_next_foldable();
                    // Auto-scroll so the new cursor block is on screen.
                    // Without this, Tab to a foldable block far above
                    // the current viewport moves the cursor but leaves
                    // the visible window unchanged, and pressing Space
                    // toggles a block the user can't see.
                    self.active_session.ensure_cursor_visible(self.transcript_viewport_h);
                    self.layout_dirty = true;
                    return true;
                }
                Key::Char(' ') if self.prompt.text().is_empty() => {
                    // Space toggles fold ONLY when prompt is empty —
                    // otherwise it inserts a literal space into the
                    // composer. This keeps the keymap compatible with
                    // typing prose.
                    self.active_session.toggle_fold_at_cursor();
                    // Fold toggle changes `layout_block(b).height` for
                    // the affected block. The cached layout tree still
                    // holds the OLD height slots, so the next draw
                    // would paint new content into stale slots and the
                    // user would see no change. The run-loop closure
                    // reads `layout_dirty` and calls
                    // `request_layout_rebuild()` for us.
                    self.layout_dirty = true;
                    return true;
                }
                // 'e' = edit & resend（对齐 web "Revise & resend" 按钮一步触发）。
                // 双重守卫：prompt 空（对齐 Space 先例，避免打字中途误触发）+
                // cursor 在 UserPrompt（否则 'e' 落到 prompt 输入字符）。
                Key::Char('e') if self.prompt.text().is_empty()
                    && self.active_session.cursor_user_prompt().is_some() => {
                    self.execute_slash_action(UiActionId::RevisePrompt);
                    return true;
                }
                // 'c' = copy cursor 当前 block 内容（对齐 web 消息卡"复制"图标
                // 一步触发）。双重守卫：prompt 空（避免打字中途误触发）+ cursor
                // 命中（否则 'c' 落到 prompt 输入字符）。走 cursor_block_to_text
                // 复用 transcript_to_text 成形契约，再 OSC52 一次性写终端剪贴板
                // ——与 /copy slash（全 transcript）职责分离：slash → 显式全量,
                // 'c' → cursor 单块。无支持的 block 时 toast 提示，避免无声失败。
                Key::Char('c') if self.prompt.text().is_empty()
                    && self.active_session.transcript_cursor.get().is_some() => {
                    match self.active_session.cursor_block_to_text() {
                        Some(text) => match crate::dialog::clipboard::copy(&text) {
                            Ok(()) => self.store.push_toast(
                                "Block copied to clipboard",
                                crate::store::types::ToastMsgVariant::Success,
                            ),
                            Err(e) => self.store.push_toast(
                                &format!("Clipboard write failed: {}", e),
                                crate::store::types::ToastMsgVariant::Error,
                            ),
                        },
                        None => self.store.push_toast(
                            "Nothing to copy at cursor",
                            crate::store::types::ToastMsgVariant::Warning,
                        ),
                    }
                    return true;
                }
                _ => {}
            }
        }

        // ── Settings screen routing (土→金:focused_pane 单点决定 ↑/↓ 行为) ──
        //
        // 介于 Session 滚动与 prompt input 之间。Settings 路由不渲染 prompt bar,
        // 但 PromptInput 仍会消费字符键(否则 Tab/↑/↓ 会被吞)。这里在交还 prompt
        // 之前先吃掉 Tab/Up/Down/Enter,Settings 才能交互(土律·第十条:
        // 阳面 ⚙ 进、Tab/↑/↓/Enter 调,阴面 store signals 收口)。
        if matches!(self.store.route.get(), Route::Settings)
            && self.handle_settings_key(key) {
                return true;
            }

        // ── Normal prompt input ──
        let consumed = match self.prompt.handle_key(key) {
            PromptAction::Submit(text) => {
                if text.starts_with('/') {
                    self.sync_slash_from_text(&text);
                    self.prompt.clear();
                    return true;
                }
                self.dispatch(text);
                return true;
            }
            PromptAction::SubmitShell(cmd) => { self.dispatch_shell(cmd); return true; }
            PromptAction::Consumed => true,
            PromptAction::None => false,
        };

        // ── Slash/command detection: check current input text on every key ──
        let current_text = self.prompt.text();
        if let Some(query) = SlashPopup::slash_token(&current_text) {
            self.slash_popup.open_with_query(query);
            self.panel = Panel::Slash;
            if consumed { return true; }
        } else if self.panel == Panel::Slash {
            // Text changed and no longer has slash token
            self.slash_popup.close();
            self.panel = Panel::None;
        }

        if consumed { return true; }

        // ── Global keys ──
        match key {
            Key::Char('q') => { self.store.request_exit(); true }
            Key::Char('h') => { self.store.navigate(Route::Home); self.prompt.focus(); true }
            Key::Char('?') => { self.toggle_help(); true }
            Key::Escape => {
                // 1. Close dialogs first
                if self.panel != Panel::None {
                    self.close_all_panels();
                    return true;
                }
                // 2. Settings 全屏页 → 退回 Home(土律·阴阳对称:OpenSettings 进,
                //    Esc 出;由 AppStore 单点权威 `navigate_home` 收口)。
                if matches!(self.store.route.get(), Route::Settings) {
                    self.store.navigate_home();
                    return true;
                }
                // 3. Double-tap Esc to interrupt running session
                let status = self.active_session.run_status.get();
                if matches!(status, RunStatus::Running | RunStatus::Sending) {
                    if self.interrupt_pending && self.interrupt_time.elapsed().as_secs() < 5 {
                        // Second Esc within 5s → abort
                        self.interrupt_pending = false;
                        if let Some(sid) = self.active_session.get_session_id() {
                            if let Some(ref api) = self.api {
                                let _ = api.abort_session(&sid);
                            }
                        }
                        self.active_session.run_status.set(RunStatus::Idle);
                        self.store.push_toast("⏹ Session interrupted", crate::store::types::ToastMsgVariant::Info);
                    } else {
                        // First Esc → show confirmation hint
                        self.interrupt_pending = true;
                        self.interrupt_time = std::time::Instant::now();
                    }
                    return true;
                }
                self.interrupt_pending = false;
                false
            }
            _ => false,
        }
    }

    /// Parse `/command` text and execute the corresponding action directly.
    /// CommandRegistry stores names WITH leading `/` (e.g. "/models" "/model").
    fn sync_slash_from_text(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.len() <= 1 {
            self.slash_popup.open();
            self.panel = Panel::Slash;
            return;
        }
        let reg = CommandRegistry::new();
        // Look up with leading `/` intact (matches CommandRegistry storage format)
        if let Some(spec) = reg.ui_slash_command(trimmed) {
            return self.execute_slash_action(spec.action_id);
        }
        // Fallback: strip trailing chars for partial match
        let all = reg.ui_all_slash_commands();
        if let Some(spec) = all.iter().find(|c| {
            c.slash.as_ref().is_some_and(|s| s.name.starts_with(trimmed) || s.aliases.iter().any(|a| a.starts_with(trimmed)))
        }) {
            return self.execute_slash_action(spec.action_id);
        }
        self.store.push_toast(&format!("Unknown command: {}", trimmed),
            crate::store::types::ToastMsgVariant::Error);
    }

    pub(crate) fn close_all_panels(&mut self) {
        self.slash_popup.close();
        self.model_select.close();
        self.agent_select.close();
        self.session_list.close();
        // Don't rebuild permission/question dialogs — they may have
        // pending requests from the server that need to stay queued.
        // Just hide the UI overlay; the requests survive for later.
        self.permission_dialog.close();
        self.question_dialog.close();
        self.rename_dialog.close();
        self.confirm_dialog.close();
        self.help.dismiss();
        self.panel = Panel::None;
    }

    /// Check if input contains an attachment command (/image), process it.
    /// Returns true if the text was consumed as an attachment command.
    fn handle_attachment_cmd(&mut self, text: &str) -> bool {
        // /image <path> — attach an image
        if let Some(path) = text.strip_prefix("/image ") {
            let path = path.trim();
            let attachment = crate::store::types::Attachment {
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                kind: crate::store::types::AttachmentKind::File {
                    path: path.to_string(),
                    lines: 0,
                },
            };
            self.active_session.add_attachment(attachment);
            self.store.push_toast(&format!("Attached: {}", path), crate::store::types::ToastMsgVariant::Success);
            return true;
        }
        // @<path> — reference a file in prompt context (kept in text)
        false
    }

    fn dispatch(&mut self, text: String) {
        // Handle attachment commands
        if self.handle_attachment_cmd(&text) {
            return;
        }
        let route = self.store.route.get();
        let sid = match route {
            Route::Home => {
                if let Some(ref api) = self.api {
                    // 创建新 session 前先重置(防御):即使经非 /new 路径进 Home,
                    // 也确保新会话不携带旧 session 的 messages/状态。
                    self.active_session.reset_for_new_session();
                    match api.create_session(None, None) {
                        Ok(info) => {
                            self.active_session.set_session_id(&info.id);
                            self.store.navigate(Route::Session { session_id: info.id.clone() });
                            self.reload_session_list();
                            info.id
                        }
                        Err(e) => { self.active_session.run_status.set(RunStatus::Error(format!("{}", e))); return; }
                    }
                } else { "echo".to_string() }
            }
            Route::Session { session_id } => session_id,
            // Settings 不发 prompt(输入在 Settings 应该走不到 dispatch,但 match 必须穷尽)。
            Route::Settings => return,
        };
        // Tell the transport to forward events for this session
        self.sf_tx.send_replace(Some(sid.clone()));
        let mid = format!("user-{}", ts_now());
        self.active_session.push_user_message(&mid, &text);
        if let Some(ref api) = self.api {
            self.active_session.run_status.set(RunStatus::Sending);
            self.layout_dirty = true;
            // Pull the user's current selections from the store so the
            // backend uses the model/agent/mode picked in the dialogs
            // instead of the workspace default.
            //
            // `selected_mode` 契约为 `"kind:id"` 复合字符串（对齐 web
            // `App.tsx:836`：`selectedMode.split(":", 2)` → 分流到
            // agent / scheduler_profile 两个槽）。agent kind 复合时优先级
            // 高于 selected_agent，避免 mode 与 agent 双权威打架。
            let model = self.store.selected_model.get();
            let mode = self.store.selected_mode.get();
            let (agent, scheduler_profile) = match mode.as_deref().and_then(|s| s.split_once(':')) {
                Some(("agent", id))   => (Some(id.to_string()), None),
                Some(("preset", id))  => (None, Some(id.to_string())),
                Some(("profile", id)) => (None, Some(id.to_string())),
                _ => (self.store.selected_agent.get(), None),
            };

            // ── 火：spawn send_prompt_with 到后台，主线程立即返回 ──
            // 关键修复：原在按键同步回调里 block_on 等网络往返（local_prompt
            // 触发 LLM 调度），冻死 revue 事件循环，乐观 push_user_message 的
            // 渲染帧出不来（"按 Enter 很久没反应"）。spawn 后主线程立刻返回，
            // 乐观消息瞬间上屏；回执经 dispatch_outcomes channel 在 Event::Tick
            // drain 回收（Sent→Running / Failed→回滚）。
            let api_c = api.clone();
            let tx = self.dispatch_outcomes.sender();
            let sid_c = sid.clone();
            let mid_c = mid.clone();
            let text_c = text.clone();
            api.handle().spawn(async move {
                let r = api_c
                    .send_prompt_with_async(&sid_c, text_c, agent, scheduler_profile, model, None)
                    .await;
                let _ = match r {
                    Ok(resp) => tx.send(dispatch_outcome::DispatchOutcome::Sent {
                        session_id: sid_c,
                        status: resp.status,
                    }),
                    Err(e) => tx.send(dispatch_outcome::DispatchOutcome::Failed {
                        session_id: sid_c,
                        user_msg_id: mid_c,
                        error: format!("{e}"),
                    }),
                };
            });
            // title_refresh_pending 改由 Tick drain 的 Sent 分支置位（确认服务端
            // 接收后才请求刷新，比发送前盲置更准）。
        } else {
            // Echo mode (no API) — respond immediately
            self.active_session.push_assistant_delta(&format!("echo-{}", ts_now()), &format!("[echo] {}", text));
            self.active_session.run_status.set(RunStatus::Idle);
            self.store.navigate(Route::Session { session_id: "echo".into() });
        }
    }

    /// Load historical messages for an existing session from the API.
    ///
    /// Delegates to the free `eager_load_session_messages` so the
    /// SessionList dialog's Enter handler and the startup
    /// `--session`/`AGENDAO_TUI_SESSION` path share one implementation.
    pub(crate) fn load_session_messages(&self, session_id: &str) {
        eager_load_session_messages(&self.active_session, self.api.as_ref(), session_id);
    }

    /// 打开一个已存在的会话(土律·第四条·单点权威)。SessionList 的 Enter、
    /// sidebar session tree 点击、以及未来任何"切到某会话"入口都复用此路径:
    /// reset(清旧会话 messages/scroll/telemetry 残留) → set id → 更新事件过滤
    /// (sf_tx,让 transport 只转发该 session 的 FrontendEvent) → 加载历史消息 →
    /// navigate → 关任何 panel。避免多入口各自拼一套"打开会话"逻辑而漂移。
    pub(crate) fn open_session(&mut self, session_id: &str) {
        self.active_session.reset_for_new_session();
        self.active_session.set_session_id(session_id);
        // 播种 usage（水律·回流）：打开即拉 `GET /session/{id}` 的
        // `telemetry.usage`（持久化累计 token/成本）,底部信息条不等下一次
        // SessionProjectionReplaced 投影——runtime 端点的 usage 只在运行中有。
        let mut ctx_tokens: u64 = 0;
        if let Some(api) = self.api.as_ref() {
            if let Ok(info) = api.get_session(session_id) {
                if let Some(t) = info.telemetry {
                    let u = t.usage;
                    ctx_tokens = u.context_tokens;
                    self.active_session.set_token_usage(
                        u.input_tokens,
                        u.output_tokens,
                        u.reasoning_tokens,
                        u.cache_read_tokens,
                        u.cache_miss_tokens,
                        u.cache_write_tokens,
                        u.context_tokens,
                        u.total_cost,
                    );
                }
            }
        }
        self.sf_tx.send_replace(Some(session_id.to_string()));
        self.load_session_messages(session_id);
        // context 进度条播种：context_tokens（最新 turn 占用）÷ 模型 context_window。
        // 模型解析链：selected_model（用户当前选择）→ session_model（会话最后
        // 使用的模型,更贴合历史会话）。投影的 compaction summary 到达后以更
        // 权威口径覆盖（同一语义,金律·无第二真相）。
        if ctx_tokens > 0 {
            let model_str = self
                .store
                .selected_model
                .get()
                .or_else(|| self.active_session.session_model.get());
            if let Some(model_str) = model_str {
                if let Some((pid, mid)) = model_str.split_once('/') {
                    let limit = self
                        .store
                        .providers
                        .get()
                        .iter()
                        .find(|p| p.id == pid)
                        .and_then(|p| p.models.iter().find(|m| m.id == mid))
                        .and_then(|m| m.context_window);
                    if let Some(limit) = limit.filter(|l| *l > 0) {
                        let pct = ((ctx_tokens as f64 / limit as f64) * 100.0).min(100.0) as u8;
                        self.active_session.set_context_pct(pct);
                    }
                }
            }
        }
        self.store
            .navigate(Route::Session { session_id: session_id.to_string() });
        self.panel = Panel::None;
    }

    /// 从 API 拉取当前 cwd 下的会话列表,回灌 `AppStore.session_list`,并
    /// 重建 sidebar 导航树(水→木:回流数据滋养下一轮导航输入)。
    pub(crate) fn reload_session_list(&mut self) {
        let Some(api) = self.api.as_ref() else { return };
        let cwd = self.store.working_dir.get();
        let cwd_filter = if cwd.is_empty() { None } else { Some(cwd.clone()) };
        match api.list_sessions_in_directory(cwd_filter) {
            Ok(sessions) => {
                let items: Vec<crate::store::types::SessionListItem> = sessions
                    .iter()
                    .map(crate::telemetry::session_tree::map_api_session_item)
                    .collect();
                self.store.session_list.set(items);
                self.refresh_sidebar_session_tree();
            }
            Err(e) => {
                self.store.push_toast(
                    &format!("Failed to refresh session list: {}", e),
                    crate::store::types::ToastMsgVariant::Error,
                );
            }
        }
    }

    /// 从 `AppStore.session_list` 重建 sidebar session 导航树(单点权威)。
    /// 每个节点带 `NavigateSession(id)` intent,供鼠标点击 → `open_session`。
    /// 展开态从 `self.session_tree_expanded` 重放(默认全折叠),并自动展开
    /// 当前活跃会话的祖先链(保证它可见,金律·活跃路径不被折叠吞掉)。
    pub(crate) fn refresh_sidebar_session_tree(&mut self) {
        let sessions = self.store.session_list.get();
        let cwd = self.store.working_dir.get();
        // 活跃会话祖先链自动展开：沿 parent_id 上溯,把每个祖先记入展开态。
        if let Some(active_id) = self.active_session.get_session_id() {
            let mut cursor = Some(active_id.as_str());
            let mut hops = 0;
            while let Some(id) = cursor {
                if hops > 32 {
                    break; // 防御环
                }
                hops += 1;
                cursor = sessions
                    .iter()
                    .find(|s| s.id == id)
                    .and_then(|s| s.parent_id.as_deref());
                if let Some(pid) = cursor {
                    self.session_tree_expanded.insert(pid.to_string());
                }
            }
        }
        let nodes =
            crate::telemetry::build_session_nav_tree(&sessions, &cwd, &self.session_tree_expanded);
        self.active_session.sidebar_trees.update(|t| {
            t.session_nodes = nodes;
        });
        self.layout_dirty = true;
    }

    pub(crate) fn dispatch_shell(&mut self, _cmd: String) {}
    pub(crate) fn toggle_help(&mut self) {
        if self.help.visible { self.help.dismiss(); self.panel = Panel::None; }
        else { self.help.toggle(); self.panel = Panel::Help; }
    }

    /// General 分类 body 键路由(木律·唯一输入权威)。
    ///
    /// ↑/↓ 移动选中行(`settings_general_selected`);Enter/Space/←/→ 触发当前行的
    /// toggle。**关键**:toggle 不新增写路径,而是复用 `execute_slash_action` 的既有
    /// `Toggle*` 权威(土律·第四条·单点权威 + 木克土:输入变体复用同一权威)。
    /// 这样 slash 命令与 Settings 行对同一偏好读写同源,不会出现两份"真相"。
    ///
    /// 返回 true = 消费。Esc 已由上层排除(冒泡 → navigate_home)。
    fn handle_general_body_key(&mut self, key: &Key) -> bool {
        use crate::store::types::GeneralRow;
        let n = GeneralRow::ALL.len();
        match key {
            Key::Up | Key::Down => {
                let dir: i32 = if matches!(key, Key::Up) { -1 } else { 1 };
                let cur = self.store.settings_general_selected.get().min(n - 1);
                let nxt = (((cur as i32 + dir) % n as i32) + n as i32) % n as i32;
                self.store.settings_general_selected.set(nxt as usize);
                self.layout_dirty = true;
                true
            }
            Key::Enter | Key::Char(' ') | Key::Left | Key::Right => {
                let row = GeneralRow::ALL[self.store.settings_general_selected.get().min(n - 1)];
                // Theme 是多值循环（4 套），←/→ 分方向；Enter/Space 约定=下一个。
                let action = general_row_toggle_action(row, matches!(key, Key::Left));
                // 复用单点 toggle 权威。execute_slash_action 会顺带 panel=None +
                // prompt.clear()——在 Settings 路由下无副作用(prompt 不渲染、panel 已 None)。
                self.execute_slash_action(action);
                self.layout_dirty = true;
                true
            }
            _ => false,
        }
    }

    /// Keybindings 分类 body 键路由:只读参考的滚动(阴面记账 scroll signal)。
    /// 数据源唯一 = `dialog::help::KEYBINDINGS`;此处只推 scroll,渲染层据此开窗。
    /// scroll clamp 到 `[0, total-1]`(渲染层再 clamp 视窗尾),不依赖 pane 高度。
    fn handle_keybindings_body_key(&mut self, key: &Key) -> bool {
        let total = crate::dialog::help::KEYBINDINGS.len();
        let max_scroll = total.saturating_sub(1);
        const PAGE: usize = 10;
        let cur = self.store.settings_keybindings_scroll.get();
        let next = match key {
            Key::Up => cur.saturating_sub(1),
            Key::Down => (cur + 1).min(max_scroll),
            Key::PageUp => cur.saturating_sub(PAGE),
            Key::PageDown => (cur + PAGE).min(max_scroll),
            Key::Home => 0,
            Key::End => max_scroll,
            _ => return false,
        };
        if next != cur {
            self.store.settings_keybindings_scroll.set(next);
            self.layout_dirty = true;
        }
        true
    }

    /// Settings 路由鼠标交互（木生火：点击直达选中/编辑/动作）。
    ///
    /// 命中几何与 `screen::settings` 布局常量同源（金律·渲染/命中一份口径）；
    /// 所有动作复用键盘既有的写路径（settings_* helper / toggle_settings_mcp /
    /// decide_settings_skill_proposal / execute_slash_action，土律·单点权威）。
    /// 返回 true = 消费。
    pub(crate) fn handle_settings_mouse(&mut self, m: &revue::event::MouseEvent) -> bool {
        use crate::screen::settings as geo;
        use crate::store::types::{GeneralRow, SettingsCategory, SettingsFocusPane};

        let x_off: u16 = if self.sidebar_visible {
            crate::app::SIDEBAR_WIDTH + 1
        } else {
            0
        };
        let x2 = x_off + geo::CATEGORIES_W + geo::VLINE_W; // Providers/List 栏左缘
        let x3 = x2 + geo::PROVIDERS_W + geo::VLINE_W; // Details 栏左缘
        let pane_h = self.terminal_h; // = 渲染侧 ctx.area.height（build 的 pane_height）
        let footer_y = pane_h.saturating_sub(3); // 各栏 footer 行（hint + 底呼吸之上）

        // ── 分类栏：点击切换分类，焦点移到 body（与 Tab 同语义）──
        if m.x >= x_off && m.x < x_off + geo::CATEGORIES_W && m.y >= geo::PANE_FIRST_ROW_Y {
            let idx = (m.y - geo::PANE_FIRST_ROW_Y) as usize;
            if idx < SettingsCategory::ALL.len() {
                self.store.settings_category.set(SettingsCategory::ALL[idx]);
                self.store.settings_focus_pane.set(SettingsFocusPane::Providers);
                self.layout_dirty = true;
                return true;
            }
            return false;
        }

        match self.store.settings_category.get() {
            // ── General body 行：点击 = 选中 + 触发同一 toggle 权威 ──
            SettingsCategory::General => {
                if m.x < x2 {
                    return false;
                }
                for (i, row) in GeneralRow::ALL.iter().copied().enumerate() {
                    let row_y = geo::GENERAL_FIRST_ROW_Y + geo::GENERAL_ROW_STRIDE * i as u16;
                    if m.y == row_y || m.y == row_y + 1 {
                        self.store.settings_general_selected.set(i);
                        self.execute_slash_action(general_row_toggle_action(row, false));
                        self.layout_dirty = true;
                        return true;
                    }
                }
                false
            }

            SettingsCategory::ModelSettings => {
                // 编辑表单激活时：Providers 列表点选锁定（防表单上下文漂移），
                // 仅 Details 字段块可点（点击 = 聚焦字段 / Protocol 循环）。
                if self.settings_edit.active {
                    if m.x < x3 {
                        return true; // 消费但不动作（表单进行中，锁定左两栏）
                    }
                    use crate::app::settings_edit_state::SettingsEditField;
                    // Add/Edit 均为 4 字段（Edit 含 Name rename 字段）：
                    // Name@3 / BaseUrl@7 / Protocol@11 / ApiKey@15（块高 3 + 空 1）。
                    let fields: &[SettingsEditField] = &[
                        SettingsEditField::Name,
                        SettingsEditField::BaseUrl,
                        SettingsEditField::Protocol,
                        SettingsEditField::ApiKey,
                    ];
                    let rel = m.y.wrapping_sub(geo::EDIT_FIELD_BLOCK_Y);
                    let idx = (rel / geo::EDIT_FIELD_BLOCK_STRIDE) as usize;
                    let in_block = rel % geo::EDIT_FIELD_BLOCK_STRIDE < 3;
                    if in_block && idx < fields.len() {
                        let f = fields[idx];
                        if f == SettingsEditField::Protocol
                            && self.settings_edit.focus == SettingsEditField::Protocol
                        {
                            // 已聚焦 Protocol 时再点 = 前进一个选项（同 → 键）。
                            let n = crate::app::settings_edit_state::PROTOCOL_OPTIONS.len();
                            self.settings_edit.protocol_idx =
                                (self.settings_edit.protocol_idx + 1) % n.max(1);
                        } else {
                            self.settings_edit.focus = f;
                            // value 行（块内第 2 行）点击文本字段 → 光标定位到
                            // 点击字符（文本起点 = x3 + 2 缩进，与 field_block_editing 同源）。
                            if rel % geo::EDIT_FIELD_BLOCK_STRIDE == 1 && m.x >= x3 + 2 {
                                let char_idx = (m.x - x3 - 2) as usize;
                                match f {
                                    SettingsEditField::Name =>
                                        self.settings_edit.name_input.set_cursor(char_idx),
                                    SettingsEditField::BaseUrl =>
                                        self.settings_edit.base_url_input.set_cursor(char_idx),
                                    SettingsEditField::ApiKey =>
                                        self.settings_edit.api_key_input.set_cursor(char_idx),
                                    SettingsEditField::Protocol => {}
                                }
                            }
                        }
                        self.layout_dirty = true;
                        return true;
                    }
                    return true;
                }

                // Providers 栏：行点选 / footer "+ Add provider"。
                if m.x >= x2 && m.x < x2 + geo::PROVIDERS_W {
                    if m.y == footer_y {
                        self.settings_enter_add_provider();
                        return true;
                    }
                    if m.y < geo::PANE_FIRST_ROW_Y {
                        return false;
                    }
                    let providers = self.store.providers.get();
                    if providers.is_empty() {
                        return false;
                    }
                    let visible = pane_h.saturating_sub(5).max(1) as usize;
                    let sel_idx = self
                        .store
                        .settings_selected_provider
                        .get()
                        .and_then(|id| providers.iter().position(|p| p.id == id))
                        .unwrap_or(0);
                    let (start, end) = crate::dialog::backdrop::list_viewport_window(
                        providers.len(),
                        sel_idx,
                        visible,
                    );
                    let i = start + (m.y - geo::PANE_FIRST_ROW_Y) as usize;
                    if i < end {
                        self.settings_select_provider_by_id(providers[i].id.clone());
                        self.store.settings_focus_pane.set(SettingsFocusPane::Providers);
                        self.layout_dirty = true;
                        return true;
                    }
                    return false;
                }

                // Details 栏：header 状态 pill 点击 = toggle disabled；⚡ Test = 测连接；
                // models 行点选 + 行尾 ✎/✕。
                if m.x >= x3 {
                    // header 行（y=1）：⚡ Test 右对齐文本（末尾 2 空格内收）→ 测连接；
                    // 状态 pill（"◆ name" 之后 gap(1) 起）→ toggle enable/disable。
                    if m.y == 1 {
                        let w = self.terminal_w;
                        const TEST_W: u16 = 9; // "⚡ Test" + 2 空格
                        if m.x >= w.saturating_sub(TEST_W) {
                            self.settings_test_provider_connection();
                            return true;
                        }
                        let providers = self.store.providers.get();
                        let sel_id = self.store.settings_selected_provider.get();
                        if let Some(p) = sel_id
                            .as_ref()
                            .and_then(|id| providers.iter().find(|p| &p.id == id))
                        {
                            let label_w = 4 + p.name.chars().count() as u16; // "  ◆ " + name
                            let pill_w: u16 = if p.disabled { 10 } else { 9 };
                            let px0 = x3 + label_w + 1;
                            if m.x >= px0 && m.x < px0 + pill_w {
                                self.settings_toggle_provider_disabled();
                                return true;
                            }
                        }
                        return false;
                    }
                    const MODELS_FIRST_ROW_Y: u16 = 17; // 见 screen::settings 组装几何
                    if m.y < MODELS_FIRST_ROW_Y {
                        return false;
                    }
                    let providers = self.store.providers.get();
                    let sel_provider_id = self.store.settings_selected_provider.get();
                    let Some(provider) = sel_provider_id
                        .as_ref()
                        .and_then(|id| providers.iter().find(|p| &p.id == id))
                    else {
                        return false;
                    };
                    let i = (m.y - MODELS_FIRST_ROW_Y) as usize;
                    if i >= provider.models.len() {
                        return false;
                    }
                    let model_key = provider.models[i].id.clone();
                    self.store.settings_selected_model.set(Some(model_key));
                    self.store.settings_focus_pane.set(SettingsFocusPane::Details);
                    let w = self.terminal_w;
                    if m.x >= w.saturating_sub(4) {
                        self.settings_confirm_delete_model();
                    } else if m.x >= w.saturating_sub(6) {
                        self.settings_open_edit_model();
                    }
                    self.layout_dirty = true;
                    return true;
                }
                false
            }

            SettingsCategory::McpServers => {
                // 列表栏行点选；点行尾开关 pill 区（末 10 列 = pill 7 + 空格 + dot + 空格，
                // 与渲染同源）= 启停切换（同 `t` 权威）。
                if m.x >= x2 && m.x < x2 + geo::LIST_COL_W {
                    if m.y < geo::PANE_FIRST_ROW_Y {
                        return false;
                    }
                    let rows = self.store.settings_mcp.get();
                    if rows.is_empty() {
                        return false;
                    }
                    let visible = pane_h.saturating_sub(5).max(1) as usize;
                    let sel = self
                        .store
                        .settings_mcp_selected
                        .get()
                        .min(rows.len() - 1);
                    let (start, end) = crate::dialog::backdrop::list_viewport_window(
                        rows.len(),
                        sel,
                        visible,
                    );
                    let i = start + (m.y - geo::PANE_FIRST_ROW_Y) as usize;
                    if i < end {
                        self.store.settings_mcp_selected.set(i);
                        self.store.settings_focus_pane.set(SettingsFocusPane::Providers);
                        if m.x >= x2 + geo::LIST_COL_W.saturating_sub(10) {
                            self.toggle_settings_mcp_enabled();
                        }
                        self.layout_dirty = true;
                        return true;
                    }
                    return false;
                }
                // Detail Status pill 点击 → toggle connect/disconnect（同 c/d 权威）；
                // 其后 On/Off pill 点击 → 启停切换（同 `t` 权威）。
                if m.x >= x3 && m.y == 1 {
                    let rows = self.store.settings_mcp.get();
                    if rows.is_empty() {
                        return false;
                    }
                    let sel = self
                        .store
                        .settings_mcp_selected
                        .get()
                        .min(rows.len() - 1);
                    let r = &rows[sel];
                    let header_w = 4 + r.name.chars().count() as u16; // "  ⚔ " + name
                    let pill_w: u16 = if r.is_connected() { 11 } else { 14 };
                    let px0 = x3 + header_w + 1;
                    if m.x >= px0 && m.x < px0 + pill_w {
                        let connect = !r.is_connected();
                        self.toggle_settings_mcp(connect);
                        self.store.settings_focus_pane.set(SettingsFocusPane::Details);
                        return true;
                    }
                    // gap(1) 后的 [ On ]/[ Off ] pill（宽 7，与渲染同源）。
                    let px1 = px0 + pill_w + 1;
                    if m.x >= px1 && m.x < px1 + 7 {
                        self.toggle_settings_mcp_enabled();
                        self.store.settings_focus_pane.set(SettingsFocusPane::Details);
                        return true;
                    }
                }
                false
            }

            SettingsCategory::Plugins => {
                // 列表栏行点选（同 MCP 口径）；点行尾开关 pill 区（末 9 列 =
                // pill 7 + 2 空格，与渲染同源）= 启停切换（同 `t` 权威）。
                if m.x >= x2 && m.x < x2 + geo::LIST_COL_W {
                    if m.y < geo::PANE_FIRST_ROW_Y {
                        return false;
                    }
                    let rows = self.store.settings_plugins.get();
                    if rows.is_empty() {
                        return false;
                    }
                    let visible = pane_h.saturating_sub(5).max(1) as usize;
                    let sel = self
                        .store
                        .settings_plugins_selected
                        .get()
                        .min(rows.len() - 1);
                    let (start, end) = crate::dialog::backdrop::list_viewport_window(
                        rows.len(),
                        sel,
                        visible,
                    );
                    let i = start + (m.y - geo::PANE_FIRST_ROW_Y) as usize;
                    if i < end {
                        self.store.settings_plugins_selected.set(i);
                        self.store.settings_focus_pane.set(SettingsFocusPane::Providers);
                        if m.x >= x2 + geo::LIST_COL_W.saturating_sub(9) {
                            self.toggle_settings_plugin();
                        }
                        self.layout_dirty = true;
                        return true;
                    }
                    return false;
                }
                false
            }

            SettingsCategory::Skills => {
                // 列表栏行点选（树状展开口径与渲染同源）；点类目头 = 选中 + 折叠/展开；
                // 点行尾开关 pill 区（末 9 列，与渲染的 pill + 2 空格同源）= 启停切换。
                if m.x >= x2 && m.x < x2 + geo::LIST_COL_W {
                    if m.y < geo::PANE_FIRST_ROW_Y {
                        return false;
                    }
                    let rows = self.store.settings_skills.get();
                    let collapsed = self.store.settings_skills_collapsed.get();
                    let lines = crate::store::types::flatten_settings_skill_rows(&rows, &collapsed);
                    if lines.is_empty() {
                        return false;
                    }
                    let visible = pane_h.saturating_sub(5).max(1) as usize;
                    let sel = self
                        .store
                        .settings_skills_selected
                        .get()
                        .min(lines.len() - 1);
                    let (start, end) = crate::dialog::backdrop::list_viewport_window(
                        lines.len(),
                        sel,
                        visible,
                    );
                    let i = start + (m.y - geo::PANE_FIRST_ROW_Y) as usize;
                    if i < end {
                        self.store.settings_skills_selected.set(i);
                        self.store.settings_focus_pane.set(SettingsFocusPane::Providers);
                        let in_pill_zone = m.x >= x2 + geo::LIST_COL_W.saturating_sub(9);
                        if in_pill_zone {
                            self.toggle_settings_skill();
                        } else if let crate::store::types::SettingsSkillLine::Category { .. } =
                            &lines[i]
                        {
                            self.settings_toggle_skill_group();
                        }
                        self.layout_dirty = true;
                        return true;
                    }
                    return false;
                }
                // Detail footer hint 的 Approve/Reject 命中区（同 a/r 权威；
                // catalog 行由 decide 内部 toast 拦截，不会误动作）。
                if m.x >= x3 && m.y == footer_y {
                    if m.x < x3 + 13 {
                        self.decide_settings_skill_proposal(true);
                        self.store.settings_focus_pane.set(SettingsFocusPane::Details);
                        return true;
                    }
                    if m.x < x3 + 27 {
                        self.decide_settings_skill_proposal(false);
                        self.store.settings_focus_pane.set(SettingsFocusPane::Details);
                        return true;
                    }
                }
                false
            }

            SettingsCategory::Tools => {
                // 列表栏行点选（同 Skills 口径）；点类目头 = 选中 + 折叠/展开；
                // 点行尾开关 pill 区 = 启停切换（protected 行由 toggle 内部 toast 拦截）。
                if m.x >= x2 && m.x < x2 + geo::LIST_COL_W {
                    if m.y < geo::PANE_FIRST_ROW_Y {
                        return false;
                    }
                    let rows = self.store.settings_tools.get();
                    let collapsed = self.store.settings_tools_collapsed.get();
                    let lines = crate::store::types::flatten_settings_tool_rows(&rows, &collapsed);
                    if lines.is_empty() {
                        return false;
                    }
                    let visible = pane_h.saturating_sub(5).max(1) as usize;
                    let sel = self
                        .store
                        .settings_tools_selected
                        .get()
                        .min(lines.len() - 1);
                    let (start, end) = crate::dialog::backdrop::list_viewport_window(
                        lines.len(),
                        sel,
                        visible,
                    );
                    let i = start + (m.y - geo::PANE_FIRST_ROW_Y) as usize;
                    if i < end {
                        self.store.settings_tools_selected.set(i);
                        self.store.settings_focus_pane.set(SettingsFocusPane::Providers);
                        let in_pill_zone = m.x >= x2 + geo::LIST_COL_W.saturating_sub(9);
                        if in_pill_zone {
                            self.toggle_settings_tool();
                        } else if let crate::store::types::SettingsToolLine::Category { .. } =
                            &lines[i]
                        {
                            self.settings_toggle_tool_group();
                        }
                        self.layout_dirty = true;
                        return true;
                    }
                    return false;
                }
                false
            }

            _ => false,
        }
    }

    /// Settings 路由滚轮分发（水生木：滚轮转选中/滚动，不再穿透到背后 session）。
    pub(crate) fn settings_wheel(&mut self, dir: i32) -> bool {
        use crate::store::types::{GeneralRow, SettingsCategory, SettingsFocusPane};
        let category = self.store.settings_category.get();
        let focus = self.store.settings_focus_pane.get();
        match focus {
            SettingsFocusPane::Categories => {
                let cur = SettingsCategory::ALL
                    .iter()
                    .position(|&c| c == category)
                    .unwrap_or(0) as i32;
                let n = SettingsCategory::ALL.len() as i32;
                let nxt = (((cur + dir) % n) + n) % n;
                self.store.settings_category.set(SettingsCategory::ALL[nxt as usize]);
            }
            SettingsFocusPane::Providers => match category {
                SettingsCategory::ModelSettings => self.settings_move_provider(dir),
                SettingsCategory::McpServers => self.settings_move_mcp(dir),
                SettingsCategory::Skills => self.settings_move_skills(dir),
                SettingsCategory::Tools => self.settings_move_tools(dir),
                SettingsCategory::Plugins => self.settings_move_plugins(dir),
                SettingsCategory::General => {
                    let n = GeneralRow::ALL.len() as i32;
                    let cur = self.store.settings_general_selected.get() as i32;
                    let nxt = (((cur + dir) % n) + n) % n;
                    self.store.settings_general_selected.set(nxt as usize);
                }
                SettingsCategory::Keybindings => {
                    let total = crate::dialog::help::KEYBINDINGS.len();
                    let cur = self.store.settings_keybindings_scroll.get();
                    let next = if dir < 0 {
                        cur.saturating_sub(3)
                    } else {
                        (cur + 3).min(total.saturating_sub(1))
                    };
                    self.store.settings_keybindings_scroll.set(next);
                }
                _ => {}
            },
            SettingsFocusPane::Details => match category {
                SettingsCategory::ModelSettings => self.settings_move_model(dir),
                SettingsCategory::McpServers => self.settings_move_mcp(dir),
                SettingsCategory::Skills => self.settings_move_skills(dir),
                SettingsCategory::Tools => self.settings_move_tools(dir),
                SettingsCategory::Plugins => self.settings_move_plugins(dir),
                SettingsCategory::General => {
                    let n = GeneralRow::ALL.len() as i32;
                    let cur = self.store.settings_general_selected.get() as i32;
                    let nxt = (((cur + dir) % n) + n) % n;
                    self.store.settings_general_selected.set(nxt as usize);
                }
                SettingsCategory::Keybindings => {
                    let total = crate::dialog::help::KEYBINDINGS.len();
                    let cur = self.store.settings_keybindings_scroll.get();
                    let next = if dir < 0 {
                        cur.saturating_sub(3)
                    } else {
                        (cur + 3).min(total.saturating_sub(1))
                    };
                    self.store.settings_keybindings_scroll.set(next);
                }
                _ => {}
            },
        }
        self.layout_dirty = true;
        true
    }

    /// Settings 全屏页键路由(火→土:键事件 → AppStore signals)。
    ///
    /// 阳面键 = 阴面写哪个 signal,完全镜像 SettingsFocusPane 三栏:
    ///   - `Tab`        → settings_focus_pane.next()(Categories→Providers→Details→…)
    ///   - `↑/↓`        → 按 focused 栏分别滚 settings_category / settings_selected_provider
    ///     (Details 栏暂无 selection 概念,不响应;Model 行编辑是 Part 7+ 的非目标)
    ///   - `Enter`      → Categories 栏:灰显项 toast"Coming soon";Providers 栏:no-op
    ///     (selected 已是 ↑/↓ 实时所选,Enter 只是 sticky 确认语义)
    ///
    /// 返回 true = 消费,false = 让位 prompt/全局键(目前 false 仅当 key 非 Tab/↑/↓/Enter)。
    pub(crate) fn handle_settings_key(&mut self, key: &Key) -> bool {
        use crate::store::types::{SettingsCategory, SettingsFocusPane};
        // ── editing in-place 优先(土律·第四条 单点权威)──
        // settings_edit.active 时把全部键交给 SettingsEditState.handle_key 收口;
        // Tab/Shift-Tab 切字段、Enter Submit、Esc Cancel、Protocol ←/→、其他派发当前 Input。
        // 编辑期间外层不接管任何键(包括 a/e/d/m 等)避免双重消费。
        if self.settings_edit.active {
            use crate::app::settings_edit_state::SettingsEditAction;
            match self.settings_edit.handle_key(key) {
                SettingsEditAction::Consumed => {
                    self.layout_dirty = true;
                    return true;
                }
                SettingsEditAction::Submit => {
                    self.submit_settings_edit();
                    // submit_settings_edit 内部 close() + refresh_providers_into_store
                    return true;
                }
                SettingsEditAction::Cancel => {
                    self.settings_edit.close();
                    self.layout_dirty = true;
                    return true;
                }
                SettingsEditAction::Pass => {
                    // 既未消费也未 submit/cancel(目前 active=false 才会到这,逻辑上不可达);
                    // 兜底为 false 让上层兜底(例如 Ctrl-C 退出)。
                    return false;
                }
            }
        }
        let category = self.store.settings_category.get();
        let focus = self.store.settings_focus_pane.get();

        // ── 非 ModelSettings / MCP / Skills / Tools / Plugins 的简单 body 路由 ──
        // General / Keybindings / About:两区(Categories | body)。
        // MCP / Skills / Tools / Plugins:三区(Categories | List=Providers | Details),走下方 match。
        if !matches!(
            category,
            SettingsCategory::ModelSettings
                | SettingsCategory::McpServers
                | SettingsCategory::Skills
                | SettingsCategory::Tools
                | SettingsCategory::Plugins
        ) && focus != SettingsFocusPane::Categories
            && !matches!(key, Key::Tab | Key::Escape)
        {
            return match category {
                SettingsCategory::General => self.handle_general_body_key(key),
                SettingsCategory::Keybindings => self.handle_keybindings_body_key(key),
                // About:body 无交互,消费导航键避免穿透。
                _ => matches!(key, Key::Up | Key::Down | Key::Enter | Key::Char(' ')),
            };
        }

        match key {
            Key::Tab => {
                let cur = self.store.settings_focus_pane.get();
                // 三栏循环:ModelSettings / MCP / Skills / Tools / Plugins。
                // 两区循环:General / Keybindings / About(Categories ⇄ Details)。
                let next = if matches!(
                    category,
                    SettingsCategory::ModelSettings
                        | SettingsCategory::McpServers
                        | SettingsCategory::Skills
                        | SettingsCategory::Tools
                        | SettingsCategory::Plugins
                ) {
                    cur.next()
                } else {
                    match cur {
                        SettingsFocusPane::Categories => SettingsFocusPane::Details,
                        _ => SettingsFocusPane::Categories,
                    }
                };
                self.store.settings_focus_pane.set(next);
                self.layout_dirty = true;
                true
            }
            Key::Up | Key::Down => {
                let focus = self.store.settings_focus_pane.get();
                let dir: i32 = if matches!(key, Key::Up) { -1 } else { 1 };
                match focus {
                    SettingsFocusPane::Categories => {
                        let cur = self.store.settings_category.get();
                        let idx = SettingsCategory::ALL.iter().position(|&c| c == cur).unwrap_or(0);
                        let n = SettingsCategory::ALL.len() as i32;
                        let nxt = (((idx as i32 + dir) % n) + n) % n;
                        self.store.settings_category.set(SettingsCategory::ALL[nxt as usize]);
                    }
                    SettingsFocusPane::Providers => {
                        match category {
                            SettingsCategory::McpServers => self.settings_move_mcp(dir),
                            SettingsCategory::Skills => self.settings_move_skills(dir),
                            SettingsCategory::Tools => self.settings_move_tools(dir),
                            SettingsCategory::Plugins => self.settings_move_plugins(dir),
                            _ => self.settings_move_provider(dir),
                        }
                    }
                    SettingsFocusPane::Details => {
                        match category {
                            SettingsCategory::McpServers => self.settings_move_mcp(dir),
                            SettingsCategory::Skills => self.settings_move_skills(dir),
                            SettingsCategory::Tools => self.settings_move_tools(dir),
                            SettingsCategory::Plugins => self.settings_move_plugins(dir),
                            _ => self.settings_move_model(dir),
                        }
                    }
                }
                self.layout_dirty = true;
                true
            }
            Key::Enter | Key::Char(' ') => {
                let focus = self.store.settings_focus_pane.get();
                if matches!(focus, SettingsFocusPane::Categories) {
                    let cat = self.store.settings_category.get();
                    // 潜入 body:三栏分类 → List(Providers);两区分类 → Details。
                    let body = if matches!(
                        cat,
                        SettingsCategory::ModelSettings
                            | SettingsCategory::McpServers
                            | SettingsCategory::Skills
                            | SettingsCategory::Tools
                            | SettingsCategory::Plugins
                    ) {
                        SettingsFocusPane::Providers
                    } else {
                        SettingsFocusPane::Details
                    };
                    self.store.settings_focus_pane.set(body);
                    self.layout_dirty = true;
                } else if category == SettingsCategory::Skills
                    && matches!(
                        focus,
                        SettingsFocusPane::Providers | SettingsFocusPane::Details
                    )
                {
                    // Skills 树：类目头行 Enter/Space = 折叠/展开；数据行 no-op。
                    self.settings_toggle_skill_group();
                } else if category == SettingsCategory::Tools
                    && matches!(
                        focus,
                        SettingsFocusPane::Providers | SettingsFocusPane::Details
                    )
                {
                    // Tools 树：类目头行 Enter/Space = 折叠/展开；数据行 no-op。
                    self.settings_toggle_tool_group();
                }
                true
            }
            // Provider/MCP CRUD / Plugins 安装 / Skills approve(木律·唯一入口)。
            Key::Char('a') => {
                let focus = self.store.settings_focus_pane.get();
                let cat = self.store.settings_category.get();
                if !matches!(
                    focus,
                    SettingsFocusPane::Providers | SettingsFocusPane::Details
                ) {
                    return false;
                }
                match cat {
                    SettingsCategory::ModelSettings
                        if matches!(focus, SettingsFocusPane::Providers) =>
                    {
                        // 主路径：弹 ProviderEditDialog（不再走进隐蔽的 in-place 表单；
                        // in-place Add 仍可由鼠标 "+ Add provider" 进入）。
                        self.settings_open_add_provider();
                        true
                    }
                    SettingsCategory::Skills => {
                        self.decide_settings_skill_proposal(true);
                        true
                    }
                    SettingsCategory::McpServers => {
                        self.settings_open_add_mcp();
                        true
                    }
                    SettingsCategory::Plugins => {
                        self.settings_open_install_plugin();
                        true
                    }
                    _ => false,
                }
            }
            Key::Char('r') => {
                let focus = self.store.settings_focus_pane.get();
                let cat = self.store.settings_category.get();
                if cat == SettingsCategory::Skills
                    && matches!(
                        focus,
                        SettingsFocusPane::Providers | SettingsFocusPane::Details
                    )
                {
                    self.decide_settings_skill_proposal(false);
                    true
                } else {
                    false
                }
            }
            Key::Char('c') => {
                let focus = self.store.settings_focus_pane.get();
                let cat = self.store.settings_category.get();
                if cat == SettingsCategory::McpServers
                    && matches!(
                        focus,
                        SettingsFocusPane::Providers | SettingsFocusPane::Details
                    )
                {
                    self.toggle_settings_mcp(true);
                    true
                } else {
                    false
                }
            }
            // `m` 仅在 ModelSettings Details focused 时响应:新增当前 provider 的 model。
            Key::Char('m') => {
                if self.store.settings_category.get() != SettingsCategory::ModelSettings {
                    return false;
                }
                if !matches!(self.store.settings_focus_pane.get(), SettingsFocusPane::Details) {
                    return false;
                }
                self.settings_open_add_model();
                true
            }
            // `t`：ModelSettings Details = 测试连接；Skills/Tools/Plugins/MCP 列表或详情 = 启停开关。
            Key::Char('t') => {
                let cat = self.store.settings_category.get();
                let focus = self.store.settings_focus_pane.get();
                match cat {
                    SettingsCategory::ModelSettings => {
                        if !matches!(focus, SettingsFocusPane::Details) {
                            return false;
                        }
                        self.settings_test_provider_connection();
                        true
                    }
                    SettingsCategory::Skills => {
                        if !matches!(
                            focus,
                            SettingsFocusPane::Providers | SettingsFocusPane::Details
                        ) {
                            return false;
                        }
                        self.toggle_settings_skill();
                        true
                    }
                    SettingsCategory::Tools => {
                        if !matches!(
                            focus,
                            SettingsFocusPane::Providers | SettingsFocusPane::Details
                        ) {
                            return false;
                        }
                        self.toggle_settings_tool();
                        true
                    }
                    SettingsCategory::McpServers => {
                        if !matches!(
                            focus,
                            SettingsFocusPane::Providers | SettingsFocusPane::Details
                        ) {
                            return false;
                        }
                        self.toggle_settings_mcp_enabled();
                        true
                    }
                    SettingsCategory::Plugins => {
                        if !matches!(
                            focus,
                            SettingsFocusPane::Providers | SettingsFocusPane::Details
                        ) {
                            return false;
                        }
                        self.toggle_settings_plugin();
                        true
                    }
                    _ => false,
                }
            }
            Key::Char('e') => {
                let cat = self.store.settings_category.get();
                // MCP:e = 编辑选中 server（弹 McpEditDialog，写 PUT /config/mcp/{key}）。
                if cat == SettingsCategory::McpServers {
                    if !matches!(
                        self.store.settings_focus_pane.get(),
                        SettingsFocusPane::Providers | SettingsFocusPane::Details
                    ) {
                        return false;
                    }
                    self.settings_open_edit_mcp();
                    return true;
                }
                if cat != SettingsCategory::ModelSettings {
                    return false;
                }
                let focus = self.store.settings_focus_pane.get();
                match focus {
                    SettingsFocusPane::Providers => {
                        // 主路径：弹 ProviderEditDialog 编辑 provider
                        // （Base URL/Protocol/API key 不再只读）。
                        self.settings_open_edit_provider();
                        true
                    }
                    SettingsFocusPane::Details => {
                        // 有选中 model → 编辑 model；无（空 provider / 未建立选中）
                        // → 回退到编辑 provider 本身，与 Details 面板 "e: Edit"
                        // hint 同语义（API key/Base URL 行的动作提示不再死路）。
                        if self.store.settings_selected_model.get().is_some() {
                            self.settings_open_edit_model();
                        } else {
                            self.settings_open_edit_provider();
                        }
                        true
                    }
                    SettingsFocusPane::Categories => false,
                }
            }
            // `E`（Providers 聚焦）：legacy in-place 编辑（Details pane 内嵌表单）。
            // 主路径已迁到 ProviderEditDialog（`e`），此键保留原表单给习惯它的用户。
            Key::Char('E') => {
                if self.store.settings_category.get() != SettingsCategory::ModelSettings {
                    return false;
                }
                if self.store.settings_focus_pane.get() != SettingsFocusPane::Providers {
                    return false;
                }
                self.settings_enter_edit_provider();
                true
            }
            // `x`：Skills 删 skill / MCP 删 server / Plugins 删 managed 条目
            // （与 `d` 同一删除权威，木克土:输入变体复用同一权威）。
            Key::Char('x') => {
                let cat = self.store.settings_category.get();
                if !matches!(
                    self.store.settings_focus_pane.get(),
                    SettingsFocusPane::Providers | SettingsFocusPane::Details
                ) {
                    return false;
                }
                match cat {
                    SettingsCategory::Skills => {
                        self.settings_confirm_delete_skill();
                        true
                    }
                    SettingsCategory::McpServers => {
                        self.settings_confirm_delete_mcp();
                        true
                    }
                    SettingsCategory::Plugins => {
                        self.settings_confirm_delete_plugin();
                        true
                    }
                    _ => false,
                }
            }
            Key::Char('d') => {
                let focus = self.store.settings_focus_pane.get();
                let cat = self.store.settings_category.get();
                if cat == SettingsCategory::McpServers
                    && matches!(
                        focus,
                        SettingsFocusPane::Providers | SettingsFocusPane::Details
                    )
                {
                    self.toggle_settings_mcp(false);
                    return true;
                }
                if cat == SettingsCategory::Skills
                    && matches!(
                        focus,
                        SettingsFocusPane::Providers | SettingsFocusPane::Details
                    )
                {
                    self.settings_confirm_delete_skill();
                    return true;
                }
                if cat == SettingsCategory::Plugins
                    && matches!(
                        focus,
                        SettingsFocusPane::Providers | SettingsFocusPane::Details
                    )
                {
                    self.settings_confirm_delete_plugin();
                    return true;
                }
                if cat != SettingsCategory::ModelSettings {
                    return false;
                }
                match focus {
                    SettingsFocusPane::Providers => {
                        self.settings_confirm_delete_provider();
                        true
                    }
                    SettingsFocusPane::Details => {
                        self.settings_confirm_delete_model();
                        true
                    }
                    SettingsFocusPane::Categories => false,
                }
            }
            _ => false,
        }
    }
}

// ── Free helpers used by both `dispatch` and `run_app_with_config` ──

pub(crate) fn ts_now() -> String {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| format!("{}", d.as_millis()))
        .unwrap_or_default()
}

/// Pull historical messages for `session_id` from the API and push
/// them into the SessionStore. Used by both the SessionList dialog's
/// Enter handler (via AppHandler::load_session_messages) and the
/// startup `AGENDAO_TUI_SESSION`/`--session` path (via run_app_with_config).
///
/// Walks every persisted MessagePart and routes it to the matching
/// transcript block — text/reasoning go to assistant/thinking,
/// tool_call goes to ToolCall, tool_result goes to ToolResult. Reset
/// the transcript first so switching sessions doesn't append to the
/// previous one.
///
/// The previous implementation collected only `part.text` from each
/// message, so any historical session that contained tool calls (the
/// common case for build-mode runs) loaded as a stream of plain
/// assistant paragraphs with no tool context — making old sessions
/// look fundamentally different from live ones.
pub(crate) fn eager_load_session_messages(
    active_session: &crate::store::session_store::SessionStore,
    api: Option<&crate::bridge::api::ApiBridge>,
    session_id: &str,
) {
    use crate::store::types::ToolPhase;
    let Some(api) = api else { return };
    active_session.messages.update(|m| m.clear());
    // 同步 session title 到 header 用的 active_session.title。此前该 Signal 只在
    // 手动 rename 时更新，加载/切换 session 后恒显初始值 "New Session"——服务端
    // 已用 LLM 生成真实 title 入库（ensure_default_session_title），但无回流通道，
    // 这里从权威（get_session）拉取同步，闭合状态所有权（阴面唯一真相 → 阳面渲染）。
    if let Ok(info) = api.get_session(session_id) {
        active_session.title.set(info.title);
    }
    match api.get_messages(session_id) {
        Ok(msgs) => {
            // 记录最后一个带 model 的 assistant 消息（context 进度条的
            // model fallback——会话自己用过的模型比全局 selected_model 更准）。
            let mut last_model: Option<String> = None;
            for msg in msgs {
                if msg.role == "assistant" {
                    if let Some(ref m) = msg.model {
                        if !m.trim().is_empty() {
                            last_model = Some(m.clone());
                        }
                    }
                }
                for (part_idx, part) in msg.parts.iter().enumerate() {
                    let pid = format!("api-{}-{}", msg.id, part_idx);
                    match part.part_type.as_str() {
                        "text" => {
                            let Some(text) = part.text.as_deref() else { continue };
                            if text.is_empty() { continue };
                            if msg.role == "user" || msg.role == "human" {
                                active_session.push_user_message(&msg.id, text);
                            } else {
                                // Use msg.id (not pid) so multiple text parts
                                // of the same message merge into one block.
                                // pid includes part_idx which would force a new
                                // block per part, showing only the last token.
                                active_session.push_assistant_delta(&msg.id, text);
                            }
                        }
                        "reasoning" => {
                            let Some(text) = part.text.as_deref() else { continue };
                            if !text.is_empty() {
                                active_session.push_thinking(&pid, text);
                            }
                        }
                        "toolCall" | "tool_call" => {
                            if let Some(ref tc) = part.tool_call {
                                let preview = serde_json::to_string(&tc.input)
                                    .unwrap_or_default();
                                active_session.upsert_tool_call(
                                    &tc.id, &tc.name, &preview, ToolPhase::Done,
                                );
                            }
                        }
                        "toolResult" | "tool_result" => {
                            if let Some(ref tr) = part.tool_result {
                                active_session.push_tool_result(
                                    &tr.tool_call_id,
                                    tr.title.as_deref().unwrap_or("tool"),
                                    &tr.content,
                                    tr.is_error,
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            active_session.session_model.set(last_model);
            active_session.run_status.set(RunStatus::Idle);
        }
        Err(e) => {
            tracing::warn!(%session_id, %e, "failed to load session messages");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use revue::event::{MouseButton, MouseEvent, MouseEventKind};

    fn mk_handler() -> AppHandler {
        let store = crate::store::app_store::AppStore::new();
        let ss = crate::store::session_store::SessionStore::new();
        let eb = crate::telemetry::event_bus::EventBus::new();
        let (sf_tx, _rx) = tokio::sync::watch::channel::<Option<String>>(None);
        AppHandler::new(
            store,
            None,
            ss,
            eb,
            sf_tx,
            dispatch_outcome::DispatchOutcomes::new(),
        )
    }

    /// sidebar 底部 ⚙ 点击（x=W-3..W, y=末行）应触发 OpenSettings。
    #[test]
    fn gear_click_opens_settings() {
        let mut h = mk_handler();
        h.sidebar_visible = true;
        h.terminal_h = 24;
        h.sidebar_tab_y = 9;
        let ev = Event::Mouse(MouseEvent::new(30, 23, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev), "gear click should be consumed");
        assert!(matches!(h.store.route.get(), Route::Settings));
    }

    /// Alt+Enter → prompt 换行（tmux/xterm 主通道，ESC+CR 可靠送达）；
    /// Shift/Ctrl+Enter 同样换行；裸 Enter 保持发送（不插换行）。
    #[test]
    fn alt_shift_ctrl_enter_insert_newline_bare_enter_sends() {
        use revue::event::KeyEvent;
        let mut h = mk_handler();
        for c in "hi".chars() {
            h.handle(&Event::Key(KeyEvent::new(Key::Char(c))));
        }
        // Alt+Enter → 换行
        assert!(h.handle(&Event::Key(KeyEvent::alt(Key::Enter))));
        assert_eq!(h.prompt.text(), "hi\n", "Alt+Enter 必须插入换行而非发送");
        // Shift+Enter → 换行
        let shift_enter = KeyEvent { key: Key::Enter, ctrl: false, alt: false, shift: true };
        assert!(h.handle(&Event::Key(shift_enter)));
        assert_eq!(h.prompt.text(), "hi\n\n");
        // Ctrl+Enter → 换行
        assert!(h.handle(&Event::Key(KeyEvent::ctrl(Key::Enter))));
        assert_eq!(h.prompt.text(), "hi\n\n\n");
        // 裸 Enter → 发送（echo 模式无 api，prompt 清空即提交语义发生）
        assert!(h.handle(&Event::Key(KeyEvent::new(Key::Enter))));
        assert_eq!(h.prompt.text(), "", "裸 Enter 应提交并清空输入框");
    }

    /// Settings 分类栏点击：分类行 y=3+i、x ∈ [x_off, x_off+22)（sidebar 展开 x_off=33）。
    #[test]
    fn settings_category_click_switches_category() {
        use crate::store::types::{SettingsCategory, SettingsFocusPane};
        let mut h = mk_handler();
        h.store.navigate_settings();
        h.sidebar_visible = true;
        // "Model Settings" 行（i=1 → y=4 0-based），x=40（1-based col 41）
        let ev = Event::Mouse(MouseEvent::new(40, 4, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert_eq!(h.store.settings_category.get(), SettingsCategory::ModelSettings);
        assert_eq!(h.store.settings_focus_pane.get(), SettingsFocusPane::Providers);
    }

    /// sidebar session tree：箭头区点击 toggle 展开/折叠（默认折叠）。
    #[test]
    fn session_tree_arrow_click_toggles_expansion() {
        let mut h = mk_handler();
        h.sidebar_visible = true;
        let dir = h.store.working_dir.get();
        let items = vec![
            crate::store::types::SessionListItem {
                id: "root".into(),
                title: "Root".into(),
                run_status: None,
                parent_id: None,
                directory: dir.clone(),
                updated: 200,
            },
            crate::store::types::SessionListItem {
                id: "fork1".into(),
                title: "Fork".into(),
                run_status: None,
                parent_id: Some("root".into()),
                directory: dir.clone(),
                updated: 100,
            },
        ];
        h.store.session_list.set(items);
        h.refresh_sidebar_session_tree();
        // 默认全折叠：root 不展开。
        let tree = h.active_session.sidebar_trees.get();
        assert!(!tree.session_nodes[0].expanded, "default collapsed");
        assert!(!h.session_tree_expanded.contains("root"));
        drop(tree);
        // 发布命中并点击箭头区（depth=0 → arrow_end=2，点 x=1）。
        h.sidebar_nav_hits = vec![crate::telemetry::sidebar::SidebarNavHit {
            y: 20,
            session_id: "root".into(),
            depth: 0,
            has_children: true,
        }];
        let ev = Event::Mouse(MouseEvent::new(1, 20, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert!(h.session_tree_expanded.contains("root"), "arrow click expands");
        assert!(h.active_session.sidebar_trees.get().session_nodes[0].expanded);
        // 再点一次 → 折叠回去。
        let ev = Event::Mouse(MouseEvent::new(1, 20, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert!(!h.session_tree_expanded.contains("root"), "second arrow click collapses");
        assert!(!h.active_session.sidebar_trees.get().session_nodes[0].expanded);
    }

    /// Settings General body 行点击：选中 + 触发同一 toggle 权威。
    #[test]
    fn settings_general_row_click_toggles_value() {
        let mut h = mk_handler();
        h.store.navigate_settings();
        h.sidebar_visible = true;
        let before = h.store.show_thinking.get();
        // 首行（i=0 → y=5 0-based），x=65（body 区）
        let ev = Event::Mouse(MouseEvent::new(65, 5, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert_eq!(h.store.show_thinking.get(), !before);
        assert_eq!(h.store.settings_general_selected.get(), 0);
    }

    // ── Settings 全分类鼠标交互（x_off=33 / x2=56 / x3=85 / footer_y=21）──

    fn provider(id: &str, models: Vec<agendao_client::ProviderModelInfo>) -> agendao_client::ProviderInfo {
        agendao_client::ProviderInfo {
            id: id.into(),
            name: format!("P-{}", id),
            models,
            base_url: Some("https://api.example.com".into()),
            protocol: Some("openai".into()),
            disabled: false,
        }
    }

    fn model(id: &str) -> agendao_client::ProviderModelInfo {
        agendao_client::ProviderModelInfo {
            id: id.into(),
            name: id.into(),
            provider: "p1".into(),
            variants: vec![],
            context_window: Some(128_000),
            max_output_tokens: None,
            cost_per_million_input: None,
            cost_per_million_output: None,
        }
    }

    fn goto_model_settings(h: &mut AppHandler, providers: Vec<agendao_client::ProviderInfo>) {
        use crate::store::types::SettingsCategory;
        h.store.navigate_settings();
        h.sidebar_visible = true;
        h.terminal_h = 24;
        h.terminal_w = 100;
        h.store.settings_category.set(SettingsCategory::ModelSettings);
        h.store.providers.set(providers);
    }

    #[test]
    fn provider_row_click_selects_provider() {
        use crate::store::types::SettingsFocusPane;
        let mut h = mk_handler();
        goto_model_settings(&mut h, vec![provider("p1", vec![]), provider("p2", vec![])]);
        // 第二个 provider（i=1 → y=4），Providers 栏 x∈[56,84)
        let ev = Event::Mouse(MouseEvent::new(60, 4, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert_eq!(h.store.settings_selected_provider.get().as_deref(), Some("p2"));
        assert_eq!(h.store.settings_focus_pane.get(), SettingsFocusPane::Providers);
    }

    #[test]
    fn add_provider_footer_click_enters_add_form() {
        let mut h = mk_handler();
        goto_model_settings(&mut h, vec![provider("p1", vec![])]);
        // footer "+ Add provider" 行 y = terminal_h - 3 = 21
        let ev = Event::Mouse(MouseEvent::new(60, 21, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert!(h.settings_edit.is_add(), "点击 + Add 应进入 Add 表单");
    }

    #[test]
    fn model_row_click_selects_and_icons_trigger_edit_delete() {
        let mut h = mk_handler();
        goto_model_settings(&mut h, vec![provider("p1", vec![model("m1"), model("m2")])]);
        h.store.settings_selected_provider.set(Some("p1".into()));
        // models 首行 y=17（i=0 m1 / i=1 m2）。点击 m2 行中部 → 选中 + 焦点 Details。
        let ev = Event::Mouse(MouseEvent::new(90, 18, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert_eq!(h.store.settings_selected_model.get().as_deref(), Some("m2"));
        // ✕（x >= 96）→ Confirm 删除弹窗。
        let ev = Event::Mouse(MouseEvent::new(97, 18, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert!(matches!(h.panel, Panel::Confirm));
        h.confirm_dialog.close();
        h.panel = Panel::None;
        // ✎（94<=x<96）→ ModelEdit 编辑弹窗（api=None → 无 prefill 降级但仍弹）。
        let ev = Event::Mouse(MouseEvent::new(94, 18, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert!(matches!(h.panel, Panel::ModelEdit));
    }

    #[test]
    fn edit_form_field_click_focuses_and_recycles_protocol() {
        let mut h = mk_handler();
        goto_model_settings(&mut h, vec![provider("p1", vec![])]);
        h.store.settings_selected_provider.set(Some("p1".into()));
        let providers = h.store.providers.get();
        h.settings_edit.enter_edit(&providers[0]);
        // Edit 模式 4 字段后 Protocol 块 y∈[11,14)：点击 → 聚焦 Protocol。
        let ev = Event::Mouse(MouseEvent::new(90, 12, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert_eq!(h.settings_edit.focus, crate::app::settings_edit_state::SettingsEditField::Protocol);
        let idx0 = h.settings_edit.protocol_idx;
        // 已聚焦时再点 → 前进一个选项（同 → 键）。
        let ev = Event::Mouse(MouseEvent::new(90, 12, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert_eq!(h.settings_edit.protocol_idx, idx0 + 1);
    }

    #[test]
    fn mcp_row_click_selects_and_pill_toggles_connect() {
        use crate::store::types::{SettingsCategory, SettingsMcpRow};
        let mut h = mk_handler();
        h.store.navigate_settings();
        h.sidebar_visible = true;
        h.terminal_h = 24;
        h.store.settings_category.set(SettingsCategory::McpServers);
        let row = |name: &str, status: &str| SettingsMcpRow {
            name: name.into(),
            status: status.into(),
            tools: 1,
            resources: 0,
            error: None,
            transport: "local".into(),
            command: Some("srv".into()),
            url: None,
            enabled: true,
        };
        h.store.settings_mcp.set(vec![row("alpha", "connected"), row("beta", "disconnected")]);
        // 列表第二行（i=1 → y=4）→ 选中 beta。
        let ev = Event::Mouse(MouseEvent::new(60, 4, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert_eq!(h.store.settings_mcp_selected.get(), 1);
        // beta 的 Status pill：header_w=4+4=8 → px0=85+8+1=94，" Disconnected "=14 宽。
        let ev = Event::Mouse(MouseEvent::new(96, 1, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        // api=None → 动作落到 toast（证明点击路由进了 toggle_settings_mcp）。
        assert!(!h.store.toasts.get().is_empty(), "pill 点击应触发 connect 动作");
    }

    #[test]
    fn skills_proposal_approve_zone_routes_to_decide() {
        use crate::store::types::{SettingsCategory, SettingsSkillRow};
        let mut h = mk_handler();
        h.store.navigate_settings();
        h.sidebar_visible = true;
        h.terminal_h = 24;
        h.store.settings_category.set(SettingsCategory::Skills);
        h.store.settings_skills.set(vec![SettingsSkillRow::Proposal {
            id: "prop-1".into(),
            title: "Add retry rule".into(),
            status: "pending".into(),
            kind: "methodology".into(),
        }]);
        // footer approve 区（x3=85 起 13 列内，y = 24-3 = 21）。
        let ev = Event::Mouse(MouseEvent::new(88, 21, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert!(!h.store.toasts.get().is_empty(), "approve 区点击应路由到 decide");
    }

    #[test]
    fn tools_row_pill_click_and_t_key_route_to_toggle() {
        use crate::store::types::{SettingsCategory, SettingsToolRow};
        let mut h = mk_handler();
        h.store.navigate_settings();
        h.sidebar_visible = true;
        h.terminal_h = 24;
        h.store.settings_category.set(SettingsCategory::Tools);
        let tool = |id: &str, family: &str, protected: bool| SettingsToolRow {
            id: id.into(),
            description: String::new(),
            family: Some(family.into()),
            protected,
            disabled: false,
        };
        h.store.settings_tools.set(vec![tool("bash", "shell", false)]);
        // 行尾开关 pill 区点击（x2=56，pill 区 = 列表栏末 9 列 x>=75；数据行 y=4）。
        let ev = Event::Mouse(MouseEvent::new(77, 4, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert_eq!(h.store.settings_tools_selected.get(), 1);
        assert!(
            !h.store.toasts.get().is_empty(),
            "pill 点击应路由到 toggle_settings_tool（api=None → toast）"
        );
        // `t` 键同权威：protected（facade/bridge）行被拦截并 toast 说明原因。
        h.store
            .settings_tools
            .set(vec![tool("tool_catalog_call", "tool_catalog", true)]);
        h.store.settings_tools_selected.set(1);
        assert!(h.handle_settings_key(&Key::Char('t')));
        let toasts = h.store.toasts.get();
        assert!(
            toasts.iter().any(|t| t.text.contains("facade/bridge")),
            "protected 拦截 toast 缺失: {:?}",
            toasts.iter().map(|t| t.text.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn wheel_in_settings_moves_selection_not_session_scroll() {
        let mut h = mk_handler();
        goto_model_settings(
            &mut h,
            vec![provider("p1", vec![]), provider("p2", vec![]), provider("p3", vec![])],
        );
        h.store.settings_selected_provider.set(Some("p1".into()));
        let ev = Event::Mouse(MouseEvent::new(60, 10, MouseEventKind::ScrollDown));
        assert!(h.handle(&ev));
        assert_eq!(h.store.settings_selected_provider.get().as_deref(), Some("p2"));
        let ev = Event::Mouse(MouseEvent::new(60, 10, MouseEventKind::ScrollUp));
        assert!(h.handle(&ev));
        assert_eq!(h.store.settings_selected_provider.get().as_deref(), Some("p1"));
    }

    #[test]
    fn model_edit_dialog_field_click_focuses() {
        let mut h = mk_handler();
        h.model_edit_dialog.open_add("p1");
        h.panel = Panel::ModelEdit;
        h.model_edit_rect = Some(revue::prelude::Rect::new(15, 2, 70, 22));
        // Name 字段块（i=1 → y ∈ [7,11)）：点击 (20, 8)。
        let ev = Event::Mouse(MouseEvent::new(20, 8, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        let expect = h.model_edit_dialog.field_at_block_index(1);
        assert_eq!(Some(h.model_edit_dialog.focus()), expect);
        assert!(expect.is_some());
    }

    #[test]
    fn provider_edit_dialog_field_click_focuses() {
        let mut h = mk_handler();
        h.provider_edit_dialog.open_add();
        h.panel = Panel::ProviderEdit;
        h.provider_edit_rect = Some(revue::prelude::Rect::new(15, 2, 76, 24));
        // BaseUrl 字段块（i=2 → y ∈ [11,15)）：点击 (20, 12)。
        let ev = Event::Mouse(MouseEvent::new(20, 12, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert_eq!(
            h.provider_edit_dialog.focus(),
            crate::dialog::ProviderEditDialog::FIELDS[2]
        );
    }

    #[test]
    fn confirm_dialog_cancel_zone_closes_without_action() {
        let mut h = mk_handler();
        h.confirm_dialog.ask("Delete Provider", "Delete provider \"p1\"?", "Delete");
        h.panel = Panel::Confirm;
        h.pending_confirm = Some(crate::app::PendingConfirm::DeleteProvider("p1".into()));
        h.confirm_rect = Some(revue::prelude::Rect::new(10, 5, 60, 5));
        // hint 行 y = 5+5-1 = 9；seg1_w=14, hint_w=29, start = 10+(60-29)/2 = 25；
        // cancel 区 [25+14+2, 25+29) = [41, 54)。
        let ev = Event::Mouse(MouseEvent::new(45, 9, MouseEventKind::Down(MouseButton::Left)));
        assert!(h.handle(&ev));
        assert!(matches!(h.panel, Panel::None), "cancel 应关闭弹窗");
        assert!(h.pending_confirm.is_none(), "cancel 应回收 pending");
    }
}
