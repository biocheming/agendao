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

use crate::app::{AppHandler, Panel, PendingConfirm};
use crate::app::dispatch_outcome;
use crate::dialog::{
    PermissionRequest, PermissionLifetime,
    QuestionOption, QuestionRequest,
    StashEntry,
    SkillEntry, SkillProposalEntry, McpEntry, RecoveryEntry, TaskEntry,
};
use crate::input::{PromptAction, SlashPopup};
use crate::store::app_store::Route;
use crate::store::types::{RunStatus, ToolPhase};
use crate::telemetry::event_handler::apply_frontend_event;

impl AppHandler {
    pub(crate) fn handle(&mut self, event: &Event) -> bool {
        match event {
            Event::Tick => {
                // Reset interrupt confirmation after 5s timeout
                if self.interrupt_pending {
                    if self.interrupt_time.elapsed().as_secs() > 5 {
                        self.interrupt_pending = false;
                    }
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
                            }
                            // status 其他值：等服务端 FrontendEvent 经 event_bus
                            // 驱动状态机，此处不抢。
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

                changed || matches!(self.active_session.run_status.get(), RunStatus::Running | RunStatus::Sending) || self.interrupt_pending
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
                self.handle_key(&key.key)
            }
            Event::Mouse(m) => {
                use revue::event::{MouseEventKind, MouseButton};
                match m.kind {
                    MouseEventKind::ScrollUp => {
                        self.active_session.scroll_up();
                        true
                    }
                    MouseEventKind::ScrollDown => {
                        self.active_session.scroll_down();
                        true
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

                        // Click on transcript → toggle fold of the clicked block.
                        // Click on prompt area → focus input.
                        // Click elsewhere → unfocus.
                        if matches!(self.store.route.get(), Route::Session { .. }) {
                            // ── Sidebar tab 点击切换（替代旧 scrollbar 命中）。
                            // 符号行 / 下划线行（y == tab_y 或 tab_y+1）点击 → active = m.x / 4
                            // （每 tab = `| 符号 ` 4 列，符号 i 在列 4i+2）。点击 active 不重渲。──
                            if self.sidebar_visible && m.x < crate::app::SIDEBAR_WIDTH
                                && (m.y == self.sidebar_tab_y || m.y == self.sidebar_tab_y + 1)
                            {
                                let new_tab = ((m.x / 4) as usize)
                                    .min(crate::telemetry::sidebar::SIDEBAR_TAB_COUNT - 1);
                                if new_tab != self.sidebar_active_tab {
                                    self.sidebar_active_tab = new_tab;
                                    self.layout_dirty = true;
                                    return true;
                                }
                            }

                            // ── Session header dir 点击：toggle 全路径 tooltip（click-to-reveal，无 motion tracking）。
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
                                let total_h = crate::screen::transcript_total_height(&msgs, self.store.show_thinking.get(), self.store.compact_density.get());
                                let max_offset = total_h.saturating_sub(transcript_h);
                                let user_offset = self.active_session.scroll_offset.get().min(max_offset);
                                let scroll_top = max_offset.saturating_sub(user_offset);
                                let row_in_content = ty.saturating_sub(transcript_y) + scroll_top;
                                // 视觉单元遍历（与渲染/total_h 同源）：unit.height 量高，
                                // 命中时 row_owners[rel_row] 把屏幕 y 映射到块——整行命中，
                                // 装饰行 None 不 toggle。聚合/单块统一，不认块类型（金律：
                                // 命中触点 1，新增聚合种类零改动）。
                                let units = crate::screen::build_render_units(&msgs, None, 0, self.store.show_thinking.get());
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
        // swallowed by PromptInput's catch-all `_ => self.input.handle_key(key)`
        // arm (prompt_input.rs:145). Without this re-order, Space is
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
                _ => {}
            }
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
                // 2. Double-tap Esc to interrupt running session
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
            c.slash.as_ref().map_or(false, |s| s.name.starts_with(trimmed) || s.aliases.iter().any(|a| a.starts_with(trimmed)))
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
        self.alert.dismiss();
        self.panel = Panel::None;
    }

    /// 切到 fork 后的新会话：reset + set_session_id + sf_tx + load + navigate。
    /// /revise（message 级 fork）调完后再 set_text 回填；/fork（整会话 fork）不回填。
    pub(crate) fn switch_to_forked_session(&mut self, info: &agendao_client::SessionInfo) {
        self.active_session.reset_for_new_session();
        self.active_session.set_session_id(&info.id);
        self.sf_tx.send_replace(Some(info.id.clone()));
        self.load_session_messages(&info.id);
        self.store.navigate(Route::Session { session_id: info.id.clone() });
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
                self.provider_dialog.open();
                self.panel = Panel::Provider;
                // 枚举 providers：复用启动期 get_all_providers 口径，映射为
                // {id, name, connected}（connected 由 resp.connected 集合判定）。
                if let Some(ref api) = self.api {
                    match api.get_all_providers() {
                        Ok(resp) => {
                            let connected: std::collections::HashSet<String> =
                                resp.connected.iter().cloned().collect();
                            let infos: Vec<crate::dialog::ProviderInfoDlg> = resp.all.into_iter()
                                .map(|p| crate::dialog::ProviderInfoDlg {
                                    id: p.id.clone(),
                                    name: p.name.clone(),
                                    connected: connected.contains(&p.id),
                                })
                                .collect();
                            self.provider_dialog.set_providers(infos);
                        }
                        Err(e) => self.store.push_toast(
                            &format!("Failed to load providers: {}", e),
                            crate::store::types::ToastMsgVariant::Error),
                    }
                }
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
                // Scope to cwd: working_dir has been canonicalized upstream
                // (workspace_key == fs::canonicalize), matching the same
                // normalization used when sessions were created. So an exact
                // string equality on the server side is safe.
                let cwd = self.store.working_dir.get();
                let cwd_filter = if cwd.is_empty() { None } else { Some(cwd.clone()) };
                self.session_list.set_directory_scope(cwd.clone());
                if let Some(ref api) = self.api {
                    match api.list_sessions_in_directory(cwd_filter) {
                        Ok(sessions) => {
                            let entries: Vec<crate::dialog::SessionEntry> = sessions.into_iter().map(|s| {
                                crate::dialog::SessionEntry {
                                    id: s.id,
                                    title: s.title,
                                    status_hint: String::new(),
                                }
                            }).collect();
                            self.session_list.set_sessions(entries);
                        }
                        Err(e) => {
                            self.session_list.set_error(format!("{}", e));
                        }
                    }
                } else {
                    self.session_list.set_error("No API connection".into());
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
                // 紧凑模式：块间 0 间隔。transcript_total_height 同口径 gap=0，
                // 渲染端跳过 child_sized("",1)——阴阳同口径（金律）。
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
                            info.id
                        }
                        Err(e) => { self.active_session.run_status.set(RunStatus::Error(format!("{}", e))); return; }
                    }
                } else { "echo".to_string() }
            }
            Route::Session { session_id } => session_id,
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

    pub(crate) fn dispatch_shell(&mut self, _cmd: String) {}
    pub(crate) fn toggle_help(&mut self) {
        if self.help.visible { self.help.dismiss(); self.panel = Panel::None; }
        else { self.help.toggle(); self.panel = Panel::Help; }
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
            for msg in msgs {
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
            active_session.run_status.set(RunStatus::Idle);
        }
        Err(e) => {
            tracing::warn!(%session_id, %e, "failed to load session messages");
        }
    }
}
