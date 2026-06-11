use super::*;
use agendao_server_core::frontend_events::FrontendEvent;
use tokio::sync::watch;

impl App {
    pub(super) fn active_session_status(&self) -> Option<SessionStatus> {
        let session_id = self.current_session_id()?;
        let session_ctx = self.context.session.read();
        Some(session_ctx.status(&session_id).clone())
    }

    pub(super) fn local_direct_idle_session(&self) -> bool {
        self.local_direct
            && matches!(
                self.active_session_status(),
                Some(SessionStatus::Idle | SessionStatus::WaitingOnUser)
            )
    }

    fn permission_interaction_active(&self) -> bool {
        !self.permission_runtime.pending_ids.is_empty() || self.permission_prompt.is_open
    }

    pub(super) fn permission_sync_interval(&self) -> Duration {
        if self.permission_interaction_active() {
            Duration::from_secs(PERMISSION_SYNC_BACKOFF_SECS)
        } else {
            Duration::from_secs(PERMISSION_SYNC_FALLBACK_SECS)
        }
    }

    pub(super) fn aux_sync_interval(&self) -> Duration {
        if self.permission_interaction_active() {
            Duration::from_secs(AUX_SYNC_BACKOFF_SECS)
        } else {
            Duration::from_secs(AUX_SYNC_INTERVAL_SECS)
        }
    }

    pub(super) fn should_schedule_aux_sync(&self) -> bool {
        self.session_list_dialog.is_open()
            || self.skill_list_dialog.is_open()
            || !self.local_direct
    }

    pub(super) fn should_schedule_perf_log(&self) -> bool {
        self.diagnostics.perf_log_info
    }

    pub(super) fn session_sidebar_visible(&self) -> bool {
        self.context
            .session_view_handle()
            .map(|sv| sv.sidebar_visible(self.terminal_width()))
            .unwrap_or(false)
    }

    pub(super) fn toggle_session_sidebar(&mut self) {
        if let Some(sv) = self.context.session_view_handle() {
            sv.toggle_sidebar(self.terminal_width());
        }
        if matches!(self.context.current_route(), Route::Session { .. }) && self.session_sidebar_visible()
        {
            self.queue_process_refresh();
        }
    }

    pub(super) fn maybe_log_perf_snapshot(&mut self) {
        if self.sync_runtime.last_perf_log.elapsed() < Duration::from_secs(PERF_LOG_INTERVAL_SECS) {
            return;
        }
        self.sync_runtime.last_perf_log = Instant::now();
        let ui_bridge = self.context.ui_bridge_snapshot();
        if self.diagnostics.perf_log_info {
            tracing::info!(
                draws = self.diagnostics.perf.draws,
                screen_snapshots = self.diagnostics.perf.screen_snapshots,
                session_sync_full = self.diagnostics.perf.session_sync_full,
                session_sync_incremental = self.diagnostics.perf.session_sync_incremental,
                question_sync = self.diagnostics.perf.question_sync,
                session_updated_events = self.diagnostics.perf.session_updated_events,
                ui_bridge_pending = ui_bridge.pending_events,
                ui_bridge_high_water = ui_bridge.high_water_mark,
                ui_bridge_coalesced = ui_bridge.coalesced_events,
                ui_bridge_dropped = ui_bridge.dropped_events,
                ui_bridge_capacity = ui_bridge.capacity,
                "tui perf snapshot"
            );
        } else {
            tracing::debug!(
                draws = self.diagnostics.perf.draws,
                screen_snapshots = self.diagnostics.perf.screen_snapshots,
                session_sync_full = self.diagnostics.perf.session_sync_full,
                session_sync_incremental = self.diagnostics.perf.session_sync_incremental,
                question_sync = self.diagnostics.perf.question_sync,
                session_updated_events = self.diagnostics.perf.session_updated_events,
                ui_bridge_pending = ui_bridge.pending_events,
                ui_bridge_high_water = ui_bridge.high_water_mark,
                ui_bridge_coalesced = ui_bridge.coalesced_events,
                ui_bridge_dropped = ui_bridge.dropped_events,
                ui_bridge_capacity = ui_bridge.capacity,
                "tui perf snapshot"
            );
        }
    }

    pub(crate) fn next_tick_deadline(&self, now: Instant) -> Option<Instant> {
        let mut deadline = None;

        let mut schedule_at = |candidate: Instant| {
            let candidate = candidate.max(now);
            match deadline {
            Some(current) if current <= candidate => {}
            _ => deadline = Some(candidate),
            }
        };

        let mut schedule_after_last_tick = |delta: Duration| {
            schedule_at(self.sync_runtime.last_tick_at + delta);
        };

        if let Some(delta) = self.toast.next_tick_after() {
            schedule_after_last_tick(delta);
        }
        if let Some(delta) = self
            .prompt
            .next_tick_after(now, self.sync_runtime.last_tick_at)
        {
            schedule_at(now + delta);
        }
        if let Some(delta) = self
            .context
            .session_view_handle()
            .and_then(|sv| sv.next_tooltip_tick_after())
        {
            schedule_at(now + delta);
        }

        if self.sync_runtime.pending_initial_submit && !self.prompt.get_input().trim().is_empty() {
            schedule_at(now);
        }

        let route = self.context.current_route();
        if let Route::Session { session_id } = &route {
            if self.sync_runtime.pending_session_sync.as_deref() == Some(session_id.as_str()) {
                if let Some(due_at) = self.sync_runtime.pending_session_sync_due_at {
                    schedule_at(due_at);
                }
            }
            if let Some(due_at) = self.sync_runtime.pending_process_refresh_due_at {
                schedule_at(due_at);
            } else if self.session_sidebar_visible() && !self.local_direct_idle_session() {
                schedule_at(self.sync_runtime.last_process_refresh + Duration::from_secs(2));
            }

            if !self.local_direct && !self.local_direct_idle_session() {
                schedule_at(
                    self.sync_runtime.last_full_session_sync
                        + Duration::from_secs(SESSION_FULL_SYNC_INTERVAL_SECS),
                );
            }
        }

        let has_active_session = matches!(route, Route::Session { .. });
        if has_active_session {
            if let Some(due_at) = self.sync_runtime.pending_question_sync_due_at {
                schedule_at(due_at);
            } else if !self.local_direct && !self.local_direct_idle_session() {
                schedule_at(
                    self.sync_runtime.last_question_sync
                        + Duration::from_secs(QUESTION_SYNC_FALLBACK_SECS),
                );
            }
            if let Some(due_at) = self.sync_runtime.pending_permission_sync_due_at {
                schedule_at(due_at);
            } else if !self.local_direct && !self.local_direct_idle_session() {
                schedule_at(
                    self.sync_runtime.last_permission_sync + self.permission_sync_interval(),
                );
            }
        }
        if self.should_schedule_aux_sync() {
            schedule_at(self.sync_runtime.last_aux_sync + self.aux_sync_interval());
        }
        if self.should_schedule_perf_log() {
            schedule_at(
                self.sync_runtime.last_perf_log + Duration::from_secs(PERF_LOG_INTERVAL_SECS),
            );
        }

        deadline
    }

    pub(super) fn sync_ui_bridge_health(&mut self) -> bool {
        let ui_bridge = self.context.ui_bridge_snapshot();
        let previous_dropped = self.sync_runtime.last_ui_bridge_dropped_events;
        self.sync_runtime.last_ui_bridge_dropped_events = ui_bridge.dropped_events;
        if ui_bridge.dropped_events <= previous_dropped {
            return false;
        }

        let dropped_delta = ui_bridge.dropped_events.saturating_sub(previous_dropped);
        self.toast.show(
            ToastVariant::Warning,
            &format!(
                "TUI event stream lagged; dropped {} queued update{}. Open /runtime for queue stats.",
                dropped_delta,
                if dropped_delta == 1 { "" } else { "s" }
            ),
            4200,
        );
        true
    }
}

pub(super) fn spawn_tui_direct_event_bridge(
    local_server: Option<Arc<LocalServerState>>,
    session_filter: SessionFilter,
    ui_bridge: crate::bridge::UiBridge,
) -> Option<tokio::task::JoinHandle<()>> {
    let state = local_server?;
    Some(tokio::spawn(async move {
        let mut session_filter_rx = session_filter.subscribe();
        loop {
            let session_id = current_session_filter(&session_filter_rx).unwrap_or_default();
            if session_id.is_empty() {
                if session_filter_rx.changed().await.is_err() {
                    break;
                }
                continue;
            }
            let sid = session_id.clone();
            let cancel = tokio_util::sync::CancellationToken::new();
            let mut rx = spawn_direct_event_loop(Arc::clone(&state), session_id, cancel.clone());
            loop {
                tokio::select! {
                    event = rx.recv() => {
                        match event {
                            Some(direct) => {
                                let _ = ui_bridge.emit(Event::Custom(Box::new(
                                    CustomEvent::FrontendEvent(Box::new(direct)),
                                )));
                            }
                            None => break,
                        }
                    }
                    changed = session_filter_rx.changed() => match changed {
                        Ok(()) => {
                            if current_session_filter(&session_filter_rx).as_deref() != Some(sid.as_str()) {
                                cancel.cancel();
                                break;
                            }
                        }
                        Err(_) => {
                            cancel.cancel();
                            return;
                        }
                    }
                }
            }
        }
    }))
}

pub(super) async fn socket_event_subscriber(
    socket_path: String,
    session_filter: SessionFilter,
    ui_bridge: crate::bridge::UiBridge,
) {
    let transport = agendao_client::transport::UnixSocketTransport::new(socket_path);
    let mut session_filter_rx = session_filter.subscribe();
    loop {
        let session_id = current_session_filter(&session_filter_rx).unwrap_or_default();
        if session_id.is_empty() {
            if session_filter_rx.changed().await.is_err() {
                break;
            }
            continue;
        }
        let Ok(mut json_rx) = transport.subscribe_events(&session_id).await else {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            continue;
        };
        'inner: loop {
            tokio::select! {
                event = json_rx.recv() => {
                    match event {
                        Some(json) => {
                            if let Ok(frontend) = serde_json::from_value::<FrontendEvent>(json) {
                                let _ = ui_bridge.emit(Event::Custom(Box::new(
                                    CustomEvent::FrontendEvent(Box::new(frontend)),
                                )));
                            }
                        }
                        None => break 'inner,
                    }
                }
                changed = session_filter_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    if current_session_filter(&session_filter_rx).as_deref() != Some(session_id.as_str()) {
                        break 'inner;
                    }
                }
            }
        }
    }
}

fn current_session_filter(session_filter_rx: &watch::Receiver<Option<String>>) -> Option<String> {
    session_filter_rx.borrow().clone()
}
