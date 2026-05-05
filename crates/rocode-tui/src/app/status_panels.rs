use super::*;

impl App {
    pub(super) fn open_overview_status_dialog(&mut self) {
        self.context
            .set_status_dialog_view(StatusDialogView::Overview);
        self.refresh_active_status_dialog();
        self.open_status_dialog_modal();
    }

    pub(super) fn open_runtime_status_dialog(&mut self) -> bool {
        if self.render_runtime_status_dialog() {
            self.context
                .set_status_dialog_view(StatusDialogView::Runtime);
            self.open_status_dialog_modal();
            true
        } else {
            false
        }
    }

    pub(super) fn open_usage_status_dialog(&mut self) -> bool {
        if self.render_usage_status_dialog() {
            self.context.set_status_dialog_view(StatusDialogView::Usage);
            self.open_status_dialog_modal();
            true
        } else {
            false
        }
    }

    pub(super) fn open_insights_status_dialog(&mut self) -> bool {
        if self.render_insights_status_dialog() {
            self.context
                .set_status_dialog_view(StatusDialogView::Insights);
            self.open_status_dialog_modal();
            true
        } else {
            false
        }
    }

    pub(super) fn open_config_validation_status_dialog(&mut self) -> bool {
        let Some(client) = self.context.get_api_client() else {
            self.toast.show(
                ToastVariant::Error,
                "API unavailable for /config/validation.",
                2400,
            );
            return false;
        };

        match client.get_config_validation() {
            Ok(snapshot) => {
                self.context
                    .set_status_dialog_view(StatusDialogView::ConfigValidation);
                self.status_dialog.set_title("Config Validation");
                self.status_dialog.set_footer_hint(Some(
                    "Esc close · workspace-scoped read model from /config/validation".to_string(),
                ));
                self.status_dialog
                    .set_status_lines(tui_config_validation_lines(&snapshot));
                self.open_status_dialog_modal();
                true
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to load config validation snapshot: {}", error),
                    3000,
                );
                false
            }
        }
    }

    pub(super) fn open_events_status_dialog(&mut self, raw_filter: Option<&str>) -> bool {
        let Some(session_id) = self.current_session_id() else {
            self.toast.show(
                ToastVariant::Warning,
                "No active session available for /events.",
                2400,
            );
            return false;
        };
        let Some(client) = self.context.get_api_client() else {
            self.toast
                .show(ToastVariant::Error, "API unavailable for /events.", 2400);
            return false;
        };

        let command = rocode_command::interactive::parse_events_browser_command(raw_filter);
        let remembered = match self.context.status_dialog_view() {
            StatusDialogView::Events(state) if state.session_id == session_id => {
                Some(state.clone())
            }
            _ => None,
        };

        let (filter, offset, preserve_previous_state, empty_page_message) = match command {
            rocode_command::interactive::InteractiveEventsCommand::ShowCurrent => {
                if let Some(state) = remembered.as_ref() {
                    (state.filter.clone(), state.offset, false, None)
                } else {
                    (rocode_command::interactive::default_events_browser_query(), 0, false, None)
                }
            }
            rocode_command::interactive::InteractiveEventsCommand::ShowFiltered {
                filter,
                page,
            } => (
                filter.clone(),
                rocode_command::interactive::events_browser_offset_for_page(&filter, page),
                false,
                (page > 1).then(|| {
                    format!(
                        "Requested page {} has no events for the current filter. Use /events first, prev, or reduce page.",
                        page
                    )
                }),
            ),
            rocode_command::interactive::InteractiveEventsCommand::JumpPage(page) => {
                let filter = remembered
                    .as_ref()
                    .map(|state| state.filter.clone())
                    .unwrap_or_else(rocode_command::interactive::default_events_browser_query);
                (
                    filter.clone(),
                    rocode_command::interactive::events_browser_offset_for_page(&filter, page),
                    false,
                    (page > 1).then(|| {
                        format!(
                            "Requested page {} has no events for the current filter. Use /events first, prev, or change filters.",
                            page
                        )
                    }),
                )
            }
            rocode_command::interactive::InteractiveEventsCommand::NextPage => {
                if let Some(state) = remembered.as_ref() {
                    (
                        state.filter.clone(),
                        state.offset.saturating_add(
                            rocode_command::interactive::events_browser_page_size(&state.filter),
                        ),
                        true,
                        None,
                    )
                } else {
                    (rocode_command::interactive::default_events_browser_query(), 0, false, None)
                }
            }
            rocode_command::interactive::InteractiveEventsCommand::PreviousPage => {
                if let Some(state) = remembered.as_ref() {
                    let step =
                        rocode_command::interactive::events_browser_page_size(&state.filter);
                    (
                        state.filter.clone(),
                        state.offset.saturating_sub(step),
                        false,
                        None,
                    )
                } else {
                    (rocode_command::interactive::default_events_browser_query(), 0, false, None)
                }
            }
            rocode_command::interactive::InteractiveEventsCommand::FirstPage => {
                if let Some(state) = remembered.as_ref() {
                    (state.filter.clone(), 0, false, None)
                } else {
                    (rocode_command::interactive::default_events_browser_query(), 0, false, None)
                }
            }
            rocode_command::interactive::InteractiveEventsCommand::Clear => {
                (rocode_command::interactive::default_events_browser_query(), 0, false, None)
            }
        };

        let query = tui_events_query(&filter, offset);
        match client.get_session_events(&session_id, &query) {
            Ok(events) => {
                if events.is_empty() && offset > 0 {
                    self.toast.show(
                        ToastVariant::Warning,
                        &empty_page_message.unwrap_or_else(|| {
                            if preserve_previous_state {
                                "No more events for the current filter. Use prev or change filters."
                                    .to_string()
                            } else {
                                "That event page is empty for the current filter. Use first, prev, or adjust filters."
                                    .to_string()
                            }
                        }),
                        2800,
                    );
                    return false;
                }

                let page_size = rocode_command::interactive::events_browser_page_size(&filter);
                let page_index =
                    rocode_command::interactive::events_browser_page_for_offset(&filter, offset);
                let can_go_prev = offset > 0;
                let can_go_next = events.len() >= page_size;
                let mut lines = vec![
                    StatusLine::title("Session Events"),
                    StatusLine::normal(format!("Session: {}", session_id)),
                    StatusLine::muted(format!(
                        "Page {} · {} · {}",
                        page_index,
                        tui_events_window_label(offset, events.len()),
                        tui_events_filter_label(&filter)
                    )),
                ];
                lines.extend(tui_event_status_lines(&events));
                if events.is_empty() {
                    lines.push(StatusLine::muted("No matching events."));
                }
                lines.push(StatusLine::muted(String::new()));
                lines.push(StatusLine::muted(format!(
                    "Navigation: {}{}{}",
                    if can_go_prev { "prev" } else { "first page" },
                    if can_go_next { " · next" } else { "" },
                    " · page <n> · clear"
                )));

                self.context
                    .set_status_dialog_view(StatusDialogView::Events(TuiEventsBrowserState {
                        session_id,
                        filter,
                        offset,
                    }));
                self.status_dialog.set_title("Events");
                self.status_dialog.set_footer_hint(Some(
                    "Esc close · ←/p prev · →/n next · Home/0 first · c clear".to_string(),
                ));
                self.status_dialog.set_status_lines(lines);
                true
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to load session events: {}", error),
                    3000,
                );
                false
            }
        }
    }

    pub(super) fn open_memory_list_status_dialog(&mut self, raw_query: Option<&str>) -> bool {
        let Some(client) = self.context.get_api_client() else {
            self.toast
                .show(ToastVariant::Error, "API unavailable for /memory.", 2400);
            return false;
        };

        let query_text = raw_query
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let query = MemoryListQuery {
            search: query_text.clone(),
            limit: Some(50),
            source_session_id: self.current_session_id(),
            ..MemoryListQuery::default()
        };

        let response = if query.search.is_some() {
            client.search_memory(Some(&query))
        } else {
            client.list_memory(Some(&query))
        };

        match response {
            Ok(response) => {
                let lines = tui_memory_list_status_lines(&query, &response);
                self.context
                    .set_status_dialog_view(StatusDialogView::MemoryList(TuiMemoryListState {
                        query: query_text,
                    }));
                self.status_dialog.set_title("Memory");
                self.status_dialog.set_footer_hint(Some(
                    "Esc close · /memory show <id> · /memory rules · /memory runs · /memory consolidate"
                        .to_string(),
                ));
                self.status_dialog.set_status_lines(lines);
                self.open_status_dialog_modal();
                true
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to load memory list: {}", error),
                    3000,
                );
                false
            }
        }
    }

    pub(super) fn open_memory_preview_status_dialog(&mut self, raw_query: Option<&str>) -> bool {
        let Some(client) = self.context.get_api_client() else {
            self.toast.show(
                ToastVariant::Error,
                "API unavailable for /memory preview.",
                2400,
            );
            return false;
        };

        let query_text = raw_query
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let query = MemoryRetrievalQuery {
            query: query_text.clone(),
            stage: None,
            limit: Some(6),
            kinds: Vec::new(),
            scopes: Vec::new(),
            session_id: self.current_session_id(),
        };

        match client.get_memory_retrieval_preview(&query) {
            Ok(response) => {
                let lines = tui_memory_preview_status_lines(&query, &response);
                self.context
                    .set_status_dialog_view(StatusDialogView::MemoryPreview(
                        TuiMemoryPreviewState { query: query_text },
                    ));
                self.status_dialog.set_title("Memory Preview");
                self.status_dialog.set_footer_hint(Some(
                    "Esc close · shows why records would be injected this turn".to_string(),
                ));
                self.status_dialog.set_status_lines(lines);
                self.open_status_dialog_modal();
                true
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to load memory retrieval preview: {}", error),
                    3000,
                );
                false
            }
        }
    }

    pub(super) fn open_memory_detail_status_dialog(&mut self, record_id: &str) -> bool {
        let Some(client) = self.context.get_api_client() else {
            self.toast
                .show(ToastVariant::Error, "API unavailable for /memory.", 2400);
            return false;
        };

        match client.get_memory_detail(record_id) {
            Ok(detail) => {
                let lines = tui_memory_detail_status_lines(&detail);
                self.context
                    .set_status_dialog_view(StatusDialogView::MemoryDetail(TuiMemoryDetailState {
                        record_id: record_id.to_string(),
                    }));
                self.status_dialog.set_title("Memory Detail");
                self.status_dialog.set_footer_hint(Some(
                    "Esc close · /memory validation <id> · /memory conflicts <id> · /memory hits record=<id>".to_string(),
                ));
                self.status_dialog.set_status_lines(lines);
                self.open_status_dialog_modal();
                true
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to load memory detail: {}", error),
                    3000,
                );
                false
            }
        }
    }

    pub(super) fn open_memory_validation_status_dialog(&mut self, record_id: &str) -> bool {
        let Some(client) = self.context.get_api_client() else {
            self.toast
                .show(ToastVariant::Error, "API unavailable for /memory.", 2400);
            return false;
        };

        match client.get_memory_validation_report(record_id) {
            Ok(report) => {
                let lines = tui_memory_validation_status_lines(&report);
                self.context
                    .set_status_dialog_view(StatusDialogView::MemoryValidation(
                        TuiMemoryDetailState {
                            record_id: record_id.to_string(),
                        },
                    ));
                self.status_dialog.set_title("Memory Validation");
                self.status_dialog.set_footer_hint(Some(
                    "Esc close · values come from /memory/{id}/validation-report".to_string(),
                ));
                self.status_dialog.set_status_lines(lines);
                self.open_status_dialog_modal();
                true
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to load memory validation report: {}", error),
                    3000,
                );
                false
            }
        }
    }

    pub(super) fn open_memory_conflicts_status_dialog(&mut self, record_id: &str) -> bool {
        let Some(client) = self.context.get_api_client() else {
            self.toast
                .show(ToastVariant::Error, "API unavailable for /memory.", 2400);
            return false;
        };

        match client.get_memory_conflicts(record_id) {
            Ok(conflicts) => {
                let lines = tui_memory_conflict_status_lines(&conflicts);
                self.context
                    .set_status_dialog_view(StatusDialogView::MemoryConflicts(
                        TuiMemoryDetailState {
                            record_id: record_id.to_string(),
                        },
                    ));
                self.status_dialog.set_title("Memory Conflicts");
                self.status_dialog.set_footer_hint(Some(
                    "Esc close · values come from /memory/{id}/conflicts".to_string(),
                ));
                self.status_dialog.set_status_lines(lines);
                self.open_status_dialog_modal();
                true
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to load memory conflicts: {}", error),
                    3000,
                );
                false
            }
        }
    }

    pub(super) fn open_memory_rule_packs_status_dialog(&mut self) -> bool {
        let Some(client) = self.context.get_api_client() else {
            self.toast.show(
                ToastVariant::Error,
                "API unavailable for /memory rules.",
                2400,
            );
            return false;
        };

        match client.list_memory_rule_packs() {
            Ok(response) => {
                let lines = tui_memory_rule_pack_status_lines(&response);
                self.context
                    .set_status_dialog_view(StatusDialogView::MemoryRulePacks);
                self.status_dialog.set_title("Memory Rule Packs");
                self.status_dialog.set_footer_hint(Some(
                    "Esc close · values come from /memory/rule-packs".to_string(),
                ));
                self.status_dialog.set_status_lines(lines);
                self.open_status_dialog_modal();
                true
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to load memory rule packs: {}", error),
                    3000,
                );
                false
            }
        }
    }

    pub(super) fn open_memory_rule_hits_status_dialog(&mut self, raw_query: Option<&str>) -> bool {
        let Some(client) = self.context.get_api_client() else {
            self.toast.show(
                ToastVariant::Error,
                "API unavailable for /memory hits.",
                2400,
            );
            return false;
        };

        let parsed = rocode_command::interactive::parse_memory_rule_hit_query(raw_query);
        let query = crate::api::MemoryRuleHitQuery {
            run_id: parsed.run_id,
            memory_id: parsed.record_id.map(rocode_types::MemoryRecordId),
            limit: parsed.limit.map(|value| value as u32),
        };

        match client.list_memory_rule_hits(Some(&query)) {
            Ok(response) => {
                let lines = tui_memory_rule_hit_status_lines(&query, &response);
                self.context
                    .set_status_dialog_view(StatusDialogView::MemoryRuleHits(
                        TuiMemoryRuleHitsState {
                            raw_query: raw_query
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .map(ToOwned::to_owned),
                        },
                    ));
                self.status_dialog.set_title("Memory Rule Hits");
                self.status_dialog.set_footer_hint(Some(
                    "Esc close · filter with /memory hits run=<id> record=<id>".to_string(),
                ));
                self.status_dialog.set_status_lines(lines);
                self.open_status_dialog_modal();
                true
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to load memory rule hits: {}", error),
                    3000,
                );
                false
            }
        }
    }

    pub(super) fn open_memory_consolidation_runs_status_dialog(&mut self) -> bool {
        let Some(client) = self.context.get_api_client() else {
            self.toast.show(
                ToastVariant::Error,
                "API unavailable for /memory runs.",
                2400,
            );
            return false;
        };

        match client.list_memory_consolidation_runs(Some(
            &crate::api::MemoryConsolidationRunQuery { limit: Some(20) },
        )) {
            Ok(response) => {
                let lines = tui_memory_consolidation_runs_status_lines(&response);
                self.context
                    .set_status_dialog_view(StatusDialogView::MemoryConsolidationRuns);
                self.status_dialog.set_title("Memory Consolidation Runs");
                self.status_dialog.set_footer_hint(Some(
                    "Esc close · values come from /memory/consolidation/runs".to_string(),
                ));
                self.status_dialog.set_status_lines(lines);
                self.open_status_dialog_modal();
                true
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to load memory consolidation runs: {}", error),
                    3000,
                );
                false
            }
        }
    }

    pub(super) fn run_memory_consolidation_status_dialog(
        &mut self,
        raw_request: Option<&str>,
    ) -> bool {
        let Some(client) = self.context.get_api_client() else {
            self.toast.show(
                ToastVariant::Error,
                "API unavailable for /memory consolidate.",
                2400,
            );
            return false;
        };

        let parsed = rocode_command::interactive::parse_memory_consolidation_request(raw_request);
        let request = crate::api::MemoryConsolidationRequest {
            limit: parsed.limit.map(|value| value as u32),
            include_candidates: parsed.include_candidates,
        };

        match client.run_memory_consolidation(&request) {
            Ok(response) => {
                let lines = tui_memory_consolidation_result_status_lines(&response);
                self.context
                    .set_status_dialog_view(StatusDialogView::MemoryConsolidationResult(
                        TuiMemoryConsolidationState {
                            raw_request: raw_request
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .map(ToOwned::to_owned),
                        },
                    ));
                self.status_dialog.set_title("Memory Consolidation");
                self.status_dialog.set_footer_hint(Some(
                    "Esc close · /memory runs · /memory hits run=<id>".to_string(),
                ));
                self.status_dialog.set_status_lines(lines);
                self.open_status_dialog_modal();
                true
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to run memory consolidation: {}", error),
                    3000,
                );
                false
            }
        }
    }

    pub(super) fn refresh_active_status_dialog(&mut self) {
        match self.context.status_dialog_view() {
            StatusDialogView::Overview => self.refresh_status_dialog(),
            StatusDialogView::Runtime => {
                let _ = self.render_runtime_status_dialog();
            }
            StatusDialogView::Usage => {
                let _ = self.render_usage_status_dialog();
            }
            StatusDialogView::Insights => {
                let _ = self.render_insights_status_dialog();
            }
            StatusDialogView::ConfigValidation => {
                let _ = self.open_config_validation_status_dialog();
            }
            StatusDialogView::Events(_) => {
                let _ = self.open_events_status_dialog(None);
            }
            StatusDialogView::MemoryList(state) => {
                let _ = self.open_memory_list_status_dialog(state.query.as_deref());
            }
            StatusDialogView::MemoryPreview(state) => {
                let _ = self.open_memory_preview_status_dialog(state.query.as_deref());
            }
            StatusDialogView::MemoryDetail(state) => {
                let _ = self.open_memory_detail_status_dialog(&state.record_id);
            }
            StatusDialogView::MemoryValidation(state) => {
                let _ = self.open_memory_validation_status_dialog(&state.record_id);
            }
            StatusDialogView::MemoryConflicts(state) => {
                let _ = self.open_memory_conflicts_status_dialog(&state.record_id);
            }
            StatusDialogView::MemoryRulePacks => {
                let _ = self.open_memory_rule_packs_status_dialog();
            }
            StatusDialogView::MemoryRuleHits(state) => {
                let _ = self.open_memory_rule_hits_status_dialog(state.raw_query.as_deref());
            }
            StatusDialogView::MemoryConsolidationRuns => {
                let _ = self.open_memory_consolidation_runs_status_dialog();
            }
            StatusDialogView::MemoryConsolidationResult(state) => {
                let _ = self.run_memory_consolidation_status_dialog(state.raw_request.as_deref());
            }
        }
    }

    pub(super) fn refresh_status_dialog(&mut self) {
        self.status_dialog.set_title("Status");
        self.status_dialog.set_footer_hint(None);
        let formatters = self
            .context
            .get_api_client()
            .and_then(|client| client.get_formatters().ok())
            .unwrap_or_default();
        let route_label = match self.context.current_route() {
            Route::Home => "home".to_string(),
            Route::Session { session_id } => format!("session ({})", session_id),
            Route::Settings => "settings".to_string(),
            Route::Help => "help".to_string(),
        };
        let session_ctx = self.context.session.read();
        let mcp_servers = self.context.mcp_servers.read();
        let lsp_status = self.context.lsp_status.read();
        let connected_mcp = mcp_servers
            .iter()
            .filter(|s| matches!(s.status, McpConnectionStatus::Connected))
            .count();
        let mut status_blocks = vec![
            StatusBlock::title("Runtime"),
            StatusBlock::normal(format!("Route: {}", route_label)),
            StatusBlock::normal(format!(
                "Directory: {}",
                self.context.directory.read().as_str()
            )),
            StatusBlock::normal(format!("Mode: {}", {
                current_mode_label(&self.context).unwrap_or_else(|| "auto".to_string())
            })),
            StatusBlock::normal(format!("Model: {}", self.current_model_label())),
            StatusBlock::normal(format!(
                "Theme: {}",
                format_theme_option_label(&self.context.current_theme_name())
            )),
            StatusBlock::normal(format!("Loaded sessions: {}", session_ctx.sessions.len())),
            StatusBlock::muted(""),
            StatusBlock::title(format!(
                "MCP Servers ({}, connected: {})",
                mcp_servers.len(),
                connected_mcp
            )),
        ];
        if mcp_servers.is_empty() {
            status_blocks.push(StatusBlock::muted("- No MCP servers"));
        } else {
            for server in mcp_servers.iter() {
                let status_text = match server.status {
                    McpConnectionStatus::Connected => "connected",
                    McpConnectionStatus::Disconnected => "disconnected",
                    McpConnectionStatus::Failed => "failed",
                    McpConnectionStatus::NeedsAuth => "needs authentication",
                    McpConnectionStatus::NeedsClientRegistration => "needs client ID",
                    McpConnectionStatus::Disabled => "disabled",
                };
                let base = format!("- {}: {}", server.name, status_text);
                match server.status {
                    McpConnectionStatus::Connected => {
                        status_blocks.push(StatusBlock::success(base))
                    }
                    McpConnectionStatus::NeedsAuth
                    | McpConnectionStatus::NeedsClientRegistration => {
                        status_blocks.push(StatusBlock::warning(base))
                    }
                    McpConnectionStatus::Failed => {
                        let text = if let Some(error) = &server.error {
                            format!("{} ({})", base, error)
                        } else {
                            base
                        };
                        status_blocks.push(StatusBlock::error(text));
                    }
                    _ => status_blocks.push(StatusBlock::muted(base)),
                }
            }
        }

        status_blocks.push(StatusBlock::muted(""));
        status_blocks.push(StatusBlock::title(format!(
            "LSP Servers ({})",
            lsp_status.len()
        )));
        if lsp_status.is_empty() {
            status_blocks.push(StatusBlock::muted("- No LSP servers"));
        } else {
            for server in lsp_status.iter() {
                status_blocks.push(StatusBlock::success(format!("- {}", server.id)));
            }
        }

        status_blocks.push(StatusBlock::muted(""));
        status_blocks.push(StatusBlock::title(format!(
            "Formatters ({})",
            formatters.len()
        )));
        if formatters.is_empty() {
            status_blocks.push(StatusBlock::muted("- No formatters"));
        } else {
            for formatter in formatters {
                status_blocks.push(StatusBlock::success(format!("- {}", formatter)));
            }
        }
        if let Route::Session { session_id } = self.context.current_route() {
            status_blocks.push(StatusBlock::muted(""));
            status_blocks.extend(self.execution_status_blocks(&session_id));
            status_blocks.push(StatusBlock::muted(""));
            status_blocks.extend(self.session_telemetry_status_blocks());
            status_blocks.push(StatusBlock::muted(""));
            status_blocks.extend(self.recovery_status_blocks(&session_id));
        }
        let lines = status_blocks
            .into_iter()
            .map(status_line_from_block)
            .collect::<Vec<_>>();
        self.status_dialog.set_status_lines(lines);
    }

    pub(super) fn execution_status_blocks(&self, session_id: &str) -> Vec<StatusBlock> {
        let topology = match self.context.execution_topology() {
            Some(topology) => topology,
            None => {
                let Some(client) = self.context.get_api_client() else {
                    return vec![
                        StatusBlock::title("Execution Topology"),
                        StatusBlock::muted("- API unavailable"),
                    ];
                };
                match client.get_session_telemetry(session_id) {
                    Ok(telemetry) => {
                        let topology = telemetry.topology.clone();
                        self.context.apply_session_telemetry_snapshot(telemetry);
                        topology
                    }
                    Err(error) => {
                        return vec![
                            StatusBlock::title("Execution Topology"),
                            StatusBlock::error(format!("- Failed to load: {}", error)),
                        ];
                    }
                }
            }
        };

        let mut blocks = vec![StatusBlock::title(format!(
            "Execution Topology (active: {}, running: {}, waiting: {}, cancelling: {}, retry: {})",
            topology.active_count,
            topology.running_count,
            topology.waiting_count,
            topology.cancelling_count,
            topology.retry_count
        ))];

        if topology.roots.is_empty() {
            blocks.push(StatusBlock::muted("- No active executions"));
            return blocks;
        }

        for (index, root) in topology.roots.iter().enumerate() {
            append_execution_status_node(&mut blocks, root, "", index + 1 == topology.roots.len());
        }

        blocks
    }

    pub(super) fn session_telemetry_status_blocks(&self) -> Vec<StatusBlock> {
        let runtime = self.context.session_runtime();
        let usage = self.context.session_usage();
        let stages = self.context.stage_summaries();
        let Some(runtime) = runtime else {
            return vec![
                StatusBlock::title("Session Telemetry"),
                StatusBlock::muted("- Telemetry snapshot not loaded"),
            ];
        };

        let mut blocks = vec![StatusBlock::title(format!(
            "Session Telemetry ({})",
            format_run_status(&runtime.run_status)
        ))];

        if let Some(stage_id) = runtime.active_stage_id.as_deref() {
            blocks.push(StatusBlock::normal(format!(
                "Active stage: {} ({} active)",
                stage_id, runtime.active_stage_count
            )));
        } else {
            blocks.push(StatusBlock::muted(format!(
                "- No active stage ({})",
                runtime.active_stage_count
            )));
        }

        if let Some(usage) = usage.as_ref() {
            if let Some(current_tokens) = self.context.current_context_tokens() {
                let context_limit = self
                    .context
                    .resolve_model_info(self.context.last_assistant_model().as_deref())
                    .map(|model| model.context_window);
                blocks.push(StatusBlock::normal(format!(
                    "Current context: {}",
                    tui_format_context_usage_label(current_tokens, context_limit)
                )));
            }
            blocks.push(StatusBlock::normal(format!(
                "Session cumulative: {} total · input {} · output {} · reasoning {} · cache H/M/W {}/{}/{} · cost ${:.4}",
                tui_format_token_count(tui_total_session_tokens(usage)),
                tui_format_token_count(usage.input_tokens),
                tui_format_token_count(usage.output_tokens),
                tui_format_token_count(usage.reasoning_tokens),
                tui_format_token_count(usage.cache_read_tokens),
                tui_format_token_count(usage.cache_miss_tokens),
                tui_format_token_count(usage.cache_write_tokens),
                usage.total_cost
            )));
        }

        if let Some(active_stage_id) = runtime.active_stage_id.as_deref() {
            if let Some(stage) = stages
                .iter()
                .find(|stage| stage.stage_id == active_stage_id)
            {
                blocks.extend(active_stage_status_blocks(stage));
            }
        }

        if !stages.is_empty() {
            blocks.push(StatusBlock::title(format!(
                "Stage Summaries ({})",
                stages.len()
            )));
            for stage in stages.iter().take(5) {
                blocks.push(StatusBlock::normal(format_stage_summary_line(stage)));
            }
            if stages.len() > 5 {
                blocks.push(StatusBlock::muted(format!(
                    "- {} more stage summaries",
                    stages.len() - 5
                )));
            }
        }

        blocks
    }

    fn render_runtime_status_dialog(&mut self) -> bool {
        let Some(session_id) = self.current_session_id() else {
            self.toast.show(
                ToastVariant::Warning,
                "No active session available for /runtime.",
                2400,
            );
            return false;
        };

        let Some(client) = self.context.get_api_client() else {
            self.toast
                .show(ToastVariant::Error, "API unavailable for /runtime.", 2400);
            return false;
        };

        match client.get_session_telemetry(&session_id) {
            Ok(telemetry) => {
                self.context
                    .apply_session_telemetry_snapshot(telemetry.clone());
                let lines = tui_runtime_status_lines(&session_id, &telemetry);
                self.status_dialog.set_title("Runtime");
                self.status_dialog.set_footer_hint(Some(
                    "Esc close · /events [stage=<id>] for raw event log".to_string(),
                ));
                self.status_dialog.set_status_lines(lines);
                true
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to load runtime telemetry: {}", error),
                    3000,
                );
                false
            }
        }
    }

    fn render_usage_status_dialog(&mut self) -> bool {
        let Some(session_id) = self.current_session_id() else {
            self.toast.show(
                ToastVariant::Warning,
                "No active session available for /usage.",
                2400,
            );
            return false;
        };

        let Some(client) = self.context.get_api_client() else {
            self.toast
                .show(ToastVariant::Error, "API unavailable for /usage.", 2400);
            return false;
        };

        match client.get_session_telemetry(&session_id) {
            Ok(telemetry) => {
                self.context
                    .apply_session_telemetry_snapshot(telemetry.clone());
                let current_context_tokens = self.context.current_context_tokens();
                let context_limit = self
                    .context
                    .resolve_model_info(self.context.last_assistant_model().as_deref())
                    .map(|model| model.context_window);
                let last_turn_tokens = self.context.last_assistant_turn_tokens();
                let lines = tui_usage_status_lines(
                    &session_id,
                    &telemetry,
                    current_context_tokens,
                    context_limit,
                    last_turn_tokens.as_ref(),
                );
                self.status_dialog.set_title("Usage");
                self.status_dialog.set_footer_hint(Some(
                    "Esc close · values come from /session/{id}/telemetry".to_string(),
                ));
                self.status_dialog.set_status_lines(lines);
                true
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to load session usage: {}", error),
                    3000,
                );
                false
            }
        }
    }

    fn render_insights_status_dialog(&mut self) -> bool {
        let Some(session_id) = self.current_session_id() else {
            self.toast.show(
                ToastVariant::Warning,
                "No active session available for /insights.",
                2400,
            );
            return false;
        };

        let Some(client) = self.context.get_api_client() else {
            self.toast
                .show(ToastVariant::Error, "API unavailable for /insights.", 2400);
            return false;
        };

        match client.get_session_insights(&session_id) {
            Ok(insights) => {
                let lines = tui_session_insights_lines(&session_id, &insights);
                self.status_dialog.set_title("Insights");
                self.status_dialog.set_footer_hint(Some(
                    "Esc close · /memory show <id> · /memory hits record=<id> · /session/{id}/insights includes telemetry, multimodal explain, memory explain, and effective policy".to_string(),
                ));
                self.status_dialog.set_status_lines(lines);
                true
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to load session insights: {}", error),
                    3000,
                );
                false
            }
        }
    }

    pub(super) fn handle_status_dialog_key(&mut self, key: KeyEvent) -> bool {
        if !self.status_dialog.is_open() {
            return false;
        }

        if !matches!(
            self.context.status_dialog_view(),
            StatusDialogView::Events(_)
        ) {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                self.close_status_dialog_modal();
            }
            return true;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.close_status_dialog_modal(),
            KeyCode::Left | KeyCode::PageUp => {
                let _ = self.open_events_status_dialog(Some("prev"));
            }
            KeyCode::Right | KeyCode::PageDown => {
                let _ = self.open_events_status_dialog(Some("next"));
            }
            KeyCode::Home => {
                let _ = self.open_events_status_dialog(Some("first"));
            }
            KeyCode::Char('p')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                let _ = self.open_events_status_dialog(Some("prev"));
            }
            KeyCode::Char('n')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                let _ = self.open_events_status_dialog(Some("next"));
            }
            KeyCode::Char('0')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                let _ = self.open_events_status_dialog(Some("first"));
            }
            KeyCode::Char('c')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                let _ = self.open_events_status_dialog(Some("clear"));
            }
            _ => {}
        }
        true
    }

    pub(super) fn recovery_status_blocks(&self, session_id: &str) -> Vec<StatusBlock> {
        let Some(client) = self.context.get_api_client() else {
            return vec![
                StatusBlock::title("Recovery Protocol"),
                StatusBlock::muted("- API unavailable"),
            ];
        };

        let recovery = match client.get_session_recovery(session_id) {
            Ok(recovery) => recovery,
            Err(error) => {
                return vec![
                    StatusBlock::title("Recovery Protocol"),
                    StatusBlock::error(format!("- Failed to load: {}", error)),
                ];
            }
        };

        recovery_status_blocks_from_protocol(&recovery)
    }

    // ── Agent task handlers ──────────────────────────────────────────────

    pub(super) fn handle_list_tasks(&mut self) {
        let tasks = global_task_registry().list();
        let now = Utc::now().timestamp();
        let mut blocks = vec![StatusBlock::title("Agent Tasks")];
        if tasks.is_empty() {
            blocks.push(StatusBlock::muted("No agent tasks"));
        } else {
            for task in &tasks {
                let (icon, status_str) = match &task.status {
                    AgentTaskStatus::Pending => ("◯", "pending".to_string()),
                    AgentTaskStatus::Running { step } => {
                        let steps = task
                            .max_steps
                            .map(|m| format!("{}/{}", step, m))
                            .unwrap_or(format!("{}/?", step));
                        ("◐", format!("running  {}", steps))
                    }
                    AgentTaskStatus::Completed { steps } => ("●", format!("done     {}", steps)),
                    AgentTaskStatus::Cancelled => ("✗", "cancelled".to_string()),
                    AgentTaskStatus::Failed { .. } => ("✗", "failed".to_string()),
                };
                let elapsed = now - task.started_at;
                let elapsed_str = if elapsed < 60 {
                    format!("{}s ago", elapsed)
                } else {
                    format!("{}m ago", elapsed / 60)
                };
                let line = format!(
                    "{}  {}  {:<20} {:<16} {}",
                    icon, task.id, task.agent_name, status_str, elapsed_str
                );
                let block = if task.status.is_terminal() {
                    StatusBlock::muted(line)
                } else {
                    StatusBlock::normal(line)
                };
                blocks.push(block);
            }
            let running = tasks
                .iter()
                .filter(|t| matches!(t.status, AgentTaskStatus::Running { .. }))
                .count();
            let done = tasks.iter().filter(|t| t.status.is_terminal()).count();
            blocks.push(StatusBlock::muted(format!(
                "{} running, {} finished",
                running, done
            )));
        }
        let lines = blocks
            .into_iter()
            .map(status_line_from_block)
            .collect::<Vec<_>>();
        self.status_dialog.set_status_lines(lines);
        self.open_status_dialog_modal();
    }

    pub(super) fn handle_show_task(&mut self, id: &str) {
        let now = Utc::now().timestamp();
        match global_task_registry().get(id) {
            Some(task) => {
                let (status_label, step_info) = match &task.status {
                    AgentTaskStatus::Pending => ("pending".to_string(), String::new()),
                    AgentTaskStatus::Running { step } => {
                        let steps = task
                            .max_steps
                            .map(|m| format!(" (step {}/{})", step, m))
                            .unwrap_or(format!(" (step {}/?)", step));
                        ("running".to_string(), steps)
                    }
                    AgentTaskStatus::Completed { steps } => {
                        ("completed".to_string(), format!(" ({} steps)", steps))
                    }
                    AgentTaskStatus::Cancelled => ("cancelled".to_string(), String::new()),
                    AgentTaskStatus::Failed { error } => {
                        (format!("failed: {}", error), String::new())
                    }
                };
                let elapsed = now - task.started_at;
                let elapsed_str = if elapsed < 60 {
                    format!("{}s ago", elapsed)
                } else {
                    format!("{}m ago", elapsed / 60)
                };
                let mut blocks = vec![
                    StatusBlock::title(format!("Task {} — {}", task.id, task.agent_name)),
                    StatusBlock::normal(format!("Status: {}{}", status_label, step_info)),
                    StatusBlock::normal(format!("Started: {}", elapsed_str)),
                    StatusBlock::normal(format!("Prompt: {}", task.prompt)),
                ];
                if !task.output_tail.is_empty() {
                    blocks.push(StatusBlock::muted(""));
                    blocks.push(StatusBlock::title("Recent output"));
                    for line in &task.output_tail {
                        blocks.push(StatusBlock::muted(format!("  {}", line)));
                    }
                }
                let lines = blocks
                    .into_iter()
                    .map(status_line_from_block)
                    .collect::<Vec<_>>();
                self.status_dialog.set_status_lines(lines);
                self.open_status_dialog_modal();
            }
            None => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Task \"{}\" not found", id),
                    2500,
                );
            }
        }
    }

    pub(super) fn handle_kill_task(&mut self, id: &str) {
        match rocode_orchestrator::global_lifecycle().cancel_task(id) {
            Ok(()) => {
                self.toast.show(
                    ToastVariant::Success,
                    &format!("Task {} cancelled", id),
                    2000,
                );
            }
            Err(err) => {
                self.toast.show(ToastVariant::Error, &err, 2500);
            }
        }
    }
}

fn tui_memory_list_status_lines(
    query: &MemoryListQuery,
    response: &crate::api::MemoryListResponse,
) -> Vec<StatusLine> {
    let mut lines = vec![StatusLine::title("Memory Records")];
    if let Some(session_id) = query.source_session_id.as_deref() {
        lines.push(StatusLine::normal(format!(
            "Session filter: {}",
            session_id
        )));
    } else {
        lines.push(StatusLine::muted("Scope: current workspace authority"));
    }
    if let Some(search) = query.search.as_deref() {
        lines.push(StatusLine::normal(format!("Search: {}", search)));
    }
    lines.push(StatusLine::muted(format!(
        "Total: {} · Search fields: {}",
        response.items.len(),
        response.contract.search_fields.join(", ")
    )));
    lines.push(StatusLine::muted(String::new()));
    if response.items.is_empty() {
        lines.push(StatusLine::muted(
            "No memory records matched the current query.",
        ));
        return lines;
    }

    for item in &response.items {
        lines.push(StatusLine::normal(format!(
            "{} · {:?} · {:?} · {:?}",
            item.id.0, item.kind, item.status, item.validation_status
        )));
        if item.linked_skill_name.is_some() || item.derived_skill_name.is_some() {
            lines.push(StatusLine::muted(format!(
                "  skills: linked={} · target={}",
                item.linked_skill_name.as_deref().unwrap_or("--"),
                item.derived_skill_name.as_deref().unwrap_or("--")
            )));
        }
        lines.push(StatusLine::muted(format!("  {}", item.title)));
        lines.push(StatusLine::muted(format!("  {}", item.summary)));
    }
    lines
}

fn tui_memory_preview_status_lines(
    query: &MemoryRetrievalQuery,
    response: &MemoryRetrievalPreviewResponse,
) -> Vec<StatusLine> {
    let packet = &response.packet;
    let mut lines = vec![StatusLine::title("Memory Retrieval Preview")];
    if let Some(session_id) = query.session_id.as_deref() {
        lines.push(StatusLine::normal(format!(
            "Session filter: {}",
            session_id
        )));
    }
    if let Some(search) = packet.query.as_deref() {
        lines.push(StatusLine::normal(format!("Query: {}", search)));
    }
    lines.push(StatusLine::muted(format!(
        "Items: {} · Budget: {}",
        packet.items.len(),
        packet
            .budget_limit
            .map(|value| value.to_string())
            .unwrap_or_else(|| "--".to_string())
    )));
    lines.push(StatusLine::muted(response.contract.note.clone()));
    lines.push(StatusLine::muted(String::new()));
    if packet.items.is_empty() {
        lines.push(StatusLine::muted(
            "No memory records would be injected for this turn.",
        ));
        return lines;
    }

    for item in &packet.items {
        lines.push(StatusLine::normal(format!(
            "{} · {:?} · {:?}",
            item.card.id.0, item.card.kind, item.card.validation_status
        )));
        lines.push(StatusLine::muted(format!("  {}", item.card.title)));
        lines.push(StatusLine::muted(format!("  why: {}", item.why_recalled)));
        lines.push(StatusLine::muted(format!(
            "  summary: {}",
            item.card.summary
        )));
        if let Some(evidence) = item.evidence_summary.as_deref() {
            lines.push(StatusLine::muted(format!("  evidence: {}", evidence)));
        }
    }
    lines
}

fn tui_memory_detail_status_lines(detail: &MemoryDetailView) -> Vec<StatusLine> {
    let record = &detail.record;
    let mut lines = vec![
        StatusLine::title("Memory Detail"),
        StatusLine::normal(format!("Id: {}", record.id.0)),
        StatusLine::normal(format!(
            "Kind: {:?} · Scope: {:?} · Status: {:?} · Validation: {:?}",
            record.kind, record.scope, record.status, record.validation_status
        )),
        StatusLine::normal(format!("Title: {}", record.title)),
        StatusLine::normal(format!("Summary: {}", record.summary)),
    ];
    if !record.trigger_conditions.is_empty() {
        lines.push(StatusLine::muted("Triggers:"));
        lines.extend(
            record
                .trigger_conditions
                .iter()
                .map(|value| StatusLine::muted(format!("  - {}", value))),
        );
    }
    if !record.normalized_facts.is_empty() {
        lines.push(StatusLine::muted("Facts:"));
        lines.extend(
            record
                .normalized_facts
                .iter()
                .map(|value| StatusLine::muted(format!("  - {}", value))),
        );
    }
    if !record.boundaries.is_empty() {
        lines.push(StatusLine::muted("Boundaries:"));
        lines.extend(
            record
                .boundaries
                .iter()
                .map(|value| StatusLine::muted(format!("  - {}", value))),
        );
    }
    if !record.evidence_refs.is_empty() {
        lines.push(StatusLine::muted("Evidence:"));
        lines.extend(record.evidence_refs.iter().map(|evidence| {
            StatusLine::muted(format!(
                "  - session={} message={} tool={} stage={} {}",
                evidence.session_id.as_deref().unwrap_or("--"),
                evidence.message_id.as_deref().unwrap_or("--"),
                evidence.tool_call_id.as_deref().unwrap_or("--"),
                evidence.stage_id.as_deref().unwrap_or("--"),
                evidence.note.as_deref().unwrap_or("")
            ))
        }));
    }
    lines
}

fn tui_memory_validation_status_lines(
    response: &MemoryValidationReportResponse,
) -> Vec<StatusLine> {
    let mut lines = vec![
        StatusLine::title("Memory Validation"),
        StatusLine::normal(format!("Record: {}", response.record_id.0)),
    ];
    match response.latest.as_ref() {
        Some(report) => {
            lines.push(StatusLine::normal(format!("Status: {:?}", report.status)));
            lines.push(StatusLine::muted(format!(
                "Checked at: {}",
                report.checked_at
            )));
            if report.issues.is_empty() {
                lines.push(StatusLine::muted("Issues: none"));
            } else {
                lines.push(StatusLine::muted("Issues:"));
                lines.extend(
                    report
                        .issues
                        .iter()
                        .map(|issue| StatusLine::muted(format!("  - {}", issue))),
                );
            }
        }
        None => lines.push(StatusLine::muted(
            "No validation report has been recorded yet.",
        )),
    }
    lines
}

fn tui_memory_conflict_status_lines(response: &MemoryConflictResponse) -> Vec<StatusLine> {
    let mut lines = vec![
        StatusLine::title("Memory Conflicts"),
        StatusLine::normal(format!("Record: {}", response.record_id.0)),
    ];
    if response.conflicts.is_empty() {
        lines.push(StatusLine::muted(
            "No conflicts detected for this memory record.",
        ));
        return lines;
    }

    for conflict in &response.conflicts {
        lines.push(StatusLine::normal(format!(
            "{} · other={} · {}",
            conflict.id, conflict.other_record_id.0, conflict.conflict_kind
        )));
        lines.push(StatusLine::muted(format!("  {}", conflict.detail)));
        lines.push(StatusLine::muted(format!(
            "  detected_at={}",
            conflict.detected_at
        )));
    }
    lines
}

fn tui_memory_rule_pack_status_lines(
    response: &crate::api::MemoryRulePackListResponse,
) -> Vec<StatusLine> {
    let mut lines = vec![
        StatusLine::title("Memory Rule Packs"),
        StatusLine::normal(format!("Total: {}", response.items.len())),
    ];
    if response.items.is_empty() {
        lines.push(StatusLine::muted("No rule packs registered."));
        return lines;
    }

    for pack in &response.items {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::normal(format!(
            "{} · {:?} · version {}",
            pack.id, pack.rule_pack_kind, pack.version
        )));
        if pack.rules.is_empty() {
            lines.push(StatusLine::muted("  rules: none"));
        } else {
            for rule in &pack.rules {
                lines.push(StatusLine::muted(format!(
                    "  - {}: {}",
                    rule.id, rule.description
                )));
            }
        }
    }
    lines
}

fn tui_memory_rule_hit_status_lines(
    query: &crate::api::MemoryRuleHitQuery,
    response: &crate::api::MemoryRuleHitListResponse,
) -> Vec<StatusLine> {
    let mut lines = vec![
        StatusLine::title("Memory Rule Hits"),
        StatusLine::normal(format!("Total: {}", response.items.len())),
    ];
    if let Some(run_id) = query.run_id.as_deref() {
        lines.push(StatusLine::muted(format!("Run filter: {}", run_id)));
    }
    if let Some(memory_id) = query.memory_id.as_ref() {
        lines.push(StatusLine::muted(format!("Record filter: {}", memory_id.0)));
    }
    if response.items.is_empty() {
        lines.push(StatusLine::muted(
            "No matching memory rule hits were found.",
        ));
        return lines;
    }

    for hit in &response.items {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::normal(format!("{} · {}", hit.id, hit.hit_kind)));
        lines.push(StatusLine::muted(format!(
            "  run={} memory={}",
            hit.run_id.as_deref().unwrap_or("--"),
            hit.memory_id
                .as_ref()
                .map(|id| id.0.as_str())
                .unwrap_or("--")
        )));
        if let Some(pack_id) = hit.rule_pack_id.as_deref() {
            lines.push(StatusLine::muted(format!("  pack={}", pack_id)));
        }
        if let Some(detail) = hit.detail.as_deref() {
            lines.push(StatusLine::muted(format!("  {}", detail)));
        }
    }
    lines
}

fn tui_memory_consolidation_runs_status_lines(
    response: &crate::api::MemoryConsolidationRunListResponse,
) -> Vec<StatusLine> {
    let mut lines = vec![
        StatusLine::title("Memory Consolidation Runs"),
        StatusLine::normal(format!("Total: {}", response.items.len())),
    ];
    if response.items.is_empty() {
        lines.push(StatusLine::muted("No consolidation runs recorded yet."));
        return lines;
    }

    for run in &response.items {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::normal(format!(
            "{} · merged {} · promoted {} · conflicts {}",
            run.run_id, run.merged_count, run.promoted_count, run.conflict_count
        )));
        lines.push(StatusLine::muted(format!(
            "  started={} finished={}",
            run.started_at,
            run.finished_at
                .map(|value: i64| value.to_string())
                .unwrap_or_else(|| "--".to_string())
        )));
    }
    lines
}

fn tui_memory_consolidation_result_status_lines(
    response: &crate::api::MemoryConsolidationResponse,
) -> Vec<StatusLine> {
    let mut lines = vec![
        StatusLine::title("Memory Consolidation"),
        StatusLine::normal(format!("Run: {}", response.run.run_id)),
        StatusLine::normal(format!(
            "Merged: {} · Promoted: {} · Conflicts: {}",
            response.run.merged_count, response.run.promoted_count, response.run.conflict_count
        )),
    ];
    if !response.promoted_record_ids.is_empty() {
        lines.push(StatusLine::muted("Promoted records:"));
        lines.extend(
            response
                .promoted_record_ids
                .iter()
                .map(|id| StatusLine::muted(format!("  - {}", id.0))),
        );
    }
    if !response.reflection_notes.is_empty() {
        lines.push(StatusLine::muted("Reflection:"));
        lines.extend(
            response
                .reflection_notes
                .iter()
                .map(|note| StatusLine::muted(format!("  - {}", note))),
        );
    }
    if !response.rule_hits.is_empty() {
        lines.push(StatusLine::muted("Rule hits:"));
        lines.extend(
            response
                .rule_hits
                .iter()
                .take(8)
                .map(|hit| StatusLine::muted(format!("  - {} ({})", hit.hit_kind, hit.id))),
        );
    }
    lines
}

fn format_run_status(status: &crate::api::SessionRunStatusKind) -> &'static str {
    match status {
        crate::api::SessionRunStatusKind::Idle => "idle",
        crate::api::SessionRunStatusKind::Running => "running",
        crate::api::SessionRunStatusKind::WaitingOnTool => "waiting_on_tool",
        crate::api::SessionRunStatusKind::WaitingOnUser => "waiting_on_user",
        crate::api::SessionRunStatusKind::Cancelling => "cancelling",
    }
}

fn tui_runtime_status_lines(
    session_id: &str,
    telemetry: &crate::api::SessionTelemetrySnapshot,
) -> Vec<StatusLine> {
    let runtime = &telemetry.runtime;
    let topology = &telemetry.topology;
    let mut lines = vec![
        StatusLine::title("Runtime Telemetry"),
        StatusLine::normal(format!("Session: {}", session_id)),
        StatusLine::normal(format!(
            "Run status: {}",
            format_run_status(&runtime.run_status)
        )),
        StatusLine::normal(format!(
            "Topology: active {} · running {} · waiting {} · cancelling {} · retry {} · done {}",
            topology.active_count,
            topology.running_count,
            topology.waiting_count,
            topology.cancelling_count,
            topology.retry_count,
            topology.done_count
        )),
        StatusLine::normal(format!("Stages observed: {}", telemetry.stages.len())),
    ];

    if let Some(active_stage_id) = runtime.active_stage_id.as_deref() {
        if let Some(stage) = telemetry
            .stages
            .iter()
            .find(|stage| stage.stage_id == active_stage_id)
        {
            lines.push(StatusLine::muted(String::new()));
            lines.push(StatusLine::title(format!(
                "Active Stage ({})",
                stage.stage_name
            )));
            lines.push(StatusLine::normal(format!(
                "Status: {}",
                format_stage_status(stage.status.clone())
            )));
            if let Some(waiting_on) = stage.waiting_on.as_deref() {
                lines.push(StatusLine::warning(format!("Waiting on: {}", waiting_on)));
            }
            if let Some(last_event) = stage.last_event.as_deref() {
                lines.push(StatusLine::muted(format!("Last event: {}", last_event)));
            }
            if let Some(budget) = stage.skill_tree_budget {
                lines.push(StatusLine::normal(format!(
                    "Skill tree budget: {}{}",
                    budget,
                    if stage.skill_tree_truncated.unwrap_or(false) {
                        " (truncated)"
                    } else {
                        ""
                    }
                )));
            }
        }
    }

    if !telemetry.stages.is_empty() {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::title(format!(
            "Stage Summaries ({})",
            telemetry.stages.len()
        )));
        for stage in &telemetry.stages {
            lines.push(StatusLine::normal(format_stage_runtime_line(stage)));
            if let Some(last_event) = stage.last_event.as_deref() {
                lines.push(StatusLine::muted(format!("  last-event {}", last_event)));
            }
            if let Some(focus) = stage.focus.as_deref() {
                lines.push(StatusLine::muted(format!("  focus {}", focus)));
            }
        }
    }

    lines.push(StatusLine::muted(String::new()));
    if runtime.active_tools.is_empty() {
        lines.push(StatusLine::muted("Active tools: none"));
    } else {
        lines.push(StatusLine::title(format!(
            "Active Tools ({})",
            runtime.active_tools.len()
        )));
        for tool in &runtime.active_tools {
            lines.push(StatusLine::normal(format!(
                "- {} · {}",
                tool.tool_name, tool.tool_call_id
            )));
        }
    }

    if let Some(question) = runtime.pending_question.as_ref() {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::warning(format!(
            "Pending question: {}",
            question.request_id
        )));
    }
    if let Some(permission) = runtime.pending_permission.as_ref() {
        lines.push(StatusLine::warning(format!(
            "Pending permission: {}",
            permission.permission_id
        )));
    }

    if !runtime.child_sessions.is_empty() {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::title(format!(
            "Child Sessions ({})",
            runtime.child_sessions.len()
        )));
        for child in &runtime.child_sessions {
            lines.push(StatusLine::normal(format!(
                "- {} ← {}",
                child.child_id, child.parent_id
            )));
        }
    }

    if let Some(memory) = telemetry.memory.as_ref() {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::title("Memory Runtime"));
        lines.push(StatusLine::normal(format!(
            "Workspace: {} · {}",
            memory.workspace_mode, memory.workspace_key
        )));
        lines.push(StatusLine::normal(format!(
            "Frozen snapshot: {} items{}",
            memory.frozen_snapshot_items,
            tui_optional_generated_at(memory.frozen_snapshot_generated_at)
        )));
        lines.push(StatusLine::normal(format!(
            "Last prefetch: {} items{}",
            memory.last_prefetch_items,
            tui_optional_generated_at(memory.last_prefetch_generated_at)
        )));
        lines.push(StatusLine::normal(format!(
            "Session records: candidate {} · validated {} · rejected {}",
            memory.candidate_count, memory.validated_count, memory.rejected_count
        )));
        lines.push(StatusLine::normal(format!(
            "Validation pressure: warnings {} · methodology {} · skill targets {}",
            memory.warning_count,
            memory.methodology_candidate_count,
            memory.derived_skill_candidate_count
        )));
        lines.push(StatusLine::normal(format!(
            "Skill linkage: linked {} · feedback lessons {}",
            memory.linked_skill_count, memory.skill_feedback_lesson_count
        )));
        lines.push(StatusLine::normal(format!(
            "Retrieval: runs {} · hits {} · used {}",
            memory.retrieval_run_count, memory.retrieval_hit_count, memory.retrieval_use_count
        )));
        if let Some(query) = memory.last_prefetch_query.as_deref() {
            lines.push(StatusLine::muted(format!("Prefetch query: {}", query)));
        }
        if let Some(run) = memory.latest_consolidation_run.as_ref() {
            lines.push(StatusLine::normal(format!(
                "Latest consolidation: {} · merged {} · promoted {} · conflicts {}",
                run.run_id, run.merged_count, run.promoted_count, run.conflict_count
            )));
        }
        if memory.recent_rule_hits.is_empty() {
            lines.push(StatusLine::muted("Recent rule hits: none"));
        } else {
            lines.push(StatusLine::title(format!(
                "Recent Rule Hits ({})",
                memory.recent_rule_hits.len()
            )));
            for hit in &memory.recent_rule_hits {
                let memory_ref = hit
                    .memory_id
                    .as_ref()
                    .map(|id| id.0.as_str())
                    .unwrap_or("workspace");
                let detail = hit.detail.as_deref().unwrap_or("no detail");
                lines.push(StatusLine::normal(format!(
                    "- {} · {}",
                    hit.hit_kind, memory_ref
                )));
                lines.push(StatusLine::muted(format!("  {}", detail)));
            }
        }
    }

    lines
}

fn tui_usage_status_lines(
    session_id: &str,
    telemetry: &crate::api::SessionTelemetrySnapshot,
    current_context_tokens: Option<u64>,
    current_context_limit: Option<u64>,
    last_turn_tokens: Option<&crate::context::TokenUsage>,
) -> Vec<StatusLine> {
    let usage = &telemetry.usage;
    let mut lines = vec![
        StatusLine::title("Session Usage"),
        StatusLine::normal(format!("Session: {}", session_id)),
    ];

    if let Some(current_tokens) = current_context_tokens {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::title("Current Context"));
        lines.push(StatusLine::normal(format!(
            "Pressure: {}",
            tui_format_context_usage_label(current_tokens, current_context_limit)
        )));
        if let Some(limit) = current_context_limit.filter(|limit| *limit > 0) {
            if let Some(percent) = tui_context_usage_percent(current_tokens, limit) {
                if let Some(note) = tui_context_pressure_note(Some(percent)) {
                    lines.push(StatusLine::muted(format!("State: {}", note)));
                }
            }
        }
    }

    if let Some(tokens) = last_turn_tokens.filter(|tokens| tui_has_turn_usage(tokens)) {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::title("Last Turn"));
        lines.push(StatusLine::normal(tui_format_last_turn_usage(tokens)));
    }

    lines.push(StatusLine::muted(String::new()));
    lines.push(StatusLine::title("Session Cumulative"));
    lines.push(StatusLine::normal(format!(
        "Total {} · Cost ${:.4}",
        tui_format_token_count(tui_total_session_tokens(usage)),
        usage.total_cost
    )));
    lines.push(StatusLine::normal(format!(
        "Input {} · Output {} · Reasoning {}",
        tui_format_token_count(usage.input_tokens),
        tui_format_token_count(usage.output_tokens),
        tui_format_token_count(usage.reasoning_tokens)
    )));
    lines.push(StatusLine::normal(format!(
        "Cache read {} · Cache miss {} · Cache write {}",
        tui_format_token_count(usage.cache_read_tokens),
        tui_format_token_count(usage.cache_miss_tokens),
        tui_format_token_count(usage.cache_write_tokens)
    )));

    if !telemetry.stages.is_empty() {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::title(format!(
            "Stage Totals ({})",
            telemetry.stages.len()
        )));
        for stage in &telemetry.stages {
            lines.push(StatusLine::normal(format_stage_usage_summary_line(stage)));
        }
    }

    lines
}

fn tui_session_insights_lines(
    session_id: &str,
    insights: &crate::api::SessionInsightsResponse,
) -> Vec<StatusLine> {
    let mut lines = vec![
        StatusLine::title("Session Insights"),
        StatusLine::normal(format!("Session: {}", session_id)),
        StatusLine::normal(format!("Title: {}", insights.title)),
        StatusLine::normal(format!("Directory: {}", insights.directory)),
        StatusLine::normal(format!(
            "Updated: {}",
            tui_format_timestamp(insights.updated)
        )),
    ];

    if let Some(telemetry) = insights.telemetry.as_ref() {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::title("Persisted Telemetry"));
        lines.push(StatusLine::normal(format!(
            "Status: {} · Version: {:?}",
            telemetry.last_run_status, telemetry.version
        )));
        lines.push(StatusLine::normal(format!(
            "Session cumulative: total {} · input {} · output {} · reasoning {}",
            tui_format_token_count(tui_total_session_tokens(&telemetry.usage)),
            tui_format_token_count(telemetry.usage.input_tokens),
            tui_format_token_count(telemetry.usage.output_tokens),
            tui_format_token_count(telemetry.usage.reasoning_tokens)
        )));
        lines.push(StatusLine::muted(format!(
            "Cache read {} · Cache miss {} · Cache write {} · Cost ${:.4}",
            tui_format_token_count(telemetry.usage.cache_read_tokens),
            tui_format_token_count(telemetry.usage.cache_miss_tokens),
            tui_format_token_count(telemetry.usage.cache_write_tokens),
            telemetry.usage.total_cost
        )));
        lines.push(StatusLine::muted(format!(
            "Persisted stages: {}",
            telemetry.stage_summaries.len()
        )));
    }

    if let Some(memory) = insights.memory.as_ref() {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::title("Memory Explain"));
        lines.push(StatusLine::normal(format!(
            "Workspace: {} · {}",
            memory.summary.workspace_mode, memory.summary.workspace_key
        )));
        lines.push(StatusLine::normal(format!(
            "Frozen snapshot: {} items{}",
            memory.summary.frozen_snapshot_items,
            tui_optional_generated_at(memory.summary.frozen_snapshot_generated_at)
        )));
        lines.push(StatusLine::normal(format!(
            "Last prefetch: {} items{}",
            memory.summary.last_prefetch_items,
            tui_optional_generated_at(memory.summary.last_prefetch_generated_at)
        )));
        if let Some(query) = memory.summary.last_prefetch_query.as_deref() {
            lines.push(StatusLine::muted(format!("Prefetch query: {}", query)));
        }
        lines.push(StatusLine::normal(format!(
            "Validation pressure: warnings {} · methodology {} · skill targets {}",
            memory.summary.warning_count,
            memory.summary.methodology_candidate_count,
            memory.summary.derived_skill_candidate_count
        )));
        if let Some(run) = memory.summary.latest_consolidation_run.as_ref() {
            lines.push(StatusLine::normal(format!(
                "Latest consolidation: {} · merged {} · promoted {} · conflicts {}",
                run.run_id, run.merged_count, run.promoted_count, run.conflict_count
            )));
        }
        if !memory.summary.recent_rule_hits.is_empty() {
            lines.push(StatusLine::title(format!(
                "Recent Rule Hits ({})",
                memory.summary.recent_rule_hits.len()
            )));
            for hit in &memory.summary.recent_rule_hits {
                lines.push(StatusLine::normal(format!("- {}", hit.hit_kind)));
                if let Some(detail) = hit.detail.as_deref() {
                    lines.push(StatusLine::muted(format!("  {}", detail)));
                }
            }
        }
        if let Some(packet) = memory.frozen_snapshot.as_ref() {
            lines.push(StatusLine::muted(format!(
                "Frozen scopes: {}",
                packet
                    .scopes
                    .iter()
                    .map(|scope| format!("{scope:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        if let Some(packet) = memory.last_prefetch_packet.as_ref() {
            lines.push(StatusLine::muted(format!(
                "Prefetch scopes: {}",
                packet
                    .scopes
                    .iter()
                    .map(|scope| format!("{scope:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let skill_linked = memory
            .recent_session_records
            .iter()
            .filter(|item| item.linked_skill_name.is_some() || item.derived_skill_name.is_some())
            .take(3)
            .collect::<Vec<_>>();
        if !skill_linked.is_empty() {
            lines.push(StatusLine::title("Skill-Linked Recent Records"));
            for item in skill_linked {
                lines.push(StatusLine::normal(format!("- {}", item.title)));
                lines.push(StatusLine::muted(format!(
                    "  linked={} · target={}",
                    item.linked_skill_name.as_deref().unwrap_or("--"),
                    item.derived_skill_name.as_deref().unwrap_or("--")
                )));
            }
        }
        let suggested_ids = memory
            .summary
            .recent_rule_hits
            .iter()
            .filter_map(|hit| hit.memory_id.as_ref().map(|id| id.0.as_str()))
            .chain(
                memory
                    .last_prefetch_packet
                    .iter()
                    .flat_map(|packet| packet.items.iter().map(|item| item.card.id.0.as_str())),
            )
            .take(3)
            .collect::<Vec<_>>();
        if !suggested_ids.is_empty() {
            lines.push(StatusLine::title("Follow-up Commands"));
            for record_id in suggested_ids {
                lines.push(StatusLine::muted(format!("/memory show {}", record_id)));
                lines.push(StatusLine::muted(format!(
                    "/memory hits record={}",
                    record_id
                )));
            }
        }
        if let Some(run) = memory.summary.latest_consolidation_run.as_ref() {
            lines.push(StatusLine::muted(format!(
                "/memory hits run={}",
                run.run_id
            )));
        }
    }

    if let Some(multimodal) = insights.multimodal.as_ref() {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::title("Multimodal Explain"));
        lines.push(StatusLine::normal(multimodal.display_label().into_owned()));
        lines.push(StatusLine::normal(format!(
            "Message: {} · attachments {} · hard block {}",
            multimodal.user_message_id,
            multimodal.attachment_count,
            if multimodal.hard_block { "yes" } else { "no" }
        )));
        lines.push(StatusLine::normal(format!(
            "Resolved model: {}",
            multimodal.resolved_model.as_deref().unwrap_or("--")
        )));
        lines.push(StatusLine::normal(format!(
            "Kinds: {}",
            if multimodal.kinds.is_empty() {
                "--".to_string()
            } else {
                multimodal.kinds.join(", ")
            }
        )));
        lines.push(StatusLine::normal(format!(
            "Badges: {}",
            if multimodal.badges.is_empty() {
                "--".to_string()
            } else {
                multimodal.badges.join(", ")
            }
        )));
        lines.push(StatusLine::normal(format!(
            "Unsupported parts: {}",
            if multimodal.unsupported_parts.is_empty() {
                "none".to_string()
            } else {
                multimodal.unsupported_parts.join(", ")
            }
        )));
        lines.push(StatusLine::normal(format!(
            "Recommended downgrade: {}",
            multimodal
                .recommended_downgrade
                .as_deref()
                .unwrap_or("none")
        )));
        lines.push(StatusLine::normal(format!(
            "Transport replaced parts: {}",
            if multimodal.transport_replaced_parts.is_empty() {
                "none".to_string()
            } else {
                multimodal.transport_replaced_parts.join(", ")
            }
        )));
        if !multimodal.attachments.is_empty() {
            lines.push(StatusLine::title("Attachments"));
            for attachment in &multimodal.attachments {
                lines.push(StatusLine::normal(format!("- {}", attachment.filename)));
                lines.push(StatusLine::muted(format!("  {}", attachment.mime)));
            }
        }
        let combined_warnings = multimodal.combined_warnings();
        if !combined_warnings.is_empty() {
            lines.push(StatusLine::title("Warnings"));
            for warning in combined_warnings {
                lines.push(StatusLine::muted(warning));
            }
        }
    }

    if let Some(policy) = insights.effective_policy.as_ref() {
        lines.push(StatusLine::muted(String::new()));
        lines.extend(tui_effective_policy_lines(policy));
    }

    lines
}

fn tui_effective_policy_lines(
    policy: &rocode_types::SessionEffectivePolicyView,
) -> Vec<StatusLine> {
    let mut lines = vec![
        StatusLine::title("Effective Policy"),
        StatusLine::normal(format!("Session: {}", policy.session_id)),
    ];

    if let Some(scheduler) = policy.scheduler.as_ref() {
        lines.push(StatusLine::normal(format!(
            "Scheduler: requested {} · effective {} · source {} · applied {}",
            scheduler.requested_profile.as_deref().unwrap_or("--"),
            scheduler.effective_profile.as_deref().unwrap_or("--"),
            scheduler.source,
            tui_yes_no(scheduler.applied)
        )));
        if scheduler.mode_kind.is_some()
            || scheduler.root_agent.is_some()
            || scheduler.resolved_agent.is_some()
        {
            lines.push(StatusLine::muted(format!(
                "Mode {} · root agent {} · resolved agent {}",
                scheduler.mode_kind.as_deref().unwrap_or("--"),
                scheduler.root_agent.as_deref().unwrap_or("--"),
                scheduler.resolved_agent.as_deref().unwrap_or("--")
            )));
        }
        if !scheduler.selection_trace.is_empty() {
            lines.push(StatusLine::muted(format!(
                "Trace {}",
                scheduler
                    .selection_trace
                    .iter()
                    .map(|step| {
                        let mut parts =
                            vec![tui_scheduler_trace_step_kind_label(&step.kind).to_string()];
                        if let Some(profile) = step.profile.as_deref() {
                            parts.push(profile.to_string());
                        }
                        if let Some(detail) = step.detail.as_deref() {
                            parts.push(detail.to_string());
                        }
                        parts.push(format!("applied {}", tui_yes_no(step.applied)));
                        parts.join(" · ")
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            )));
        }
        if let Some(warning) = scheduler.warning.as_deref() {
            lines.push(StatusLine::warning(format!("Warning {}", warning)));
        }
    }

    if let Some(provider) = policy.provider.as_ref() {
        lines.push(StatusLine::normal(format!(
            "Provider: {} · variant {}",
            provider.resolved_model,
            provider.variant.as_deref().unwrap_or("--")
        )));
        if let Some(descriptor) = provider.configured_descriptor.as_ref() {
            lines.push(StatusLine::muted(format!(
                "Configured descriptor: base {} · env {}",
                descriptor.base_url.as_deref().unwrap_or("--"),
                tui_join_or_placeholder(&descriptor.env)
            )));
            if let Some(profile) = descriptor.profile.as_ref() {
                lines.push(StatusLine::muted(format!(
                    "Configured profile: {}",
                    tui_provider_profile_summary(profile)
                )));
            }
        }
        if let Some(error) = provider.configured_descriptor_error.as_deref() {
            lines.push(StatusLine::warning(format!(
                "Descriptor projection error: {}",
                error
            )));
        }
        if let Some(runtime) = provider.runtime_profile.as_ref() {
            lines.push(StatusLine::muted(format!(
                "Runtime profile: {} · hash {}",
                tui_provider_profile_summary(&runtime.profile),
                runtime.profile_hash
            )));
        }
    }

    if let Some(skill_tree) = policy.skill_tree.as_ref() {
        lines.push(StatusLine::normal(format!(
            "Skill tree: configured {} · enabled {} · applied {} · source {}",
            tui_yes_no(skill_tree.configured),
            tui_yes_no(skill_tree.enabled),
            tui_yes_no(skill_tree.applied),
            skill_tree.source
        )));
        if skill_tree.estimated_tokens.is_some()
            || skill_tree.token_budget.is_some()
            || skill_tree.truncation_strategy.is_some()
            || skill_tree.truncated.is_some()
        {
            lines.push(StatusLine::muted(format!(
                "Estimated {} · budget {} · truncation {} · truncated {}",
                skill_tree
                    .estimated_tokens
                    .map(tui_format_token_count)
                    .unwrap_or_else(|| "--".to_string()),
                skill_tree
                    .token_budget
                    .map(tui_format_token_count)
                    .unwrap_or_else(|| "--".to_string()),
                skill_tree.truncation_strategy.as_deref().unwrap_or("--"),
                skill_tree.truncated.map(tui_yes_no).unwrap_or("--")
            )));
        }
    }

    if let Some(memory) = policy.memory.as_ref() {
        lines.push(StatusLine::normal(format!(
            "Memory: {} · {}",
            memory.workspace_mode, memory.workspace_key
        )));
        lines.push(StatusLine::muted(format!(
            "Scopes {} · frozen snapshot {} · last prefetch {}",
            if memory.allowed_scopes.is_empty() {
                "--".to_string()
            } else {
                memory
                    .allowed_scopes
                    .iter()
                    .map(|scope| format!("{scope:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            memory.frozen_snapshot_items,
            memory.last_prefetch_items
        )));
    }

    lines.push(StatusLine::normal(format!(
        "Compaction: auto {} · prune {} · reserved {}",
        tui_yes_no(policy.compaction.auto),
        tui_yes_no(policy.compaction.prune),
        policy
            .compaction
            .reserved
            .map(tui_format_token_count)
            .unwrap_or_else(|| "--".to_string())
    )));

    if let Some(external) = policy.external_adapter.as_ref() {
        lines.push(StatusLine::normal(format!(
            "External adapter: source {} · policy {} · batch {}",
            external.last_ingress_source,
            external.last_ingress_policy.as_deref().unwrap_or("--"),
            external
                .last_ingress_batch_count
                .map(|value: u64| value.to_string())
                .unwrap_or_else(|| "--".to_string())
        )));
    }

    if !policy.warnings.is_empty() {
        lines.push(StatusLine::title("Warnings"));
        for warning in &policy.warnings {
            lines.push(StatusLine::warning(warning.clone()));
        }
    }

    lines
}

fn tui_provider_profile_summary(profile: &rocode_types::ProviderProfileDescriptorView) -> String {
    let mut parts = vec![
        format!("source {}", profile.source),
        format!("family {}", profile.api_family),
        format!("shape {}", profile.api_shape),
        format!("transport {}", profile.transport),
        format!("usage {}", profile.usage_shape),
        format!("cache {}", profile.cache_family),
    ];
    if !profile.quirks.is_empty() {
        parts.push(format!("quirks {}", profile.quirks.join(", ")));
    }
    parts.join(" · ")
}

fn tui_join_or_placeholder(values: &[String]) -> String {
    if values.is_empty() {
        "--".to_string()
    } else {
        values.join(", ")
    }
}

fn tui_yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn tui_scheduler_trace_step_kind_label(
    kind: &rocode_types::SessionEffectiveSchedulerTraceStepKind,
) -> &'static str {
    match kind {
        rocode_types::SessionEffectiveSchedulerTraceStepKind::RequestedProfile => {
            "requested_profile"
        }
        rocode_types::SessionEffectiveSchedulerTraceStepKind::CommandWorkflowOverride => {
            "command_workflow_override"
        }
        rocode_types::SessionEffectiveSchedulerTraceStepKind::SessionPinnedProfile => {
            "session_pinned_profile"
        }
        rocode_types::SessionEffectiveSchedulerTraceStepKind::LegacySessionPinnedProfile => {
            "legacy_session_pinned_profile"
        }
        rocode_types::SessionEffectiveSchedulerTraceStepKind::ConfigDefaultProfile => {
            "config_default_profile"
        }
        rocode_types::SessionEffectiveSchedulerTraceStepKind::AutoRoute => "auto_route",
        rocode_types::SessionEffectiveSchedulerTraceStepKind::SoftFallback => "soft_fallback",
    }
}

fn tui_config_validation_lines(
    snapshot: &rocode_types::ConfigPolicyValidationSnapshot,
) -> Vec<StatusLine> {
    let error_count = snapshot
        .reports
        .iter()
        .filter(|item| item.severity == rocode_types::ConfigPolicyValidationSeverity::Error)
        .count();
    let warning_count = snapshot
        .reports
        .iter()
        .filter(|item| item.severity == rocode_types::ConfigPolicyValidationSeverity::Warning)
        .count();

    let mut lines = vec![
        StatusLine::title("Workspace Config Validation"),
        StatusLine::normal("Source: /config/validation"),
        StatusLine::normal(format!("Revision: {}", snapshot.revision)),
        StatusLine::normal(format!(
            "Generated: {}",
            tui_format_timestamp(snapshot.generated_at_ms)
        )),
        StatusLine::normal(format!(
            "Findings: {} ({} errors, {} warnings)",
            snapshot.reports.len(),
            error_count,
            warning_count
        )),
    ];

    if snapshot.reports.is_empty() {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::muted(
            "No validation findings are present in the current config snapshot.",
        ));
        return lines;
    }

    let mut grouped: std::collections::BTreeMap<
        rocode_types::ConfigPolicyValidationOwner,
        Vec<&rocode_types::ConfigPolicyValidationItem>,
    > = std::collections::BTreeMap::new();
    for item in &snapshot.reports {
        grouped.entry(item.owner).or_default().push(item);
    }

    for (owner, items) in grouped {
        lines.push(StatusLine::muted(String::new()));
        lines.push(StatusLine::title(format!(
            "{} ({})",
            tui_config_validation_owner_label(owner),
            items.len()
        )));
        for item in items {
            let headline = format!(
                "[{}] {} · {}",
                tui_config_validation_severity_label(item.severity),
                item.code,
                item.path
            );
            lines.push(match item.severity {
                rocode_types::ConfigPolicyValidationSeverity::Warning => {
                    StatusLine::warning(headline)
                }
                rocode_types::ConfigPolicyValidationSeverity::Error => StatusLine::error(headline),
            });
            lines.push(StatusLine::muted(format!(
                "  Scope: {}",
                tui_config_validation_scope_label(
                    item.scope.kind,
                    item.scope.subject_id.as_deref()
                )
            )));
            lines.push(StatusLine::muted(format!(
                "  Effect: {}",
                tui_config_validation_effect_label(item.effect)
            )));
            lines.push(StatusLine::muted(format!("  Message: {}", item.message)));
            if let Some(fallback) = item.fallback.as_deref() {
                lines.push(StatusLine::muted(format!("  Fallback: {}", fallback)));
            }
        }
    }

    lines
}

fn tui_config_validation_owner_label(
    owner: rocode_types::ConfigPolicyValidationOwner,
) -> &'static str {
    match owner {
        rocode_types::ConfigPolicyValidationOwner::Scheduler => "Scheduler",
        rocode_types::ConfigPolicyValidationOwner::SkillTree => "Skill Tree",
        rocode_types::ConfigPolicyValidationOwner::ProviderProfile => "Provider Profile",
        rocode_types::ConfigPolicyValidationOwner::ExternalAdapter => "External Adapter",
    }
}

fn tui_config_validation_severity_label(
    severity: rocode_types::ConfigPolicyValidationSeverity,
) -> &'static str {
    match severity {
        rocode_types::ConfigPolicyValidationSeverity::Warning => "warning",
        rocode_types::ConfigPolicyValidationSeverity::Error => "error",
    }
}

fn tui_config_validation_effect_label(
    effect: rocode_types::ConfigPolicyValidationEffect,
) -> &'static str {
    match effect {
        rocode_types::ConfigPolicyValidationEffect::SoftFallback => "soft fallback",
        rocode_types::ConfigPolicyValidationEffect::FailClosedBootstrap => "fail-closed bootstrap",
        rocode_types::ConfigPolicyValidationEffect::FailClosedRequestGate => {
            "fail-closed request gate"
        }
    }
}

fn tui_config_validation_scope_label(
    kind: rocode_types::ConfigPolicyValidationScopeKind,
    subject_id: Option<&str>,
) -> String {
    let base = match kind {
        rocode_types::ConfigPolicyValidationScopeKind::SchedulerPath => "scheduler path",
        rocode_types::ConfigPolicyValidationScopeKind::SkillTree => "skill tree",
        rocode_types::ConfigPolicyValidationScopeKind::Provider => "provider",
        rocode_types::ConfigPolicyValidationScopeKind::ExternalAdapter => "external adapter",
    };
    match subject_id {
        Some(id) if !id.is_empty() => format!("{base} · {id}"),
        _ => base.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::SessionInsightsResponse;
    use rocode_types::{
        ConfigPolicyValidationEffect, ConfigPolicyValidationItem, ConfigPolicyValidationOwner,
        ConfigPolicyValidationScope, ConfigPolicyValidationScopeKind,
        ConfigPolicyValidationSeverity, ConfigPolicyValidationSnapshot, MemoryScope,
        ProviderConnectionDescriptorCandidate, ProviderProfileDescriptorView,
        SessionEffectiveCompactionPolicy, SessionEffectiveExternalAdapterPolicy,
        SessionEffectiveMemoryPolicy, SessionEffectivePolicyView, SessionEffectiveProviderPolicy,
        SessionEffectiveProviderRuntimeProfile, SessionEffectiveSchedulerPolicy,
        SessionEffectiveSchedulerTraceStep, SessionEffectiveSchedulerTraceStepKind,
        SessionEffectiveSkillTreePolicy,
    };

    #[test]
    fn session_insights_surface_effective_policy_sections() {
        let insights = SessionInsightsResponse {
            id: "sess_123".to_string(),
            title: "Session title".to_string(),
            directory: "/workspace/project".to_string(),
            updated: 1_714_560_000_000,
            telemetry: None,
            memory: None,
            multimodal: None,
            effective_policy: Some(SessionEffectivePolicyView {
                session_id: "sess_123".to_string(),
                scheduler: Some(SessionEffectiveSchedulerPolicy {
                    requested_profile: Some("prometheus".to_string()),
                    effective_profile: Some("prometheus".to_string()),
                    source: "session_pinned_profile".to_string(),
                    applied: true,
                    mode_kind: Some("orchestrator".to_string()),
                    root_agent: Some("planner".to_string()),
                    resolved_agent: Some("planner".to_string()),
                    selection_trace: vec![SessionEffectiveSchedulerTraceStep {
                        kind: SessionEffectiveSchedulerTraceStepKind::SessionPinnedProfile,
                        profile: Some("prometheus".to_string()),
                        detail: Some(
                            "session metadata pinned this scheduler profile".to_string(),
                        ),
                        applied: true,
                    }],
                    warning: Some(
                        "configured scheduler defaults could not be resolved; continuing without scheduler profile"
                            .to_string(),
                    ),
                }),
                provider: Some(SessionEffectiveProviderPolicy {
                    provider_id: "openai".to_string(),
                    model_id: "gpt-4o".to_string(),
                    resolved_model: "openai/gpt-4o".to_string(),
                    variant: Some("fast".to_string()),
                    configured_descriptor: Some(ProviderConnectionDescriptorCandidate {
                        provider_id: "openai".to_string(),
                        name: Some("OpenAI".to_string()),
                        base_url: Some("https://api.openai.com/v1".to_string()),
                        env: vec!["OPENAI_API_KEY".to_string()],
                        profile: Some(ProviderProfileDescriptorView {
                            provider_id: "openai".to_string(),
                            npm: "@ai-sdk/openai".to_string(),
                            source: "bundled_default".to_string(),
                            api_family: "closeai-compatible".to_string(),
                            api_shape: "chat-completions".to_string(),
                            transport: "bearer".to_string(),
                            usage_shape: "closeai-cached-tokens".to_string(),
                            cache_family: "closeai-compatible".to_string(),
                            quirks: vec!["responses-fallback-to-chat".to_string()],
                        }),
                    }),
                    configured_descriptor_error: None,
                    runtime_profile: Some(SessionEffectiveProviderRuntimeProfile {
                        profile: ProviderProfileDescriptorView {
                            provider_id: "openai".to_string(),
                            npm: "@ai-sdk/openai".to_string(),
                            source: "runtime_fingerprint".to_string(),
                            api_family: "closeai-compatible".to_string(),
                            api_shape: "responses".to_string(),
                            transport: "bearer".to_string(),
                            usage_shape: "closeai-cached-tokens".to_string(),
                            cache_family: "closeai-compatible".to_string(),
                            quirks: Vec::new(),
                        },
                        profile_hash: "1234567890abcdef".to_string(),
                    }),
                }),
                skill_tree: Some(SessionEffectiveSkillTreePolicy {
                    configured: true,
                    enabled: true,
                    applied: true,
                    source: "config_composition".to_string(),
                    estimated_tokens: Some(256),
                    token_budget: Some(512),
                    truncation_strategy: Some("tail".to_string()),
                    truncated: Some(false),
                }),
                memory: Some(SessionEffectiveMemoryPolicy {
                    workspace_key: "/workspace/project".to_string(),
                    workspace_mode: "workspace_shared".to_string(),
                    allowed_scopes: vec![
                        MemoryScope::WorkspaceShared,
                        MemoryScope::SessionEphemeral,
                    ],
                    frozen_snapshot_items: 2,
                    last_prefetch_items: 5,
                }),
                compaction: SessionEffectiveCompactionPolicy {
                    auto: false,
                    prune: true,
                    reserved: Some(512),
                },
                external_adapter: Some(SessionEffectiveExternalAdapterPolicy {
                    last_ingress_source: "external:generic-webhook:generic".to_string(),
                    last_ingress_policy: Some("external_adapter_metadata_only".to_string()),
                    last_ingress_batch_count: Some(1),
                }),
                warnings: vec![
                    "provider descriptor projection failed for `openai`: invalid profile"
                        .to_string(),
                ],
            }),
        };

        let texts = tui_session_insights_lines("sess_123", &insights)
            .into_iter()
            .map(|line| line.text)
            .collect::<Vec<_>>();

        assert!(texts.iter().any(|line| line == "Effective Policy"));
        assert!(texts.iter().any(|line| line == "Session: sess_123"));
        assert!(texts.iter().any(|line| {
            line.contains("Scheduler: requested prometheus")
                && line.contains("source session_pinned_profile")
                && line.contains("applied yes")
        }));
        assert!(texts
            .iter()
            .any(|line| line.contains("Trace session_pinned_profile")));
        assert!(texts.iter().any(
            |line| line.contains("Warning configured scheduler defaults could not be resolved")
        ));
        assert!(texts
            .iter()
            .any(|line| line.contains("Provider: openai/gpt-4o · variant fast")));
        assert!(texts.iter().any(|line| {
            line.contains("Configured profile:")
                && line.contains("family closeai-compatible")
                && line.contains("quirks responses-fallback-to-chat")
        }));
        assert!(texts
            .iter()
            .any(|line| line.contains("Runtime profile:") && line.contains("shape responses")));
        assert!(texts.iter().any(|line| {
            line.contains("Skill tree: configured yes")
                && line.contains("source config_composition")
        }));
        assert!(texts.iter().any(|line| {
            line.contains("Memory: workspace_shared") && line.contains("/workspace/project")
        }));
        assert!(texts.iter().any(|line| {
            line.contains("Scopes WorkspaceShared, SessionEphemeral")
                && line.contains("frozen snapshot 2")
        }));
        assert!(texts.iter().any(|line| {
            line.contains("Compaction: auto no")
                && line.contains("prune yes")
                && line.contains("reserved 512")
        }));
        assert!(texts.iter().any(|line| {
            line.contains("External adapter: source external:generic-webhook:generic")
                && line.contains("policy external_adapter_metadata_only")
        }));
        assert!(texts
            .iter()
            .any(|line| { line.contains("provider descriptor projection failed") }));
    }

    #[test]
    fn config_validation_surface_groups_findings() {
        let snapshot = ConfigPolicyValidationSnapshot {
            revision: 7,
            generated_at_ms: 1_714_560_000_000,
            reports: vec![
                ConfigPolicyValidationItem {
                    owner: ConfigPolicyValidationOwner::Scheduler,
                    scope: ConfigPolicyValidationScope {
                        kind: ConfigPolicyValidationScopeKind::SchedulerPath,
                        subject_id: Some("auto".to_string()),
                    },
                    path: "scheduler.auto.default_profile".to_string(),
                    severity: ConfigPolicyValidationSeverity::Error,
                    effect: ConfigPolicyValidationEffect::SoftFallback,
                    code: "scheduler_profile_missing".to_string(),
                    message: "scheduler default profile could not be resolved".to_string(),
                    fallback: Some(
                        "scheduler auto route will continue without pinned profile".to_string(),
                    ),
                },
                ConfigPolicyValidationItem {
                    owner: ConfigPolicyValidationOwner::ProviderProfile,
                    scope: ConfigPolicyValidationScope {
                        kind: ConfigPolicyValidationScopeKind::Provider,
                        subject_id: Some("openai".to_string()),
                    },
                    path: "providers.openai.profile.api_shape".to_string(),
                    severity: ConfigPolicyValidationSeverity::Warning,
                    effect: ConfigPolicyValidationEffect::FailClosedBootstrap,
                    code: "provider_profile_shape_unknown".to_string(),
                    message:
                        "api_shape is unknown and the provider cannot bootstrap a typed profile"
                            .to_string(),
                    fallback: None,
                },
            ],
        };

        let texts = tui_config_validation_lines(&snapshot)
            .into_iter()
            .map(|line| line.text)
            .collect::<Vec<_>>();

        assert!(texts
            .iter()
            .any(|line| line == "Workspace Config Validation"));
        assert!(texts
            .iter()
            .any(|line| line == "Source: /config/validation"));
        assert!(texts
            .iter()
            .any(|line| line.contains("Findings: 2 (1 errors, 1 warnings)")));
        assert!(texts.iter().any(|line| line == "Scheduler (1)"));
        assert!(texts
            .iter()
            .any(|line| line.contains("[error] scheduler_profile_missing")));
        assert!(texts
            .iter()
            .any(|line| line.contains("Scope: scheduler path · auto")));
        assert!(texts.iter().any(|line| line
            .contains("Fallback: scheduler auto route will continue without pinned profile")));
        assert!(texts.iter().any(|line| line == "Provider Profile (1)"));
        assert!(texts
            .iter()
            .any(|line| line.contains("[warning] provider_profile_shape_unknown")));
        assert!(texts
            .iter()
            .any(|line| line.contains("Scope: provider · openai")));
    }
}

fn tui_events_query(
    input: &rocode_command::interactive::InteractiveEventsQuery,
    offset: usize,
) -> crate::api::SessionEventsQuery {
    crate::api::SessionEventsQuery {
        stage_id: input.stage_id.clone(),
        execution_id: input.execution_id.clone(),
        event_type: input.event_type.clone(),
        since: input.since,
        limit: input.limit,
        offset: Some(offset),
    }
}

fn tui_format_timestamp(ts: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts)
        .map(|value| value.with_timezone(&chrono::Local))
        .map(|value| value.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| ts.to_string())
}

fn tui_optional_generated_at(ts: Option<i64>) -> String {
    ts.and_then(|value| chrono::DateTime::<chrono::Utc>::from_timestamp_millis(value))
        .map(|value| value.with_timezone(&chrono::Local))
        .map(|value| format!(" @ {}", value.format("%Y-%m-%d %H:%M:%S")))
        .unwrap_or_default()
}

fn tui_events_filter_label(input: &rocode_command::interactive::InteractiveEventsQuery) -> String {
    let mut parts = Vec::new();
    if let Some(stage_id) = input.stage_id.as_deref() {
        parts.push(format!("stage={stage_id}"));
    }
    if let Some(execution_id) = input.execution_id.as_deref() {
        parts.push(format!("exec={execution_id}"));
    }
    if let Some(event_type) = input.event_type.as_deref() {
        parts.push(format!("type={event_type}"));
    }
    if let Some(since) = input.since {
        parts.push(format!("since={since}"));
    }
    parts.push(format!(
        "limit={}",
        rocode_command::interactive::events_browser_page_size(input)
    ));
    parts.join(" · ")
}

fn tui_events_window_label(offset: usize, count: usize) -> String {
    if count == 0 {
        return "items 0".to_string();
    }
    format!("items {}-{}", offset + 1, offset + count)
}

fn tui_event_payload_summary(payload: &serde_json::Value) -> Option<String> {
    match payload {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(text.trim().to_string()),
        value => serde_json::to_string(value).ok(),
    }
    .filter(|text| !text.is_empty())
    .map(|text| {
        let compact = text.replace('\n', " ");
        if compact.chars().count() > 140 {
            let truncated = compact.chars().take(137).collect::<String>();
            format!("{}...", truncated)
        } else {
            compact
        }
    })
}

fn tui_event_status_lines(
    events: &[rocode_command::stage_protocol::StageEvent],
) -> Vec<StatusLine> {
    let mut lines = Vec::new();
    for event in events {
        let ts = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(event.ts)
            .map(|value| value.with_timezone(&chrono::Local))
            .map(|value| value.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| event.ts.to_string());
        let mut headline = format!("{} · {} · {:?}", ts, event.event_type, event.scope);
        if let Some(stage_id) = event.stage_id.as_deref() {
            headline.push_str(&format!(" · stage {}", stage_id));
        }
        if let Some(execution_id) = event.execution_id.as_deref() {
            headline.push_str(&format!(" · exec {}", execution_id));
        }
        lines.push(StatusLine::normal(headline));
        if let Some(payload) = tui_event_payload_summary(&event.payload) {
            lines.push(StatusLine::muted(format!("  {}", payload)));
        }
    }
    lines
}

fn format_stage_status(status: rocode_command::stage_protocol::StageStatus) -> &'static str {
    match status {
        rocode_command::stage_protocol::StageStatus::Running => "running",
        rocode_command::stage_protocol::StageStatus::Waiting => "waiting",
        rocode_command::stage_protocol::StageStatus::Done => "done",
        rocode_command::stage_protocol::StageStatus::Cancelled => "cancelled",
        rocode_command::stage_protocol::StageStatus::Cancelling => "cancelling",
        rocode_command::stage_protocol::StageStatus::Blocked => "blocked",
        rocode_command::stage_protocol::StageStatus::Retrying => "retrying",
    }
}

fn format_stage_summary_line(stage: &rocode_command::stage_protocol::StageSummary) -> String {
    let mut suffix = Vec::new();
    if let (Some(index), Some(total)) = (stage.index, stage.total) {
        suffix.push(format!("{}/{}", index, total));
    }
    if let (Some(step), Some(step_total)) = (stage.step, stage.step_total) {
        suffix.push(format!("step {}/{}", step, step_total));
    }
    if let Some(waiting_on) = stage.waiting_on.as_deref() {
        suffix.push(format!("waiting {}", waiting_on));
    }
    let suffix = if suffix.is_empty() {
        String::new()
    } else {
        format!(" [{}]", suffix.join(" · "))
    };
    format!(
        "- {} ({}){}",
        stage.stage_name,
        format_stage_status(stage.status),
        suffix
    )
}

fn format_stage_runtime_line(stage: &rocode_command::stage_protocol::StageSummary) -> String {
    let mut parts = vec![format!(
        "- {} ({})",
        stage.stage_name,
        format_stage_status(stage.status.clone())
    )];
    if let (Some(index), Some(total)) = (stage.index, stage.total) {
        parts.push(format!("{}/{}", index, total));
    }
    if let (Some(step), Some(step_total)) = (stage.step, stage.step_total) {
        parts.push(format!("step {}/{}", step, step_total));
    }
    if let Some(waiting_on) = stage.waiting_on.as_deref() {
        parts.push(format!("waiting {}", waiting_on));
    }
    if let Some(retry_attempt) = stage.retry_attempt {
        parts.push(format!("retry {}", retry_attempt));
    }
    if stage.active_agent_count > 0 {
        parts.push(format!("agents {}", stage.active_agent_count));
    }
    if stage.active_tool_count > 0 {
        parts.push(format!("tools {}", stage.active_tool_count));
    }
    if stage.child_session_count > 0 {
        parts.push(format!("child {}", stage.child_session_count));
    }
    if let Some(budget) = stage.skill_tree_budget {
        parts.push(format!(
            "budget {}{}",
            budget,
            if stage.skill_tree_truncated.unwrap_or(false) {
                " truncated"
            } else {
                ""
            }
        ));
    }
    if let Some(tokens) = stage.context_tokens.or(stage.estimated_context_tokens) {
        parts.push(format!("ctx {}", tui_format_token_count(tokens)));
    }
    parts.join(" · ")
}

fn format_stage_usage_summary_line(stage: &rocode_command::stage_protocol::StageSummary) -> String {
    let mut parts = vec![format!(
        "- {} ({})",
        stage.stage_name,
        format_stage_status(stage.status.clone())
    )];
    if let Some(prompt_tokens) = stage.prompt_tokens {
        parts.push(format!("in {}", tui_format_token_count(prompt_tokens)));
    }
    if let Some(completion_tokens) = stage.completion_tokens {
        parts.push(format!("out {}", tui_format_token_count(completion_tokens)));
    }
    if let Some(reasoning_tokens) = stage.reasoning_tokens.filter(|value| *value > 0) {
        parts.push(format!(
            "reason {}",
            tui_format_token_count(reasoning_tokens)
        ));
    }
    if let Some(cache_read_tokens) = stage.cache_read_tokens.filter(|value| *value > 0) {
        parts.push(format!(
            "cache-r {}",
            tui_format_token_count(cache_read_tokens)
        ));
    }
    if let Some(cache_miss_tokens) = stage.cache_miss_tokens.filter(|value| *value > 0) {
        parts.push(format!(
            "cache-m {}",
            tui_format_token_count(cache_miss_tokens)
        ));
    }
    if let Some(cache_write_tokens) = stage.cache_write_tokens.filter(|value| *value > 0) {
        parts.push(format!(
            "cache-w {}",
            tui_format_token_count(cache_write_tokens)
        ));
    }
    if let Some(budget) = stage.skill_tree_budget {
        parts.push(format!(
            "budget {}{}",
            budget,
            if stage.skill_tree_truncated.unwrap_or(false) {
                " truncated"
            } else {
                ""
            }
        ));
    }
    if let Some(waiting_on) = stage.waiting_on.as_deref() {
        parts.push(format!("waiting {}", waiting_on));
    }
    if let Some(retry_attempt) = stage.retry_attempt {
        parts.push(format!("retry {}", retry_attempt));
    }
    parts.join(" · ")
}

fn tui_total_session_tokens(usage: &rocode_session::SessionUsage) -> u64 {
    usage.input_tokens + usage.output_tokens + usage.reasoning_tokens
}

fn tui_context_usage_percent(used: u64, limit: u64) -> Option<u64> {
    rocode_types::context_usage_percent(used, limit)
}

fn tui_context_pressure_note(percent: Option<u64>) -> Option<&'static str> {
    rocode_types::context_pressure_label(percent)
}

fn tui_format_context_usage_label(used: u64, limit: Option<u64>) -> String {
    let Some(limit) = limit.filter(|limit| *limit > 0) else {
        return tui_format_token_count(used);
    };

    let percent = tui_context_usage_percent(used, limit).unwrap_or(0);
    format!(
        "{}/{} ({}%)",
        tui_format_token_count(used),
        tui_format_token_count(limit),
        percent
    )
}

fn tui_has_turn_usage(tokens: &crate::context::TokenUsage) -> bool {
    tokens.input > 0
        || tokens.output > 0
        || tokens.reasoning > 0
        || tokens.cache_read > 0
        || tokens.cache_miss > 0
        || tokens.cache_write > 0
}

fn tui_format_last_turn_usage(tokens: &crate::context::TokenUsage) -> String {
    let mut parts = vec![
        format!("Input {}", tui_format_token_count(tokens.input)),
        format!("Output {}", tui_format_token_count(tokens.output)),
    ];
    if tokens.reasoning > 0 {
        parts.push(format!(
            "Reasoning {}",
            tui_format_token_count(tokens.reasoning)
        ));
    }
    if tokens.cache_read > 0 || tokens.cache_miss > 0 || tokens.cache_write > 0 {
        if tokens.cache_miss > 0 {
            parts.push(format!(
                "Cache H/M {}/{}",
                tui_format_token_count(tokens.cache_read),
                tui_format_token_count(tokens.cache_miss)
            ));
        } else {
            parts.push(format!(
                "Cache R/W {}/{}",
                tui_format_token_count(tokens.cache_read),
                tui_format_token_count(tokens.cache_write)
            ));
        }
    }
    parts.join(" · ")
}

fn tui_format_token_count(value: u64) -> String {
    if value >= 1_000_000 {
        let compact = value as f64 / 1_000_000.0;
        return if compact.fract() == 0.0 {
            format!("{compact:.0}M")
        } else {
            format!("{compact:.1}M")
        };
    }
    if value >= 1_000 {
        let compact = value as f64 / 1_000.0;
        return if compact.fract() == 0.0 {
            format!("{compact:.0}K")
        } else {
            format!("{compact:.1}K")
        };
    }
    value.to_string()
}

fn active_stage_status_blocks(
    stage: &rocode_command::stage_protocol::StageSummary,
) -> Vec<StatusBlock> {
    let mut blocks = vec![StatusBlock::title(format!(
        "Active Stage Detail ({})",
        stage.stage_name
    ))];
    if let Some(waiting_on) = stage.waiting_on.as_deref() {
        blocks.push(StatusBlock::warning(format!("Waiting on: {}", waiting_on)));
    }
    if let Some(last_event) = stage.last_event.as_deref() {
        blocks.push(StatusBlock::muted(format!("Last event: {}", last_event)));
    }
    if let Some(budget) = stage.skill_tree_budget {
        blocks.push(StatusBlock::normal(format!(
            "Skill tree budget: {}{}",
            budget,
            stage
                .skill_tree_truncated
                .unwrap_or(false)
                .then_some(" (truncated)")
                .unwrap_or("")
        )));
    }
    if let Some(strategy) = stage.skill_tree_truncation_strategy.as_deref() {
        blocks.push(StatusBlock::muted(format!(
            "Truncation strategy: {}",
            strategy
        )));
    }
    if let Some(tokens) = stage.context_tokens.or(stage.estimated_context_tokens) {
        blocks.push(StatusBlock::muted(format!(
            "Context tokens: {}",
            tui_format_token_count(tokens)
        )));
    }
    if let Some(prompt_tokens) = stage.prompt_tokens {
        blocks.push(StatusBlock::normal(format!(
            "Stage usage: in {} out {} reasoning {}",
            tui_format_token_count(prompt_tokens),
            tui_format_token_count(stage.completion_tokens.unwrap_or(0)),
            tui_format_token_count(stage.reasoning_tokens.unwrap_or(0))
        )));
    }
    blocks
}
