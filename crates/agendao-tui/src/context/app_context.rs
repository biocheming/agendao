use agendao_command_render::output_blocks::SchedulerStageBlock;
use agendao_command_runtime::interactive::InteractiveEventsQuery;
use agendao_stage_protocol::StageSummary;
use agendao_types::{SessionUsage, SessionUsageBooks};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use crate::api::{ApiClient, SessionExecutionTopology, SessionTelemetrySnapshot};
use crate::bridge::{UiBridge, UiBridgeSnapshot};
use crate::components::SessionView;
use crate::context::{
    collect_attached_sessions_from_stage_summaries, AttachedSessionInfo, KeybindRegistry, Message,
    MessagePart, MessageRole, SessionContext, TokenUsage,
};
use crate::event::{CustomEvent, Event};
use crate::router::Router;
use crate::theme::Theme;
use agendao_config::{Config as AppConfig, RuntimeBudgetConfig, UiPreferencesConfig};
use agendao_core::process_registry::ProcessInfo;
use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;

#[derive(Clone)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub models: Vec<ModelInfo>,
}

#[derive(Clone)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub context_window: u64,
    pub max_output_tokens: u64,
    pub supports_vision: bool,
    pub supports_tools: bool,
    pub cost_per_million_input: Option<f64>,
    pub cost_per_million_output: Option<f64>,
}

#[derive(Clone)]
pub struct McpServerStatus {
    pub name: String,
    pub status: McpConnectionStatus,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum McpConnectionStatus {
    Connected,
    Disconnected,
    Failed,
    NeedsAuth,
    NeedsClientRegistration,
    Disabled,
}

#[derive(Clone)]
pub struct LspStatus {
    pub id: String,
    pub root: String,
    pub status: LspConnectionStatus,
}

#[derive(Clone, Debug)]
pub enum LspConnectionStatus {
    Connected,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarMode {
    Auto,
    Show,
    Hide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SidebarTab {
    #[default]
    Session,
    Workspace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarLifecycleState {
    pub mode: SidebarMode,
    pub visible: bool,
    pub active_tab: SidebarTab,
    pub process_selected: usize,
    pub process_focus: bool,
    pub attached_session_selected: usize,
    pub attached_session_focus: bool,
    pub workspace_selected: usize,
    pub workspace_focus: bool,
}

impl Default for SidebarLifecycleState {
    fn default() -> Self {
        Self {
            mode: SidebarMode::Auto,
            visible: false,
            active_tab: SidebarTab::Session,
            process_selected: 0,
            process_focus: false,
            attached_session_selected: 0,
            attached_session_focus: false,
            workspace_selected: 0,
            workspace_focus: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageDensity {
    Compact,
    Cozy,
}

pub const SESSION_SIDEBAR_WIDE_THRESHOLD: u16 = 120;

#[derive(Clone, Debug)]
pub struct UiPreferencesState {
    pub show_header: bool,
    pub show_scrollbar: bool,
    pub tips_hidden: bool,
    pub show_timestamps: bool,
    pub show_thinking: bool,
    pub show_tool_details: bool,
    pub message_density: MessageDensity,
    pub semantic_highlight: bool,
}

impl Default for UiPreferencesState {
    fn default() -> Self {
        Self {
            show_header: true,
            show_scrollbar: false,
            tips_hidden: false,
            show_timestamps: false,
            show_thinking: true,
            show_tool_details: true,
            message_density: MessageDensity::Compact,
            semantic_highlight: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SelectionState {
    pub current_agent: String,
    pub current_scheduler_profile: Option<String>,
    pub current_model: Option<String>,
    pub current_provider: Option<String>,
    pub current_variant: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct TuiEventsBrowserState {
    pub session_id: String,
    pub filter: InteractiveEventsQuery,
    pub offset: usize,
}

#[derive(Clone, Debug, Default)]
pub struct TuiMemoryListState {
    pub query: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TuiMemoryDetailState {
    pub record_id: String,
}

#[derive(Clone, Debug, Default)]
pub struct TuiMemoryPreviewState {
    pub query: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct TuiMemoryRuleHitsState {
    pub raw_query: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct TuiMemoryConsolidationState {
    pub raw_request: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DialogSlot {
    Alert,
    Help,
    RecoveryAction,
    Status,
    SessionRename,
    SessionExport,
    PromptStash,
    SkillList,
    SlashPopup,
    CommandPalette,
    ModelSelect,
    AgentSelect,
    SessionList,
    ThemeList,
    Mcp,
    Timeline,
    Fork,
    Provider,
    Subagent,
    ToolCallCancel,
    Tag,
}

#[derive(Clone, Debug, Default)]
pub enum StatusDialogView {
    #[default]
    Overview,
    Runtime,
    Usage,
    Insights,
    ConfigValidation,
    Events(TuiEventsBrowserState),
    MemoryList(TuiMemoryListState),
    MemoryPreview(TuiMemoryPreviewState),
    MemoryDetail(TuiMemoryDetailState),
    MemoryValidation(TuiMemoryDetailState),
    MemoryConflicts(TuiMemoryDetailState),
    MemoryRulePacks,
    MemoryRuleHits(TuiMemoryRuleHitsState),
    MemoryConsolidationRuns,
    MemoryConsolidationResult(TuiMemoryConsolidationState),
}

#[derive(Clone, Debug, Default)]
pub struct DialogLifecycleState {
    pub status_dialog_view: StatusDialogView,
    pub open_dialogs: Vec<DialogSlot>,
}

#[derive(Clone, Debug, Default)]
pub struct SessionState {
    pub data: SessionContext,
}

impl Deref for SessionState {
    type Target = SessionContext;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl DerefMut for SessionState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

#[derive(Clone, Debug, Default)]
pub struct SessionAuthorityState {
    pub attached_sessions: Vec<AttachedSessionInfo>,
    pub execution_topology: Option<SessionExecutionTopology>,
    pub stage_summaries: Vec<StageSummary>,
    pub session_usage: Option<SessionUsage>,
    pub session_usage_books: Option<SessionUsageBooks>,
    pub session_context_compaction_summary: Option<crate::api::ContextCompactionSummary>,
    pub session_context_compaction_lifecycle_summary:
        Option<crate::api::ContextCompactionLifecycleSummary>,
    pub session_cache_semantics: Option<crate::api::SessionCacheSemanticsSummary>,
    pub session_context_closure_contract: Option<crate::api::SessionContextClosureContract>,
    pub session_runtime: Option<crate::api::SessionRuntimeState>,
}

impl MessageDensity {
    pub fn from_str_lossy(s: &str) -> Self {
        if s.eq_ignore_ascii_case("cozy") {
            Self::Cozy
        } else {
            Self::Compact
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Cozy => "cozy",
        }
    }
}

const DIALOG_CLOSE_PRIORITY: [DialogSlot; 21] = [
    DialogSlot::Alert,
    DialogSlot::Help,
    DialogSlot::RecoveryAction,
    DialogSlot::Status,
    DialogSlot::SessionRename,
    DialogSlot::SessionExport,
    DialogSlot::PromptStash,
    DialogSlot::SkillList,
    DialogSlot::SlashPopup,
    DialogSlot::CommandPalette,
    DialogSlot::ModelSelect,
    DialogSlot::AgentSelect,
    DialogSlot::SessionList,
    DialogSlot::ThemeList,
    DialogSlot::Mcp,
    DialogSlot::Timeline,
    DialogSlot::Fork,
    DialogSlot::Provider,
    DialogSlot::Subagent,
    DialogSlot::ToolCallCancel,
    DialogSlot::Tag,
];

const DIALOG_SCROLL_PRIORITY: [DialogSlot; 14] = [
    DialogSlot::Status,
    DialogSlot::PromptStash,
    DialogSlot::SkillList,
    DialogSlot::SlashPopup,
    DialogSlot::CommandPalette,
    DialogSlot::ModelSelect,
    DialogSlot::AgentSelect,
    DialogSlot::SessionList,
    DialogSlot::ThemeList,
    DialogSlot::Mcp,
    DialogSlot::Timeline,
    DialogSlot::Fork,
    DialogSlot::Provider,
    DialogSlot::Subagent,
];

pub struct AppContext {
    pub theme: RwLock<Theme>,
    pub theme_name: RwLock<String>,
    pub router: RwLock<Router>,
    pub keybind: RwLock<KeybindRegistry>,
    pub session: RwLock<SessionState>,
    session_authority: RwLock<HashMap<String, SessionAuthorityState>>,
    session_view: RwLock<Option<SessionView>>,
    pub providers: RwLock<Vec<ProviderInfo>>,
    pub mcp_servers: RwLock<Vec<McpServerStatus>>,
    pub lsp_status: RwLock<Vec<LspStatus>>,
    pub ui_bridge: UiBridge,
    selection: RwLock<SelectionState>,
    pub directory: RwLock<String>,
    dialog_lifecycle: RwLock<DialogLifecycleState>,
    pub animations_enabled: RwLock<bool>,
    pub pending_permissions: RwLock<usize>,
    ui_preferences: RwLock<UiPreferencesState>,
    runtime_budget: RwLock<RuntimeBudgetConfig>,
    recent_models: RwLock<Vec<(String, String)>>,
    pub has_connected_provider: RwLock<bool>,
    pub processes: RwLock<Vec<ProcessInfo>>,
    pub api_client: RwLock<Option<Arc<ApiClient>>>,
}

impl AppContext {
    pub fn new() -> Self {
        let default_theme_name = default_theme_name();
        let default_theme = Theme::by_name(&default_theme_name).unwrap_or_else(Theme::dark);
        Self {
            theme: RwLock::new(default_theme),
            theme_name: RwLock::new(default_theme_name),
            router: RwLock::new(Router::new()),
            keybind: RwLock::new(KeybindRegistry::new()),
            session: RwLock::new(SessionState {
                data: SessionContext::new(),
                ..Default::default()
            }),
            session_authority: RwLock::new(HashMap::new()),
            session_view: RwLock::new(None),
            providers: RwLock::new(Vec::new()),
            mcp_servers: RwLock::new(Vec::new()),
            lsp_status: RwLock::new(Vec::new()),
            ui_bridge: UiBridge::new(),
            selection: RwLock::new(SelectionState::default()),
            directory: RwLock::new(String::new()),
            dialog_lifecycle: RwLock::new(DialogLifecycleState::default()),
            animations_enabled: RwLock::new(true),
            pending_permissions: RwLock::new(0),
            ui_preferences: RwLock::new(UiPreferencesState::default()),
            runtime_budget: RwLock::new(RuntimeBudgetConfig::default()),
            recent_models: RwLock::new(Vec::new()),
            has_connected_provider: RwLock::new(false),
            processes: RwLock::new(Vec::new()),
            api_client: RwLock::new(None),
        }
    }

    pub fn apply_session_telemetry_snapshot(&self, telemetry: SessionTelemetrySnapshot) {
        let session_id = telemetry.runtime.session_id.clone();
        let stages = telemetry.stages;
        let attached_sessions = {
            let session = self.session.read();
            collect_attached_sessions_from_stage_summaries(&stages, &session.sessions)
        };
        let mut authority = self.session_authority.write();
        let entry = authority.entry(session_id).or_default();
        entry.execution_topology = Some(telemetry.topology);
        entry.stage_summaries = stages;
        entry.attached_sessions = attached_sessions;
        entry.session_usage = Some(telemetry.usage);
        entry.session_usage_books = Some(telemetry.usage_books);
        entry.session_context_compaction_summary = telemetry.context_compaction_summary;
        entry.session_context_compaction_lifecycle_summary =
            telemetry.context_compaction_lifecycle_summary;
        entry.session_cache_semantics = telemetry.cache_semantics;
        entry.session_context_closure_contract = telemetry.context_closure_contract;
        entry.session_runtime = Some(telemetry.runtime);
    }

    pub fn apply_session_runtime_snapshot(&self, runtime: crate::api::SessionRuntimeState) {
        let session_id = runtime.session_id.clone();
        self.session_authority
            .write()
            .entry(session_id)
            .or_default()
            .session_runtime = Some(runtime);
    }

    pub fn apply_tool_call_upsert(
        &self,
        session_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        phase: agendao_server_core::runtime_events::ToolCallPhase,
    ) -> bool {
        let mut authority = self.session_authority.write();
        let runtime = authority
            .entry(session_id.to_string())
            .or_default()
            .session_runtime
            .get_or_insert_with(|| crate::api::SessionRuntimeState {
                session_id: session_id.to_string(),
                run_status: crate::api::SessionRunStatusKind::Idle,
                current_message_id: None,
                usage: None,
                active_stage_id: None,
                active_stage_count: 0,
                active_tools: Vec::new(),
                pending_question: None,
                pending_permission: None,
                pending_followup_count: 0,
                attached_sessions: Vec::new(),
            });

        match phase {
            agendao_server_core::runtime_events::ToolCallPhase::Start => {
                if runtime.active_tools.is_empty()
                    && matches!(runtime.run_status, crate::api::SessionRunStatusKind::Idle)
                {
                    runtime.run_status = crate::api::SessionRunStatusKind::WaitingOnTool;
                }
                if runtime
                    .active_tools
                    .iter()
                    .all(|tool| tool.tool_call_id != tool_call_id)
                {
                    runtime.active_tools.push(crate::api::ActiveToolSummary {
                        tool_call_id: tool_call_id.to_string(),
                        tool_name: tool_name.to_string(),
                        started_at: chrono::Utc::now().timestamp_millis(),
                    });
                }
            }
            agendao_server_core::runtime_events::ToolCallPhase::Complete => {
                runtime
                    .active_tools
                    .retain(|tool| tool.tool_call_id != tool_call_id);
                if runtime.active_tools.is_empty()
                    && matches!(
                        runtime.run_status,
                        crate::api::SessionRunStatusKind::WaitingOnTool
                    )
                {
                    runtime.run_status = crate::api::SessionRunStatusKind::Idle;
                }
            }
        }
        true
    }

    pub fn apply_session_projection_snapshot(
        &self,
        session_id: &str,
        topology: Option<SessionExecutionTopology>,
        stages: Vec<StageSummary>,
        usage: Option<SessionUsage>,
        usage_books: Option<SessionUsageBooks>,
        context_compaction_summary: Option<crate::api::ContextCompactionSummary>,
        context_compaction_lifecycle_summary: Option<crate::api::ContextCompactionLifecycleSummary>,
        cache_semantics: Option<crate::api::SessionCacheSemanticsSummary>,
        context_closure_contract: Option<crate::api::SessionContextClosureContract>,
    ) {
        let attached_sessions = {
            let session = self.session.read();
            collect_attached_sessions_from_stage_summaries(&stages, &session.sessions)
        };
        let mut authority = self.session_authority.write();
        let entry = authority.entry(session_id.to_string()).or_default();
        entry.execution_topology = topology;
        entry.stage_summaries = stages;
        entry.attached_sessions = attached_sessions;
        entry.session_usage = usage;
        entry.session_usage_books = usage_books;
        entry.session_context_compaction_summary = context_compaction_summary;
        entry.session_context_compaction_lifecycle_summary =
            context_compaction_lifecycle_summary;
        entry.session_cache_semantics = cache_semantics;
        entry.session_context_closure_contract = context_closure_contract;
    }

    pub fn apply_scheduler_stage_summary(&self, session_id: &str, block: &SchedulerStageBlock) {
        let summary = block.to_summary();
        if summary.stage_id.is_empty() {
            return;
        }

        let sessions = self.session.read().sessions.clone();
        let mut authority = self.session_authority.write();
        let entry = authority.entry(session_id.to_string()).or_default();
        if let Some(existing) = entry
            .stage_summaries
            .iter_mut()
            .find(|stage| stage.stage_id == summary.stage_id)
        {
            *existing = summary;
        } else {
            entry.stage_summaries.push(summary);
            entry.stage_summaries.sort_by(|left, right| {
                let left_index = left.index.unwrap_or(u64::MAX);
                let right_index = right.index.unwrap_or(u64::MAX);
                left_index
                    .cmp(&right_index)
                    .then_with(|| left.stage_id.cmp(&right.stage_id))
            });
        }
        entry.attached_sessions =
            collect_attached_sessions_from_stage_summaries(&entry.stage_summaries, &sessions);
    }

    pub fn navigate(&self, route: crate::router::Route) {
        match &route {
            crate::router::Route::Session { session_id } => {
                self.session
                    .write()
                    .set_current_session_id(session_id.clone());
                self.sync_session_view_route(session_id);
            }
            _ => {
                self.session.write().clear_current_session_id();
                self.clear_session_view_handle();
            }
        }
        self.router.write().navigate(route);
    }

    pub fn navigate_home(&self) {
        self.navigate(crate::router::Route::Home);
    }

    pub fn navigate_session(&self, session_id: impl Into<String>) {
        self.navigate(crate::router::Route::Session {
            session_id: session_id.into(),
        });
    }

    pub fn emit_ui_event(&self, event: Event) -> bool {
        self.ui_bridge.emit(event)
    }

    pub fn emit_custom_event(&self, event: CustomEvent) -> bool {
        self.ui_bridge.emit_custom(event)
    }

    pub fn record_ui_event(&self, event: &crate::event::Event) {
        self.ui_bridge.record(event);
    }

    pub fn ui_bridge_snapshot(&self) -> UiBridgeSnapshot {
        self.ui_bridge.snapshot()
    }

    pub fn ui_bridge_pending_event_count(&self) -> usize {
        self.ui_bridge_snapshot().pending_events
    }

    pub fn ui_bridge_notified(&self) -> tokio::sync::futures::Notified<'_> {
        self.ui_bridge.notified()
    }

    pub fn drain_ui_events(&self, limit: usize) -> Vec<Event> {
        self.ui_bridge.drain(limit)
    }

    pub fn current_route(&self) -> crate::router::Route {
        self.router.read().current().clone()
    }

    pub fn current_route_session_id(&self) -> Option<String> {
        self.router.read().session_id().map(str::to_string)
    }

    pub(crate) fn graph_root_session_id(&self, session_id: &str) -> String {
        let session = self.session.read();
        let mut current = session_id;
        let mut root = session_id.to_string();
        while let Some(parent_id) = session
            .sessions
            .get(current)
            .and_then(|session| session.parent_id.as_deref())
        {
            root = parent_id.to_string();
            current = parent_id;
        }
        root
    }

    pub fn attached_sessions(&self) -> Vec<AttachedSessionInfo> {
        self.current_route_session_id()
            .as_deref()
            .map(|session_id| {
                let graph_root_id = self.graph_root_session_id(session_id);
                self.attached_sessions_for(&graph_root_id)
            })
            .unwrap_or_default()
    }

    pub fn attached_sessions_for(&self, session_id: &str) -> Vec<AttachedSessionInfo> {
        self.session_authority
            .read()
            .get(session_id)
            .map(|state| state.attached_sessions.clone())
            .unwrap_or_default()
    }

    pub fn set_attached_sessions(&self, session_id: &str, attached_sessions: Vec<AttachedSessionInfo>) {
        self.session_authority
            .write()
            .entry(session_id.to_string())
            .or_default()
            .attached_sessions = attached_sessions;
    }

    pub fn execution_topology(&self) -> Option<SessionExecutionTopology> {
        self.current_route_session_id()
            .as_deref()
            .and_then(|session_id| self.execution_topology_for(session_id))
    }

    pub fn execution_topology_for(&self, session_id: &str) -> Option<SessionExecutionTopology> {
        self.session_authority
            .read()
            .get(session_id)
            .and_then(|state| state.execution_topology.clone())
    }

    pub fn stage_summaries(&self) -> Vec<StageSummary> {
        self.current_route_session_id()
            .as_deref()
            .map(|session_id| self.stage_summaries_for(session_id))
            .unwrap_or_default()
    }

    pub fn stage_summaries_for(&self, session_id: &str) -> Vec<StageSummary> {
        self.session_authority
            .read()
            .get(session_id)
            .map(|state| state.stage_summaries.clone())
            .unwrap_or_default()
    }

    pub fn session_usage(&self) -> Option<SessionUsage> {
        self.current_route_session_id()
            .as_deref()
            .and_then(|session_id| self.session_usage_for(session_id))
    }

    pub fn session_usage_for(&self, session_id: &str) -> Option<SessionUsage> {
        self.session_authority
            .read()
            .get(session_id)
            .and_then(|state| state.session_usage.clone())
    }

    pub fn session_usage_books(&self) -> Option<SessionUsageBooks> {
        self.current_route_session_id()
            .as_deref()
            .and_then(|session_id| self.session_usage_books_for(session_id))
    }

    pub fn session_usage_books_for(&self, session_id: &str) -> Option<SessionUsageBooks> {
        self.session_authority
            .read()
            .get(session_id)
            .and_then(|state| state.session_usage_books.clone())
    }

    pub fn session_context_compaction_summary(
        &self,
    ) -> Option<crate::api::ContextCompactionSummary> {
        self.current_route_session_id()
            .as_deref()
            .and_then(|session_id| self.session_context_compaction_summary_for(session_id))
    }

    pub fn session_context_compaction_summary_for(
        &self,
        session_id: &str,
    ) -> Option<crate::api::ContextCompactionSummary> {
        self.session_authority
            .read()
            .get(session_id)
            .and_then(|state| state.session_context_compaction_summary.clone())
    }

    pub fn session_context_compaction_lifecycle_summary(
        &self,
    ) -> Option<crate::api::ContextCompactionLifecycleSummary> {
        self.current_route_session_id()
            .as_deref()
            .and_then(|session_id| self.session_context_compaction_lifecycle_summary_for(session_id))
    }

    pub fn session_context_compaction_lifecycle_summary_for(
        &self,
        session_id: &str,
    ) -> Option<crate::api::ContextCompactionLifecycleSummary> {
        self.session_authority.read().get(session_id).and_then(|state| {
            state
                .session_context_compaction_lifecycle_summary
                .clone()
        })
    }

    pub fn session_cache_semantics(&self) -> Option<crate::api::SessionCacheSemanticsSummary> {
        self.current_route_session_id()
            .as_deref()
            .and_then(|session_id| {
                self.session_authority
                    .read()
                    .get(session_id)
                    .and_then(|state| state.session_cache_semantics.clone())
            })
    }

    pub fn session_context_closure_contract(
        &self,
    ) -> Option<crate::api::SessionContextClosureContract> {
        self.current_route_session_id()
            .as_deref()
            .and_then(|session_id| self.session_context_closure_contract_for(session_id))
    }

    pub fn session_context_closure_contract_for(
        &self,
        session_id: &str,
    ) -> Option<crate::api::SessionContextClosureContract> {
        self.session_authority.read().get(session_id).and_then(|state| {
            state.session_context_closure_contract.clone()
        })
    }

    pub fn session_runtime(&self) -> Option<crate::api::SessionRuntimeState> {
        self.current_route_session_id()
            .as_deref()
            .and_then(|session_id| self.session_runtime_for(session_id))
    }

    pub fn session_runtime_for(&self, session_id: &str) -> Option<crate::api::SessionRuntimeState> {
        self.session_authority
            .read()
            .get(session_id)
            .and_then(|state| state.session_runtime.clone())
    }

    pub fn current_context_tokens(&self) -> Option<u64> {
        self.current_route_session_id()
            .as_deref()
            .and_then(|session_id| self.current_context_tokens_for(session_id))
    }

    pub fn current_context_tokens_for(&self, session_id: &str) -> Option<u64> {
        let session = self.session.read();
        let authority = self.session_authority.read().get(session_id).cloned();
        current_context_tokens_from_state(&session, authority.as_ref())
    }

    pub fn session_terminal_tail_status(&self, session_id: &str) -> Option<String> {
        let session = self.session.read();
        let messages = session.data.messages.get(session_id)?;
        let last_assistant = messages
            .iter()
            .rev()
            .find(|message| matches!(message.role, MessageRole::Assistant))?;

        if last_assistant
            .error
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Some("error".to_string());
        }

        if last_assistant.completed_at.is_some() {
            return Some("complete".to_string());
        }

        None
    }

    pub fn current_session_terminal_tail_status(&self) -> Option<String> {
        let session_id = self.current_route_session_id()?;
        self.session_terminal_tail_status(&session_id)
    }

    pub fn last_assistant_turn_tokens(&self) -> Option<TokenUsage> {
        self.session
            .read()
            .current_messages()
            .iter()
            .rev()
            .find(|message| matches!(message.role, MessageRole::Assistant))
            .map(|message| message.tokens.clone())
    }

    pub fn last_assistant_model(&self) -> Option<String> {
        self.session
            .read()
            .current_messages()
            .iter()
            .rev()
            .find(|message| matches!(message.role, MessageRole::Assistant))
            .and_then(|message| message.model.as_ref())
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty())
    }

    pub fn go_back(&self) -> Option<crate::router::Route> {
        let previous_route = {
            let mut router = self.router.write();
            if router.go_back() {
                Some(router.current().clone())
            } else {
                None
            }
        };
        if let Some(route) = &previous_route {
            match route {
                crate::router::Route::Session { session_id } => {
                    self.session
                        .write()
                        .set_current_session_id(session_id.clone());
                    self.sync_session_view_route(session_id);
                }
                _ => {
                    self.session.write().clear_current_session_id();
                    self.clear_session_view_handle();
                }
            }
        }
        previous_route
    }

    pub fn session_view_handle(&self) -> Option<SessionView> {
        self.session_view.read().clone()
    }

    pub fn ensure_session_view_handle(&self, session_id: &str) -> SessionView {
        {
            let current = self.session_view.read();
            if let Some(view) = current
                .as_ref()
                .filter(|view| view.session_id() == session_id)
            {
                return view.clone();
            }
        }

        let view = SessionView::new(session_id.to_string());
        *self.session_view.write() = Some(view.clone());
        view
    }

    pub fn clear_session_view_handle(&self) {
        *self.session_view.write() = None;
    }

    fn sync_session_view_route(&self, session_id: &str) {
        let stale = self
            .session_view
            .read()
            .as_ref()
            .map(|view| view.session_id() != session_id)
            .unwrap_or(false);
        if stale {
            self.clear_session_view_handle();
        }
    }

    pub fn status_dialog_view(&self) -> StatusDialogView {
        self.dialog_lifecycle.read().status_dialog_view.clone()
    }

    pub fn set_status_dialog_view(&self, view: StatusDialogView) {
        self.dialog_lifecycle.write().status_dialog_view = view;
    }

    pub fn sync_dialog_open(&self, slot: DialogSlot, is_open: bool) {
        let mut lifecycle = self.dialog_lifecycle.write();
        let existing = lifecycle
            .open_dialogs
            .iter()
            .position(|current| *current == slot);
        match (is_open, existing) {
            (true, None) => lifecycle.open_dialogs.push(slot),
            (false, Some(index)) => {
                lifecycle.open_dialogs.remove(index);
            }
            _ => {}
        }
    }

    pub fn close_dialog(&self, slot: DialogSlot) {
        self.sync_dialog_open(slot, false);
    }

    pub fn is_dialog_open(&self, slot: DialogSlot) -> bool {
        self.dialog_lifecycle.read().open_dialogs.contains(&slot)
    }

    pub fn has_open_dialogs(&self) -> bool {
        !self.dialog_lifecycle.read().open_dialogs.is_empty()
    }

    pub fn has_blocking_dialogs(&self) -> bool {
        self.dialog_lifecycle
            .read()
            .open_dialogs
            .iter()
            .any(|slot| *slot != DialogSlot::SlashPopup)
    }

    pub fn top_close_dialog(&self) -> Option<DialogSlot> {
        let lifecycle = self.dialog_lifecycle.read();
        DIALOG_CLOSE_PRIORITY
            .iter()
            .copied()
            .find(|slot| lifecycle.open_dialogs.contains(slot))
    }

    pub fn top_scroll_dialog(&self) -> Option<DialogSlot> {
        let lifecycle = self.dialog_lifecycle.read();
        DIALOG_SCROLL_PRIORITY
            .iter()
            .copied()
            .find(|slot| lifecycle.open_dialogs.contains(slot))
    }

    pub fn toggle_header(&self) {
        let value = {
            let mut prefs = self.ui_preferences.write();
            prefs.show_header = !prefs.show_header;
            prefs.show_header
        };
        self.persist_ui_preferences(UiPreferencesConfig {
            show_header: Some(value),
            ..Default::default()
        });
    }

    pub fn toggle_scrollbar(&self) {
        let value = {
            let mut prefs = self.ui_preferences.write();
            prefs.show_scrollbar = !prefs.show_scrollbar;
            prefs.show_scrollbar
        };
        self.persist_ui_preferences(UiPreferencesConfig {
            show_scrollbar: Some(value),
            ..Default::default()
        });
    }

    pub fn toggle_tips_hidden(&self) {
        let value = {
            let mut prefs = self.ui_preferences.write();
            prefs.tips_hidden = !prefs.tips_hidden;
            prefs.tips_hidden
        };
        self.persist_ui_preferences(UiPreferencesConfig {
            tips_hidden: Some(value),
            ..Default::default()
        });
    }

    pub fn set_model(&self, model: String, provider: String) {
        self.set_model_selection(model, Some(provider));
    }

    pub fn set_model_selection(&self, model: String, provider: Option<String>) {
        let mut selection = self.selection.write();
        selection.current_model = Some(model);
        selection.current_provider = provider;
    }

    pub fn set_model_variant(&self, variant: Option<String>) {
        self.selection.write().current_variant = variant;
    }

    pub fn current_model_variant(&self) -> Option<String> {
        self.selection.read().current_variant.clone()
    }

    pub fn current_model(&self) -> Option<String> {
        self.selection.read().current_model.clone()
    }

    pub fn current_provider(&self) -> Option<String> {
        self.selection.read().current_provider.clone()
    }

    pub fn resolve_model_info(&self, model_ref: Option<&str>) -> Option<ModelInfo> {
        let fallback_model = self.current_model();
        let target = model_ref
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or(fallback_model)?;
        self.providers
            .read()
            .iter()
            .flat_map(|provider| provider.models.iter())
            .find(|model| {
                model.id == target
                    || model
                        .id
                        .rsplit_once('/')
                        .map(|(_, suffix)| suffix == target)
                        .unwrap_or(false)
            })
            .cloned()
    }

    pub fn set_agent(&self, agent: String) {
        let mut selection = self.selection.write();
        selection.current_agent = agent;
        selection.current_scheduler_profile = None;
    }

    pub fn set_scheduler_profile(&self, profile: Option<String>) {
        let has_profile = profile.is_some();
        let mut selection = self.selection.write();
        selection.current_scheduler_profile = profile;
        if has_profile {
            selection.current_agent.clear();
        }
    }

    pub fn current_agent(&self) -> String {
        self.selection.read().current_agent.clone()
    }

    pub fn current_scheduler_profile(&self) -> Option<String> {
        self.selection.read().current_scheduler_profile.clone()
    }

    pub fn selection_state(&self) -> SelectionState {
        self.selection.read().clone()
    }

    pub fn toggle_animations(&self) {
        let mut enabled = self.animations_enabled.write();
        *enabled = !*enabled;
    }

    pub fn set_pending_permissions(&self, count: usize) {
        *self.pending_permissions.write() = count;
    }

    pub fn queued_prompts_for_session(&self, session_id: &str) -> usize {
        self.session_runtime_for(session_id)
            .as_ref()
            .map(|runtime| runtime.pending_followup_count as usize)
            .unwrap_or(0)
    }

    pub fn set_has_connected_provider(&self, connected: bool) {
        *self.has_connected_provider.write() = connected;
    }

    pub fn toggle_timestamps(&self) {
        let value = {
            let mut prefs = self.ui_preferences.write();
            prefs.show_timestamps = !prefs.show_timestamps;
            prefs.show_timestamps
        };
        self.persist_ui_preferences(UiPreferencesConfig {
            show_timestamps: Some(value),
            ..Default::default()
        });
    }

    pub fn toggle_thinking(&self) {
        let value = {
            let mut prefs = self.ui_preferences.write();
            prefs.show_thinking = !prefs.show_thinking;
            prefs.show_thinking
        };
        self.persist_ui_preferences(UiPreferencesConfig {
            show_thinking: Some(value),
            ..Default::default()
        });
    }

    pub fn toggle_tool_details(&self) {
        let value = {
            let mut prefs = self.ui_preferences.write();
            prefs.show_tool_details = !prefs.show_tool_details;
            prefs.show_tool_details
        };
        self.persist_ui_preferences(UiPreferencesConfig {
            show_tool_details: Some(value),
            ..Default::default()
        });
    }

    pub fn toggle_message_density(&self) {
        let density_str = {
            let mut prefs = self.ui_preferences.write();
            prefs.message_density = match prefs.message_density {
                MessageDensity::Compact => MessageDensity::Cozy,
                MessageDensity::Cozy => MessageDensity::Compact,
            };
            prefs.message_density.as_str().to_string()
        };
        self.persist_ui_preferences(UiPreferencesConfig {
            message_density: Some(density_str),
            ..Default::default()
        });
    }

    pub fn toggle_semantic_highlight(&self) {
        let value = {
            let mut prefs = self.ui_preferences.write();
            prefs.semantic_highlight = !prefs.semantic_highlight;
            prefs.semantic_highlight
        };
        self.persist_ui_preferences(UiPreferencesConfig {
            semantic_highlight: Some(value),
            ..Default::default()
        });
    }

    pub fn load_recent_models(&self) -> Vec<(String, String)> {
        self.recent_models.read().clone()
    }

    pub fn save_recent_models(&self, recent: &[(String, String)]) {
        let updated = recent.to_vec();
        *self.recent_models.write() = updated.clone();

        let Some(client) = self.get_api_client() else {
            tracing::warn!("failed to persist recent models: API client unavailable");
            return;
        };

        let payload = updated
            .iter()
            .map(|(provider, model)| RecentModelEntry {
                provider: provider.clone(),
                model: model.clone(),
            })
            .collect::<Vec<_>>();
        match client.put_recent_models(&payload) {
            Ok(persisted) => {
                *self.recent_models.write() = persisted
                    .into_iter()
                    .map(|entry| (entry.provider, entry.model))
                    .collect();
            }
            Err(err) => {
                tracing::warn!(%err, "failed to persist recent models");
            }
        }
    }

    pub fn toggle_theme_mode(&self) -> bool {
        let current = normalize_theme_name(&self.current_theme_name());
        let Some((base, variant)) = split_theme_variant(&current) else {
            return false;
        };
        let next = if variant == "dark" { "light" } else { "dark" };
        self.commit_theme_by_name(&format!("{base}@{next}"))
    }

    pub fn set_theme_by_name(&self, name: &str) -> bool {
        if let Some(theme) = Theme::by_name(name) {
            *self.theme.write() = theme;
            *self.theme_name.write() = normalize_theme_name(name);
            return true;
        }
        false
    }

    pub fn commit_theme_by_name(&self, name: &str) -> bool {
        if !self.set_theme_by_name(name) {
            return false;
        }
        self.persist_ui_preferences(UiPreferencesConfig {
            theme: Some(normalize_theme_name(name)),
            ..Default::default()
        });
        true
    }

    pub fn current_theme_name(&self) -> String {
        self.theme_name.read().clone()
    }

    pub fn available_theme_names(&self) -> Vec<String> {
        let mut names = Theme::builtin_theme_names()
            .into_iter()
            .flat_map(|name| [format!("{name}@dark"), format!("{name}@light")])
            .collect::<Vec<_>>();
        names.sort_by_key(|a| a.to_lowercase());
        names
    }

    pub fn set_api_client(&self, client: Arc<ApiClient>) {
        *self.api_client.write() = Some(client);
    }

    pub fn get_api_client(&self) -> Option<Arc<ApiClient>> {
        self.api_client.read().clone()
    }

    pub fn apply_config(&self, config: &AppConfig) {
        let runtime_budget = RuntimeBudgetConfig::from_config(Some(config));
        self.ui_bridge
            .set_capacity(runtime_budget.max_ui_bridge_queue);
        *self.runtime_budget.write() = runtime_budget;
        let ui = config.ui_preferences.as_ref();
        let theme_name = ui
            .and_then(|prefs| prefs.theme.as_deref())
            .map(normalize_theme_name)
            .unwrap_or_else(default_theme_name);
        if !self.set_theme_by_name(&theme_name) {
            let fallback = default_theme_name();
            let _ = self.set_theme_by_name(&fallback);
        }

        *self.ui_preferences.write() = UiPreferencesState {
            show_header: ui.and_then(|prefs| prefs.show_header).unwrap_or(true),
            show_scrollbar: ui.and_then(|prefs| prefs.show_scrollbar).unwrap_or(false),
            tips_hidden: ui.and_then(|prefs| prefs.tips_hidden).unwrap_or(false),
            show_timestamps: ui.and_then(|prefs| prefs.show_timestamps).unwrap_or(false),
            show_thinking: ui.and_then(|prefs| prefs.show_thinking).unwrap_or(true),
            show_tool_details: ui.and_then(|prefs| prefs.show_tool_details).unwrap_or(true),
            message_density: MessageDensity::from_str_lossy(
                ui.and_then(|prefs| prefs.message_density.as_deref())
                    .unwrap_or("compact"),
            ),
            semantic_highlight: ui
                .and_then(|prefs| prefs.semantic_highlight)
                .unwrap_or(false),
        };
    }

    pub fn ui_preferences(&self) -> UiPreferencesState {
        self.ui_preferences.read().clone()
    }

    pub fn runtime_budget(&self) -> RuntimeBudgetConfig {
        self.runtime_budget.read().clone()
    }

    pub fn show_header_enabled(&self) -> bool {
        self.ui_preferences.read().show_header
    }

    pub fn show_scrollbar_enabled(&self) -> bool {
        self.ui_preferences.read().show_scrollbar
    }

    pub fn tips_hidden(&self) -> bool {
        self.ui_preferences.read().tips_hidden
    }

    pub fn show_timestamps_enabled(&self) -> bool {
        self.ui_preferences.read().show_timestamps
    }

    pub fn show_thinking_enabled(&self) -> bool {
        self.ui_preferences.read().show_thinking
    }

    pub fn show_tool_details_enabled(&self) -> bool {
        self.ui_preferences.read().show_tool_details
    }

    pub fn message_density(&self) -> MessageDensity {
        self.ui_preferences.read().message_density
    }

    pub fn semantic_highlight_enabled(&self) -> bool {
        self.ui_preferences.read().semantic_highlight
    }

    pub fn apply_resolved_workspace_context(&self, context: &ResolvedWorkspaceContext) {
        self.apply_config(&context.config);
        let recent_models = if !context.recent_models.is_empty() {
            context
                .recent_models
                .iter()
                .map(|entry| (entry.provider.clone(), entry.model.clone()))
                .collect()
        } else {
            recent_models_from_config(&context.config)
        };
        *self.recent_models.write() = recent_models;
    }

    pub fn sync_ui_preferences_from_server(&self) -> anyhow::Result<()> {
        let client = self
            .get_api_client()
            .ok_or_else(|| anyhow::anyhow!("API client unavailable"))?;
        match client.get_workspace_context() {
            Ok(context) => self.apply_resolved_workspace_context(&context),
            Err(error) => {
                tracing::warn!(%error, "failed to fetch workspace context; falling back to config");
                let config = client.get_config()?;
                self.apply_config(&config);
                *self.recent_models.write() = recent_models_from_config(&config);
            }
        }
        Ok(())
    }

    fn persist_ui_preferences(&self, prefs: UiPreferencesConfig) {
        if let Err(err) = self.patch_ui_preferences(prefs) {
            tracing::warn!(%err, "failed to persist TUI ui preferences");
        }
    }

    fn patch_ui_preferences(&self, prefs: UiPreferencesConfig) -> anyhow::Result<()> {
        let client = self
            .get_api_client()
            .ok_or_else(|| anyhow::anyhow!("API client unavailable"))?;
        let patch = serde_json::to_value(AppConfig {
            ui_preferences: Some(prefs),
            ..Default::default()
        })?;
        let updated = client.patch_config(&patch)?;
        self.apply_config(&updated);
        Ok(())
    }

    /// Get active tool calls from the server-side session runtime state.
    /// Returns an empty HashMap if session_runtime is not available.
    pub fn get_active_tool_calls(&self) -> HashMap<String, ToolCallInfo> {
        self.session_runtime()
            .as_ref()
            .map(|runtime| {
                runtime
                    .active_tools
                    .iter()
                    .map(|tool| {
                        (
                            tool.tool_call_id.clone(),
                            ToolCallInfo {
                                id: tool.tool_call_id.clone(),
                                tool_name: tool.tool_name.clone(),
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get pending permission from the server-side session runtime state.
    /// Returns None if session_runtime is not available or no pending permission.
    pub fn get_pending_permission(&self) -> Option<(String, PermissionRequestInfo)> {
        self.current_route_session_id()
            .as_deref()
            .and_then(|session_id| self.get_pending_permission_for(session_id))
    }

    pub fn get_pending_permission_for(
        &self,
        session_id: &str,
    ) -> Option<(String, PermissionRequestInfo)> {
        self.session_runtime_for(session_id).as_ref().and_then(|runtime| {
            runtime.pending_permission.as_ref().map(|perm| {
                (
                    perm.permission_id.clone(),
                    PermissionRequestInfo {
                        id: perm.permission_id.clone(),
                        session_id: runtime.session_id.clone(),
                        tool: perm.tool.clone().unwrap_or_default(),
                        permission_class: None,
                        scope_key: None,
                        scope_label: None,
                        matcher_label: None,
                        grant_target_summary: None,
                        risk_tags: Vec::new(),
                        input: serde_json::Value::Null,
                        message: String::new(),
                    },
                )
            })
        })
    }

    /// Check if there's a pending question from the server-side session runtime state.
    pub fn has_pending_question(&self) -> bool {
        self.current_route_session_id()
            .as_deref()
            .is_some_and(|session_id| self.has_pending_question_for(session_id))
    }

    pub fn has_pending_question_for(&self, session_id: &str) -> bool {
        self.session_runtime_for(session_id)
            .as_ref()
            .map(|r| r.pending_question.is_some())
            .unwrap_or(false)
    }

    /// Get pending question request_id from the server-side session runtime state.
    pub fn get_pending_question_id(&self) -> Option<String> {
        self.current_route_session_id()
            .as_deref()
            .and_then(|session_id| self.get_pending_question_id_for(session_id))
    }

    pub fn get_pending_question_id_for(&self, session_id: &str) -> Option<String> {
        self.session_runtime_for(session_id)
            .as_ref()
            .and_then(|r| r.pending_question.as_ref().map(|q| q.request_id.clone()))
    }
}

fn current_context_tokens_from_state(
    state: &SessionState,
    authority: Option<&SessionAuthorityState>,
) -> Option<u64> {
    // Keep the primary TUI context meter aligned with root-session governance.
    // Stage estimates remain visible in runtime/status views, but they should
    // not override the owner session's authoritative live/request counters.
    let usage_context_tokens = authority
        .and_then(|state| state.session_usage_books.as_ref())
        .as_ref()
        .and_then(|books| books.live_context_tokens)
        .or_else(|| {
            authority
                .and_then(|state| state.session_usage.as_ref())
                .as_ref()
                .and_then(|usage| usage.live_context_tokens())
        })
        .or_else(|| {
            authority
                .and_then(|state| state.session_usage_books.as_ref())
                .as_ref()
                .and_then(|books| books.request_context_tokens)
        })
        .filter(|tokens| *tokens > 0);
    let active_stage_id = authority
        .and_then(|state| state.session_runtime.as_ref())
        .as_ref()
        .and_then(|runtime| runtime.active_stage_id.as_deref());
    let active_stage_context_tokens = active_stage_id.and_then(|active_stage_id| {
        authority
            .into_iter()
            .flat_map(|state| state.stage_summaries.iter())
            .find(|stage| stage.stage_id == active_stage_id)
            .and_then(|stage| stage.estimated_context_tokens)
    });
    let estimated_history_context_tokens = state
        .current_session_id
        .as_ref()
        .and_then(|session_id| state.messages.get(session_id))
        .and_then(|messages| estimate_context_tokens_from_history(messages));
    agendao_types::current_context_tokens_from_sources(
        usage_context_tokens,
        active_stage_context_tokens,
    )
    .or(estimated_history_context_tokens)
}

fn estimate_context_tokens_from_history(messages: &[Message]) -> Option<u64> {
    let tail_start = messages
        .iter()
        .rposition(message_marks_compaction_boundary)
        .unwrap_or(0);
    let tail = &messages[tail_start..];

    let mut total_chars = 0usize;
    for message in tail {
        total_chars = total_chars.saturating_add(message.content.len());
        for part in &message.parts {
            match part {
                MessagePart::Text { text } | MessagePart::Reasoning { text } => {
                    total_chars = total_chars.saturating_add(text.len());
                }
                MessagePart::File { path, mime } => {
                    total_chars = total_chars
                        .saturating_add(path.len())
                        .saturating_add(mime.len());
                }
                MessagePart::Image { url } => {
                    total_chars = total_chars.saturating_add(url.len());
                }
                MessagePart::ToolCall {
                    id,
                    name,
                    arguments,
                } => {
                    total_chars = total_chars
                        .saturating_add(id.len())
                        .saturating_add(name.len())
                        .saturating_add(arguments.len());
                }
                MessagePart::ToolResult {
                    id, result, title, ..
                } => {
                    total_chars = total_chars
                        .saturating_add(id.len())
                        .saturating_add(result.len())
                        .saturating_add(title.as_deref().map(str::len).unwrap_or(0));
                }
            }
        }
    }

    (total_chars > 0).then_some(std::cmp::max(1, total_chars / 4) as u64)
}

fn message_marks_compaction_boundary(message: &Message) -> bool {
    if message.content.starts_with("Compacted ") {
        return true;
    }
    message.parts.iter().any(|part| match part {
        MessagePart::Text { text } | MessagePart::Reasoning { text } => {
            text.starts_with("Compacted ")
        }
        _ => false,
    })
}

impl Default for AppContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about an active tool call, used for cancel dialog.
#[derive(Clone, Debug)]
pub struct ToolCallInfo {
    pub id: String,
    pub tool_name: String,
}

/// Information about a permission request.
#[derive(Clone, Debug)]
pub struct PermissionRequestInfo {
    pub id: String,
    pub session_id: String,
    pub tool: String,
    pub permission_class: Option<String>,
    pub scope_key: Option<String>,
    pub scope_label: Option<String>,
    pub matcher_label: Option<String>,
    pub grant_target_summary: Option<String>,
    pub risk_tags: Vec<String>,
    pub input: serde_json::Value,
    pub message: String,
}

fn default_theme_name() -> String {
    format!("opencode@{}", detect_terminal_theme_mode())
}

fn normalize_theme_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return default_theme_name();
    }

    if let Some((base, variant)) = split_theme_variant(trimmed) {
        return format!("{base}@{variant}");
    }

    if trimmed.eq_ignore_ascii_case("dark") {
        return "opencode@dark".to_string();
    }
    if trimmed.eq_ignore_ascii_case("light") {
        return "opencode@light".to_string();
    }

    format!("{trimmed}@dark")
}

fn detect_terminal_theme_mode() -> &'static str {
    if let Ok(mode) = std::env::var("AGENDAO_THEME_MODE") {
        if mode.eq_ignore_ascii_case("light") {
            return "light";
        }
        if mode.eq_ignore_ascii_case("dark") {
            return "dark";
        }
    }

    // Common terminal convention: COLORFGBG="fg;bg", where bg in 0..=6 is dark
    // and 7..=15 is light.
    if let Ok(colorfgbg) = std::env::var("COLORFGBG") {
        if let Some(last) = colorfgbg.split(';').next_back() {
            if let Ok(code) = last.parse::<u8>() {
                return if code <= 6 { "dark" } else { "light" };
            }
        }
    }

    "dark"
}

fn recent_models_from_config(config: &AppConfig) -> Vec<(String, String)> {
    config
        .ui_preferences
        .as_ref()
        .map(|prefs| {
            prefs
                .recent_models
                .iter()
                .map(|entry| (entry.provider.clone(), entry.model.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn split_theme_variant(name: &str) -> Option<(&str, &str)> {
    let (base, variant) = name.rsplit_once('@').or_else(|| name.rsplit_once(':'))?;
    if base.is_empty() || !matches!(variant, "dark" | "light") {
        return None;
    }
    Some((base, variant))
}

#[cfg(test)]
mod tests {
    use super::{current_context_tokens_from_state, AppContext, SessionAuthorityState, SessionState};
    use crate::api::SessionRuntimeState;
    use crate::context::{Message, MessagePart, MessageRole, Session, TokenUsage};
    use agendao_command_render::output_blocks::SchedulerStageBlock;
    use agendao_stage_protocol::{StageStatus, StageSummary};
    use agendao_types::{SessionUsage, SessionUsageBooks, WorkflowUsageSummary};
    use chrono::Utc;

    #[test]
    fn session_view_handle_follows_route_lifecycle() {
        let context = AppContext::new();

        context.navigate_session("session-1");
        let view = context.ensure_session_view_handle("session-1");
        assert_eq!(view.session_id(), "session-1");
        assert_eq!(
            context
                .session_view_handle()
                .as_ref()
                .map(|view| view.session_id()),
            Some("session-1")
        );

        context.navigate_session("session-2");
        assert!(context.session_view_handle().is_none());

        let view = context.ensure_session_view_handle("session-2");
        assert_eq!(view.session_id(), "session-2");

        context.navigate_home();
        assert!(context.session_view_handle().is_none());
    }

    #[test]
    fn per_session_authority_store_switches_with_route() {
        let context = AppContext::new();
        {
            let mut session = context.session.write();
            session.upsert_session(Session {
                id: "session-1".to_string(),
                title: "One".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                parent_id: None,
                share: None,
                metadata: None,
            });
            session.upsert_session(Session {
                id: "session-2".to_string(),
                title: "Two".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                parent_id: None,
                share: None,
                metadata: None,
            });
        }

        context.apply_session_runtime_snapshot(SessionRuntimeState {
            session_id: "session-1".to_string(),
            run_status: crate::api::SessionRunStatusKind::Running,
            current_message_id: None,
            usage: None,
            active_stage_id: Some("stage-1".to_string()),
            active_stage_count: 1,
            active_tools: Vec::new(),
            pending_question: None,
            pending_permission: None,
            pending_followup_count: 0,
            attached_sessions: Vec::new(),
        });
        context.apply_session_runtime_snapshot(SessionRuntimeState {
            session_id: "session-2".to_string(),
            run_status: crate::api::SessionRunStatusKind::WaitingOnTool,
            current_message_id: None,
            usage: None,
            active_stage_id: Some("stage-2".to_string()),
            active_stage_count: 1,
            active_tools: Vec::new(),
            pending_question: None,
            pending_permission: None,
            pending_followup_count: 0,
            attached_sessions: Vec::new(),
        });

        context.navigate_session("session-1");
        assert_eq!(
            context
                .session_runtime()
                .as_ref()
                .and_then(|runtime| runtime.active_stage_id.as_deref()),
            Some("stage-1")
        );

        context.navigate_session("session-2");
        assert_eq!(
            context
                .session_runtime()
                .as_ref()
                .and_then(|runtime| runtime.active_stage_id.as_deref()),
            Some("stage-2")
        );
    }

    #[test]
    fn current_context_tokens_prefers_root_usage_over_active_stage_estimate() {
        let state = SessionState::default();
        let authority = SessionAuthorityState {
            session_usage_books: Some(SessionUsageBooks {
            request_context_tokens: Some(52_830),
            live_context_tokens: Some(52_830),
            workflow_cumulative: WorkflowUsageSummary::default(),
        }),
            session_runtime: Some(SessionRuntimeState {
            session_id: "sess_123".to_string(),
            run_status: crate::api::SessionRunStatusKind::Running,
            current_message_id: None,
            usage: None,
            active_stage_id: Some("stage-exec".to_string()),
            active_stage_count: 1,
            active_tools: Vec::new(),
            pending_question: None,
            pending_permission: None,
            pending_followup_count: 0,
            attached_sessions: Vec::new(),
        }),
            stage_summaries: vec![stage_summary("stage-exec", Some(1_105_000))],
            ..Default::default()
        };

        assert_eq!(current_context_tokens_from_state(&state, Some(&authority)), Some(52_830));
    }

    #[test]
    fn current_context_tokens_falls_back_to_request_usage_before_stage_estimate() {
        let state = SessionState::default();
        let authority = SessionAuthorityState {
            session_usage_books: Some(SessionUsageBooks {
            request_context_tokens: Some(48_000),
            live_context_tokens: None,
            workflow_cumulative: WorkflowUsageSummary::default(),
        }),
            session_runtime: Some(SessionRuntimeState {
            session_id: "sess_456".to_string(),
            run_status: crate::api::SessionRunStatusKind::Running,
            current_message_id: None,
            usage: None,
            active_stage_id: Some("stage-exec".to_string()),
            active_stage_count: 1,
            active_tools: Vec::new(),
            pending_question: None,
            pending_permission: None,
            pending_followup_count: 0,
            attached_sessions: Vec::new(),
        }),
            stage_summaries: vec![stage_summary("stage-exec", Some(990_000))],
            ..Default::default()
        };

        assert_eq!(current_context_tokens_from_state(&state, Some(&authority)), Some(48_000));
    }

    #[test]
    fn current_context_tokens_uses_stage_estimate_only_when_root_usage_missing() {
        let state = SessionState::default();
        let authority = SessionAuthorityState {
            session_usage: Some(SessionUsage {
            context_tokens: 0,
            ..SessionUsage::default()
        }),
            session_runtime: Some(SessionRuntimeState {
            session_id: "sess_789".to_string(),
            run_status: crate::api::SessionRunStatusKind::Running,
            current_message_id: None,
            usage: None,
            active_stage_id: Some("stage-exec".to_string()),
            active_stage_count: 1,
            active_tools: Vec::new(),
            pending_question: None,
            pending_permission: None,
            pending_followup_count: 0,
            attached_sessions: Vec::new(),
        }),
            stage_summaries: vec![stage_summary("stage-exec", Some(256_000))],
            ..Default::default()
        };

        assert_eq!(current_context_tokens_from_state(&state, Some(&authority)), Some(256_000));
    }

    #[test]
    fn current_context_tokens_falls_back_to_history_estimate_like_web() {
        let mut state = SessionState::default();
        state.current_session_id = Some("session-1".to_string());
        state.messages.insert(
            "session-1".to_string(),
            vec![Message {
                id: "assistant-1".to_string(),
                role: MessageRole::Assistant,
                content: "abcd".repeat(500),
                created_at: Utc::now(),
                agent: None,
                model: None,
                mode: None,
                finish: None,
                error: None,
                completed_at: None,
                cost: 0.0,
                tokens: TokenUsage::default(),
                metadata: None,
                multimodal: None,
                parts: vec![MessagePart::Text {
                    text: "efgh".repeat(500),
                }],
            }],
        );

        assert_eq!(current_context_tokens_from_state(&state, None), Some(1000));
    }

    #[test]
    fn apply_scheduler_stage_summary_upserts_current_session_stage() {
        let context = AppContext::new();
        context.navigate_session("session-1");

        context.apply_scheduler_stage_summary(
            "session-1",
            &SchedulerStageBlock {
                stage_id: Some("stage-1".to_string()),
                profile: Some("atlas".to_string()),
                stage: "plan".to_string(),
                title: "Plan".to_string(),
                text: "planning".to_string(),
                stage_index: Some(1),
                stage_total: Some(3),
                step: Some(1),
                status: Some("running".to_string()),
                focus: Some("inspect".to_string()),
                last_event: Some("Step 1 started".to_string()),
                waiting_on: Some("model".to_string()),
                estimated_context_tokens: Some(1234),
                skill_tree_budget: None,
                skill_tree_truncation_strategy: None,
                skill_tree_truncated: None,
                retry_attempt: None,
                activity: Some("Inspecting repository".to_string()),
                loop_budget: Some("step-limit:5".to_string()),
                available_skill_count: None,
                available_agent_count: None,
                available_category_count: None,
                active_skills: Vec::new(),
                active_agents: vec!["planner".to_string()],
                active_categories: Vec::new(),
                done_agent_count: 0,
                total_agent_count: 1,
                prompt_tokens: Some(100),
                context_tokens: Some(100),
                completion_tokens: Some(50),
                reasoning_tokens: Some(25),
                cache_read_tokens: Some(10),
                cache_miss_tokens: Some(0),
                cache_write_tokens: Some(5),
                decision: None,
                attached_session_id: None,
            },
        );

        let summaries = context.stage_summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].stage_id, "stage-1");
        assert_eq!(
            summaries[0].activity.as_deref(),
            Some("Inspecting repository")
        );

        context.apply_scheduler_stage_summary(
            "session-1",
            &SchedulerStageBlock {
                stage_id: Some("stage-1".to_string()),
                profile: Some("atlas".to_string()),
                stage: "plan".to_string(),
                title: "Plan".to_string(),
                text: "planning".to_string(),
                stage_index: Some(1),
                stage_total: Some(3),
                step: Some(2),
                status: Some("waiting".to_string()),
                focus: Some("inspect".to_string()),
                last_event: Some("Tool started: Read".to_string()),
                waiting_on: Some("tool".to_string()),
                estimated_context_tokens: Some(2345),
                skill_tree_budget: None,
                skill_tree_truncation_strategy: None,
                skill_tree_truncated: None,
                retry_attempt: None,
                activity: Some("Reading Cargo.toml".to_string()),
                loop_budget: Some("step-limit:5".to_string()),
                available_skill_count: None,
                available_agent_count: None,
                available_category_count: None,
                active_skills: Vec::new(),
                active_agents: vec!["planner".to_string()],
                active_categories: Vec::new(),
                done_agent_count: 0,
                total_agent_count: 1,
                prompt_tokens: Some(120),
                context_tokens: Some(120),
                completion_tokens: Some(50),
                reasoning_tokens: Some(25),
                cache_read_tokens: Some(20),
                cache_miss_tokens: Some(0),
                cache_write_tokens: Some(5),
                decision: None,
                attached_session_id: None,
            },
        );

        let summaries = context.stage_summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].step, Some(2));
        assert_eq!(summaries[0].waiting_on.as_deref(), Some("tool"));
        assert_eq!(summaries[0].activity.as_deref(), Some("Reading Cargo.toml"));
    }

    #[test]
    fn current_session_terminal_tail_status_detects_complete_and_error() {
        let context = AppContext::new();
        let session_id = {
            let mut session = context.session.write();
            let session_id = session.data.create_session(Some("Test".to_string()));
            session.data.add_message(
                &session_id,
                Message {
                    id: "assistant-complete".to_string(),
                    role: MessageRole::Assistant,
                    content: "done".to_string(),
                    created_at: Utc::now(),
                    agent: None,
                    model: None,
                    mode: None,
                    finish: Some("stop".to_string()),
                    error: None,
                    completed_at: Some(Utc::now()),
                    cost: 0.0,
                    tokens: TokenUsage {
                        output: 12,
                        ..TokenUsage::default()
                    },
                    metadata: None,
                    multimodal: None,
                    parts: Vec::new(),
                },
            );
            session_id
        };
        context.navigate_session(session_id.clone());
        assert_eq!(
            context.current_session_terminal_tail_status().as_deref(),
            Some("complete")
        );

        {
            let mut session = context.session.write();
            session.data.add_message(
                &session_id,
                Message {
                    id: "assistant-error".to_string(),
                    role: MessageRole::Assistant,
                    content: "boom".to_string(),
                    created_at: Utc::now(),
                    agent: None,
                    model: None,
                    mode: None,
                    finish: Some("error".to_string()),
                    error: Some("boom".to_string()),
                    completed_at: Some(Utc::now()),
                    cost: 0.0,
                    tokens: TokenUsage::default(),
                    metadata: None,
                    multimodal: None,
                    parts: Vec::new(),
                },
            );
        }

        assert_eq!(
            context.current_session_terminal_tail_status().as_deref(),
            Some("error")
        );
    }

    fn stage_summary(stage_id: &str, estimated_context_tokens: Option<u64>) -> StageSummary {
        StageSummary {
            stage_id: stage_id.to_string(),
            stage_name: "Execution".to_string(),
            index: None,
            total: None,
            step: None,
            step_total: None,
            status: StageStatus::Running,
            prompt_tokens: None,
            context_tokens: None,
            completion_tokens: None,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_miss_tokens: None,
            cache_write_tokens: None,
            focus: None,
            last_event: None,
            waiting_on: None,
            activity: None,
            estimated_context_tokens,
            skill_tree_budget: None,
            skill_tree_truncation_strategy: None,
            skill_tree_truncated: None,
            retry_attempt: None,
            active_agent_count: 0,
            active_tool_count: 0,
            attached_session_count: 0,
            primary_attached_session_id: None,
        }
    }
}
