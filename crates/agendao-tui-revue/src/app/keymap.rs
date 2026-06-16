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
use crate::dialog::{
    PermissionReply, PermissionRequest, PermissionLifetime,
    QuestionOption, QuestionRequest,
    StashEntry,
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
                            self.panel = Panel::Permission;
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
                            self.panel = Panel::Question;
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
                        // ── Permission dialog click ──
                        if matches!(self.panel, Panel::Permission) {
                            if let Some((id, reply)) = self.permission_dialog.handle_click(m.x, m.y) {
                                if let Some(ref api) = self.api {
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
                                if !self.permission_dialog.visible {
                                    self.panel = Panel::None;
                                }
                                return true;
                            }
                        }

                        // Click on transcript → toggle fold of the clicked block.
                        // Click on prompt area → focus input.
                        // Click elsewhere → unfocus.
                        if matches!(self.store.route.get(), Route::Session { .. }) {
                            // ── Sidebar scrollbar click (before the
                            // transcript fold handler, so clicking
                            // the scrollbar never falls into the
                            // transcript row-walk). ──
                            if let Some((sb_area, (content_h, viewport_h))) = self
                                .sidebar_scrollbar_area
                                .zip(self.sidebar_scrollbar_metrics)
                            {
                                let overlay = crate::widget::ScrollbarOverlay::new(
                                    (0, 0),
                                    sb_area,
                                    content_h,
                                    viewport_h,
                                    self.sidebar_scroll_offset,
                                );
                                if let Some(hit) = overlay.hit_test(m.x, m.y) {
                                    let max_offset = content_h.saturating_sub(viewport_h);
                                    match hit {
                                        crate::widget::ScrollbarHit::ArrowUp => {
                                            self.sidebar_scroll_offset = 0;
                                            self.layout_dirty = true;
                                            return true;
                                        }
                                        crate::widget::ScrollbarHit::ArrowDown => {
                                            self.sidebar_scroll_offset = max_offset;
                                            self.layout_dirty = true;
                                            return true;
                                        }
                                        crate::widget::ScrollbarHit::PageUp => {
                                            self.sidebar_scroll_offset =
                                                self.sidebar_scroll_offset.saturating_sub(viewport_h);
                                            self.layout_dirty = true;
                                            return true;
                                        }
                                        crate::widget::ScrollbarHit::PageDown => {
                                            self.sidebar_scroll_offset = (self.sidebar_scroll_offset + viewport_h).min(max_offset);
                                            self.layout_dirty = true;
                                            return true;
                                        }
                                        crate::widget::ScrollbarHit::BeginDrag(drag) => {
                                            self.sidebar_scrollbar_drag = Some(drag);
                                            return true;
                                        }
                                    }
                                }
                            }

                            let ty = m.y;
                            let transcript_y = self.transcript_area_y;
                            let transcript_h = self.transcript_viewport_h;
                            if ty >= transcript_y && ty < transcript_y + transcript_h {
                                // Click is inside transcript area.
                                // Compute which row in content space was clicked.
                                let msgs = self.active_session.messages.get();
                                let total_h: u16 = msgs.iter()
                                    .map(|b| crate::screen::layout_block(b, 0).height.saturating_add(1))
                                    .sum::<u16>()
                                    .saturating_add(1);
                                let max_offset = total_h.saturating_sub(transcript_h);
                                let user_offset = self.active_session.scroll_offset.get().min(max_offset);
                                let scroll_top = max_offset.saturating_sub(user_offset);
                                let row_in_content = ty.saturating_sub(transcript_y) + scroll_top;
                                // Walk through blocks to find which one was clicked.
                                let mut acc: u16 = 0;
                                let mut clicked_idx = None;
                                for (i, block) in msgs.iter().enumerate() {
                                    let bh = crate::screen::layout_block(block, 0).height;
                                    // Each block occupies bh rows + 1 row gap (except last)
                                    let block_end = acc + bh;
                                    if row_in_content < block_end {
                                        clicked_idx = Some(i);
                                        break;
                                    }
                                    acc = block_end + 1; // +1 for gap between blocks
                                }
                                if let Some(idx) = clicked_idx {
                                    self.active_session.toggle_fold(idx);
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
                        // Active thumb drag on the sidebar scrollbar.
                        // Sidebar offset is rows-from-top, not rows-
                        // from-bottom, so the drag translates 1:1.
                        if let (Some(drag), Some((sb_area, (content_h, viewport_h)))) = (
                            self.sidebar_scrollbar_drag,
                            self.sidebar_scrollbar_area.zip(self.sidebar_scrollbar_metrics),
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
                            self.sidebar_scroll_offset = new_top.min(max_offset);
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
                        if self.sidebar_scrollbar_drag.take().is_some() {
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
        // ── Panel/Overlay routing: each panel gets exclusive key access ──
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
                        // Confirmed action — stored as confirm_dialog's title
                    }
                    self.panel = Panel::None;
                }
                if !self.confirm_dialog.visible { self.panel = Panel::None; }
                return true;
            }
            Panel::Stash => {
                if let Some(_text) = self.stash_dialog.handle_key(key) {
                    // Restore selected stash entry to prompt
                    // (in future: set prompt text directly)
                    self.store.push_toast("Stash entry selected", crate::store::types::ToastMsgVariant::Info);
                    self.panel = Panel::None;
                    return true;
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
            Panel::Question => {
                if let Some(selected) = self.question_dialog.handle_key(key) {
                    if let Some(ref api) = self.api {
                        let id = "";
                        let answers: Vec<Vec<String>> = selected.iter().map(|&i| vec![format!("{}", i)]).collect();
                        let _ = api.reply_question(id, answers);
                    }
                    self.panel = Panel::None;
                }
                if !self.question_dialog.visible { self.panel = Panel::None; }
                return true;
            }
            Panel::Permission => {
                if let Some((id, reply)) = self.permission_dialog.handle_key(key) {
                    // Send the permission reply to the API. The dialog now
                    // returns the originating request id alongside the
                    // reply — without it the server can't match the
                    // response to a pending permission and the prompt
                    // loop blocks forever.
                    if let Some(ref api) = self.api {
                        // Server expects bare lifetime tokens
                        // (`once`/`turn`/`session`/`always`/`reject`),
                        // NOT the `allow_*` aliases. Sending `allow_once`
                        // produces "Invalid permission reply: allow_once"
                        // and aborts the prompt loop with `cancelled
                        // before model call`.
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
                    // Only collapse the panel when the dialog actually
                    // dismissed itself. Multiple permission requests can
                    // be queued — each Enter peels off one and keeps the
                    // dialog open for the next. Hard-coding `panel = None`
                    // here would dismiss the overlay even though more
                    // requests are still pending.
                    if !self.permission_dialog.visible {
                        self.panel = Panel::None;
                    }
                }
                if !self.permission_dialog.visible { self.panel = Panel::None; }
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
            Panel::Alert => {
                self.alert.handle_key(key);
                if !self.alert.visible { self.panel = Panel::None; }
                return true;
            }
            Panel::None => {}
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
            Key::Char('h') => { self.store.navigate(Route::Home); true }
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
            UiActionId::OpenModeList => {
                if let Some(ref api) = self.api {
                    match api.list_execution_modes() {
                        Ok(modes) => {
                            let names: Vec<&str> = modes.iter().map(|m| m.name.as_str()).collect();
                            self.store.push_toast(
                                &format!("Modes: {}", names.join(", ")),
                                crate::store::types::ToastMsgVariant::Info,
                            );
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
            UiActionId::OpenThemeList => {
                self.store.push_toast("Theme list coming soon", crate::store::types::ToastMsgVariant::Info);
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
            // Pull the user's current selections from the store so the
            // backend uses the model/agent picked in the dialog instead of
            // the workspace default. `selected_mode` is execution mode
            // (build/plan), NOT a scheduler profile — passing it into the
            // profile slot makes the server reject the request as "profile
            // could not be resolved: build". Leave profile as None until
            // we wire up actual scheduler profile UI.
            let model = self.store.selected_model.get();
            let agent = self.store.selected_agent.get();
            match api.send_prompt_with(&sid, text, agent, None, model, None) {
                Ok(r) => {
                    // The actual response arrives via FrontendEvent stream
                    if r.status == "queued" || r.status == "awaiting_user" {
                        self.active_session.run_status.set(RunStatus::Running);
                    } else {
                        // Sent synchronously; status will be updated by events
                    }
                    // 标记：一轮结束后（Idle）刷新 title——服务端可能已用 LLM
                    // 生成新 title（ensure_default_session_title），无事件回流。
                    self.title_refresh_pending = true;
                }
                Err(e) => {
                    self.active_session.push_notice(
                        &format!("err-{}", ts_now()),
                        &format!("Failed to send: {}", e),
                    );
                    self.active_session.run_status.set(RunStatus::Error(format!("{}", e)));
                }
            }
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
