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

use crate::app::{AppHandler, Panel, PendingConfirm};
use crate::store::app_store::Route;
use crate::store::types::{RunStatus, ToolPhase};
use crate::dialog::{
    StashEntry,
    SkillEntry, SkillProposalEntry, McpEntry, RecoveryEntry, TaskEntry,
};

impl AppHandler {
    /// 切到 fork 后的新会话：reset + set_session_id + sf_tx + load + navigate。
    /// /revise（message 级 fork）调完后再 set_text 回填；/fork（整会话 fork）不回填。
    pub(crate) fn switch_to_forked_session(&mut self, info: &agendao_client::SessionInfo) {
        self.active_session.reset_for_new_session();
        self.active_session.set_session_id(&info.id);
        self.sf_tx.send_replace(Some(info.id.clone()));
        self.load_session_messages(&info.id);
        self.store.navigate(Route::Session { session_id: info.id.clone() });
        self.reload_session_list();
    }

    pub(crate) fn execute_slash_action(&mut self, action_id: UiActionId) {
        self.panel = Panel::None;
        self.prompt.clear();
        match action_id {
            UiActionId::ShowHelp | UiActionId::ShowStatus => {
                self.help.toggle();
                if self.help.visible { self.panel = Panel::Help; }
            }
            UiActionId::NewSession => {
                // 重置 active_session 到新会话初始态:清空当前 session 的消息/状态,
                // 否则 navigate(Home) 后输入消息会追加到旧 session 残留(数据错位)。
                self.active_session.reset_for_new_session();
                self.store.navigate(Route::Home);
                self.prompt.focus();
                self.store.push_toast("New session created", crate::store::types::ToastMsgVariant::Success);
            }
            UiActionId::AbortExecution => {
                // Cancel running tools
                let tools = self.active_session.active_tools.get();
                let running: Vec<String> = tools.iter()
                    .filter(|t| t.phase == ToolPhase::Running)
                    .map(|t| t.id.clone())
                    .collect();
                if !running.is_empty() {
                    if let Some(sid) = self.active_session.get_session_id() {
                        if let Some(ref api) = self.api {
                            for tool_id in &running {
                                let _ = api.cancel_tool_call(&sid, tool_id);
                            }
                            self.store.push_toast(&format!("Cancelled {} tool(s)", running.len()),
                                crate::store::types::ToastMsgVariant::Info);
                        }
                    }
                } else {
                    // Fallback: abort whole session
                    if let Some(sid) = self.active_session.get_session_id() {
                        if let Some(ref api) = self.api {
                            let _ = api.abort_session(&sid);
                            self.active_session.run_status.set(RunStatus::Idle);
                            self.store.push_toast("Session aborted", crate::store::types::ToastMsgVariant::Info);
                        }
                    }
                }
            }
            UiActionId::OpenStash | UiActionId::PromptStashList => {
                self.stash_dialog.set_entries(self.stash_entries.clone());
                self.stash_dialog.open();
                self.panel = Panel::Stash;
            }
            UiActionId::PromptStashPush => {
                let text = self.prompt.text();
                if !text.trim().is_empty() {
                    let entry = StashEntry {
                        text,
                        created_at: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0),
                    };
                    self.stash_entries.push(entry);
                    // 水律：push 后立即落盘，下一轮/下次启动可复用。
                    crate::dialog::prompt_stash::save_stash(&self.stash_entries);
                    self.prompt.clear();
                    self.store.push_toast("✏️ Stashed", crate::store::types::ToastMsgVariant::Success);
                }
            }
            UiActionId::Exit => {
                self.store.request_exit();
            }
            UiActionId::OpenModelList => {
                self.model_select.open();
                self.panel = Panel::ModelSelect;
            }
            UiActionId::OpenAgentList => {
                self.agent_select.open();
                self.panel = Panel::AgentSelect;
            }
            UiActionId::OpenSkills => {
                // 读视图 first slice（道纪第十条）：列表权威已成，挂载需
                // manage_skill + scoping 独立工程，故只 toast 不伪成功。
                if let Some(ref api) = self.api {
                    match api.list_skills(None) {
                        Ok(skills) => {
                            let entries: Vec<SkillEntry> = skills.into_iter()
                                .map(|s| SkillEntry {
                                    name: s.name,
                                    description: s.description,
                                    location: s.location,
                                })
                                .collect();
                            if entries.is_empty() {
                                self.store.push_toast(
                                    "No skills available",
                                    crate::store::types::ToastMsgVariant::Warning,
                                );
                            } else {
                                self.skill_list.set_skills(entries);
                                self.skill_list.open();
                                self.panel = Panel::SkillList;
                            }
                        }
                        Err(e) => {
                            self.store.push_toast(
                                &format!("Failed to load skills: {}", e),
                                crate::store::types::ToastMsgVariant::Error,
                            );
                        }
                    }
                }
            }
            UiActionId::OpenSkillProposals => {
                // 读视图 first slice：approve/reject 需 update_skill_proposal_status
                // + confirm，留 B 层第三批（道纪第十条：不伪"已批准"）。
                if let Some(ref api) = self.api {
                    match api.list_skill_proposals("pending") {
                        Ok(proposals) => {
                            let entries: Vec<SkillProposalEntry> = proposals.into_iter()
                                .map(|p| SkillProposalEntry {
                                    id: p.id,
                                    title: p.title,
                                    status: format!("{:?}", p.status).to_lowercase(),
                                    kind: format!("{:?}", p.proposal_kind).to_lowercase(),
                                })
                                .collect();
                            if entries.is_empty() {
                                self.store.push_toast(
                                    "No pending proposals",
                                    crate::store::types::ToastMsgVariant::Warning,
                                );
                            } else {
                                self.skill_proposal.set_proposals(entries);
                                self.skill_proposal.open();
                                self.panel = Panel::SkillProposal;
                            }
                        }
                        Err(e) => {
                            self.store.push_toast(
                                &format!("Failed to load proposals: {}", e),
                                crate::store::types::ToastMsgVariant::Error,
                            );
                        }
                    }
                }
            }
            UiActionId::OpenMcpList => {
                if let Some(ref api) = self.api {
                    match api.get_mcp_status() {
                        Ok(mcps) => {
                            let entries: Vec<McpEntry> = mcps.into_iter()
                                .map(|m| McpEntry {
                                    name: m.name,
                                    status: m.status,
                                    tools: m.tools,
                                    resources: m.resources,
                                })
                                .collect();
                            if entries.is_empty() {
                                self.store.push_toast(
                                    "No MCP servers configured",
                                    crate::store::types::ToastMsgVariant::Warning,
                                );
                            } else {
                                self.mcp_list.set_entries(entries);
                                self.mcp_list.open();
                                self.panel = Panel::McpList;
                            }
                        }
                        Err(e) => {
                            self.store.push_toast(
                                &format!("Failed to load MCP status: {}", e),
                                crate::store::types::ToastMsgVariant::Error,
                            );
                        }
                    }
                }
            }
            UiActionId::OpenRecoveryList => {
                // per-session：需 active session_id。
                if let Some(sid) = self.active_session.get_session_id() {
                    if let Some(ref api) = self.api {
                        match api.get_session_recovery(&sid) {
                            Ok(proto) => {
                                let mut entries: Vec<RecoveryEntry> = Vec::new();
                                for a in proto.actions {
                                    entries.push(RecoveryEntry {
                                        label: format!("action: {}", a.label),
                                        detail: a.description,
                                        action_kind: Some(a.kind),
                                        target_id: a.target_id,
                                    });
                                }
                                for c in proto.checkpoints {
                                    entries.push(RecoveryEntry {
                                        label: format!("checkpoint: [{}] {}", c.status, c.label),
                                        detail: c.summary.unwrap_or_else(|| c.kind),
                                        action_kind: None,
                                        target_id: None,
                                    });
                                }
                                if entries.is_empty() {
                                    self.store.push_toast(
                                        "No recovery actions or checkpoints",
                                        crate::store::types::ToastMsgVariant::Warning,
                                    );
                                } else {
                                    self.recovery_list.set_entries(entries);
                                    self.recovery_list.open();
                                    self.panel = Panel::Recovery;
                                }
                            }
                            Err(e) => {
                                self.store.push_toast(
                                    &format!("Failed to load recovery: {}", e),
                                    crate::store::types::ToastMsgVariant::Error,
                                );
                            }
                        }
                    }
                } else {
                    self.store.push_toast(
                        "Open a session first",
                        crate::store::types::ToastMsgVariant::Warning,
                    );
                }
            }
            UiActionId::ListTasks => {
                if let Some(ref api) = self.api {
                    match api.list_tasks() {
                        Ok(tasks) => {
                            let entries: Vec<TaskEntry> = tasks.into_iter()
                                .map(|t| TaskEntry {
                                    id: t.id,
                                    agent_name: t.agent_name,
                                    status: t.status,
                                    step: t.step,
                                    max_steps: t.max_steps,
                                })
                                .collect();
                            if entries.is_empty() {
                                self.store.push_toast(
                                    "No active agent tasks",
                                    crate::store::types::ToastMsgVariant::Warning,
                                );
                            } else {
                                self.task_list.set_entries(entries);
                                self.task_list.open();
                                self.panel = Panel::TaskList;
                            }
                        }
                        Err(e) => {
                            self.store.push_toast(
                                &format!("Failed to load tasks: {}", e),
                                crate::store::types::ToastMsgVariant::Error,
                            );
                        }
                    }
                }
            }
            UiActionId::OpenModeList => {
                if let Some(ref api) = self.api {
                    match api.list_execution_modes() {
                        Ok(modes) => {
                            // 映射成 ModeEntry：携 kind 而不只是 name，dispatch
                            // 处才能按 kind 分流到 agent / scheduler_profile 槽
                            // （对齐 web `App.tsx:836`）。
                            let entries: Vec<crate::dialog::ModeEntry> = modes
                                .into_iter()
                                .filter(|m| !m.hidden.unwrap_or(false))
                                .map(|m| crate::dialog::ModeEntry {
                                    kind: m.kind,
                                    id: m.id,
                                    display: m.name,
                                    description: m.description,
                                })
                                .collect();
                            if entries.is_empty() {
                                self.store.push_toast(
                                    "No execution modes available",
                                    crate::store::types::ToastMsgVariant::Warning,
                                );
                            } else {
                                self.mode_select.open_with(entries);
                                self.panel = Panel::ModeSelect;
                            }
                        }
                        Err(e) => {
                            self.store.push_toast(&format!("Failed to load modes: {}", e), crate::store::types::ToastMsgVariant::Error);
                        }
                    }
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
                    // 整会话 fork（message_id=None）；message 级 fork 需 cursor
                    // 选中消息（B 层 edit&resend），本轮不做。
                    self.fork_dialog.open(&sid, None);
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
                let Some(ref api) = self.api else { return; };
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
                // 与 web 的「点 share → toast URL」行为一致。
                if let Some(sid) = self.active_session.get_session_id() {
                    if let Some(ref api) = self.api {
                        match api.share_session(&sid) {
                            Ok(resp) => self.store.push_toast(
                                &format!("Shared: {}", resp.url),
                                crate::store::types::ToastMsgVariant::Success,
                            ),
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
                let text = self.active_session.transcript_to_text();
                match crate::dialog::clipboard::copy(&text) {
                    Ok(()) => self.store.push_toast(
                        "Transcript copied to clipboard",
                        crate::store::types::ToastMsgVariant::Success,
                    ),
                    Err(e) => self.store.push_toast(
                        &format!("Clipboard write failed: {}", e),
                        crate::store::types::ToastMsgVariant::Error,
                    ),
                }
            }
            UiActionId::CompactSession => {
                // Part 3: 暂不收 focus（需独立 dialog），先 None。
                if let Some(sid) = self.active_session.get_session_id() {
                    if let Some(ref api) = self.api {
                        match api.compact_session(&sid, None) {
                            Ok(_) => self.store.push_toast(
                                "Compaction triggered",
                                crate::store::types::ToastMsgVariant::Success,
                            ),
                            Err(e) => self.store.push_toast(
                                &format!("Compact failed: {}", e),
                                crate::store::types::ToastMsgVariant::Error,
                            ),
                        }
                    }
                }
            }
            UiActionId::ConnectProvider => {
                // TUI 路径已迁至全屏 Settings 页(`OpenSettings`),`ConnectProvider`
                // 仅 CLI 互动模式仍消费(agendao-command-runtime)。TUI 这里诚实标注:
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
            }
            UiActionId::DeleteSession => {
                if let Some(sid) = self.active_session.get_session_id() {
                    let title = self.active_session.title.get();
                    self.confirm_dialog.ask(
                        "Delete Session",
                        &format!("Delete \"{}\"? This cannot be undone.", title),
                        "Delete");
                    // 判别器携带「确认什么」——Confirm 只回 bool，靠它路由
                    // 到 delete_session（土律：单一 Confirm 变体服务所有确认）。
                    self.pending_confirm = Some(PendingConfirm::DeleteSession(sid));
                    self.panel = Panel::Confirm;
                }
            }
            UiActionId::OpenSessionList => {
                self.session_list.open();
                self.session_list.loading = true;
                self.panel = Panel::SessionList;
                let cwd = self.store.working_dir.get();
                self.session_list.set_directory_scope(cwd.clone());
                self.reload_session_list();
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
                    self.session_list.set_error("No sessions in this directory".into());
                } else {
                    self.session_list.set_sessions(entries);
                }
            }
            UiActionId::ToggleSidebar => {
                self.store.push_toast("Sidebar toggled", crate::store::types::ToastMsgVariant::Info);
            }
            UiActionId::ToggleThinking => {
                let next = !self.store.show_thinking.get();
                self.store.show_thinking.set(next);
                self.store.push_toast(
                    &format!("Thinking blocks: {}", if next { "shown" } else { "hidden" }),
                    crate::store::types::ToastMsgVariant::Info,
                );
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
            UiActionId::ToggleAppearance => {
                // 运行时主题切换：翻转 variant（ds::theme 唯一翻转入口）→
                // set_theme(theme_for) 同步 revue 渲染 → toast。阴阳同口径：
                // store.theme_variant 记账，revue ThemeManager 渲染，两者经此同步。
                let next = crate::ds::theme::toggle_variant(self.store.theme_variant.get());
                self.store.theme_variant.set(next);
                revue::style::set_theme(crate::ds::theme::theme_for(next));
                self.store.push_toast(
                    &format!("Theme: {}", crate::ds::theme::variant_label(next)),
                    crate::store::types::ToastMsgVariant::Info,
                );
            }
            // 道纪第十条：伪权威诚实标注。这些 action 的 spec 已注册（slash
            // 可触发），但 server 端无对应 API（grep 全无端点）——落到通用
            // "coming soon" 兜底会掩盖「server 根本没这个能力」的真相。这里
            // 显式分流，让用户看到权威缺口而非伪期待。server 补齐后再开路由。
            UiActionId::Undo
            | UiActionId::Redo
            | UiActionId::Timeline
            | UiActionId::NavigateParentSession
            | UiActionId::VoiceInput => {
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
            UiActionId::OpenPresetList => {
                // 道纪第十条：preset 数据就是 /mode 端点扁平化，无独立权威。
                // 诚实标注 + 复用 /mode 路径，不伪"独立 preset 列表"。
                self.store.push_toast(
                    "Presets are part of /mode — opening mode list",
                    crate::store::types::ToastMsgVariant::Info,
                );
                self.execute_slash_action(UiActionId::OpenModeList);
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
