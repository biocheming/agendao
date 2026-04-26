use parking_lot::RwLock;
use rocode_command::interactive::InteractiveEventsQuery;
use rocode_command::stage_protocol::StageSummary;
use rocode_session::SessionUsage;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use crate::api::{ApiClient, SessionExecutionTopology, SessionTelemetrySnapshot};
use crate::bridge::{UiBridge, UiBridgeSnapshot};
use crate::components::SessionView;
use crate::context::{
    ChildSessionInfo, KeybindRegistry, Message, MessagePart, MessageRole, SessionContext,
    TokenUsage,
};
use crate::event::{CustomEvent, Event};
use crate::router::Router;
use crate::theme::Theme;
use rocode_config::{Config as AppConfig, UiPreferencesConfig};
use rocode_core::process_registry::ProcessInfo;
use rocode_runtime_context::ResolvedWorkspaceContext;
use rocode_state::RecentModelEntry;

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
    pub child_session_selected: usize,
    pub child_session_focus: bool,
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
            child_session_selected: 0,
            child_session_focus: false,
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
    pub child_sessions: Vec<ChildSessionInfo>,
    pub execution_topology: Option<SessionExecutionTopology>,
    pub stage_summaries: Vec<StageSummary>,
    pub session_usage: Option<SessionUsage>,
    pub session_runtime: Option<crate::api::SessionRuntimeState>,
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

const DIALOG_SCROLL_PRIORITY: [DialogSlot; 13] = [
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
    pub queued_prompts: RwLock<HashMap<String, usize>>,
    ui_preferences: RwLock<UiPreferencesState>,
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
            queued_prompts: RwLock::new(HashMap::new()),
            ui_preferences: RwLock::new(UiPreferencesState::default()),
            recent_models: RwLock::new(Vec::new()),
            has_connected_provider: RwLock::new(false),
            processes: RwLock::new(Vec::new()),
            api_client: RwLock::new(None),
        }
    }

    pub fn apply_session_telemetry_snapshot(&self, telemetry: SessionTelemetrySnapshot) {
        let mut session = self.session.write();
        session.execution_topology = Some(telemetry.topology);
        session.stage_summaries = telemetry.stages;
        session.session_usage = Some(telemetry.usage);
        session.session_runtime = Some(telemetry.runtime);
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

    pub fn drain_ui_events(&self, limit: usize) -> Vec<Event> {
        self.ui_bridge.drain(limit)
    }

    pub fn current_route(&self) -> crate::router::Route {
        self.router.read().current().clone()
    }

    pub fn current_route_session_id(&self) -> Option<String> {
        self.router.read().session_id().map(str::to_string)
    }

    pub fn child_sessions(&self) -> Vec<ChildSessionInfo> {
        self.session.read().child_sessions.clone()
    }

    pub fn set_child_sessions(&self, child_sessions: Vec<ChildSessionInfo>) {
        self.session.write().child_sessions = child_sessions;
    }

    pub fn execution_topology(&self) -> Option<SessionExecutionTopology> {
        self.session.read().execution_topology.clone()
    }

    pub fn stage_summaries(&self) -> Vec<StageSummary> {
        self.session.read().stage_summaries.clone()
    }

    pub fn session_usage(&self) -> Option<SessionUsage> {
        self.session.read().session_usage.clone()
    }

    pub fn session_runtime(&self) -> Option<crate::api::SessionRuntimeState> {
        self.session.read().session_runtime.clone()
    }

    pub fn current_context_tokens(&self) -> Option<u64> {
        current_context_tokens_from_state(&self.session.read())
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

    pub fn set_queued_prompts(&self, session_id: &str, count: usize) {
        let mut queued = self.queued_prompts.write();
        if count == 0 {
            queued.remove(session_id);
        } else {
            queued.insert(session_id.to_string(), count);
        }
    }

    pub fn queued_prompts_for_session(&self, session_id: &str) -> usize {
        self.queued_prompts
            .read()
            .get(session_id)
            .copied()
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
            legacy_recent_models_from_config(&context.config)
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
                *self.recent_models.write() = legacy_recent_models_from_config(&config);
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
        self.session_runtime().as_ref().and_then(|runtime| {
            runtime.pending_permission.as_ref().map(|perm| {
                (
                    perm.permission_id.clone(),
                    PermissionRequestInfo {
                        id: perm.permission_id.clone(),
                        session_id: runtime.session_id.clone(),
                        tool: String::new(), // Extract from info if needed
                        input: perm.info.clone(),
                        message: String::new(),
                    },
                )
            })
        })
    }

    /// Check if there's a pending question from the server-side session runtime state.
    pub fn has_pending_question(&self) -> bool {
        self.session_runtime()
            .as_ref()
            .map(|r| r.pending_question.is_some())
            .unwrap_or(false)
    }

    /// Get pending question request_id from the server-side session runtime state.
    pub fn get_pending_question_id(&self) -> Option<String> {
        self.session_runtime()
            .as_ref()
            .and_then(|r| r.pending_question.as_ref().map(|q| q.request_id.clone()))
    }
}

fn current_context_tokens_from_state(state: &SessionState) -> Option<u64> {
    let usage_context_tokens = state
        .session_usage
        .as_ref()
        .map(|usage| usage.context_tokens)
        .filter(|tokens| *tokens > 0);
    let active_stage_id = state
        .session_runtime
        .as_ref()
        .and_then(|runtime| runtime.active_stage_id.as_deref());
    if let Some(active_stage_id) = active_stage_id {
        if let Some(tokens) = state
            .stage_summaries
            .iter()
            .find(|stage| stage.stage_id == active_stage_id)
            .and_then(|stage| stage.estimated_context_tokens)
            .filter(|tokens| *tokens > 0)
        {
            return Some(usage_context_tokens.map_or(tokens, |usage| usage.max(tokens)));
        }
    }

    usage_context_tokens
        .or_else(|| estimate_current_context_tokens_from_messages(&state.data.current_messages()))
}

fn estimate_current_context_tokens_from_messages(messages: &[&Message]) -> Option<u64> {
    let tail_start = messages
        .iter()
        .rposition(|message| {
            message.parts.iter().any(
                |part| matches!(part, MessagePart::Text { text } if text.starts_with("Compacted ")),
            )
        })
        .unwrap_or(0);
    let tail = &messages[tail_start..];

    latest_prompt_input_tokens(tail).or_else(|| estimate_tail_content_tokens(tail))
}

fn latest_prompt_input_tokens(messages: &[&Message]) -> Option<u64> {
    messages.iter().rev().find_map(|message| {
        if !matches!(message.role, MessageRole::Assistant) {
            return None;
        }
        if message.tokens.input > 0 {
            return Some(message.tokens.input);
        }
        message
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("scheduler_stage_prompt_tokens"))
            .and_then(|value| value.as_u64())
            .filter(|tokens| *tokens > 0)
    })
}

fn estimate_tail_content_tokens(messages: &[&Message]) -> Option<u64> {
    let total_chars: usize = messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .map(|part| match part {
            MessagePart::Text { text } | MessagePart::Reasoning { text } => text.len(),
            MessagePart::File { path, mime } => path.len() + mime.len(),
            MessagePart::Image { url } => url.len(),
            MessagePart::ToolCall {
                name, arguments, ..
            } => name.len() + arguments.len(),
            MessagePart::ToolResult { result, title, .. } => {
                result.len() + title.as_ref().map_or(0, |value| value.len())
            }
        })
        .sum();
    if total_chars == 0 {
        None
    } else {
        Some((total_chars as u64 / 4).max(1))
    }
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
    if let Ok(mode) =
        std::env::var("ROCODE_THEME_MODE").or_else(|_| std::env::var("OPENCODE_THEME_MODE"))
    {
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

fn legacy_recent_models_from_config(config: &AppConfig) -> Vec<(String, String)> {
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
    use super::AppContext;

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
}
