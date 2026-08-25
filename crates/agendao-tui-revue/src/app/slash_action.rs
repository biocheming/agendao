//! 土/金 — Slash 命令执行下游：UiActionId → side effect + dialog 启动。
//!
//! 从 keymap.rs 抽出（keymap 累积到 1638 行越软限，本文件容下原 794-1422 行的
//! `execute_slash_action` 巨型 match 与紧邻的 `switch_to_forked_session`
//! helper）。与已有的 `panel_dispatch.rs` 形成下游派发的双文件对称：
//!
//! - `panel_dispatch.rs` ─ Panel 按键路由（panel ⇢ action）
//! - `slash_action.rs`  ─ UiActionId 派发（action ⇢ dialog/state/route）
//!
//! 两者均通过 `impl AppHandler` 在外部文件分散承载（Rust 允许同 impl 块跨文件），
//! 不引入 trait 间接层（土律：唯一编排承载）。
//!
//! 语义不变：本文件是 keymap.rs 的纯位置迁移，未改任何 action 行为。
//! keymap.rs 文件头注释（火主轴）保留，本文件承担土/金职责。
//!
//! 道纪闭环：
//! - 金律「成形语法」— slash 命令统一的输出形态（开 dialog / push panel /
//!   切 route / toast）有了独立承载，不再与 key 路由（火）混居
//! - 土律「唯一编排承载」— `AppHandler` 仍是唯一所有者；只是其方法分布
//!   在多个文件，不改变所有权拓扑

use agendao_command::UiActionId;

use crate::app::app_op;
use crate::app::{AppHandler, Panel, PendingConfirm};
use crate::dialog::StashEntry;
use crate::store::app_store::Route;
use crate::store::types::{RunStatus, ToolPhase};

impl AppHandler {
    /// U6⑤ 公共点火闸（弹窗打开拉取）：防抖（单闸）+ bridge 取用 +
    /// pending 标记 + 回执通道。返回 `Some` = 可 spawn；`None` 时已自行
    /// toast（防抖提示或无 bridge 静默返回——与旧同步路径的 `if let Some
    /// (api)` 不报警口径一致），调用方直接 return。
    fn begin_dialog_fetch(
        &mut self,
        pending_label: &str,
    ) -> Option<(
        crate::bridge::api::ApiBridge,
        tokio::runtime::Handle,
        tokio::sync::mpsc::UnboundedSender<app_op::AppOpOutcome>,
    )> {
        if let Some(cur) = self.store.dialog_fetch_pending.get() {
            self.store.push_toast(
                &format!("Still working: {} — wait for it to finish", cur),
                crate::store::types::ToastMsgVariant::Info,
            );
            return None;
        }
        let api = self.api.clone()?;
        self.store
            .dialog_fetch_pending
            .set(Some(pending_label.to_string()));
        let handle = api.handle().clone();
        let tx = self.app_ops.sender();
        Some((api, handle, tx))
    }

    /// 切到 fork 后的新会话：reset + set_session_id + sf_tx + load + navigate。
    /// /revise（message 级 fork）调完后再 set_text 回填；/fork（整会话 fork）不回填。
    pub(crate) fn switch_to_forked_session(&mut self, info: &agendao_client::SessionInfo) {
        self.active_session.reset_for_new_session();
        self.active_session.set_session_id(&info.id);
        self.sf_tx.send_replace(Some(info.id.clone()));
        self.load_session_messages(&info.id);
        self.store.navigate(Route::Session {
            session_id: info.id.clone(),
        });
        self.reload_session_list();
    }

    /// U4：把未发送草稿写入 stash 并立即落盘（/stash 可找回）。
    /// 空草稿、或文本本身是 slash 命令（"/new" 这类触发文本不是草稿）
    /// 时不收。返回是否实际 stash 了。
    pub(crate) fn stash_unsent_draft(&mut self) -> bool {
        let text = self.prompt.text();
        if text.trim().is_empty() || text.trim_start().starts_with('/') {
            return false;
        }
        self.stash_explicit_content(&text);
        true
    }

    /// 将显式文本写入 Stash 并立即落盘持久化
    pub(crate) fn stash_explicit_content(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        let entry = StashEntry {
            text: text.to_string(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        };
        self.stash_entries.push(entry);
        crate::dialog::prompt_stash::save_stash(&self.stash_entries);
    }

    /// 应用层统一消费外部编辑器 Handoff 结果（执行 Stash 保存、回填、Toast 与 Redraw 标记）
    pub(crate) fn handle_editor_handoff_outcome(
        &mut self,
        outcome: crate::terminal_lifecycle::HandoffOutcome,
    ) {
        match outcome {
            crate::terminal_lifecycle::HandoffOutcome::Success {
                new_content,
                requires_full_redraw,
            } => {
                self.prompt.set_text(&new_content);
                if requires_full_redraw {
                    self.layout_dirty = true;
                    self.prompt_dirty = true;
                    self.transcript_dirty = true;
                }
                self.store.push_toast(
                    "✏️ Draft updated from editor",
                    crate::store::types::ToastMsgVariant::Success,
                );
            }
            crate::terminal_lifecycle::HandoffOutcome::ConflictRetained {
                stash_content,
                error_message,
            } => {
                self.stash_explicit_content(&stash_content);
                self.layout_dirty = true;
                self.prompt_dirty = true;
                self.transcript_dirty = true;
                self.store.push_toast(
                    &format!("{}: saved to /stash", error_message),
                    crate::store::types::ToastMsgVariant::Warning,
                );
            }
            crate::terminal_lifecycle::HandoffOutcome::Cancelled => {
                self.layout_dirty = true;
                self.prompt_dirty = true;
                self.transcript_dirty = true;
                self.store.push_toast(
                    "External editor cancelled (original draft kept)",
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
        }
    }

    pub(crate) fn execute_slash_action(&mut self, action_id: UiActionId) {
        self.panel = Panel::None;
        // ExternalEditor, QueuePrompt, SteerPrompt, InterruptTurn 需要读取或保留当前 Prompt 草稿，绕过通用的清空前置逻辑
        if !matches!(
            action_id,
            UiActionId::ExternalEditor
                | UiActionId::QueuePrompt
                | UiActionId::SteerPrompt
                | UiActionId::InterruptTurn
        ) {
            // U4 草稿保护：动作触发即清 prompt，清前先把草稿 stash 落盘——
            // 此前 sidebar/鼠标/快捷键触发 /new 等动作时草稿被无声销毁。
            // （PromptStashPush 同此通道：旧实现 clear 在前、臂内永远读到空
            // 文本，是死路径；现在统一在清前捕获。）
            let stashed = self.stash_unsent_draft();
            self.prompt.clear();
            if stashed {
                if matches!(action_id, UiActionId::PromptStashPush) {
                    self.store
                        .push_toast("✏️ Stashed", crate::store::types::ToastMsgVariant::Success);
                } else if !matches!(
                    action_id,
                    UiActionId::OpenStash | UiActionId::PromptStashList
                ) {
                    // 查看类动作不打扰（草稿已保全，列表立即可见）。
                    self.store.push_toast(
                        "Draft stashed (/stash to restore)",
                        crate::store::types::ToastMsgVariant::Info,
                    );
                }
            }
        }
        match action_id {
            UiActionId::ShowHelp | UiActionId::ShowStatus => {
                self.help.toggle();
                if self.help.visible {
                    self.panel = Panel::Help;
                }
            }
            UiActionId::NewSession => {
                // 重置 active_session 到新会话初始态:清空当前 session 的消息/状态,
                // 否则 navigate(Home) 后输入消息会追加到旧 session 残留(数据错位)。
                self.active_session.reset_for_new_session();
                self.store.navigate(Route::Home);
                self.prompt.focus();
                self.store.push_toast(
                    "New session created",
                    crate::store::types::ToastMsgVariant::Success,
                );
            }
            UiActionId::AbortExecution => {
                // Cancel running tools
                let tools = self.active_session.active_tools.get();
                let running: Vec<String> = tools
                    .iter()
                    .filter(|t| t.phase == ToolPhase::Running)
                    .map(|t| t.id.clone())
                    .collect();
                if !running.is_empty() {
                    if let Some(sid) = self.active_session.get_session_id() {
                        if let Some(ref api) = self.api {
                            for tool_id in &running {
                                let _ = api.cancel_tool_call(&sid, tool_id);
                            }
                            self.store.push_toast(
                                &format!("Cancelled {} tool(s)", running.len()),
                                crate::store::types::ToastMsgVariant::Info,
                            );
                        }
                    }
                } else {
                    // Fallback: abort whole session
                    if let Some(sid) = self.active_session.get_session_id() {
                        if let Some(ref api) = self.api {
                            let _ = api.abort_session(&sid);
                            self.active_session.set_run_status(RunStatus::Idle);
                            self.store.push_toast(
                                "Session aborted",
                                crate::store::types::ToastMsgVariant::Info,
                            );
                        }
                    }
                }
            }
            UiActionId::QueuePrompt => {
                let text = self.prompt.text();
                if text.trim().is_empty() {
                    self.store.push_toast(
                        "Prompt is empty. Type text to queue.",
                        crate::store::types::ToastMsgVariant::Warning,
                    );
                    return;
                }
                let sid = self
                    .active_session
                    .get_session_id()
                    .unwrap_or_else(|| "home".into());
                let source = crate::shadow::InteractionSource::SlashQueue;
                if !self.should_use_gateway(&sid, &source) {
                    self.store.push_toast(
                        "⚠️ Gateway disabled or tripped: /queue cannot be processed",
                        crate::store::types::ToastMsgVariant::Warning,
                    );
                    return;
                }

                let draft_rev = self.prompt.draft_revision();
                let (cmd, ctx) = crate::command_gateway::CommandGateway::prepare_submit(
                    sid,
                    draft_rev,
                    &text,
                    agendao_types::submission::SubmissionMode::Queue,
                );

                if let Some(ref api) = self.api {
                    let api = api.clone();
                    let handle = api.handle().clone();
                    let tx = self.dispatch_outcomes.sender();
                    handle.spawn(async move {
                        let res: Result<crate::command_gateway::GatewayServerResponse, String> =
                            api.submit_gateway_command_async(
                                crate::command_gateway::GatewayCommand::Submit(cmd),
                            )
                            .await
                            .map_err(|e| e.to_string());
                        let disp_res = match res {
                            Ok(crate::command_gateway::GatewayServerResponse::Submit(disp)) => {
                                Ok(disp)
                            }
                            Ok(crate::command_gateway::GatewayServerResponse::Interrupt(_)) => {
                                Err("Protocol error: unexpected interrupt response".into())
                            }
                            Err(e) => Err(e),
                        };
                        let _ = tx.send(
                            crate::app::dispatch_outcome::DispatchOutcome::GatewaySubmit {
                                ctx,
                                response: disp_res,
                            },
                        );
                    });
                }
            }
            UiActionId::SteerPrompt => {
                let text = self.prompt.text();
                if text.trim().is_empty() {
                    self.store.push_toast(
                        "Prompt is empty. Type steering text.",
                        crate::store::types::ToastMsgVariant::Warning,
                    );
                    return;
                }
                // 必须具备活跃 Turn
                let active_turn = self.active_session.active_turn_id.get();
                let Some(turn_id) = active_turn else {
                    self.store.push_toast(
                        "No active turn to steer",
                        crate::store::types::ToastMsgVariant::Warning,
                    );
                    return;
                };

                let sid = self
                    .active_session
                    .get_session_id()
                    .unwrap_or_else(|| "home".into());
                let source = crate::shadow::InteractionSource::SlashSteer;
                if !self.should_use_gateway(&sid, &source) {
                    self.store.push_toast(
                        "⚠️ Gateway disabled or tripped: /steer cannot be processed",
                        crate::store::types::ToastMsgVariant::Warning,
                    );
                    return;
                }

                let draft_rev = self.prompt.draft_revision();
                let (cmd, ctx) = crate::command_gateway::CommandGateway::prepare_submit(
                    sid,
                    draft_rev,
                    &text,
                    agendao_types::submission::SubmissionMode::Steer {
                        expected_turn_id: turn_id,
                    },
                );

                if let Some(ref api) = self.api {
                    let api = api.clone();
                    let handle = api.handle().clone();
                    let tx = self.dispatch_outcomes.sender();
                    handle.spawn(async move {
                        let res: Result<crate::command_gateway::GatewayServerResponse, String> =
                            api.submit_gateway_command_async(
                                crate::command_gateway::GatewayCommand::Submit(cmd),
                            )
                            .await
                            .map_err(|e| e.to_string());
                        let disp_res = match res {
                            Ok(crate::command_gateway::GatewayServerResponse::Submit(disp)) => {
                                Ok(disp)
                            }
                            Ok(crate::command_gateway::GatewayServerResponse::Interrupt(_)) => {
                                Err("Protocol error: unexpected interrupt response".into())
                            }
                            Err(e) => Err(e),
                        };
                        let _ = tx.send(
                            crate::app::dispatch_outcome::DispatchOutcome::GatewaySubmit {
                                ctx,
                                response: disp_res,
                            },
                        );
                    });
                }
            }
            UiActionId::InterruptTurn => {
                let active_turn = self.active_session.active_turn_id.get();
                let Some(turn_id) = active_turn else {
                    self.store.push_toast(
                        "No active turn to interrupt",
                        crate::store::types::ToastMsgVariant::Warning,
                    );
                    return;
                };

                let sid = self
                    .active_session
                    .get_session_id()
                    .unwrap_or_else(|| "home".into());
                let source = crate::shadow::InteractionSource::SlashInterrupt;
                let (cmd, ctx) =
                    crate::command_gateway::CommandGateway::prepare_interrupt(sid.clone(), turn_id);

                if self.should_use_gateway(&sid, &source) {
                    if let Some(ref api) = self.api {
                        let api = api.clone();
                        let handle = api.handle().clone();
                        let tx = self.dispatch_outcomes.sender();
                        handle.spawn(async move {
                            let res: Result<crate::command_gateway::GatewayServerResponse, String> =
                                api.submit_gateway_command_async(
                                    crate::command_gateway::GatewayCommand::Interrupt(cmd),
                                )
                                .await
                                .map_err(|e| e.to_string());
                            let int_disp = match res {
                                Ok(crate::command_gateway::GatewayServerResponse::Interrupt(
                                    disp,
                                )) => Ok(disp),
                                Ok(crate::command_gateway::GatewayServerResponse::Submit(_)) => {
                                    Err("Protocol error: unexpected submit response".into())
                                }
                                Err(e) => Err(e),
                            };
                            let _ = tx.send(
                                crate::app::dispatch_outcome::DispatchOutcome::GatewayInterrupt {
                                    ctx,
                                    response: int_disp,
                                },
                            );
                        });
                    }
                } else if let Some(ref api) = self.api {
                    // Gateway 熔断或未放量：回退走旧 abort_session_async
                    let api_c = api.clone();
                    let tx = self.dispatch_outcomes.sender();
                    let sid_c = sid.clone();

                    api.handle().spawn(async move {
                        let r = api_c.abort_session_async(&sid_c).await;
                        let (is_ok, err_str) = match r {
                            Ok(value) => {
                                let aborted = value
                                    .get("aborted")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(true);
                                (aborted, None)
                            }
                            Err(e) => (false, Some(e.to_string())),
                        };
                        let _ = tx.send(
                            crate::app::dispatch_outcome::DispatchOutcome::LegacyInterrupt {
                                session_id: sid_c,
                                success: is_ok,
                                error: err_str,
                            },
                        );
                    });
                }
            }
            UiActionId::OpenStash | UiActionId::PromptStashList => {
                self.stash_dialog.set_entries(self.stash_entries.clone());
                self.stash_dialog.open();
                self.panel = Panel::Stash;
            }
            UiActionId::PromptStashPush => {
                // U4：stash 已在函数顶部统一收口（清 prompt 前捕获草稿）。
                // 旧实现在此处读 prompt.text()，但 clear 在前，永远读空——
                // 死路径，已由顶部捕获替代。
            }
            UiActionId::Exit => {
                self.store.request_exit();
                // exiting 信号无读者（保留为状态记录）；真正的退出经
                // quit_requested 旗标 → run 循环 app.quit() 收口。
                self.quit_requested = true;
            }
            UiActionId::OpenModelList => {
                let (current_model, _) =
                    super::effective_session_identity(&self.store, &self.active_session);
                self.model_select.set_current(current_model);
                // F2：打开即拉 recent models 填充 "★ Recent" 区块（此前
                // set_recent 无调用者，区块永远空——死代码）。U6⑤：拉取移
                // 后台，dialog 即时打开（与原口径一致：失败仅 warn 不阻塞）。
                if let Some((api, handle, tx)) = self.begin_dialog_fetch("Loading models") {
                    handle.spawn(async move {
                        // 失败口径=旧同步路径：仅 warn，不 toast 不阻塞（弹窗
                        // 已开）；以空表回执代失败，drain 对空表跳过 set_recent。
                        let result = api
                            .get_recent_models_async()
                            .await
                            .map(app_op::DialogFetchData::RecentModels)
                            .unwrap_or_else(|e| {
                                tracing::warn!(%e, "get_recent_models failed");
                                app_op::DialogFetchData::RecentModels(Vec::new())
                            });
                        let _ = tx.send(app_op::AppOpOutcome::DialogFetchDone(Ok(result)));
                    });
                }
                self.model_select.open();
                self.panel = Panel::ModelSelect;
            }
            UiActionId::OpenAgentList => {
                let (_, current_agent) =
                    super::effective_session_identity(&self.store, &self.active_session);
                self.agent_select.set_current(current_agent);
                self.agent_select.open();
                self.panel = Panel::AgentSelect;
            }
            UiActionId::OpenNotifications => {
                // U7③：通知中心——只读回看 toast_history（含已过期/dismiss）。
                self.notification_dialog.open();
                self.panel = Panel::Notifications;
            }
            UiActionId::OpenSkills => {
                // 读视图 first slice（道纪第十条）：列表权威已成，挂载需
                // manage_skill + scoping 独立工程，故只 toast 不伪成功。
                // U6⑤：拉取移后台——空/失败/成功分支全部由 drain 数据驱动
                // （与原同步口径逐条对应）。
                if let Some((api, handle, tx)) = self.begin_dialog_fetch("Loading skills") {
                    handle.spawn(async move {
                        let result = api
                            .list_skills_async(None)
                            .await
                            .map(app_op::DialogFetchData::Skills)
                            .map_err(|e| format!("Failed to load skills: {}", e));
                        let _ = tx.send(app_op::AppOpOutcome::DialogFetchDone(result));
                    });
                }
            }
            UiActionId::OpenSkillProposals => {
                // 读视图 first slice：approve/reject 需 update_skill_proposal_status
                // + confirm，留 B 层第三批（道纪第十条：不伪"已批准"）。
                if let Some((api, handle, tx)) = self.begin_dialog_fetch("Loading proposals") {
                    handle.spawn(async move {
                        let result = api
                            .list_skill_proposals_async("draft")
                            .await
                            .map(app_op::DialogFetchData::SkillProposals)
                            .map_err(|e| format!("Failed to load proposals: {}", e));
                        let _ = tx.send(app_op::AppOpOutcome::DialogFetchDone(result));
                    });
                }
            }
            UiActionId::OpenMcpList => {
                if let Some((api, handle, tx)) = self.begin_dialog_fetch("Loading MCP servers") {
                    handle.spawn(async move {
                        let result = api
                            .get_mcp_status_async()
                            .await
                            .map(app_op::DialogFetchData::McpStatus)
                            .map_err(|e| format!("Failed to load MCP status: {}", e));
                        let _ = tx.send(app_op::AppOpOutcome::DialogFetchDone(result));
                    });
                }
            }
            UiActionId::OpenRecoveryList => {
                // per-session：需 active session_id。
                if let Some(sid) = self.active_session.get_session_id() {
                    if let Some((api, handle, tx)) = self.begin_dialog_fetch("Loading recovery") {
                        handle.spawn(async move {
                            let result = api
                                .get_session_recovery_async(&sid)
                                .await
                                .map(|p| app_op::DialogFetchData::Recovery(Box::new(p)))
                                .map_err(|e| format!("Failed to load recovery: {}", e));
                            let _ = tx.send(app_op::AppOpOutcome::DialogFetchDone(result));
                        });
                    }
                } else {
                    self.store.push_toast(
                        "Open a session first",
                        crate::store::types::ToastMsgVariant::Warning,
                    );
                }
            }
            UiActionId::OpenModeList => {
                if let Some((api, handle, tx)) = self.begin_dialog_fetch("Loading modes") {
                    handle.spawn(async move {
                        let result = api
                            .list_execution_modes_async()
                            .await
                            .map(app_op::DialogFetchData::Modes)
                            .map_err(|e| format!("Failed to load modes: {}", e));
                        let _ = tx.send(app_op::AppOpOutcome::DialogFetchDone(result));
                    });
                }
            }
            UiActionId::RenameSession => {
                if let Some(sid) = self.active_session.get_session_id() {
                    let title = self.active_session.title.get();
                    self.rename_dialog.open(&sid, &title);
                    self.panel = Panel::Rename;
                }
            }
            UiActionId::ForkSession => {
                if let Some(sid) = self.active_session.get_session_id() {
                    // F9：拉最近消息列进 fork dialog 供选择锚点；首项
                    // "(latest)" = 整会话 fork。拉取失败退化为整会话 fork
                    // 单项（不阻塞入口）。
                    let mut options = vec![crate::dialog::ForkMessageOption {
                        message_id: None,
                        label: "(latest) — fork whole session".to_string(),
                    }];
                    if let Some(ref api) = self.api {
                        match api.get_messages(&sid) {
                            Ok(messages) => {
                                // 最新在上，最多 20 条；预览取首个 text part。
                                for m in messages.iter().rev().take(20) {
                                    let preview = m
                                        .parts
                                        .iter()
                                        .find_map(|p| p.text.as_deref())
                                        .map(|t| {
                                            let flat: String = t.chars().take(60).collect();
                                            flat.replace('\n', " ")
                                        })
                                        .unwrap_or_default();
                                    options.push(crate::dialog::ForkMessageOption {
                                        message_id: Some(m.id.clone()),
                                        label: format!("[{}] {}", m.role, preview),
                                    });
                                }
                            }
                            Err(e) => {
                                self.store.push_toast(
                                    &format!(
                                        "Message list unavailable ({e}) — fork whole session only"
                                    ),
                                    crate::store::types::ToastMsgVariant::Warning,
                                );
                            }
                        }
                    }
                    self.fork_dialog.open_with_messages(&sid, options);
                    self.panel = Panel::Fork;
                }
            }
            UiActionId::RevisePrompt => {
                // Part 6: web `editAndResendMessage` 的 TUI 对位。
                // 木→火→水→木闭环：光标选 UserPrompt（火）→ fork+回填（水）
                // → 输入框带原文等用户改（木）→ 用户 Enter 发送（火）。
                let Some((prompt_id, content)) = self.active_session.cursor_user_prompt() else {
                    self.store.push_toast(
                        "No user prompt under cursor — use Tab/j/k to select one",
                        crate::store::types::ToastMsgVariant::Warning,
                    );
                    return;
                };
                let Some(sid) = self.active_session.get_session_id() else {
                    self.store.push_toast(
                        "No active session to fork from",
                        crate::store::types::ToastMsgVariant::Warning,
                    );
                    return;
                };
                let Some(ref api) = self.api else {
                    return;
                };
                match api.fork_session(&sid, Some(&prompt_id)) {
                    Ok(info) => {
                        // 切到 fork 后的新会话（reset+set_session_id+sf_tx+load+navigate）。
                        self.switch_to_forked_session(&info);
                        // 回填输入框（木）——用户编辑后 Enter 发新 prompt。
                        self.prompt.set_text(&content);
                        self.store.push_toast(
                            "Forked — edit prompt and Enter to resend",
                            crate::store::types::ToastMsgVariant::Success,
                        );
                    }
                    Err(e) => self.store.push_toast(
                        &format!("Revise failed: {}", e),
                        crate::store::types::ToastMsgVariant::Error,
                    ),
                }
            }
            UiActionId::ExportSession => {
                if let Some(sid) = self.active_session.get_session_id() {
                    // 土律：transcript→text 走唯一序列化权威
                    // (session_store::transcript_to_text)，与 /copy 共用。
                    let text = self.active_session.transcript_to_text();
                    self.export_dialog.open(&sid, &text);
                    self.panel = Panel::Export;
                }
            }
            UiActionId::ShareSession => {
                // Part 4: /share 不再走 export dialog 二次确认，直接 API。
                // F11：URL 同时进剪贴板（OSC52）；写失败不吞——toast 如实标注。
                if let Some(sid) = self.active_session.get_session_id() {
                    if let Some(ref api) = self.api {
                        match api.share_session(&sid) {
                            Ok(resp) => {
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
                }
            }
            UiActionId::UnshareSession => {
                // Part 5
                if let Some(sid) = self.active_session.get_session_id() {
                    if let Some(ref api) = self.api {
                        match api.unshare_session(&sid) {
                            Ok(true) => self.store.push_toast(
                                "Session unshared",
                                crate::store::types::ToastMsgVariant::Success,
                            ),
                            Ok(false) => self.store.push_toast(
                                "Session was not shared",
                                crate::store::types::ToastMsgVariant::Warning,
                            ),
                            Err(e) => self.store.push_toast(
                                &format!("Unshare failed: {}", e),
                                crate::store::types::ToastMsgVariant::Error,
                            ),
                        }
                    }
                }
            }
            UiActionId::CopySession => {
                // Part 2: 与 /export 复用同一序列化（土律），但不开 dialog——
                // 直接 OSC52 写终端剪贴板（A4 helper）。
                // U18①：copy_with_fallback——OSC52 失败兜底临时文件 + 路径 toast。
                let text = self.active_session.transcript_to_text();
                let r = crate::dialog::clipboard::copy_with_fallback(&text);
                self.toast_copy_outcome(r, "Transcript copied to clipboard");
            }
            UiActionId::CompactSession => {
                // F10：`/compact <focus>` 的 focus 由 sync_slash_from_text 暂存。
                let focus = self.pending_compact_focus.take();
                // U6 异步化：触发调用可耗数秒 → 后台 task；在飞期间
                // run_status=Sending 转 spinner（处理中指示），重复触发防抖。
                if self.compact_in_flight {
                    self.store.push_toast(
                        "Compaction already in progress",
                        crate::store::types::ToastMsgVariant::Info,
                    );
                } else if let Some(sid) = self.active_session.get_session_id() {
                    if let Some(api) = self.api.clone() {
                        self.compact_in_flight = true;
                        self.active_session.set_run_status(RunStatus::Sending);
                        let tx = self.app_ops.sender();
                        let handle = api.handle().clone();
                        handle.spawn(async move {
                            let result = api
                                .compact_session_async(&sid, focus.as_deref())
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string());
                            let _ =
                                tx.send(crate::app::app_op::AppOpOutcome::CompactionTriggered {
                                    session_id: sid,
                                    focus,
                                    result,
                                });
                        });
                    }
                }
            }
            UiActionId::ConnectProvider => {
                // TUI 路径已迁至全屏 Settings 页(`OpenSettings`),`ConnectProvider`
                // 仅 CLI 互动模式仍消费(agendao-command::interactive)。TUI 这里诚实标注:
                // 触发即提示用户走新入口,避免悄无声响地什么都不发生(土律·第十条)。
                self.store.push_toast(
                    "Provider settings moved to /settings",
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
            UiActionId::OpenSettings => {
                // 阳面唯一入口:⚙ click / `/settings` slash 都走这里。
                // 步骤(土律·第七条·木生火生土):
                //   1) navigate → Route::Settings(金面立即切到 SettingsScreen)
                //   2) `refresh_providers_into_store` 拉 providers + connected
                //      回灌 store(单点权威 — 与 submit_provider_edit 共用同一抽函数)
                // 拉取失败诚实 toast,Route 已切换不回滚(空态 Details 栏会显示
                // "Select a provider",符合可观测性权利)。
                self.store.navigate_settings();
                self.refresh_providers_into_store();
                self.refresh_mcp_into_store();
                self.refresh_skills_into_store();
                self.refresh_tools_into_store();
                self.refresh_plugins_into_store();
            }
            UiActionId::DeleteSession => {
                if let Some(sid) = self.active_session.get_session_id() {
                    let title = self.active_session.title.get();
                    self.confirm_dialog.ask(
                        "Delete Session",
                        &format!("Delete \"{}\"? This cannot be undone.", title),
                        "Delete",
                    );
                    // 判别器携带「确认什么」——Confirm 只回 bool，靠它路由
                    // 到 delete_session（土律：单一 Confirm 变体服务所有确认）。
                    self.pending_confirm = Some(PendingConfirm::DeleteSession(sid));
                    self.panel = Panel::Confirm;
                }
            }
            UiActionId::OpenSessionList => {
                // U6⑤：弹窗立开 + 真 loading 态（session_list.loading 分支原
                // 为死代码）；拉取移后台，drain 处填充/置错。reload_session_list
                // 的 store 刷新也随 drain 完成（Sidebar 树同步更新）。
                self.session_list.open();
                self.session_list.loading = true;
                self.panel = Panel::SessionList;
                let cwd = self.store.working_dir.get();
                self.session_list.set_directory_scope(cwd.clone());
                if let Some((api, handle, tx)) = self.begin_dialog_fetch("Loading sessions") {
                    let cwd_filter = if cwd.is_empty() { None } else { Some(cwd) };
                    handle.spawn(async move {
                        let result = api
                            .list_sessions_in_directory_async(cwd_filter)
                            .await
                            .map(app_op::DialogFetchData::Sessions)
                            .map_err(|e| format!("Failed to refresh session list: {}", e));
                        let _ = tx.send(app_op::AppOpOutcome::DialogFetchDone(result));
                    });
                } else if self.api.is_none() {
                    // 无桥：立即可判空态（drain 不会来）。U15②：空 ≠ 错误，
                    // 填空列表走 muted 空态分支，不 set_error 红字。
                    self.session_list.set_sessions(Vec::new());
                } else {
                    // debounce：在途拉取属于别的弹窗（begin_dialog_fetch 已
                    // toast），loading 不能干等——用 store 现有缓存填充；
                    // 若在途的恰是 sessions 拉取，drain 到达后自然覆盖。
                    let entries: Vec<crate::dialog::SessionEntry> = self
                        .store
                        .session_list
                        .get()
                        .into_iter()
                        .map(|s| crate::dialog::SessionEntry {
                            id: s.id,
                            title: s.title,
                            status_hint: String::new(),
                        })
                        .collect();
                    if entries.is_empty() {
                        // U15②：缓存为空 = 空态，非错误。
                        self.session_list.set_sessions(Vec::new());
                    } else {
                        self.session_list.set_sessions(entries);
                    }
                }
            }
            UiActionId::ToggleSidebar => {
                self.store.push_toast(
                    "Sidebar toggled",
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
            UiActionId::ToggleThinking => {
                let next = self
                    .toggle_details_section(crate::details_policy::DetailsSection::Thinking)
                    .is_some_and(|v| {
                        matches!(v, crate::details_policy::DetailVisibility::Expanded)
                    });
                self.store.show_thinking.set(next);
                self.store.push_toast(
                    &format!("Thinking blocks: {}", if next { "shown" } else { "hidden" }),
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
            UiActionId::Details => {
                let section = match self.pending_details_section.take().as_deref() {
                    Some("thinking") => Some(crate::details_policy::DetailsSection::Thinking),
                    Some("tools") => Some(crate::details_policy::DetailsSection::Tools),
                    Some("todo") => Some(crate::details_policy::DetailsSection::Todo),
                    Some("subagents") => Some(crate::details_policy::DetailsSection::Subagents),
                    _ => None,
                };
                if let Some(section) = section {
                    if let Some(next) = self.toggle_details_section(section) {
                        self.store.push_toast(
                            &format!("Details {section:?}: {next:?}"),
                            crate::store::types::ToastMsgVariant::Info,
                        );
                    } else {
                        self.store.push_toast(
                            "No details available for that section",
                            crate::store::types::ToastMsgVariant::Info,
                        );
                    }
                } else {
                    self.store.push_toast(
                        "Usage: /details thinking|tools|todo|subagents",
                        crate::store::types::ToastMsgVariant::Info,
                    );
                }
            }
            UiActionId::ToggleScrollbar => {
                let next = !self.store.show_scrollbar.get();
                self.store.show_scrollbar.set(next);
                self.store.push_toast(
                    &format!("Scrollbar: {}", if next { "shown" } else { "hidden" }),
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
            UiActionId::ToggleHeader => {
                // 隐藏时 render 端 header 块整段跳过 + dir_hit=None + transcript_area_y
                // 从 3 降 1（app/mod.rs 几何同口径），header_y 从 1 降 0（dir 点击不命中）。
                let next = !self.store.show_header.get();
                self.store.show_header.set(next);
                self.store.push_toast(
                    &format!("Header: {}", if next { "shown" } else { "hidden" }),
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
            UiActionId::ToggleTips => {
                // 隐藏时 hint 行内容置空（几何不变，保 PromptGeom y_top 同口径）。
                let next = !self.store.show_tips.get();
                self.store.show_tips.set(next);
                self.store.push_toast(
                    &format!("Tips: {}", if next { "shown" } else { "hidden" }),
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
            UiActionId::ToggleDensity => {
                // 紧凑模式：块间 0 间隔。transcript_total_height 同口径 gap=0,
                // 渲染端跳过 child_sized("",1)——阴阳同口径(金律)。
                let next = !self.store.compact_density.get();
                self.store.compact_density.set(next);
                self.store.push_toast(
                    &format!("Density: {}", if next { "compact" } else { "comfortable" }),
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
            UiActionId::ToggleTimestamps => {
                // 道纪第十条：TUI 当前不渲染 timestamp（TranscriptBlock 无时间字段），
                // 翻转一个无消费端的 signal 即伪权威。诚实标注缺口而非伪成功。
                self.store.push_toast(
                    "Timestamps: TUI does not render them yet (no time field on blocks)",
                    crate::store::types::ToastMsgVariant::Warning,
                );
            }
            UiActionId::ToggleAppearance
            | UiActionId::AppearanceNext
            | UiActionId::AppearancePrev => {
                // 主题循环唯一权威（土律归一）：ToggleAppearance=下一个（Ctrl+P
                // palette 兼容入口），AppearanceNext/Prev=Settings Theme 行 →/←。
                // 色板 + revue 主题信号经 ds::theme::apply_theme 单点收口；
                // CSS `:root` 变量写 pending 槽，由 app 事件闭包应用到 stylesheet。
                let cur = self.store.theme_id.get();
                let next = if action_id == UiActionId::AppearancePrev {
                    cur.prev()
                } else {
                    cur.next()
                };
                self.pending_theme_vars = Some(crate::ds::theme::apply_theme(next));
                self.store.theme_id.set(next);
                // 持久化（config `theme` 键）fire-and-forget：失败仅日志，不阻塞换肤。
                if let Some(ref api) = self.api {
                    let api_c = api.clone();
                    let id_str = next.id().to_string();
                    api.handle().spawn(async move {
                        if let Err(e) = api_c
                            .patch_config_async(serde_json::json!({ "theme": id_str }))
                            .await
                        {
                            tracing::warn!(%e, "theme persist failed");
                        }
                    });
                }
                self.store.push_toast(
                    &format!("Theme: {}", next.label()),
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
            UiActionId::ExternalEditor => {
                let initial_text = self.prompt.text();
                let session_id = self
                    .active_session
                    .get_session_id()
                    .unwrap_or_else(|| "home".into());
                let draft_revision = self.prompt.draft_revision();
                let target = crate::terminal_lifecycle::DraftTarget {
                    session_id: session_id.clone(),
                    draft_revision,
                };

                let backend = crate::terminal_lifecycle::CrosstermTerminalBackend;
                // 在 revue 当前 App::run 同步阻塞模型下，handler 阻塞期间不处理 TUI 事件；
                // 生产环境不传入无消费者的 reader_control，避免 request_pause 超时。
                let mut lifecycle =
                    crate::terminal_lifecycle::TerminalLifecycle::new(backend, None);

                let active_session = self.active_session.clone();
                let draft_revision_handle = self.prompt.draft_revision_handle();
                let res = if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    tokio::task::block_in_place(|| {
                        handle.block_on(async {
                            lifecycle
                                .execute_editor_handoff(target, &initial_text, move || {
                                    let latest_session = active_session
                                        .get_session_id()
                                        .unwrap_or_else(|| "home".into());
                                    let latest_rev = draft_revision_handle
                                        .load(std::sync::atomic::Ordering::SeqCst);
                                    (latest_session, latest_rev)
                                })
                                .await
                        })
                    })
                } else if let Ok(rt) = tokio::runtime::Runtime::new() {
                    rt.block_on(async {
                        lifecycle
                            .execute_editor_handoff(target, &initial_text, move || {
                                let latest_session = active_session
                                    .get_session_id()
                                    .unwrap_or_else(|| "home".into());
                                let latest_rev =
                                    draft_revision_handle.load(std::sync::atomic::Ordering::SeqCst);
                                (latest_session, latest_rev)
                            })
                            .await
                    })
                } else {
                    Err(crate::terminal_lifecycle::HandoffError::ProcessLaunchError(
                        "Failed to initialize async runtime for editor handoff".into(),
                    ))
                };

                match res {
                    Ok(outcome) => {
                        self.handle_editor_handoff_outcome(outcome);
                    }
                    Err(err) => {
                        self.layout_dirty = true;
                        self.prompt_dirty = true;
                        self.transcript_dirty = true;
                        self.store.push_toast(
                            &format!("Editor error: {}", err),
                            crate::store::types::ToastMsgVariant::Error,
                        );
                    }
                }
            }
            // 道纪第十条：伪权威诚实标注。这些 action 的 spec 已注册（slash
            // 可触发），但 server 端无对应 API（grep 全无端点）——落到通用
            // "coming soon" 兜底会掩盖「server 根本没这个能力」的真相。这里
            // 显式分流，让用户看到权威缺口而非伪期待。server 补齐后再开路由。
            UiActionId::Undo | UiActionId::Redo => {
                // spec 语义是会话级"撤回/恢复上一条消息"（Revert last
                // message），server 无端点 → 诚实标注；同时指向 prompt
                // 文本级撤销（Ctrl+Z/Ctrl+Y，U2 已通），避免用户以为
                // "撤销"在整个 TUI 里不存在。
                self.store.push_toast(
                    &format!("{:?}: session-level revert not supported by server yet — prompt text undo is Ctrl+Z / Ctrl+Y", action_id),
                    crate::store::types::ToastMsgVariant::Warning,
                );
            }
            UiActionId::Timeline | UiActionId::NavigateParentSession | UiActionId::VoiceInput => {
                self.store.push_toast(
                    &format!("{:?}: not supported by server yet", action_id),
                    crate::store::types::ToastMsgVariant::Warning,
                );
            }
            UiActionId::OpenThemeList => {
                // /themes (Ctrl+T)：ToggleAppearance 已通（Ctrl+P palette 翻转 dark/light），
                // 但多主题选择器待续。诚实标注指向可用入口，不伪"coming soon"空话。
                self.store.push_toast(
                    "Toggle dark/light via Ctrl+P → Toggle appearance; full theme picker coming soon",
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
            UiActionId::ToggleToolDetails => {
                // 已有 per-block fold（Space 键 toggle），无全局 toggle 权威。
                self.store.push_toast(
                    "Use Space on a tool block to fold/unfold",
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
            UiActionId::ToggleCommandPalette => {
                // slash_popup 已是命令面板（/ 或 Ctrl+P），无独立 toggle。
                self.store.push_toast(
                    "Use Ctrl+P or type / to open the command palette",
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
            UiActionId::ToggleSemanticHighlight => {
                // TUI 用 NoopTheme，无代码语法高亮能力。
                self.store.push_toast(
                    "Code highlighting not available in TUI yet",
                    crate::store::types::ToastMsgVariant::Warning,
                );
            }
            UiActionId::ToggleMcp => {
                // MCP 状态权威在 sidebar tab（Ctrl+B 开 sidebar 后切 tab）。
                self.store.push_toast(
                    "MCP status lives in sidebar tab (Ctrl+B)",
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
            _ => {
                self.store.push_toast(
                    &format!("{:?} — coming soon", action_id),
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
        }
    }
}
