//! 火 — Panel/Overlay 按键分发（从 keymap::handle_key 抽出）。
//!
//! keymap.rs 逼近 1500 行软限，把 panel 按键路由单独成文，使 handle_key
//! 重新聚焦 transcript/prompt（道纪：承载边界 + 唯一分发口径）。
//! 语义不变：每个 panel 独占键；Panel::None 贯穿返回 false，让键继续
//! 流向 transcript 滚动与 prompt 输入。

use revue::event::Key;
use crate::app::{AppHandler, Panel, PendingConfirm};
use crate::dialog::PermissionReply;
use crate::store::app_store::Route;

impl AppHandler {
    /// Panel/Overlay 按键分发。返回 true=已消费；false=贯穿（仅 Panel::None）。
    pub(super) fn route_panel_key(&mut self, key: &Key) -> bool {
        match &self.panel {
            Panel::Slash => {
                match self.slash_popup.handle_key(key) {
                    Some(action_id) => {
                        self.execute_slash_action(action_id);
                    }
                    None => {
                        if !self.slash_popup.is_open() { self.panel = Panel::None; }
                    }
                }
                return true;
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
                        let msg = format!("Model: {} ({})", selected.display, qualified);
                        self.store.push_toast(&msg, crate::store::types::ToastMsgVariant::Success);
                        self.panel = Panel::None;
                    }
                    crate::dialog::ModelDialogOutcome::Notice(reason) => {
                        // Surface the reason ("Provider X not connected", etc.)
                        // so the user sees why Enter didn't close the dialog.
                        // Without this, the previous silent return left the
                        // dialog "stuck open" with no clue.
                        self.store.push_toast(&reason, crate::store::types::ToastMsgVariant::Warning);
                    }
                    crate::dialog::ModelDialogOutcome::None => {}
                }
                if !self.model_select.is_open() { self.panel = Panel::None; }
                return true;
            }
            Panel::ModeSelect => {
                if let Some(picked) = self.mode_select.handle_key(key) {
                    // store 契约：`"kind:id"` 复合（对齐 web `App.tsx:836`）；
                    // dispatch 处再 split 分流到 agent / scheduler_profile。
                    let composite = picked.composite();
                    self.store.selected_mode.set(Some(composite.clone()));
                    let msg = format!("Mode: {} ({})", picked.display, composite);
                    self.store.push_toast(&msg, crate::store::types::ToastMsgVariant::Success);
                    self.panel = Panel::None;
                }
                if !self.mode_select.is_open() { self.panel = Panel::None; }
                return true;
            }
            Panel::AgentSelect => {
                if let Some(selected) = self.agent_select.handle_key(key) {
                    self.store.selected_agent.set(Some(selected.name.clone()));
                    let msg = format!("Switched to agent: {}", selected.display);
                    self.store.push_toast(&msg, crate::store::types::ToastMsgVariant::Success);
                    self.panel = Panel::None;
                }
                if !self.agent_select.visible { self.panel = Panel::None; }
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
                                            crate::store::types::ToastMsgVariant::Success),
                                        Ok(false) => self.store.push_toast(
                                            "Session not found",
                                            crate::store::types::ToastMsgVariant::Warning),
                                        Err(e) => self.store.push_toast(
                                            &format!("Delete failed: {}", e),
                                            crate::store::types::ToastMsgVariant::Error),
                                    }
                                }
                                // 退出已删会话路由：重置 transcript + 回 Home，
                                // 避免停在幽灵会话上（金：交付成形不残留失效态）。
                                self.active_session.reset_for_new_session();
                                self.store.navigate_home();
                            }
                            // PendingConfirm 当前单变体，Some(DeleteSession) 已穷尽
                            // 所有 Some；None 收尾。未来新增变体会让此 match 变非
                            // 穷尽 → 编译报错 → 强制补臂（不静默吞错）。
                            None => {}
                        }
                    } else {
                        // 取消：回收 pending。
                        self.pending_confirm = None;
                    }
                    self.panel = Panel::None;
                }
                if !self.confirm_dialog.visible { self.panel = Panel::None; }
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
                if !self.stash_dialog.is_open() { self.panel = Panel::None; }
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
                if !self.rename_dialog.is_open() { self.panel = Panel::None; }
                return true;
            }
            Panel::SessionList => {
                if let Some(entry) = self.session_list.handle_key(key) {
                    // User selected a session — navigate to it
                    self.active_session.set_session_id(&entry.id);
                    self.sf_tx.send_replace(Some(entry.id.clone()));
                    self.load_session_messages(&entry.id);
                    self.store.navigate(Route::Session { session_id: entry.id });
                    self.panel = Panel::None;
                    return true;
                }
                if !self.session_list.is_open() { self.panel = Panel::None; }
                return true;
            }
            Panel::Help => {
                self.help.handle_key(key);
                if !self.help.visible { self.panel = Panel::None; }
                return true;
            }
            Panel::SkillList => {
                if let Some(entry) = self.skill_list.handle_key(key) {
                    // 道纪第十条：诚实标注 first slice —— 列表权威已成，
                    // 挂载（manage_skill + scoping）尚待独立工程，故只 toast
                    // 不假装"已挂载"。
                    self.store.push_toast(
                        &format!("Skill selected (read-only): {} — {}", entry.name, entry.location),
                        crate::store::types::ToastMsgVariant::Info,
                    );
                    self.panel = Panel::None;
                    return true;
                }
                if !self.skill_list.is_open() { self.panel = Panel::None; }
                return true;
            }
            Panel::SkillProposal => {
                if let Some(entry) = self.skill_proposal.handle_key(key) {
                    // 读视图 first slice：approve/reject 需 update_skill_proposal_status
                    // + confirm，留 B 层第三批。诚实标注，不伪"已批准"。
                    self.store.push_toast(
                        &format!("Proposal selected (read-only): [{}] {}", entry.status, entry.title),
                        crate::store::types::ToastMsgVariant::Info,
                    );
                    self.panel = Panel::None;
                    return true;
                }
                if !self.skill_proposal.is_open() { self.panel = Panel::None; }
                return true;
            }
            Panel::McpList => {
                if let Some(entry) = self.mcp_list.handle_key(key) {
                    // 读视图：connect/disconnect/restart 需独立 dialog + API，留后续。
                    self.store.push_toast(
                        &format!("MCP selected (read-only): [{}] {}", entry.status, entry.name),
                        crate::store::types::ToastMsgVariant::Info,
                    );
                    self.panel = Panel::None;
                    return true;
                }
                if !self.mcp_list.is_open() { self.panel = Panel::None; }
                return true;
            }
            Panel::Recovery => {
                if let Some(entry) = self.recovery_list.handle_key(key) {
                    // 读视图：execute recovery action 需 confirm + execute_session_recovery，留后续。
                    self.store.push_toast(
                        &format!("Recovery entry selected (read-only): {}", entry.label),
                        crate::store::types::ToastMsgVariant::Info,
                    );
                    self.panel = Panel::None;
                    return true;
                }
                if !self.recovery_list.is_open() { self.panel = Panel::None; }
                return true;
            }
            Panel::TaskList => {
                if let Some(entry) = self.task_list.handle_key(key) {
                    // 读视图：cancel_task 需 confirm + DELETE，留后续。
                    self.store.push_toast(
                        &format!("Task selected (read-only): [{}] {} ({})", entry.status, entry.agent_name, entry.id),
                        crate::store::types::ToastMsgVariant::Info,
                    );
                    self.panel = Panel::None;
                    return true;
                }
                if !self.task_list.is_open() { self.panel = Panel::None; }
                return true;
            }
            Panel::Alert => {
                self.alert.handle_key(key);
                if !self.alert.visible { self.panel = Panel::None; }
                return true;
            }
            Panel::Fork => {
                if let Some((sid, mid)) = self.fork_dialog.handle_key(key) {
                    if let Some(ref api) = self.api {
                        match api.fork_session(&sid, mid.as_deref()) {
                            Ok(info) => self.store.push_toast(
                                &format!("Forked → {}", info.title),
                                crate::store::types::ToastMsgVariant::Success),
                            Err(e) => self.store.push_toast(
                                &format!("Fork failed: {}", e),
                                crate::store::types::ToastMsgVariant::Error),
                        }
                    }
                    self.panel = Panel::None;
                    return true;
                }
                if !self.fork_dialog.is_open() { self.panel = Panel::None; }
                return true;
            }
            Panel::Export => {
                if let Some(action) = self.export_dialog.handle_key(key) {
                    match action {
                        crate::dialog::ExportAction::Copy(text) => {
                            // OSC52 非显示序列，直接写 stdout 安全；dialog 已标 copied。
                            // 不关 dialog：用户看到 ✓，可继续 s:share 或 Esc。
                            if let Err(e) = crate::dialog::clipboard::copy(&text) {
                                self.store.push_toast(
                                    &format!("Clipboard write failed: {}", e),
                                    crate::store::types::ToastMsgVariant::Error);
                            }
                        }
                        crate::dialog::ExportAction::Share(sid) => {
                            if let Some(ref api) = self.api {
                                match api.share_session(&sid) {
                                    Ok(resp) => self.store.push_toast(
                                        &format!("Shared: {}", resp.url),
                                        crate::store::types::ToastMsgVariant::Success),
                                    Err(e) => self.store.push_toast(
                                        &format!("Share failed: {}", e),
                                        crate::store::types::ToastMsgVariant::Error),
                                }
                            }
                            self.export_dialog.close();
                            self.panel = Panel::None;
                        }
                    }
                    return true;
                }
                if !self.export_dialog.is_open() { self.panel = Panel::None; }
                return true;
            }
            Panel::Provider => {
                if let Some(action) = self.provider_dialog.handle_key(key) {
                    match action {
                        crate::dialog::ProviderAction::Toggle(_pid) => {
                            // bridge 无 disconnect API —— 诚实 toast，不伪成功（避有阳无阴）。
                            self.store.push_toast(
                                "Disconnect not supported in TUI yet",
                                crate::store::types::ToastMsgVariant::Warning);
                        }
                        crate::dialog::ProviderAction::SetAuth(pid, key) => {
                            // dialog 的 ApiKey 流程语义是「连接 provider」，故调
                            // connect_provider（带 key），而非仅 set_auth。
                            if let Some(ref api) = self.api {
                                match api.connect_provider(&pid, &key, None, None) {
                                    Ok(_) => self.store.push_toast(
                                        &format!("Connected: {}", pid),
                                        crate::store::types::ToastMsgVariant::Success),
                                    Err(e) => self.store.push_toast(
                                        &format!("Connect failed: {}", e),
                                        crate::store::types::ToastMsgVariant::Error),
                                }
                            }
                            self.provider_dialog.close();
                            self.panel = Panel::None;
                        }
                        crate::dialog::ProviderAction::RegisterCustom(_pid, _url) => {
                            // register_custom_provider 需 id/protocol/api_key，dialog 仅收
                            // url —— 诚实标注缺口，不伪注册。
                            self.store.push_toast(
                                "Custom registration needs id/protocol/key (not yet collected)",
                                crate::store::types::ToastMsgVariant::Warning);
                        }
                    }
                    return true;
                }
                if !self.provider_dialog.is_open() { self.panel = Panel::None; }
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
                            let reply_str = match reply {
                                PermissionReply::AllowOnce => "once",
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
                    if let Some((qid, labels)) = self.question_dialog.handle_key(key) {
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
                    return true;
                }
            }
        }
        false
    }
}
