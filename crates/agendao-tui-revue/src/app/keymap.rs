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
use crate::store::types::RunStatus;
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
                            // Session tree 行 → open_session(命中 y 来自 render 发布的 nav_hits)
                            if let Some(sid) = self
                                .sidebar_nav_hits
                                .iter()
                                .find(|hit| hit.y == m.y)
                                .map(|hit| hit.session_id.clone())
                            {
                                self.open_session(&sid);
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
                                let total_h = crate::screen::transcript_total_height(&msgs, self.store.show_thinking.get(), self.store.compact_density.get());
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
                                let units = crate::screen::build_render_units(&msgs, None, 0, self.store.show_thinking.get(), None);
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
        if matches!(self.store.route.get(), Route::Settings) {
            if self.handle_settings_key(key) {
                return true;
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
        self.sf_tx.send_replace(Some(session_id.to_string()));
        self.load_session_messages(session_id);
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
    pub(crate) fn refresh_sidebar_session_tree(&mut self) {
        let sessions = self.store.session_list.get();
        let cwd = self.store.working_dir.get();
        let nodes = crate::telemetry::build_session_nav_tree(&sessions, &cwd);
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
        use agendao_command::UiActionId;
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
                let action = match row {
                    GeneralRow::ShowThinking => UiActionId::ToggleThinking,
                    GeneralRow::ShowScrollbar => UiActionId::ToggleScrollbar,
                    GeneralRow::ShowHeader => UiActionId::ToggleHeader,
                    GeneralRow::ShowTips => UiActionId::ToggleTips,
                    GeneralRow::CompactDensity => UiActionId::ToggleDensity,
                    GeneralRow::Theme => UiActionId::ToggleAppearance,
                };
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

    /// Settings 全屏页键路由(火→土:键事件 → AppStore signals)。
    ///
    /// 阳面键 = 阴面写哪个 signal,完全镜像 SettingsFocusPane 三栏:
    ///   - `Tab`        → settings_focus_pane.next()(Categories→Providers→Details→…)
    ///   - `↑/↓`        → 按 focused 栏分别滚 settings_category / settings_selected_provider
    ///                    (Details 栏暂无 selection 概念,不响应;Model 行编辑是 Part 7+ 的非目标)
    ///   - `Enter`      → Categories 栏:灰显项 toast"Coming soon";Providers 栏:no-op
    ///                    (selected 已是 ↑/↓ 实时所选,Enter 只是 sticky 确认语义)
    ///
    /// 返回 true = 消费,false = 让位 prompt/全局键(目前 false 仅当 key 非 Tab/↑/↓/Enter)。
    pub(crate) fn handle_settings_key(&mut self, key: &Key) -> bool {
        use crate::store::types::{SettingsCategory, SettingsFocusPane, ToastMsgVariant};
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

        // ── 非 ModelSettings 分类的 body 路由(木律·唯一输入权威)──
        // 这些分类只有两区(Categories | body);focus 落在 body(非 Categories)时,
        // 除 Tab/Esc 外的键交给分类专属 body 处理器。ModelSettings 保持三栏原逻辑。
        if category != SettingsCategory::ModelSettings
            && focus != SettingsFocusPane::Categories
            && !matches!(key, Key::Tab | Key::Escape)
        {
            return match category {
                SettingsCategory::General => self.handle_general_body_key(key),
                SettingsCategory::Keybindings => self.handle_keybindings_body_key(key),
                // About / 占位分类:body 无交互,消费导航键避免穿透到 provider 逻辑;
                // Esc 已在上面排除,继续冒泡给外层 → navigate_home。
                _ => matches!(key, Key::Up | Key::Down | Key::Enter | Key::Char(' ')),
            };
        }

        match key {
            Key::Tab => {
                let cur = self.store.settings_focus_pane.get();
                // ModelSettings 三栏循环;其余分类两区循环(Categories ⇄ Details)。
                let next = if category == SettingsCategory::ModelSettings {
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
                        let providers = self.store.providers.get();
                        if providers.is_empty() { return true; }
                        let cur = self.store.settings_selected_provider.get();
                        let idx = cur.as_ref()
                            .and_then(|id| providers.iter().position(|p| &p.id == id))
                            .unwrap_or(0);
                        let n = providers.len() as i32;
                        let nxt = (((idx as i32 + dir) % n) + n) % n;
                        let new_id = providers[nxt as usize].id.clone();
                        // provider 切换 → 清 selected_model:model_key 是 provider-scoped,
                        // 同名 key 在不同 provider 是不同记录,跨 provider 残留会让
                        // Details 渲染指向不存在条目(土律·第四条 单点权威 + 第九条·配对销毁)。
                        if cur.as_deref() != Some(new_id.as_str()) {
                            self.store.settings_selected_model.set(None);
                        }
                        self.store.settings_selected_provider.set(Some(new_id));
                    }
                    SettingsFocusPane::Details => {
                        // Details ↑/↓ 切当前 provider 的 models 列表;空列表/无 provider 选中 → 不动。
                        // 首次进入(selected_model = None)按方向键 → 自动落到 models[0]
                        // (lazy auto-select,符合道纪·第十条·诚实状态:None 表示 "尚未浏览",
                        // 第一次方向键即开始浏览)。
                        let providers = self.store.providers.get();
                        let sel_provider_id = self.store.settings_selected_provider.get();
                        let Some(provider) = sel_provider_id
                            .as_ref()
                            .and_then(|id| providers.iter().find(|p| &p.id == id))
                        else { return true; };
                        if provider.models.is_empty() { return true; }
                        let model_keys: Vec<&str> =
                            provider.models.iter().map(|m| m.id.as_str()).collect();
                        let cur = self.store.settings_selected_model.get();
                        let new_key = match cur.as_deref().and_then(|k| model_keys.iter().position(|x| *x == k)) {
                            None => model_keys[0].to_string(),
                            Some(idx) => {
                                let n = model_keys.len() as i32;
                                let nxt = (((idx as i32 + dir) % n) + n) % n;
                                model_keys[nxt as usize].to_string()
                            }
                        };
                        self.store.settings_selected_model.set(Some(new_key));
                    }
                }
                self.layout_dirty = true;
                true
            }
            Key::Enter => {
                let focus = self.store.settings_focus_pane.get();
                if matches!(focus, SettingsFocusPane::Categories) {
                    let cat = self.store.settings_category.get();
                    if cat.is_implemented() {
                        // 潜入 body:ModelSettings → Providers 栏;其余 → Details(单区 body)。
                        let body = if cat == SettingsCategory::ModelSettings {
                            SettingsFocusPane::Providers
                        } else {
                            SettingsFocusPane::Details
                        };
                        self.store.settings_focus_pane.set(body);
                        self.layout_dirty = true;
                    } else {
                        self.store.push_toast(
                            &format!("{} — coming soon", cat.label()),
                            ToastMsgVariant::Info,
                        );
                    }
                }
                true
            }
            // Provider CRUD(木律·唯一入口):Providers focused 时 a/e/d。
            // a/e:in-place 编辑,焦点切到 Details 让用户原地填字段(金律·唯一成形权威 —
            //      Details pane 同时是只读 view 与编辑 form,无 modal dialog 第二窗口)。
            // d:Confirm 二次确认后删(走 ConfirmDialog,逻辑不变)。
            // 道纪·第九条·配对销毁:enter_* 配 close(submit/cancel/Tab 切离三路),
            // api_key Input 在 close 中 clear()(明文不驻留)。
            Key::Char('a') => {
                if matches!(self.store.settings_focus_pane.get(), SettingsFocusPane::Providers) {
                    self.settings_edit.enter_add();
                    // Add 草稿对应的虚拟"(new provider)"行在 Providers pane 末尾,
                    // selected_provider 留 None 即可(Details 完全从 settings_edit 读字段)。
                    self.store.settings_focus_pane.set(SettingsFocusPane::Details);
                    self.layout_dirty = true;
                    true
                } else {
                    false
                }
            }
            // `m` 仅在 Details focused 时响应:新增当前 provider 的 model。
            // 设计上 `a`/`m` 分离避免双义:`a` 唯一对 provider,`m` 唯一对 model,
            // 焦点切换时 hint 也直接对应(金律·成形权威)。
            Key::Char('m') => {
                if !matches!(self.store.settings_focus_pane.get(), SettingsFocusPane::Details) {
                    return false;
                }
                let sel_provider_id = self.store.settings_selected_provider.get();
                let Some(provider_id) = sel_provider_id else { return true; };
                self.model_edit_dialog.open_add(&provider_id);
                self.panel = crate::app::Panel::ModelEdit;
                true
            }
            Key::Char('e') => {
                let focus = self.store.settings_focus_pane.get();
                match focus {
                    SettingsFocusPane::Providers => {
                        let providers = self.store.providers.get();
                        let sel_id = self.store.settings_selected_provider.get();
                        let Some(sel) = sel_id
                            .as_ref()
                            .and_then(|id| providers.iter().find(|p| &p.id == id))
                        else {
                            return true;
                        };
                        // in-place 编辑:把字段交给 Details pane,焦点切 Details。
                        self.settings_edit.enter_edit(sel);
                        self.store.settings_focus_pane.set(SettingsFocusPane::Details);
                        self.layout_dirty = true;
                        true
                    }
                    SettingsFocusPane::Details => {
                        // Edit model:先 GET raw ModelConfig 做 prefill(土律·第十条),
                        // 否则 PUT 半空覆写会丢 cost/reasoning/temperature 等高级字段。
                        let providers = self.store.providers.get();
                        let sel_provider_id = self.store.settings_selected_provider.get();
                        let sel_model_key = self.store.settings_selected_model.get();
                        let (Some(provider_id), Some(model_key)) =
                            (sel_provider_id, sel_model_key)
                        else { return true; };
                        let Some(provider) =
                            providers.iter().find(|p| p.id == provider_id)
                        else { return true; };
                        let Some(model_info) =
                            provider.models.iter().find(|m| m.id == model_key)
                        else { return true; };
                        // 先尝试 GET raw config(可能不存在 = 配置文件未声明此 model,
                        // 仅插件目录有 = 半空 fallback,诚实告知)。
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
                        // 存入 raw ModelConfig 全量副本(土律·第十条·完整 prefill):
                        // 补 max_output 输入框预填,且 submit 时作为 PUT 合并基底,
                        // 保住 cost/reasoning/temperature 等 form 不暴露的字段。
                        if let Some(cfg) = prefill {
                            self.model_edit_dialog.set_prefill(cfg);
                        }
                        self.panel = crate::app::Panel::ModelEdit;
                        true
                    }
                    SettingsFocusPane::Categories => false,
                }
            }
            Key::Char('d') => {
                let focus = self.store.settings_focus_pane.get();
                match focus {
                    SettingsFocusPane::Providers => {
                        let providers = self.store.providers.get();
                        let sel_id = self.store.settings_selected_provider.get();
                        let Some(sel) = sel_id
                            .as_ref()
                            .and_then(|id| providers.iter().find(|p| &p.id == id))
                        else {
                            return true;
                        };
                        self.confirm_dialog.ask(
                            "Delete Provider",
                            &format!(
                                "Delete provider \"{}\"? This removes the config entry and stored API key.",
                                sel.name,
                            ),
                            "Delete",
                        );
                        self.pending_confirm = Some(
                            crate::app::PendingConfirm::DeleteProvider(sel.id.clone()),
                        );
                        self.panel = crate::app::Panel::Confirm;
                        true
                    }
                    SettingsFocusPane::Details => {
                        let providers = self.store.providers.get();
                        let sel_provider_id = self.store.settings_selected_provider.get();
                        let sel_model_key = self.store.settings_selected_model.get();
                        let (Some(provider_id), Some(model_key)) =
                            (sel_provider_id, sel_model_key)
                        else { return true; };
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
                        self.pending_confirm = Some(
                            crate::app::PendingConfirm::DeleteProviderModel {
                                provider_id,
                                model_key,
                            },
                        );
                        self.panel = crate::app::Panel::Confirm;
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
