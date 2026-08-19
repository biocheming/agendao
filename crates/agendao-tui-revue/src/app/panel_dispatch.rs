//! 火 — Panel/Overlay 按键分发（从 keymap::handle_key 抽出）。
//!
//! keymap.rs 逼近 1500 行软限，把 panel 按键路由单独成文，使 handle_key
//! 重新聚焦 transcript/prompt（道纪：承载边界 + 唯一分发口径）。
//! 语义不变：每个 panel 独占键；Panel::None 贯穿返回 false，让键继续
//! 流向 transcript 滚动与 prompt 输入。

use crate::app::{AppHandler, Panel, PendingConfirm};
use crate::dialog::PermissionReply;
use agendao_command::UiActionId;
use revue::event::{Key, KeyEvent};

fn is_revision_conflict(error: &str) -> bool {
    error.to_ascii_lowercase().contains("revision conflict")
}

impl AppHandler {
    /// U17③：当前 panel 是否拥有滚轮（↑↓ 语义的列表类弹窗）。表单/确认类
    /// 不在列——它们的滚轮维持原行为（滚背后 transcript，如权限弹窗长文
    /// 场景）。单一判别点，keymap 滚轮路由只问这里（土律）。
    pub(crate) fn panel_owns_wheel(&self) -> bool {
        matches!(
            self.panel,
            Panel::SessionList
                | Panel::ModelSelect
                | Panel::ModeSelect
                | Panel::AgentSelect
                | Panel::SkillList
                | Panel::SkillProposal
                | Panel::McpList
                | Panel::Recovery
                | Panel::Notifications
                | Panel::Stash
                | Panel::Fork
                | Panel::TaskState
        )
    }

    /// Panel/Overlay 按键分发。返回 true=已消费；false=贯穿（仅 Panel::None）。
    pub(super) fn route_panel_key(&mut self, key: &Key) -> bool {
        match &self.panel {
            Panel::Slash => {
                // U3：popup 是输入框的视图——只拦截导航/填回/执行/恢复四类
                // 语义键，其余全部 Pass 贯穿给 prompt（query 由 keymap 从
                // 输入框文本重新派生，popup 不再自维字符缓冲）。
                use crate::input::slash_popup::SlashKeyOutcome;
                let consumed = match self.slash_popup.handle_key(key) {
                    SlashKeyOutcome::FillBack {
                        command,
                        takes_args,
                    } => {
                        self.prompt.set_text(&command);
                        if !takes_args {
                            self.panel = Panel::None;
                        }
                        // takes_args：popup 已转 ArgHint 保持打开
                        true
                    }
                    SlashKeyOutcome::Submit => {
                        // ArgHint 下 Enter：走与裸 Enter 完全相同的 submit
                        // 路径（/ 开头 → sync_slash_from_text 解析执行）。
                        self.panel = Panel::None;
                        match self.prompt.handle_key(&Key::Enter) {
                            crate::input::PromptAction::Submit(text) => {
                                if text.starts_with('/') {
                                    self.sync_slash_from_text(&text);
                                    self.prompt.clear();
                                } else {
                                    self.dispatch(text);
                                }
                            }
                            crate::input::PromptAction::SubmitShell(cmd) => {
                                self.dispatch_shell(cmd);
                            }
                            _ => {}
                        }
                        true
                    }
                    SlashKeyOutcome::Restore => {
                        let pre = self.slash_popup.pre_slash_text.clone();
                        self.prompt.set_text(&pre);
                        self.panel = Panel::None;
                        true
                    }
                    SlashKeyOutcome::Consumed => true,
                    // Pass：贯穿给 prompt（route_panel_key 返回 false）。
                    SlashKeyOutcome::Pass => false,
                };
                return consumed;
            }
            Panel::ModelSelect => {
                match self.model_select.handle_key(key) {
                    crate::dialog::ModelDialogOutcome::Selected(selected) => {
                        // Server resolves models via `provider_id/model_id`
                        // (parse_model_string in agendao-provider). Storing only
                        // the bare model_id makes server_send_prompt fail with
                        // "Model not found: <id>" because the same model_id
                        // can exist in multiple aggregator providers.
                        let qualified = format!("{}/{}", selected.provider, selected.model_id);
                        self.store.selected_model.set(Some(qualified.clone()));
                        // F2：选中即记入 recent（置顶去重 cap）并异步持久化——
                        // 此前仅启动时把 workspace context 原样回写，用户选择
                        // 从不落盘，"★ Recent" 永不填充。
                        let recent = self
                            .model_select
                            .record_recent(&selected.provider, &selected.model_id);
                        if let Some(ref api) = self.api {
                            let api_c = api.clone();
                            api.handle().spawn(async move {
                                let entries: Vec<agendao_state::RecentModelEntry> = recent
                                    .into_iter()
                                    .map(|(provider, model)| agendao_state::RecentModelEntry {
                                        provider,
                                        model,
                                    })
                                    .collect();
                                if let Err(e) = api_c.put_recent_models_async(entries).await {
                                    tracing::warn!(%e, "put_recent_models failed");
                                }
                            });
                        }
                        let msg = format!("Model: {} ({})", selected.display, qualified);
                        self.store
                            .push_toast(&msg, crate::store::types::ToastMsgVariant::Success);
                        self.panel = Panel::None;
                    }
                    crate::dialog::ModelDialogOutcome::Notice(reason) => {
                        // Surface the reason ("Provider X not connected", etc.)
                        // so the user sees why Enter didn't close the dialog.
                        // Without this, the previous silent return left the
                        // dialog "stuck open" with no clue.
                        self.store
                            .push_toast(&reason, crate::store::types::ToastMsgVariant::Warning);
                    }
                    crate::dialog::ModelDialogOutcome::None => {}
                }
                if !self.model_select.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::ModeSelect => {
                if let Some(picked) = self.mode_select.handle_key(key) {
                    // store 契约：`"kind:id"` 复合（对齐 web `App.tsx:836`）；
                    // dispatch 处再 split 分流到 agent / scheduler。
                    let composite = picked.composite();
                    self.store.selected_mode.set(Some(composite.clone()));
                    let msg = format!("Mode: {} ({})", picked.display, composite);
                    self.store
                        .push_toast(&msg, crate::store::types::ToastMsgVariant::Success);
                    self.panel = Panel::None;
                }
                if !self.mode_select.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::AgentSelect => {
                if let Some(selected) = self.agent_select.handle_key(key) {
                    self.store.selected_agent.set(Some(selected.name.clone()));
                    let msg = format!("Switched to agent: {}", selected.display);
                    self.store
                        .push_toast(&msg, crate::store::types::ToastMsgVariant::Success);
                    self.panel = Panel::None;
                }
                if !self.agent_select.visible {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::Confirm => {
                if let Some(confirmed) = self.confirm_dialog.handle_key(key) {
                    if confirmed {
                        // 按 pending_confirm 判别器路由确认动作；take() 保证
                        // pending 不跨轮悬空（道纪第九条：写入即承诺回收）。
                        match self.pending_confirm.take() {
                            Some(PendingConfirm::DeleteSession(sid)) => {
                                if let Some(ref api) = self.api {
                                    match api.delete_session(&sid) {
                                        Ok(true) => self.store.push_toast(
                                            "Session deleted",
                                            crate::store::types::ToastMsgVariant::Success,
                                        ),
                                        Ok(false) => self.store.push_toast(
                                            "Session not found",
                                            crate::store::types::ToastMsgVariant::Warning,
                                        ),
                                        Err(e) => self.store.push_toast(
                                            &format!("Delete failed: {}", e),
                                            crate::store::types::ToastMsgVariant::Error,
                                        ),
                                    }
                                }
                                self.reload_session_list();
                                // 退出已删会话路由：重置 transcript + 回 Home，
                                // 避免停在幽灵会话上（金：交付成形不残留失效态）。
                                self.active_session.reset_for_new_session();
                                self.store.navigate_home();
                            }
                            Some(PendingConfirm::ExecuteRecovery { session_id, action }) => {
                                if let Some(ref api) = self.api {
                                    match api.execute_session_recovery(&session_id, action) {
                                        Ok(_) => self.store.push_toast(
                                            "Recovery action executed",
                                            crate::store::types::ToastMsgVariant::Success,
                                        ),
                                        Err(e) => self.store.push_toast(
                                            &format!("Recovery failed: {}", e),
                                            crate::store::types::ToastMsgVariant::Error,
                                        ),
                                    }
                                }
                            }
                            Some(PendingConfirm::DeleteSessionsBatch(ids)) => {
                                // 批量删:逐个调 delete_session,累积 ok/fail 计数,
                                // 单次 toast 汇报结果。中途失败不阻塞剩余项(尽量删除)。
                                // 成功项从 session_list 摘除——下次回到 list 看到刷新态。
                                let mut ok_ids: Vec<String> = Vec::new();
                                let mut fail = 0;
                                if let Some(ref api) = self.api {
                                    for id in &ids {
                                        match api.delete_session(id) {
                                            Ok(true) => ok_ids.push(id.clone()),
                                            Ok(false) | Err(_) => fail += 1,
                                        }
                                    }
                                }
                                let ok_n = ok_ids.len();
                                self.session_list.forget_sessions(&ok_ids);
                                self.reload_session_list();
                                // 若当前会话也在删除列表里,重置 transcript + 回 Home
                                // (避免幽灵会话——同单删 arm 语义)。
                                if let Some(cur) = self.active_session.get_session_id() {
                                    if ok_ids.iter().any(|i| i == &cur) {
                                        self.active_session.reset_for_new_session();
                                        self.store.navigate_home();
                                    }
                                }
                                let (variant, msg) = if fail == 0 {
                                    (
                                        crate::store::types::ToastMsgVariant::Success,
                                        format!("Deleted {} session(s)", ok_n),
                                    )
                                } else if ok_n == 0 {
                                    (
                                        crate::store::types::ToastMsgVariant::Error,
                                        format!("Failed to delete {} session(s)", fail),
                                    )
                                } else {
                                    (
                                        crate::store::types::ToastMsgVariant::Warning,
                                        format!("Deleted {}, failed {}", ok_n, fail),
                                    )
                                };
                                self.store.push_toast(&msg, variant);
                            }
                            // PendingConfirm 多变体已穷尽 Some；None 收尾。
                            // 新增变体会让此 match 变非穷尽 → 编译报错 → 强制补臂。
                            Some(PendingConfirm::DeleteProvider(id)) => {
                                self.delete_provider_action(&id);
                            }
                            Some(PendingConfirm::DeleteProviderModel {
                                provider_id,
                                model_key,
                            }) => {
                                self.delete_provider_model_action(&provider_id, &model_key);
                            }
                            Some(PendingConfirm::DeleteSkill(name)) => {
                                self.delete_skill_action(&name);
                            }
                            Some(PendingConfirm::DeleteMcp(name)) => {
                                self.delete_mcp_action(&name);
                            }
                            Some(PendingConfirm::DeletePlugin(name)) => {
                                self.delete_plugin_action(&name);
                            }
                            None => {}
                        }
                    } else {
                        // 取消：回收 pending。
                        self.pending_confirm = None;
                    }
                    // U15①：回来源 panel（SessionList 批量删回列表；其余
                    // 来源默认 None）。replace 取出即复位，不跨轮悬空。
                    self.panel = std::mem::replace(&mut self.confirm_return, Panel::None);
                } else if !self.confirm_dialog.visible {
                    // handle_key 未给结论但 dialog 已不可见（保险路径）——
                    // 必须 else：否则上一行刚恢复的来源 panel 会被这行
                    // 无条件覆盖回 None（U15① 初版实测 bug）。
                    self.panel = std::mem::replace(&mut self.confirm_return, Panel::None);
                }
                return true;
            }
            Panel::Stash => {
                let prev_len = self.stash_entries.len();
                if let Some(text) = self.stash_dialog.handle_key(key) {
                    // 水生木闭环：恢复的 stash 文本回填输入框权威（修原 _text
                    // 丢弃 bug —— 恢复项被直接扔掉、只 toast）。
                    self.prompt.set_text(&text);
                    self.panel = Panel::None;
                    return true;
                }
                // dialog 持有 entries 克隆；delete 发生在 dialog 内。检测条目数
                // 变化即同步回权威 self.stash_entries 并落盘（土律：唯一所有权；
                // 水律：回流落盘）。导航键不改变条目数，不触发写。
                let cur = self.stash_dialog.entries().to_vec();
                if cur.len() != prev_len {
                    self.stash_entries = cur;
                    crate::dialog::prompt_stash::save_stash(&self.stash_entries);
                }
                if !self.stash_dialog.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::Rename => {
                if let Some((sid, new_title)) = self.rename_dialog.handle_key(key) {
                    if let Some(ref api) = self.api {
                        let _ = api.update_session_title(&sid, &new_title);
                    }
                    self.active_session.title.set(new_title);
                    self.panel = Panel::None;
                    return true;
                }
                if !self.rename_dialog.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::SessionList => {
                if let Some(action) = self.session_list.handle_key(key) {
                    match action {
                        crate::dialog::SessionListAction::Open(entry) => {
                            // 复用单点 open_session 权威(reset + set + sf_tx + load +
                            // navigate + panel=None),与 sidebar tree 点击同一路径。
                            self.open_session(&entry.id);
                        }
                        crate::dialog::SessionListAction::DeleteBatch(ids) => {
                            // 'D' 触发批量删除:走 Confirm 同栈,与单删共享成形。
                            // dialog 不关闭——批量删完成后回到 list,用户能继续操作。
                            self.confirm_dialog.ask(
                                "Delete Sessions",
                                &format!("Delete {} session(s)? This cannot be undone.", ids.len()),
                                "Delete",
                            );
                            self.pending_confirm = Some(PendingConfirm::DeleteSessionsBatch(ids));
                            // U15①：dialog 保持打开 → Confirm 解决后回列表继续
                            // 操作（原无条件 panel=None，forget_sessions 回填
                            // 白写、用户被踢出列表，与上行注释矛盾）。
                            self.confirm_return = Panel::SessionList;
                            self.panel = Panel::Confirm;
                        }
                        crate::dialog::SessionListAction::NewSession => {
                            // U15②：空态 'n' → 复用 NewSession 唯一成形路径
                            // （reset + navigate Home + focus + toast），不在
                            // 此处二次成形（金律）。
                            self.execute_slash_action(UiActionId::NewSession);
                            self.panel = Panel::None;
                        }
                    }
                    return true;
                }
                if !self.session_list.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::Help => {
                self.help.handle_key(key);
                if !self.help.visible {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::TaskState => {
                let ledger = self.active_session.task_ledger.get();
                if let Some(action) = self.task_state_dialog.handle_key(key, ledger.as_deref()) {
                    match action {
                        crate::dialog::TaskStateAction::Apply(op) => {
                            let Some(session_id) = self.active_session.get_session_id() else {
                                self.store.push_toast(
                                    "No active session for Task State update",
                                    crate::store::types::ToastMsgVariant::Error,
                                );
                                return true;
                            };
                            let Some(expected_revision) =
                                ledger.as_ref().map(|value| value.revision)
                            else {
                                return true;
                            };
                            let Some(api) = self.api.as_ref() else {
                                self.store.push_toast(
                                    "No API bridge for Task State update",
                                    crate::store::types::ToastMsgVariant::Error,
                                );
                                return true;
                            };
                            match api.apply_task_ledger_op(&session_id, expected_revision, op) {
                                Ok(snapshot) => {
                                    self.active_session.apply_task_ledger_snapshot(snapshot);
                                    self.store.push_toast(
                                        "Task state saved",
                                        crate::store::types::ToastMsgVariant::Success,
                                    );
                                }
                                Err(error) if is_revision_conflict(&error.to_string()) => {
                                    if let Ok(snapshot) = api.get_task_ledger(&session_id) {
                                        self.active_session.apply_task_ledger_snapshot(snapshot);
                                    }
                                    self.store.push_toast(
                                        "Task state changed elsewhere; latest revision loaded. Review and retry.",
                                        crate::store::types::ToastMsgVariant::Warning,
                                    );
                                }
                                Err(error) => self.store.push_toast(
                                    &format!("Task state update failed: {error}"),
                                    crate::store::types::ToastMsgVariant::Error,
                                ),
                            }
                        }
                        crate::dialog::TaskStateAction::NavigateEvidence(reference) => {
                            if self
                                .active_session
                                .focus_transcript_reference(&reference, self.transcript_viewport_h)
                            {
                                self.task_state_dialog.dismiss();
                                self.panel = Panel::None;
                                self.layout_dirty = true;
                            } else {
                                self.store.push_toast(
                                    &format!("Evidence {reference} is not present in the loaded transcript"),
                                    crate::store::types::ToastMsgVariant::Warning,
                                );
                            }
                        }
                    }
                }
                if !self.task_state_dialog.visible {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::SkillList => {
                if let Some(action) = self.skill_list.handle_key(key) {
                    match action {
                        crate::dialog::SkillListAction::View(entry) => {
                            // F8：Enter 拉详情回填 dialog（保持打开）。失败诚实
                            // 报错，不伪造详情（道纪第十条）。
                            if let Some(ref api) = self.api {
                                match api.get_skill_detail(&entry.name) {
                                    Ok(detail) => {
                                        let meta = &detail.skill.meta;
                                        let mut lines = vec![
                                            format!("name: {}", meta.name),
                                            format!("description: {}", meta.description),
                                            format!(
                                                "category: {}",
                                                meta.category.as_deref().unwrap_or("-")
                                            ),
                                            format!("location: {}", meta.location),
                                            format!("source: {}", detail.source),
                                            format!("writable: {}", detail.writable),
                                        ];
                                        if !meta.supporting_files.is_empty() {
                                            lines.push(format!(
                                                "supporting files: {}",
                                                meta.supporting_files.len()
                                            ));
                                        }
                                        lines.push(String::new());
                                        lines.extend(
                                            detail.skill.content.lines().map(str::to_string),
                                        );
                                        self.skill_list
                                            .show_detail(format!("Skill: {}", meta.name), lines);
                                    }
                                    Err(e) => self.store.push_toast(
                                        &format!("Skill detail failed: {e}"),
                                        crate::store::types::ToastMsgVariant::Error,
                                    ),
                                }
                            }
                        }
                        crate::dialog::SkillListAction::OpenSettings => {
                            // U16：空态 's' → 全屏 Settings（skills 管理面），
                            // 复用唯一入口（金律：成形点单一）。
                            self.execute_slash_action(UiActionId::OpenSettings);
                            self.panel = Panel::None;
                        }
                    }
                    return true;
                }
                if !self.skill_list.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::SkillProposal => {
                if let Some(action) = self.skill_proposal.handle_key(key) {
                    match action {
                        crate::dialog::SkillProposalAction::Approve(e) => {
                            self.execute_proposal_status(&e, "accepted");
                        }
                        crate::dialog::SkillProposalAction::Reject(e) => {
                            self.execute_proposal_status(&e, "rejected");
                        }
                        crate::dialog::SkillProposalAction::View(e) => {
                            self.store.push_toast(
                                &format!("[{}] {} — {}", e.status, e.title, e.kind),
                                crate::store::types::ToastMsgVariant::Info,
                            );
                            self.panel = Panel::None;
                        }
                    }
                    return true;
                }
                if !self.skill_proposal.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::McpList => {
                if let Some(action) = self.mcp_list.handle_key(key) {
                    match action {
                        crate::dialog::McpAction::Connect(e) => {
                            // 前置校验：已 connected 不重复 connect（避免 no-op round-trip）。
                            if e.status == "connected" {
                                self.store.push_toast(
                                    "Already connected",
                                    crate::store::types::ToastMsgVariant::Warning,
                                );
                            } else {
                                self.execute_mcp_toggle(&e, true);
                            }
                        }
                        crate::dialog::McpAction::Disconnect(e) => {
                            // 前置校验：未 connected 无 disconnect 语义。
                            if e.status != "connected" {
                                self.store.push_toast(
                                    "Not connected",
                                    crate::store::types::ToastMsgVariant::Warning,
                                );
                            } else {
                                self.execute_mcp_toggle(&e, false);
                            }
                        }
                        crate::dialog::McpAction::AuthStart(e) => {
                            if let Some(ref api) = self.api {
                                match api.start_mcp_auth(&e.name) {
                                    Ok(info) => {
                                        // URL 走 transcript notice（toast 截断长 URL 不可拷贝）。
                                        self.active_session.push_notice(
                                            &format!("mcp-auth-{}", e.name),
                                            &format!(
                                                "🔐 MCP `{}` OAuth — open in browser to authorize:\n{}",
                                                e.name, info.authorization_url
                                            ),
                                        );
                                        self.store.push_toast(
                                            &format!("OAuth started: {} (URL in transcript, press A after authorizing)", e.name),
                                            crate::store::types::ToastMsgVariant::Info,
                                        );
                                    }
                                    Err(err) => self.store.push_toast(
                                        &format!("OAuth start failed: {err}"),
                                        crate::store::types::ToastMsgVariant::Error,
                                    ),
                                }
                            }
                        }
                        crate::dialog::McpAction::AuthFinish(e) => {
                            if let Some(ref api) = self.api {
                                match api.authenticate_mcp(&e.name) {
                                    Ok(_) => {
                                        self.refresh_mcp_into_store();
                                        self.store.push_toast(
                                            &format!("MCP authenticated: {}", e.name),
                                            crate::store::types::ToastMsgVariant::Success,
                                        );
                                    }
                                    Err(err) => self.store.push_toast(
                                        &format!("Authenticate failed: {err}"),
                                        crate::store::types::ToastMsgVariant::Error,
                                    ),
                                }
                            }
                        }
                        crate::dialog::McpAction::AuthRemove(e) => {
                            if let Some(ref api) = self.api {
                                match api.remove_mcp_auth(&e.name) {
                                    Ok(_) => {
                                        self.refresh_mcp_into_store();
                                        self.store.push_toast(
                                            &format!("MCP auth cleared: {}", e.name),
                                            crate::store::types::ToastMsgVariant::Success,
                                        );
                                    }
                                    Err(err) => self.store.push_toast(
                                        &format!("Remove auth failed: {err}"),
                                        crate::store::types::ToastMsgVariant::Error,
                                    ),
                                }
                            }
                        }
                        crate::dialog::McpAction::Add => {
                            // F12：复用 Settings 的 McpEditDialog（add 模式）。
                            self.mcp_list.close();
                            self.settings_open_add_mcp();
                        }
                        crate::dialog::McpAction::Edit(e) => {
                            // F12：settings 行需 config 合并字段——先刷新再找行。
                            self.refresh_mcp_into_store();
                            let rows = self.store.settings_mcp.get();
                            match rows.iter().position(|r| r.name == e.name) {
                                Some(idx) => {
                                    self.store.settings_mcp_selected.set(idx);
                                    drop(rows);
                                    self.mcp_list.close();
                                    self.settings_open_edit_mcp();
                                }
                                None => self.store.push_toast(
                                    &format!(
                                        "No config entry for `{}` — open Settings to add",
                                        e.name
                                    ),
                                    crate::store::types::ToastMsgVariant::Warning,
                                ),
                            }
                        }
                        crate::dialog::McpAction::View(e) => {
                            self.store.push_toast(
                                &format!(
                                    "[{}] {} · tools:{} res:{}",
                                    e.status, e.name, e.tools, e.resources
                                ),
                                crate::store::types::ToastMsgVariant::Info,
                            );
                            self.panel = Panel::None;
                        }
                    }
                    return true;
                }
                if !self.mcp_list.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::Recovery => {
                if let Some(action) = self.recovery_list.handle_key(key) {
                    match action {
                        crate::dialog::RecoveryAction::Execute { label, action_kind } => {
                            // session_id 从 active_session 取（dialog 不持有；modal 不变量
                            // 保证 open→confirm 期间不变）。None→toast 不伪执行（道纪第十条）。
                            if let Some(sid) = self.active_session.get_session_id() {
                                // U26⑤：confirm 文案写清后果（第十条）——"proceed?"
                                // 不说代价等于让用户盲签；按 kind 给一句诚实后果。
                                let consequence = match &action_kind {
                                    agendao_client::RecoveryActionKind::AbortRun =>
                                        "This stops the entire run; in-flight tool calls are cancelled.",
                                    agendao_client::RecoveryActionKind::Retry =>
                                        "This re-runs the failed step; side effects already applied may repeat.",
                                    agendao_client::RecoveryActionKind::Resume =>
                                        "This continues the request while preserving verified prior work.",
                                };
                                self.confirm_dialog.ask(
                                    "Execute Recovery",
                                    &format!("Execute {} — proceed? {}", label, consequence),
                                    "Execute",
                                );
                                self.pending_confirm = Some(PendingConfirm::ExecuteRecovery {
                                    session_id: sid,
                                    action: action_kind,
                                });
                                self.recovery_list.close();
                                self.panel = Panel::Confirm;
                            } else {
                                self.store.push_toast(
                                    "No active session",
                                    crate::store::types::ToastMsgVariant::Warning,
                                );
                            }
                        }
                        crate::dialog::RecoveryAction::View(e) => {
                            self.store.push_toast(
                                &format!("{} — {}", e.label, e.detail),
                                crate::store::types::ToastMsgVariant::Info,
                            );
                            self.panel = Panel::None;
                        }
                    }
                    return true;
                }
                if !self.recovery_list.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::Notifications => {
                // 只读回看：导航 + Esc（条目无可执行语义，道纪第十条）。
                let count = self.store.toast_history.get().len();
                self.notification_dialog.handle_key(key, count);
                if !self.notification_dialog.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::ModelEdit => {
                if let Some(action) = self.model_edit_dialog.handle_key(key) {
                    match action {
                        crate::dialog::ModelEditAction::Submit(s) => {
                            self.submit_model_edit(*s);
                        }
                        crate::dialog::ModelEditAction::Cancel => {}
                    }
                    self.panel = Panel::None;
                    return true;
                }
                if !self.model_edit_dialog.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::McpEdit => {
                if let Some(action) = self.mcp_edit_dialog.handle_key(key) {
                    match action {
                        crate::dialog::McpEditAction::Submit(s) => {
                            self.submit_mcp_edit(*s);
                        }
                        crate::dialog::McpEditAction::Cancel => {}
                    }
                    self.panel = Panel::None;
                    return true;
                }
                if !self.mcp_edit_dialog.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::PluginEdit => {
                if let Some(action) = self.plugin_edit_dialog.handle_key(key) {
                    match action {
                        crate::dialog::PluginEditAction::Submit(s) => {
                            self.install_plugin_action(*s);
                        }
                        crate::dialog::PluginEditAction::Cancel => {}
                    }
                    self.panel = Panel::None;
                    return true;
                }
                if !self.plugin_edit_dialog.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::ProviderEdit => {
                if let Some(action) = self.provider_edit_dialog.handle_key(key) {
                    match action {
                        crate::dialog::ProviderEditAction::Submit(s) => {
                            // 载荷即 ProviderEditSubmission,直连既有写入链路
                            // (client → server → refresh_providers_into_store)。
                            self.submit_provider_edit(*s);
                        }
                        crate::dialog::ProviderEditAction::Cancel => {}
                    }
                    self.panel = Panel::None;
                    return true;
                }
                if !self.provider_edit_dialog.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::Fork => {
                if let Some((sid, mid)) = self.fork_dialog.handle_key(key) {
                    if let Some(ref api) = self.api {
                        match api.fork_session(&sid, mid.as_deref()) {
                            Ok(info) => {
                                // 切到 fork 后的新会话（对齐 /revise；整会话 fork 不回填输入框）。
                                self.switch_to_forked_session(&info);
                                self.store.push_toast(
                                    &format!("Forked → {}", info.title),
                                    crate::store::types::ToastMsgVariant::Success,
                                );
                            }
                            Err(e) => self.store.push_toast(
                                &format!("Fork failed: {}", e),
                                crate::store::types::ToastMsgVariant::Error,
                            ),
                        }
                    }
                    self.panel = Panel::None;
                    return true;
                }
                if !self.fork_dialog.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::Export => {
                if let Some(action) = self.export_dialog.handle_key(key) {
                    match action {
                        crate::dialog::ExportAction::Copy(text) => {
                            // OSC52 非显示序列，直接写 stdout 安全；dialog 已标 copied。
                            // 不关 dialog：用户看到 ✓，可继续 s:share 或 Esc。
                            // U18①：OSC52 失败兜底临时文件——成功路径不 toast
                            //（dialog ✓ 已反馈），兜底/双失败必须可观测。
                            use crate::dialog::clipboard::CopyOutcome;
                            match crate::dialog::clipboard::copy_with_fallback(&text) {
                                Ok(CopyOutcome::Clipboard) => self.export_dialog.mark_copied(),
                                Ok(CopyOutcome::FileFallback(path)) => self.store.push_toast(
                                    &format!("Clipboard unavailable — saved to {}", path.display()),
                                    crate::store::types::ToastMsgVariant::Warning,
                                ),
                                Err(e) => self.store.push_toast(
                                    &format!("Copy failed: {}", e),
                                    crate::store::types::ToastMsgVariant::Error,
                                ),
                            }
                        }
                        crate::dialog::ExportAction::Share(sid) => {
                            if let Some(ref api) = self.api {
                                match api.share_session(&sid) {
                                    Ok(resp) => {
                                        // F11：URL 同时进剪贴板；写失败如实标注。
                                        let msg = match crate::dialog::clipboard::copy(&resp.url) {
                                            Ok(()) => format!("Shared (URL copied): {}", resp.url),
                                            Err(_) => format!("Shared (copy failed): {}", resp.url),
                                        };
                                        self.store.push_toast(
                                            &msg,
                                            crate::store::types::ToastMsgVariant::Success,
                                        );
                                    }
                                    Err(e) => self.store.push_toast(
                                        &format!("Share failed: {}", e),
                                        crate::store::types::ToastMsgVariant::Error,
                                    ),
                                }
                            }
                            self.export_dialog.close();
                            self.panel = Panel::None;
                        }
                    }
                    return true;
                }
                if !self.export_dialog.is_open() {
                    self.panel = Panel::None;
                }
                return true;
            }
            Panel::None => {
                // 内联 permission/question（终端内联 CLI 风格）：visible
                // 时独占键。↑↓ 切选项；transcript 滚动走 PageUp/Down 不冲突，
                // 故无需处理键冲突。
                if self.permission_dialog.visible {
                    if let Some((id, reply)) = self.permission_dialog.handle_key(key) {
                        // 发送 permission 回复。dialog 返回原始 request id +
                        // reply —— 缺 id 则 server 无法匹配 pending permission，
                        // prompt loop 永久阻塞。
                        if let Some(ref api) = self.api {
                            // Server 期望 bare lifetime token
                            // (`once`/`turn`/`session`/`always`/`reject`)，
                            // 不是 `allow_*` 别名。
                            let mode = match reply {
                                PermissionReply::TrustWorkspace => {
                                    Some(agendao_client::SessionPermissionMode::TrustedWorkspace)
                                }
                                PermissionReply::FullAccess => {
                                    Some(agendao_client::SessionPermissionMode::UnsandboxedYolo)
                                }
                                _ => None,
                            };
                            if let Some(mode) = mode {
                                let Some(session_id) = self.active_session.get_session_id() else {
                                    self.store.push_toast(
                                        "Cannot change permission mode without an active session",
                                        crate::store::types::ToastMsgVariant::Error,
                                    );
                                    return true;
                                };
                                match api.set_session_permission_mode(&session_id, mode) {
                                    Ok(_) => self.store.push_toast(
                                        match mode {
                                            agendao_client::SessionPermissionMode::TrustedWorkspace => {
                                                "Workspace trusted for this session"
                                            }
                                            agendao_client::SessionPermissionMode::UnsandboxedYolo => {
                                                "Full access enabled for this session"
                                            }
                                            agendao_client::SessionPermissionMode::Default => {
                                                "Default permission mode restored"
                                            }
                                        },
                                        crate::store::types::ToastMsgVariant::Warning,
                                    ),
                                    Err(e) => {
                                        self.store.push_toast(
                                            &format!("permission mode update failed: {}", e),
                                            crate::store::types::ToastMsgVariant::Error,
                                        );
                                        return true;
                                    }
                                }
                            }
                            let reply_str = match reply {
                                PermissionReply::AllowOnce
                                | PermissionReply::TrustWorkspace
                                | PermissionReply::FullAccess => "once",
                                PermissionReply::AllowTurn => "turn",
                                PermissionReply::AllowSession => "session",
                                PermissionReply::Deny => "reject",
                            };
                            if let Err(e) = api.reply_permission(&id, reply_str, None) {
                                self.store.push_toast(
                                    &format!("permission reply failed: {}", e),
                                    crate::store::types::ToastMsgVariant::Error,
                                );
                            }
                        }
                    }
                    return true;
                }
                if self.question_dialog.visible {
                    match self.question_dialog.handle_key(key) {
                        Some(crate::dialog::QuestionKeyOutcome::Answered(qid, labels)) => {
                            if let Some(ref api) = self.api {
                                // server 期望 Vec<Vec<String>>:外层每题、内层每题选中的值。
                                // 本 dialog 一次一题,故包一层。labels 已是 option.label
                                // (与 web `InteractionOverlays.tsx:132` 同契约)。
                                let answers = vec![labels];
                                if let Err(e) = api.reply_question(&qid, answers) {
                                    self.store.push_toast(
                                        &format!("question reply failed: {}", e),
                                        crate::store::types::ToastMsgVariant::Error,
                                    );
                                }
                            }
                        }
                        Some(crate::dialog::QuestionKeyOutcome::Skipped) => {
                            // U8：显式跳过（s 键）必须留痕——agent 将收不到
                            // 答案继续运行，用 Warning toast 告知后果。
                            self.store.push_toast(
                                "Question skipped — the agent continues without an answer",
                                crate::store::types::ToastMsgVariant::Warning,
                            );
                        }
                        None => {}
                    }
                    return true;
                }
            }
        }
        false
    }

    /// approve/reject proposal 共用：调 update_skill_proposal_status，
    /// Ok → remove_by_id 回流 + toast Success；Err → toast Error + 列表不变
    /// （悲观执行，无需回滚）。dialog 保持打开支持批量（水生木闭环）。
    fn execute_proposal_status(&mut self, entry: &crate::dialog::SkillProposalEntry, status: &str) {
        if let Some(ref api) = self.api {
            match api.update_skill_proposal_status(&entry.id, status) {
                Ok(_) => {
                    self.skill_proposal.remove_by_id(&entry.id);
                    self.store.push_toast(
                        &format!("Proposal {}: {}", status, entry.title),
                        crate::store::types::ToastMsgVariant::Success,
                    );
                }
                Err(e) => self.store.push_toast(
                    &format!("{} failed: {}", status, e),
                    crate::store::types::ToastMsgVariant::Error,
                ),
            }
        }
    }

    /// connect(true)/disconnect(false) MCP 共用：调 API，Ok → refresh_mcp_into_store
    /// 回流（Settings store + dialog 同源）+ toast Success；Err → toast Error。
    fn execute_mcp_toggle(&mut self, entry: &crate::dialog::McpEntry, connect: bool) {
        if let Some(ref api) = self.api {
            let result = if connect {
                api.connect_mcp(&entry.name)
            } else {
                api.disconnect_mcp(&entry.name)
            };
            match result {
                Ok(_) => {
                    self.refresh_mcp_into_store();
                    self.store.push_toast(
                        &format!(
                            "MCP {}: {}",
                            if connect { "connected" } else { "disconnected" },
                            entry.name
                        ),
                        crate::store::types::ToastMsgVariant::Success,
                    );
                }
                Err(e) => self.store.push_toast(
                    &format!(
                        "{} failed: {}",
                        if connect { "Connect" } else { "Disconnect" },
                        e
                    ),
                    crate::store::types::ToastMsgVariant::Error,
                ),
            }
        }
    }

    /// Panel/Overlay 的 Ctrl 组合键分发（U2·修饰键透传）。返回 true=已消费。
    ///
    /// 文本输入弹窗把完整 KeyEvent 透传给焦点字段的 revue Input
    /// （readline 编辑集：A/E/W/U/K/Z/Y、词跳）；无文本输入的 panel 吞掉
    /// chord——防漏到全局键（q 退出 / h 首页等），也防退化成字面字母。
    /// Panel::None 贯穿返回 false，让 chord 继续流向 Settings 表单/prompt。
    pub(super) fn route_panel_ctrl_key(&mut self, event: &KeyEvent) -> bool {
        match &self.panel {
            Panel::None => false,
            Panel::ModelEdit => self.model_edit_dialog.handle_ctrl_key(event),
            Panel::McpEdit => self.mcp_edit_dialog.handle_ctrl_key(event),
            Panel::PluginEdit => self.plugin_edit_dialog.handle_ctrl_key(event),
            Panel::ProviderEdit => self.provider_edit_dialog.handle_ctrl_key(event),
            Panel::Rename => self.rename_dialog.handle_ctrl_key(event),
            // Slash popup 是输入框的视图（U3）：Ctrl chord 贯穿给 prompt。
            Panel::Slash => false,
            _ => true,
        }
    }

    /// Panel/Overlay 的粘贴分发（U1·bracketed paste）。返回 true=已消费。
    ///
    /// 文本弹窗进焦点字段 Input；ModelSelect/SessionList 进实时过滤 query；
    /// 其余 panel 吞掉——粘贴不穿透到弹窗背后的 prompt（土律·第十条：
    /// 弹窗开着时输入所有权归弹窗）。
    pub(super) fn route_panel_paste(&mut self, text: &str) -> bool {
        match &self.panel {
            Panel::None => false,
            Panel::ModelEdit => self.model_edit_dialog.paste_text(text),
            Panel::McpEdit => self.mcp_edit_dialog.paste_text(text),
            Panel::PluginEdit => self.plugin_edit_dialog.paste_text(text),
            Panel::ProviderEdit => self.provider_edit_dialog.paste_text(text),
            Panel::Rename => self.rename_dialog.paste_text(text),
            Panel::ModelSelect => self.model_select.paste_query(text),
            Panel::SessionList => self.session_list.paste_query(text),
            // U17⑤：skill_list 也有了实时过滤 query。
            Panel::SkillList => self.skill_list.paste_query(text),
            // Slash popup 是输入框的视图（U3）：粘贴贯穿给 prompt。
            Panel::Slash => false,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_revision_conflict;

    #[test]
    fn revision_conflict_matching_is_case_insensitive() {
        assert!(is_revision_conflict(
            "Revision conflict on task-ledger: expected 2, current 3"
        ));
        assert!(is_revision_conflict("revision conflict"));
        assert!(!is_revision_conflict("permission denied"));
    }
}
