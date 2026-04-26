#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CliRecentSessionInfo {
    title: Option<String>,
    model_label: Option<String>,
    preset_label: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct CliObservedExecutionTopology {
    active: bool,
    root_id: Option<String>,
    scheduler_id: Option<String>,
    active_stage_id: Option<String>,
    stage_order: Vec<String>,
    nodes: HashMap<String, CliObservedExecutionNode>,
}

#[derive(Debug, Clone)]
struct CliObservedExecutionNode {
    kind: String,
    label: String,
    status: String,
    waiting_on: Option<String>,
    recent_event: Option<String>,
    children: Vec<String>,
}

#[derive(Clone)]
enum CliActiveAbortHandle {
    /// Server-side execution — abort via HTTP POST.
    Server {
        api_client: Arc<CliApiClient>,
        session_id: String,
    },
}

const CLI_PROMPT_SUGGESTION_LIMIT: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliPromptValueKind {
    Model,
    Agent,
    Preset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CliPromptCommandSpec {
    name: &'static str,
    takes_value: Option<CliPromptValueKind>,
    description: &'static str,
}

const CLI_PROMPT_COMMANDS: &[CliPromptCommandSpec] = &[
    CliPromptCommandSpec {
        name: "help",
        takes_value: None,
        description: "show help",
    },
    CliPromptCommandSpec {
        name: "abort",
        takes_value: None,
        description: "cancel active run",
    },
    CliPromptCommandSpec {
        name: "clear",
        takes_value: None,
        description: "clear screen",
    },
    CliPromptCommandSpec {
        name: "recover",
        takes_value: None,
        description: "list recovery actions",
    },
    CliPromptCommandSpec {
        name: "runtime",
        takes_value: None,
        description: "show runtime telemetry",
    },
    CliPromptCommandSpec {
        name: "usage",
        takes_value: None,
        description: "show session usage",
    },
    CliPromptCommandSpec {
        name: "events",
        takes_value: None,
        description: "browse runtime events",
    },
    CliPromptCommandSpec {
        name: "model",
        takes_value: Some(CliPromptValueKind::Model),
        description: "switch model",
    },
    CliPromptCommandSpec {
        name: "models",
        takes_value: None,
        description: "list models",
    },
    CliPromptCommandSpec {
        name: "voice",
        takes_value: None,
        description: "record voice input",
    },
    CliPromptCommandSpec {
        name: "agent",
        takes_value: Some(CliPromptValueKind::Agent),
        description: "switch agent",
    },
    CliPromptCommandSpec {
        name: "agents",
        takes_value: None,
        description: "list agents",
    },
    CliPromptCommandSpec {
        name: "preset",
        takes_value: Some(CliPromptValueKind::Preset),
        description: "switch preset",
    },
    CliPromptCommandSpec {
        name: "presets",
        takes_value: None,
        description: "list presets",
    },
    CliPromptCommandSpec {
        name: "providers",
        takes_value: None,
        description: "list providers",
    },
    CliPromptCommandSpec {
        name: "sessions",
        takes_value: None,
        description: "list sessions",
    },
    CliPromptCommandSpec {
        name: "parent",
        takes_value: None,
        description: "return to parent session",
    },
    CliPromptCommandSpec {
        name: "child",
        takes_value: None,
        description: "list or focus child sessions",
    },
    CliPromptCommandSpec {
        name: "tasks",
        takes_value: None,
        description: "list agent tasks",
    },
    CliPromptCommandSpec {
        name: "compact",
        takes_value: None,
        description: "compact conversation history",
    },
    CliPromptCommandSpec {
        name: "copy",
        takes_value: None,
        description: "copy last reply",
    },
];

#[derive(Debug, Clone, Default)]
struct CliPromptCatalog {
    models: Vec<String>,
    agents: Vec<String>,
    presets: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct CliPromptSelectionState {
    model: String,
    agent: String,
    preset: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CliPromptAssistView {
    screen_lines: Vec<String>,
    completion: Option<PromptCompletion>,
}

#[derive(Debug)]
struct CliPromptChrome {
    mode_label: Mutex<String>,
    model_label: Mutex<String>,
    selection: Mutex<CliPromptSelectionState>,
    catalog: Mutex<CliPromptCatalog>,
    frontend_projection: Arc<Mutex<CliFrontendProjection>>,
    style: CliStyle,
}

impl CliPromptChrome {
    fn new(
        runtime: &CliExecutionRuntime,
        style: &CliStyle,
        _current_dir: &Path,
        config: &Config,
        provider_registry: &ProviderRegistry,
        agent_registry: &AgentRegistry,
    ) -> Self {
        Self {
            mode_label: Mutex::new(cli_mode_label(runtime)),
            model_label: Mutex::new(format!("Model {}", runtime.resolved_model_label)),
            selection: Mutex::new(CliPromptSelectionState {
                model: runtime.resolved_model_label.clone(),
                agent: runtime.resolved_agent_name.clone(),
                preset: runtime.resolved_scheduler_profile_name.clone(),
            }),
            catalog: Mutex::new(CliPromptCatalog {
                models: cli_prompt_models(provider_registry),
                agents: cli_prompt_agents(agent_registry),
                presets: cli_available_presets(config),
            }),
            frontend_projection: runtime.frontend_projection.clone(),
            style: style.clone(),
        }
    }

    fn update_from_runtime(&self, runtime: &CliExecutionRuntime) {
        if let Ok(mut mode) = self.mode_label.lock() {
            *mode = cli_mode_label(runtime);
        }
        if let Ok(mut model) = self.model_label.lock() {
            *model = format!("Model {}", runtime.resolved_model_label);
        }
        if let Ok(mut selection) = self.selection.lock() {
            selection.model = runtime.resolved_model_label.clone();
            selection.agent = runtime.resolved_agent_name.clone();
            selection.preset = runtime.resolved_scheduler_profile_name.clone();
        }
    }

    fn update_model_catalog(&self, models: Vec<String>) {
        if let Ok(mut catalog) = self.catalog.lock() {
            catalog.models = models;
        }
    }

    fn assist(&self, line: &str, cursor_pos: usize) -> CliPromptAssistView {
        let selection = self
            .selection
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default();
        let catalog = self
            .catalog
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default();
        cli_prompt_assist_view(&catalog, &selection, line, cursor_pos)
    }

    fn frame(&self, line: &str, cursor_pos: usize) -> PromptFrame {
        let mode = self
            .mode_label
            .lock()
            .map(|value| value.clone())
            .unwrap_or_else(|_| "Agent build".to_string());
        let model = self
            .model_label
            .lock()
            .map(|value| value.clone())
            .unwrap_or_else(|_| "Model auto".to_string());
        let footer = self
            .frontend_projection
            .lock()
            .map(|projection| projection.footer_text())
            .unwrap_or_else(|_| {
                " Ready  •  Alt+Enter/Ctrl+J newline  •  /help  •  Ctrl+D exit ".to_string()
            });
        let assist = self.assist(line, cursor_pos);
        let mut screen_lines = cli_prompt_screen_lines();
        screen_lines.extend(assist.screen_lines);
        PromptFrame::boxed_with_footer(&mode, &model, &footer, &self.style)
            .with_screen_lines(screen_lines)
    }
}

fn cli_prompt_models(provider_registry: &ProviderRegistry) -> Vec<String> {
    let mut models = provider_registry
        .list()
        .into_iter()
        .flat_map(|provider| {
            let provider_id = provider.id().to_string();
            provider
                .models()
                .into_iter()
                .map(move |model| format!("{}/{}", provider_id, model.id))
        })
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    models
}

fn cli_prompt_agents(agent_registry: &AgentRegistry) -> Vec<String> {
    agent_registry
        .list()
        .into_iter()
        .map(|agent| agent.name.clone())
        .collect()
}

fn cli_prompt_assist_view(
    catalog: &CliPromptCatalog,
    selection: &CliPromptSelectionState,
    line: &str,
    cursor_pos: usize,
) -> CliPromptAssistView {
    let prefix = cli_prompt_prefix(line, cursor_pos);
    let trimmed = prefix.trim_start();
    if !trimmed.starts_with('/') {
        return CliPromptAssistView::default();
    }

    let body = &trimmed[1..];
    let body = body.trim_start();
    if body.is_empty() {
        return cli_prompt_command_assist("");
    }

    let Some((command_token, remainder)) = cli_prompt_split_command(body) else {
        return CliPromptAssistView::default();
    };
    let command_name = command_token.to_ascii_lowercase();

    if remainder.is_none() {
        if let Some(spec) = cli_prompt_command_spec(&command_name) {
            if spec.takes_value.is_some() && !prefix.ends_with(' ') {
                return cli_prompt_value_assist(spec, "", catalog, selection, false);
            }
        }
        return cli_prompt_command_assist(&command_name);
    }

    let Some(spec) = cli_prompt_command_spec(&command_name) else {
        return CliPromptAssistView::default();
    };
    let Some(value_kind) = spec.takes_value else {
        return CliPromptAssistView::default();
    };
    let query = remainder.unwrap_or("").trim();
    cli_prompt_value_assist(
        CliPromptCommandSpec {
            name: spec.name,
            takes_value: Some(value_kind),
            description: spec.description,
        },
        query,
        catalog,
        selection,
        true,
    )
}

fn cli_prompt_prefix(line: &str, cursor_pos: usize) -> String {
    line.chars().take(cursor_pos).collect()
}

fn cli_prompt_split_command(body: &str) -> Option<(&str, Option<&str>)> {
    let trimmed = body.trim_start();
    if trimmed.is_empty() {
        return None;
    }

    for (idx, ch) in trimmed.char_indices() {
        if ch.is_whitespace() {
            return Some((&trimmed[..idx], Some(trimmed[idx..].trim_start())));
        }
    }

    Some((trimmed, None))
}

fn cli_prompt_command_spec(name: &str) -> Option<CliPromptCommandSpec> {
    CLI_PROMPT_COMMANDS
        .iter()
        .copied()
        .find(|spec| spec.name.eq_ignore_ascii_case(name))
}

fn cli_prompt_command_assist(query: &str) -> CliPromptAssistView {
    let matches =
        cli_prompt_ranked_matches(CLI_PROMPT_COMMANDS.iter().map(|spec| spec.name), query);
    if matches.is_empty() {
        return CliPromptAssistView::default();
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "Commands ({} match{})",
        matches.len(),
        if matches.len() == 1 { "" } else { "es" }
    ));

    for name in matches.iter().take(CLI_PROMPT_SUGGESTION_LIMIT) {
        let spec = cli_prompt_command_spec(name).expect("command spec");
        lines.push(format!("  /{:<10} {}", spec.name, spec.description));
    }
    if matches.len() > CLI_PROMPT_SUGGESTION_LIMIT {
        lines.push(format!(
            "  ... {} more",
            matches.len() - CLI_PROMPT_SUGGESTION_LIMIT
        ));
    }

    let completion = matches.first().and_then(|name| {
        cli_prompt_command_spec(name).map(|spec| PromptCompletion {
            line: if spec.takes_value.is_some() {
                format!("/{} ", spec.name)
            } else {
                format!("/{}", spec.name)
            },
            cursor_pos: if spec.takes_value.is_some() {
                spec.name.len() + 2
            } else {
                spec.name.len() + 1
            },
        })
    });

    CliPromptAssistView {
        screen_lines: lines,
        completion,
    }
}

fn cli_prompt_value_assist(
    spec: CliPromptCommandSpec,
    query: &str,
    catalog: &CliPromptCatalog,
    selection: &CliPromptSelectionState,
    can_complete_value: bool,
) -> CliPromptAssistView {
    let values = match spec.takes_value {
        Some(CliPromptValueKind::Model) => &catalog.models,
        Some(CliPromptValueKind::Agent) => &catalog.agents,
        Some(CliPromptValueKind::Preset) => &catalog.presets,
        None => return CliPromptAssistView::default(),
    };
    let matches = cli_prompt_ranked_matches(values.iter().map(String::as_str), query);
    if matches.is_empty() {
        return CliPromptAssistView::default();
    }

    let active = match spec.takes_value {
        Some(CliPromptValueKind::Model) => Some(selection.model.as_str()),
        Some(CliPromptValueKind::Agent) => Some(selection.agent.as_str()),
        Some(CliPromptValueKind::Preset) => selection.preset.as_deref(),
        None => None,
    };

    let mut lines = Vec::new();
    lines.push(format!(
        "/{} suggestions ({} match{})",
        spec.name,
        matches.len(),
        if matches.len() == 1 { "" } else { "es" }
    ));
    for value in matches.iter().take(CLI_PROMPT_SUGGESTION_LIMIT) {
        let active_suffix = if active.is_some_and(|current| current.eq_ignore_ascii_case(value)) {
            " [active]"
        } else {
            ""
        };
        lines.push(format!("  {}{}", value, active_suffix));
    }
    if matches.len() > CLI_PROMPT_SUGGESTION_LIMIT {
        lines.push(format!(
            "  ... {} more",
            matches.len() - CLI_PROMPT_SUGGESTION_LIMIT
        ));
    }
    lines.push("  Tab completes best match".to_string());

    let completion = if can_complete_value {
        matches.first().map(|value| PromptCompletion {
            line: format!("/{} {}", spec.name, value),
            cursor_pos: spec.name.len() + value.len() + 2,
        })
    } else {
        Some(PromptCompletion {
            line: format!("/{} ", spec.name),
            cursor_pos: spec.name.len() + 2,
        })
    };

    CliPromptAssistView {
        screen_lines: lines,
        completion,
    }
}

fn cli_prompt_ranked_matches<'a>(
    candidates: impl IntoIterator<Item = &'a str>,
    query: &str,
) -> Vec<String> {
    let normalized_query = query.trim().to_ascii_lowercase();
    let mut prefix_matches = Vec::new();
    let mut contains_matches = Vec::new();

    for candidate in candidates {
        let normalized_candidate = candidate.to_ascii_lowercase();
        if normalized_query.is_empty() || normalized_candidate.starts_with(&normalized_query) {
            prefix_matches.push(candidate.to_string());
        } else if normalized_candidate.contains(&normalized_query) {
            contains_matches.push(candidate.to_string());
        }
    }

    prefix_matches.sort();
    prefix_matches.dedup();
    contains_matches.sort();
    contains_matches.retain(|item| !prefix_matches.contains(item));
    prefix_matches.extend(contains_matches);
    prefix_matches
}

struct CliTerminalSurface {
    style: CliStyle,
    frontend_projection: Arc<Mutex<CliFrontendProjection>>,
    prompt_session: Mutex<Option<Arc<PromptSession>>>,
    prompt_suspended: AtomicBool,
    busy_flag: Arc<AtomicBool>,
}

impl CliTerminalSurface {
    fn new(
        style: CliStyle,
        frontend_projection: Arc<Mutex<CliFrontendProjection>>,
        busy_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            style,
            frontend_projection,
            prompt_session: Mutex::new(None),
            prompt_suspended: AtomicBool::new(false),
            busy_flag,
        }
    }

    fn set_prompt_session(&self, prompt_session: Arc<PromptSession>) {
        if let Ok(mut slot) = self.prompt_session.lock() {
            *slot = Some(prompt_session);
        }
    }

    fn print_block(&self, block: OutputBlock) -> anyhow::Result<()> {
        self.append_rendered(&render_cli_block_rich(&block, &self.style))?;
        Ok(())
    }

    fn print_text(&self, text: &str) -> io::Result<()> {
        self.append_rendered(text)
    }

    fn clear_transcript(&self) -> io::Result<()> {
        if let Ok(mut projection) = self.frontend_projection.lock() {
            projection.transcript.clear();
        }
        self.refresh_prompt()
    }

    fn replace_transcript(&self, transcript: CliRetainedTranscript) -> io::Result<()> {
        if let Ok(mut projection) = self.frontend_projection.lock() {
            projection.transcript = transcript.clone();
            projection.scroll_offset = 0;
        }

        let prompt = self
            .prompt_session
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned());

        if let Some(prompt_session) = prompt {
            if !self.prompt_suspended.load(Ordering::Relaxed) {
                let _ = prompt_session.suspend();
            }
            let write_result: io::Result<()> = {
                print!("\x1B[2J\x1B[1;1H{}", transcript.rendered_text());
                io::stdout().flush()
            };
            let _ = prompt_session.resume();
            self.prompt_suspended.store(false, Ordering::Relaxed);
            write_result?;
        } else {
            print!("\x1B[2J\x1B[1;1H{}", transcript.rendered_text());
            io::stdout().flush()?;
        }

        self.refresh_prompt()
    }

    fn append_rendered(&self, rendered: &str) -> io::Result<()> {
        if let Ok(mut projection) = self.frontend_projection.lock() {
            projection.transcript.append_rendered(rendered);
            projection.scroll_offset = 0;
        }
        let prompt = self
            .prompt_session
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned());

        if let Some(prompt_session) = prompt {
            let busy = self.busy_flag.load(Ordering::Relaxed);
            if !self.prompt_suspended.load(Ordering::Relaxed) {
                let _ = prompt_session.suspend();
                self.prompt_suspended.store(true, Ordering::Relaxed);
            }
            let write_result: io::Result<()> = {
                print!("{}", rendered);
                io::stdout().flush()
            };
            if !busy {
                let _ = prompt_session.resume();
                self.prompt_suspended.store(false, Ordering::Relaxed);
            }
            write_result
        } else {
            print!("{}", rendered);
            io::stdout().flush()
        }
    }

    fn refresh_prompt(&self) -> io::Result<()> {
        if let Some(prompt) = self
            .prompt_session
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned())
        {
            prompt.refresh()?;
        }
        Ok(())
    }

    fn ensure_prompt_visible(&self) -> io::Result<()> {
        if self.prompt_suspended.swap(false, Ordering::Relaxed) {
            if let Some(prompt_session) = self
                .prompt_session
                .lock()
                .ok()
                .and_then(|slot| slot.as_ref().cloned())
            {
                let _ = prompt_session.resume();
            }
        }
        Ok(())
    }
}

enum CliDispatchInput {
    Line(String),
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum CliFrontendPhase {
    #[default]
    Idle,
    Busy,
    Waiting,
    Cancelling,
    Failed,
}

const CLI_TRANSCRIPT_MAX_LINES: usize = 1200;

#[derive(Debug, Clone, Default)]
struct CliRetainedTranscript {
    committed_lines: Vec<String>,
    open_line: String,
}

impl CliRetainedTranscript {
    fn append_rendered(&mut self, rendered: &str) {
        let normalized = strip_ansi(rendered).replace('\r', "");
        for chunk in normalized.split_inclusive('\n') {
            if let Some(content) = chunk.strip_suffix('\n') {
                self.open_line.push_str(content);
                self.committed_lines
                    .push(std::mem::take(&mut self.open_line));
                self.trim_to_budget();
            } else {
                self.open_line.push_str(chunk);
            }
        }
    }

    fn clear(&mut self) {
        self.committed_lines.clear();
        self.open_line.clear();
    }

    fn rendered_text(&self) -> String {
        let mut out = String::new();
        for line in &self.committed_lines {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&self.open_line);
        out
    }

    fn line_count(&self) -> usize {
        self.committed_lines.len() + usize::from(!self.open_line.is_empty())
    }

    fn last_line(&self) -> Option<&str> {
        if !self.open_line.is_empty() {
            Some(self.open_line.as_str())
        } else {
            self.committed_lines.last().map(String::as_str)
        }
    }

    #[cfg(test)]
    fn viewport_lines(&self, width: usize, max_rows: usize, scroll_offset: usize) -> Vec<String> {
        let mut rows = Vec::new();
        for line in &self.committed_lines {
            extend_wrapped_lines(&mut rows, line, width);
        }
        if !self.open_line.is_empty() || rows.is_empty() {
            extend_wrapped_lines(&mut rows, &self.open_line, width);
        }
        if rows.is_empty() {
            rows.push("No messages yet. Send a prompt to start.".to_string());
        }
        if rows.len() <= max_rows {
            return rows;
        }
        // Without scroll: show the last max_rows lines (tail).
        // With scroll_offset > 0: slide the window up by scroll_offset rows.
        let tail_start = rows.len().saturating_sub(max_rows);
        let start = tail_start.saturating_sub(scroll_offset);
        let end = (start + max_rows).min(rows.len());
        rows[start..end].to_vec()
    }

    /// Total wrapped row count (for calculating max scroll offset).
    #[cfg(test)]
    fn total_rows(&self, width: usize) -> usize {
        let mut count = 0usize;
        for line in &self.committed_lines {
            count += wrap_display_text(line, width.max(1)).len();
        }
        if !self.open_line.is_empty() {
            count += wrap_display_text(&self.open_line, width.max(1)).len();
        }
        count.max(1)
    }

    fn trim_to_budget(&mut self) {
        if self.committed_lines.len() > CLI_TRANSCRIPT_MAX_LINES {
            let overflow = self.committed_lines.len() - CLI_TRANSCRIPT_MAX_LINES;
            self.committed_lines.drain(0..overflow);
        }
    }
}

/// Cumulative token usage and cost for the current session.
#[derive(Debug, Clone, Default)]
struct CliSessionTokenStats {
    total_tokens: u64,
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    total_cost: f64,
}

impl CliSessionTokenStats {
    pub(super) fn sync_from_usage(
        &mut self,
        input_tokens: u64,
        output_tokens: u64,
        reasoning_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
        total_cost: f64,
    ) {
        self.input_tokens = input_tokens;
        self.output_tokens = output_tokens;
        self.reasoning_tokens = reasoning_tokens;
        self.cache_read_tokens = cache_read_tokens;
        self.cache_write_tokens = cache_write_tokens;
        self.total_tokens = input_tokens
            + output_tokens
            + reasoning_tokens
            + cache_read_tokens
            + cache_write_tokens;
        self.total_cost = total_cost;
    }
}

#[derive(Debug, Clone, Default)]
struct CliLastTurnTokenStats {
    input_tokens: u64,
    output_tokens: u64,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Default)]
struct CliModelCatalogEntry {
    context_window: Option<u64>,
    cost_per_million_input: Option<f64>,
    cost_per_million_output: Option<f64>,
}

/// MCP server status snapshot for sidebar display.
#[derive(Debug, Clone)]
struct CliMcpServerStatus {
    name: String,
    status: String,
    tools: usize,
    error: Option<String>,
}

impl From<McpStatusInfo> for CliMcpServerStatus {
    fn from(info: McpStatusInfo) -> Self {
        Self {
            name: info.name,
            status: info.status,
            tools: info.tools,
            error: info.error,
        }
    }
}

#[derive(Debug, Clone)]
struct CliFrontendProjection {
    phase: CliFrontendPhase,
    active_label: Option<String>,
    view_label: Option<String>,
    queue_len: usize,
    active_stage: Option<SchedulerStageBlock>,
    session_runtime: Option<SessionRuntimeState>,
    stage_summaries: Vec<rocode_command::stage_protocol::StageSummary>,
    telemetry_topology: Option<SessionExecutionTopology>,
    events_browser: Option<CliEventsBrowserState>,
    transcript: CliRetainedTranscript,
    #[cfg_attr(not(test), allow(dead_code))]
    sidebar_collapsed: bool,
    active_collapsed: bool,
    session_title: Option<String>,
    current_model_label: Option<String>,
    /// Scroll offset for Messages panel: 0 = bottom (latest), N = scrolled up N rows.
    scroll_offset: usize,
    /// Cumulative token usage for the current session.
    token_stats: CliSessionTokenStats,
    /// Latest completed turn token usage, sourced from usage SSE events.
    last_turn_tokens: CliLastTurnTokenStats,
    model_catalog: std::collections::HashMap<String, CliModelCatalogEntry>,
    /// MCP server statuses fetched from the server.
    mcp_servers: Vec<CliMcpServerStatus>,
    /// LSP server names fetched from the server.
    lsp_servers: Vec<String>,
}

impl Default for CliFrontendProjection {
    fn default() -> Self {
        Self {
            phase: CliFrontendPhase::default(),
            active_label: None,
            view_label: None,
            queue_len: 0,
            active_stage: None,
            session_runtime: None,
            stage_summaries: Vec::new(),
            telemetry_topology: None,
            events_browser: None,
            transcript: CliRetainedTranscript::default(),
            sidebar_collapsed: true,
            active_collapsed: true,
            session_title: None,
            current_model_label: None,
            scroll_offset: 0,
            token_stats: CliSessionTokenStats::default(),
            last_turn_tokens: CliLastTurnTokenStats::default(),
            model_catalog: std::collections::HashMap::new(),
            mcp_servers: Vec::new(),
            lsp_servers: Vec::new(),
        }
    }
}

impl CliFrontendProjection {
    fn current_context_tokens(&self) -> Option<u64> {
        let active_stage_id = self
            .session_runtime
            .as_ref()
            .and_then(|runtime| runtime.active_stage_id.as_deref());
        if let Some(active_stage_id) = active_stage_id {
            if let Some(tokens) = self
                .stage_summaries
                .iter()
                .find(|stage| stage.stage_id == active_stage_id)
                .and_then(|stage| stage.estimated_context_tokens)
                .filter(|tokens| *tokens > 0)
            {
                return Some(tokens);
            }
        }

        self.stage_summaries
            .iter()
            .rev()
            .find_map(|stage| stage.estimated_context_tokens.filter(|tokens| *tokens > 0))
    }

    fn context_pressure_note(&self, percent: Option<u64>) -> Option<&'static str> {
        match percent {
            Some(percent) if percent >= 95 => Some("compact now"),
            Some(percent) if percent >= 90 => Some("auto-compact soon"),
            Some(percent) if percent >= 80 => Some("warning"),
            _ => None,
        }
    }

    fn footer_text(&self) -> String {
        let state = match self.phase {
            CliFrontendPhase::Idle => "Ready".to_string(),
            CliFrontendPhase::Busy => "Busy".to_string(),
            CliFrontendPhase::Waiting => "Waiting".to_string(),
            CliFrontendPhase::Cancelling => "Cancelling".to_string(),
            CliFrontendPhase::Failed => "Error".to_string(),
        };
        let mut parts = vec![format!(" {} ", state)];
        if let Some(active) = self
            .active_label
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            parts.push(active.to_string());
        }
        if let Some(view) = self.view_label.as_deref().filter(|value| !value.is_empty()) {
            parts.push(view.to_string());
        }
        if self.queue_len > 0 {
            parts.push(format!("queue {}", self.queue_len));
        }
        if let Some(current_tokens) = self.current_context_tokens() {
            let context_window = self
                .current_model_label
                .as_deref()
                .and_then(|label| self.model_catalog.get(label))
                .and_then(|entry| entry.context_window)
                .filter(|value| *value > 0);
            if let Some(limit) = context_window {
                let percent = (((current_tokens as f64 / limit as f64) * 100.0).round() as u64)
                    .max(1);
                let mut label = format!(
                    "ctx {}/{} {}%",
                    format_token_count(current_tokens),
                    format_token_count(limit),
                    percent
                );
                if let Some(note) = self.context_pressure_note(Some(percent)) {
                    label.push(' ');
                    label.push_str(note);
                }
                parts.push(label);
            } else {
                parts.push(format!("ctx {}", format_token_count(current_tokens)));
            }
        } else if self.token_stats.total_tokens > 0 {
            parts.push(format!("total {}", format_token_count(self.token_stats.total_tokens)));
        }
        if let Some(browser) = self.events_browser.as_ref() {
            let page = (browser.offset / browser.filter.limit.unwrap_or(24).max(1)) + 1;
            parts.push(format!("events p{}", page));
        }
        parts.push("Alt+Enter/Ctrl+J newline".to_string());
        parts.push("/help".to_string());
        parts.push("/runtime".to_string());
        parts.push("/insights".to_string());
        parts.push("/child".to_string());
        if !matches!(self.phase, CliFrontendPhase::Idle) {
            parts.push("/abort".to_string());
        }
        parts.push("Ctrl+D exit".to_string());
        format!(" {} ", parts.join("  •  "))
    }
}

impl CliObservedExecutionTopology {
    fn reset_for_run(&mut self, agent_name: &str, scheduler_profile: Option<&str>) {
        self.active = true;
        self.root_id = Some("prompt".to_string());
        self.scheduler_id = scheduler_profile.map(|_| "scheduler".to_string());
        self.active_stage_id = None;
        self.stage_order.clear();
        self.nodes.clear();
        self.nodes.insert(
            "prompt".to_string(),
            CliObservedExecutionNode {
                kind: "prompt".to_string(),
                label: format!("Prompt run ({})", agent_name),
                status: "running".to_string(),
                waiting_on: Some("model".to_string()),
                recent_event: Some("Prompt run started".to_string()),
                children: Vec::new(),
            },
        );
        if let Some(profile) = scheduler_profile {
            self.nodes.insert(
                "scheduler".to_string(),
                CliObservedExecutionNode {
                    kind: "scheduler".to_string(),
                    label: format!("Scheduler run ({})", profile),
                    status: "running".to_string(),
                    waiting_on: Some("model".to_string()),
                    recent_event: Some("Scheduler orchestration started".to_string()),
                    children: Vec::new(),
                },
            );
            self.attach_child("prompt", "scheduler");
        }
    }

    fn observe_block(&mut self, block: &OutputBlock) {
        match block {
            OutputBlock::SchedulerStage(stage) => self.observe_scheduler_stage(stage),
            OutputBlock::Tool(tool) => self.observe_tool(tool),
            _ => {}
        }
    }

    fn observe_scheduler_stage(
        &mut self,
        stage: &rocode_command::output_blocks::SchedulerStageBlock,
    ) {
        let stage_id = stage.stage_id.clone().unwrap_or_else(|| {
            format!(
                "stage:{}:{}",
                stage
                    .stage_index
                    .unwrap_or((self.stage_order.len() + 1) as u64),
                stage.stage
            )
        });
        let parent_id = self
            .scheduler_id
            .clone()
            .unwrap_or_else(|| self.root_id.clone().unwrap_or_else(|| "prompt".to_string()));
        let status = stage
            .status
            .clone()
            .unwrap_or_else(|| "running".to_string());
        let node = self
            .nodes
            .entry(stage_id.clone())
            .or_insert(CliObservedExecutionNode {
                kind: "stage".to_string(),
                label: stage.title.clone(),
                status: status.clone(),
                waiting_on: stage.waiting_on.clone(),
                recent_event: stage.last_event.clone(),
                children: Vec::new(),
            });
        node.label = stage.title.clone();
        node.status = status.clone();
        node.waiting_on = stage.waiting_on.clone();
        node.recent_event = stage.last_event.clone();
        self.attach_child(&parent_id, &stage_id);
        if !self.stage_order.iter().any(|id| id == &stage_id) {
            self.stage_order.push(stage_id.clone());
        }
        if matches!(
            status.as_str(),
            "running" | "waiting" | "cancelling" | "retry"
        ) {
            self.active_stage_id = Some(stage_id.clone());
        }
        if let Some(scheduler_id) = self.scheduler_id.clone() {
            if let Some(scheduler) = self.nodes.get_mut(&scheduler_id) {
                scheduler.waiting_on = stage.waiting_on.clone();
                scheduler.recent_event = stage.last_event.clone();
                scheduler.status = if status == "waiting" {
                    "waiting".to_string()
                } else {
                    "running".to_string()
                };
            }
        }
    }

    fn observe_tool(&mut self, tool: &rocode_command::output_blocks::ToolBlock) {
        let parent_id = self
            .active_stage_id
            .clone()
            .or_else(|| self.scheduler_id.clone())
            .or_else(|| self.root_id.clone())
            .unwrap_or_else(|| "prompt".to_string());
        let tool_id = format!("tool:{}:{}", parent_id, tool.name);
        let status = match tool.phase {
            rocode_command::output_blocks::ToolPhase::Start
            | rocode_command::output_blocks::ToolPhase::Running => "running",
            rocode_command::output_blocks::ToolPhase::Done => "done",
            rocode_command::output_blocks::ToolPhase::Error => "error",
        }
        .to_string();
        let node = self
            .nodes
            .entry(tool_id.clone())
            .or_insert(CliObservedExecutionNode {
                kind: "tool".to_string(),
                label: tool.name.clone(),
                status: status.clone(),
                waiting_on: Some("tool".to_string()),
                recent_event: tool.detail.clone(),
                children: Vec::new(),
            });
        node.status = status.clone();
        node.waiting_on = if matches!(tool.phase, rocode_command::output_blocks::ToolPhase::Done) {
            None
        } else {
            Some("tool".to_string())
        };
        node.recent_event = tool.detail.clone();
        self.attach_child(&parent_id, &tool_id);
    }

    fn start_question(&mut self, count: usize) {
        let parent_id = self
            .active_stage_id
            .clone()
            .or_else(|| self.scheduler_id.clone())
            .or_else(|| self.root_id.clone())
            .unwrap_or_else(|| "prompt".to_string());
        let question_id = format!("question:{}:{}", parent_id, count);
        self.nodes.insert(
            question_id.clone(),
            CliObservedExecutionNode {
                kind: "question".to_string(),
                label: format!("Question ({})", count),
                status: "waiting".to_string(),
                waiting_on: Some("user".to_string()),
                recent_event: Some("Waiting for user answer".to_string()),
                children: Vec::new(),
            },
        );
        self.attach_child(&parent_id, &question_id);
    }

    fn finish_question(&mut self, outcome: &str) {
        for node in self
            .nodes
            .values_mut()
            .filter(|node| node.kind == "question")
        {
            if node.status == "waiting" {
                node.status = outcome.to_string();
                node.waiting_on = None;
                node.recent_event = Some(format!("Question {}", outcome));
            }
        }
    }

    fn finish_run(&mut self, outcome: Option<String>) {
        self.active = false;
        if let Some(root_id) = self.root_id.clone() {
            if let Some(root) = self.nodes.get_mut(&root_id) {
                root.status = outcome
                    .clone()
                    .unwrap_or_else(|| "completed".to_string())
                    .to_lowercase();
                root.waiting_on = None;
                root.recent_event = outcome;
            }
        }
    }

    fn attach_child(&mut self, parent_id: &str, child_id: &str) {
        if let Some(parent) = self.nodes.get_mut(parent_id) {
            if !parent.children.iter().any(|id| id == child_id) {
                parent.children.push(child_id.to_string());
            }
        }
    }
}

fn cli_print_execution_topology(
    observed_topology: &Arc<Mutex<CliObservedExecutionTopology>>,
    runtime: Option<&CliExecutionRuntime>,
    style: &CliStyle,
) {
    let Ok(topology) = observed_topology.lock() else {
        let _ = print_cli_list_on_surface(
            runtime,
            "Execution Topology",
            None,
            &[style.dim("unavailable")],
            style,
        );
        return;
    };
    if topology.nodes.is_empty() {
        let _ = print_cli_list_on_surface(
            runtime,
            "Execution Topology",
            None,
            &[style.dim("no observed execution topology")],
            style,
        );
        return;
    }
    let mut lines = Vec::new();
    if topology.active {
        lines.push(style.bold_green("active"));
    } else {
        lines.push(style.dim("idle · last observed topology"));
    }
    if let Some(root_id) = topology.root_id.as_deref() {
        cli_collect_execution_node(&topology, root_id, "", true, &mut lines);
    }
    let _ = print_cli_list_on_surface(runtime, "Execution Topology", None, &lines, style);
}

fn cli_collect_execution_node(
    topology: &CliObservedExecutionTopology,
    node_id: &str,
    prefix: &str,
    is_last: bool,
    lines: &mut Vec<String>,
) {
    let Some(node) = topology.nodes.get(node_id) else {
        return;
    };
    let branch = if prefix.is_empty() {
        ""
    } else if is_last {
        "└─ "
    } else {
        "├─ "
    };
    let mut line = format!("{}{}{} · {}", prefix, branch, node.label, node.status);
    if let Some(waiting_on) = node.waiting_on.as_deref() {
        line.push_str(&format!(" · waiting {}", waiting_on));
    }
    if let Some(recent_event) = node.recent_event.as_deref() {
        line.push_str(&format!(" · {}", recent_event));
    }
    lines.push(line);
    let child_prefix = if prefix.is_empty() {
        "  ".to_string()
    } else if is_last {
        format!("{}   ", prefix)
    } else {
        format!("{}│  ", prefix)
    };
    for (index, child_id) in node.children.iter().enumerate() {
        cli_collect_execution_node(
            topology,
            child_id,
            &child_prefix,
            index + 1 == node.children.len(),
            lines,
        );
    }
}

#[derive(Debug, Clone, Default)]
struct CliSchedulerResolution {
    defaults: Option<SchedulerRequestDefaults>,
    profile_model: Option<(String, String)>,
}

fn resolve_scheduler_profile_config(
    config: &Config,
    requested_scheduler_profile: Option<&str>,
) -> anyhow::Result<Option<(String, SchedulerProfileConfig)>> {
    let requested = requested_scheduler_profile
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let scheduler_path = config
        .scheduler_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(name) = requested {
        if let Ok(preset) = SchedulerPresetKind::from_str(name) {
            return Ok(Some((
                name.to_string(),
                SchedulerProfileConfig {
                    orchestrator: Some(preset.as_str().to_string()),
                    ..Default::default()
                },
            )));
        }

        let path = scheduler_path.ok_or_else(|| {
            anyhow::anyhow!(
                "Scheduler profile could not be resolved: `{}`. No scheduler config is configured.",
                name
            )
        })?;
        let scheduler_config = SchedulerConfig::load_from_file(path).map_err(|error| {
            anyhow::anyhow!(
                "Scheduler profile could not be resolved: `{}`. Failed to load scheduler config: {}",
                name,
                error
            )
        })?;
        let profile = scheduler_config.profile(name).map_err(|error| {
            anyhow::anyhow!(
                "Scheduler profile could not be resolved: `{}`. {}",
                name,
                error
            )
        })?;
        return Ok(Some((name.to_string(), profile.clone())));
    }

    if let Some(path) = scheduler_path {
        let scheduler_config = match SchedulerConfig::load_from_file(path) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!(path = %path, %error, "failed to load scheduler config");
                return Ok(None);
            }
        };

        if let Some(name) = scheduler_config.default_profile_key() {
            if let Ok(profile) = scheduler_config.profile(name) {
                return Ok(Some((name.to_string(), profile.clone())));
            }
        }
        return Ok(None);
    }

    Ok(None)
}

fn resolve_scheduler_runtime(
    config: &Config,
    requested_scheduler_profile: Option<&str>,
) -> anyhow::Result<CliSchedulerResolution> {
    let Some((profile_name, profile)) =
        resolve_scheduler_profile_config(config, requested_scheduler_profile)?
    else {
        return Ok(CliSchedulerResolution::default());
    };

    let defaults = match scheduler_plan_from_profile(Some(profile_name.clone()), &profile) {
        Ok(plan) => Some(scheduler_request_defaults_from_plan(&plan)),
        Err(error) => {
            if requested_scheduler_profile
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some()
            {
                return Err(anyhow::anyhow!(
                    "Scheduler profile could not be resolved: `{}`. Failed to build profile plan: {}",
                    profile_name,
                    error
                ));
            }
            tracing::warn!(
                profile = %profile_name,
                %error,
                "failed to build default scheduler profile plan"
            );
            None
        }
    };
    let profile_model = profile
        .model
        .as_ref()
        .map(|model| (model.provider_id.clone(), model.model_id.clone()));

    Ok(CliSchedulerResolution {
        defaults,
        profile_model,
    })
}

async fn build_cli_execution_runtime(
    input: CliRuntimeBuildInput<'_>,
) -> anyhow::Result<CliExecutionRuntime> {
    let CliRuntimeBuildInput {
        config,
        agent_registry,
        selection,
    } = input;
    let observed_topology = Arc::new(Mutex::new(CliObservedExecutionTopology::default()));
    let frontend_projection = Arc::new(Mutex::new(CliFrontendProjection::default()));
    let scheduler_stage_snapshots = Arc::new(Mutex::new(HashMap::new()));
    let scheduler_resolution =
        resolve_scheduler_runtime(config, selection.requested_scheduler_profile.as_deref())?;
    let scheduler_defaults = scheduler_resolution.defaults.clone();
    let scheduler_profile_name = scheduler_defaults
        .as_ref()
        .and_then(|defaults| defaults.profile_name.clone());
    let scheduler_root_agent = scheduler_defaults
        .as_ref()
        .and_then(|defaults| defaults.root_agent_name.clone());
    let agent_name = resolve_requested_agent_name(
        config,
        selection.requested_agent.as_deref(),
        scheduler_defaults.as_ref(),
    );

    let mut agent_info = agent_registry
        .get(&agent_name)
        .cloned()
        .unwrap_or_else(AgentInfo::build);

    if let Some(ref model_id) = selection.model {
        let provider_id = selection.provider.clone().unwrap_or_else(|| {
            "openai".to_string()
        });
        agent_info = agent_info.with_model(model_id.clone(), provider_id);
    } else if let Some((provider_id, model_id)) = scheduler_resolution.profile_model.clone() {
        agent_info = agent_info.with_model(model_id, provider_id);
    }

    let resolved_model_label = agent_info
        .model
        .as_ref()
        .map(|m| format!("{}/{}", m.provider_id, m.model_id))
        .unwrap_or_else(|| "auto".to_string());
    if let Ok(mut projection) = frontend_projection.lock() {
        projection.current_model_label = Some(resolved_model_label.clone());
    }

    // Shared spinner guard slot — closures capture this; process_message_with_mode
    // swaps in the real spinner's guard each cycle.
    let spinner_guard: Arc<std::sync::Mutex<SpinnerGuard>> =
        Arc::new(std::sync::Mutex::new(SpinnerGuard::noop()));
    let prompt_session_slot: Arc<std::sync::Mutex<Option<Arc<PromptSession>>>> =
        Arc::new(std::sync::Mutex::new(None));

    tracing::info!(
        requested_agent = ?selection.requested_agent,
        requested_scheduler_profile = ?selection.requested_scheduler_profile,
        resolved_agent = %agent_name,
        scheduler_profile = ?scheduler_profile_name,
        scheduler_root_agent = ?scheduler_root_agent,
        resolved_model = %resolved_model_label,
        "resolved cli runtime execution configuration"
    );

    Ok(CliExecutionRuntime {
        resolved_agent_name: agent_name,
        resolved_scheduler_profile_name: scheduler_profile_name,
        resolved_model_label,
        observed_topology,
        frontend_projection,
        scheduler_stage_snapshots,
        terminal_surface: None,
        prompt_chrome: None,
        prompt_session: None,
        prompt_session_slot,
        queued_inputs: Arc::new(AsyncMutex::new(VecDeque::new())),
        busy_flag: Arc::new(AtomicBool::new(false)),
        exit_requested: Arc::new(AtomicBool::new(false)),
        active_abort: Arc::new(AsyncMutex::new(None)),
        recovery_base_prompt: None,
        spinner_guard,
        api_client: None,
        server_session_id: None,
        related_session_ids: Arc::new(Mutex::new(BTreeSet::new())),
        root_session_transcript: Arc::new(Mutex::new(CliRetainedTranscript::default())),
        child_session_transcripts: Arc::new(Mutex::new(HashMap::new())),
        stream_accumulators: Arc::new(Mutex::new(HashMap::new())),
        render_states: Arc::new(Mutex::new(HashMap::new())),
        focused_session_id: Arc::new(Mutex::new(None)),
        permission_memory: Arc::new(AsyncMutex::new(PermissionMemory::new())),
        show_thinking: Arc::new(AtomicBool::new(selection.show_thinking)),
    })
}

fn cli_available_presets(config: &Config) -> Vec<String> {
    let mut names = BTreeSet::new();
    for preset in SchedulerPresetKind::public_presets() {
        names.insert(preset.as_str().to_string());
    }

    if let Some(path) = config
        .scheduler_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Ok(scheduler_config) = SchedulerConfig::load_from_file(path) {
            for name in scheduler_config.profiles.keys() {
                names.insert(name.clone());
            }
        }
    }

    names.into_iter().collect()
}

fn cli_list_presets(
    config: &Config,
    active_profile: Option<&str>,
    runtime: Option<&CliExecutionRuntime>,
) {
    let style = CliStyle::detect();
    let lines = cli_available_presets(config)
        .into_iter()
        .map(|preset| {
            let active = if active_profile == Some(preset.as_str()) {
                format!(" {}", style.bold_green("← active"))
            } else {
                String::new()
            };
            format!("{preset}{active}")
        })
        .collect::<Vec<_>>();
    let _ = print_cli_list_on_surface(runtime, "Available Presets", None, &lines, &style);
}

fn cli_copy_target_transcript(runtime: &CliExecutionRuntime) -> Option<String> {
    if let Some(focused_session_id) = cli_focused_session_id(runtime) {
        return runtime
            .child_session_transcripts
            .lock()
            .ok()
            .and_then(|transcripts| {
                transcripts
                    .get(&focused_session_id)
                    .map(CliRetainedTranscript::rendered_text)
            });
    }

    runtime
        .root_session_transcript
        .lock()
        .ok()
        .map(|transcript| transcript.rendered_text())
}

/// Render a list as plain text (title + indented items) — no box frame.
/// Uses `\r\n` line endings for correct display when the terminal is in raw mode.
fn render_cli_list(
    title: &str,
    footer: Option<&str>,
    lines: &[String],
    style: &CliStyle,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "\r\n  {} {}\r\n",
        style.bold_cyan(style.bullet()),
        style.bold(title),
    ));
    if lines.is_empty() {
        out.push_str(&format!("    {}\r\n", style.dim("(none)")));
    } else {
        for line in lines {
            out.push_str(&format!("    {}\r\n", line));
        }
    }
    if let Some(footer) = footer {
        out.push_str(&format!("    {}\r\n", style.dim(footer)));
    }
    out.push_str("\r\n");
    out
}

fn print_cli_list_on_surface(
    runtime: Option<&CliExecutionRuntime>,
    title: &str,
    footer: Option<&str>,
    lines: &[String],
    style: &CliStyle,
) -> io::Result<()> {
    let rendered = render_cli_list(title, footer, lines, style);
    if let Some(surface) = runtime.and_then(|runtime| runtime.terminal_surface.as_ref()) {
        surface.print_text(&rendered)
    } else {
        print!("{}", rendered);
        io::stdout().flush()
    }
}

fn cli_mode_label(runtime: &CliExecutionRuntime) -> String {
    match runtime.resolved_scheduler_profile_name.as_deref() {
        Some(profile) => format!("Preset {}", profile),
        None => format!("Agent {}", runtime.resolved_agent_name),
    }
}

fn cli_refresh_prompt(runtime: &CliExecutionRuntime) {
    if let Some(prompt_session) = runtime.prompt_session.as_ref() {
        let _ = prompt_session.refresh();
    }
}

fn cli_prompt_screen_lines() -> Vec<String> {
    Vec::new()
}

fn cli_session_hint_string(
    session: &crate::api_client::SessionListItem,
    key: &str,
) -> Option<String> {
    let hints = session.hints.as_ref()?;
    let value = match key {
        "current_model" => hints.current_model.as_deref(),
        "model_provider" => hints.model_provider.as_deref(),
        "model_id" => hints.model_id.as_deref(),
        "scheduler_profile" => hints.scheduler_profile.as_deref(),
        "resolved_scheduler_profile" => hints.resolved_scheduler_profile.as_deref(),
        "agent" => hints.agent.as_deref(),
        _ => None,
    }?;

    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn cli_recent_session_info_for_directory(
    sessions: &[crate::api_client::SessionListItem],
    current_dir: &Path,
) -> Option<CliRecentSessionInfo> {
    let current_dir = current_dir.display().to_string();
    let session = sessions
        .iter()
        .filter(|session| session.directory == current_dir)
        .max_by_key(|session| session.time.updated)
        .or_else(|| sessions.iter().max_by_key(|session| session.time.updated))?;

    let model_label = cli_session_hint_string(session, "current_model").or_else(|| {
        cli_session_hint_string(session, "model_provider")
            .zip(cli_session_hint_string(session, "model_id"))
            .map(|(provider, model)| format!("{provider}/{model}"))
    });
    let preset_label = cli_session_hint_string(session, "scheduler_profile")
        .or_else(|| cli_session_hint_string(session, "resolved_scheduler_profile"))
        .or_else(|| {
            cli_session_hint_string(session, "agent").map(|agent| format!("agent:{agent}"))
        });
    let title = (!session.title.trim().is_empty()).then(|| session.title.trim().to_string());

    Some(CliRecentSessionInfo {
        title,
        model_label,
        preset_label,
    })
}

async fn cli_load_recent_session_info(
    api_client: &CliApiClient,
    current_dir: &Path,
) -> Option<CliRecentSessionInfo> {
    let sessions = api_client.list_sessions(None, Some(20)).await.ok()?;
    cli_recent_session_info_for_directory(&sessions, current_dir)
}

fn cli_render_startup_banner(style: &CliStyle, recent: Option<&CliRecentSessionInfo>) -> String {
    let mut out = String::new();
    out.push_str("\r\n");

    for (idx, line) in logo_lines("").into_iter().enumerate() {
        let rendered = if idx == 0 {
            style.bold_rgb(&line, 94, 196, 255)
        } else {
            style.rgb(&line, 145, 167, 196)
        };
        out.push_str(&rendered);
        out.push_str("\r\n");
    }

    out.push_str(&style.dim(&format!(
        "{APP_SHORT_NAME} {APP_VERSION_DATE} · {APP_TAGLINE}"
    )));
    out.push_str("\r\n");

    if let Some(recent) = recent {
        if let Some(title) = recent.title.as_deref() {
            out.push_str(&style.bold("Last session: "));
            out.push_str(title);
            out.push_str("\r\n");
        }
        out.push_str(&style.bold("Last model: "));
        out.push_str(recent.model_label.as_deref().unwrap_or("—"));
        out.push_str("\r\n");
        out.push_str(&style.bold("Last preset: "));
        out.push_str(recent.preset_label.as_deref().unwrap_or("—"));
        out.push_str("\r\n");
    }

    out.push_str("\r\n");
    out
}

#[cfg(test)]
mod frontend_state_tests {
    use super::resolve_scheduler_runtime;
    use rocode_config::Config;

    #[test]
    fn explicit_unknown_scheduler_profile_fails_instead_of_silent_fallback() {
        let error = resolve_scheduler_runtime(&Config::default(), Some("missing"))
            .expect_err("explicitly requested unknown scheduler profile should fail");
        let message = error.to_string();

        assert!(message.contains("Scheduler profile could not be resolved"));
        assert!(message.contains("missing"));
    }

    #[test]
    fn no_requested_scheduler_profile_keeps_default_cli_behavior() {
        let resolution = resolve_scheduler_runtime(&Config::default(), None)
            .expect("missing scheduler config without explicit request should not fail");

        assert!(resolution.defaults.is_none());
        assert!(resolution.profile_model.is_none());
    }
}
