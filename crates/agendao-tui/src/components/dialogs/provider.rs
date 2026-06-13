use agendao_config::ModelConfig;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
};
use reratui::hooks::use_context;
use reratui::Component;
use std::collections::HashSet;

use crate::api::{
    ConnectProtocolOption, ProviderConnectDraft, ProviderConnectSchemaResponse,
    ProviderConnectionDescriptorCandidate, ResolveProviderConnectResponse,
};
use crate::theme::Theme;
use crate::ui::{BufferSurface, RenderSurface};

#[derive(Clone, Debug)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub env_hint: String,
    pub base_url: Option<String>,
    pub protocol: Option<String>,
    pub descriptor_candidate: Option<ProviderConnectionDescriptorCandidate>,
    pub descriptor_candidate_error: Option<String>,
    pub model_count: usize,
    pub status: ProviderStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProviderStatus {
    Connected,
    Disconnected,
    Error,
}

fn display_protocol_label(protocol: &str) -> &str {
    if protocol.eq_ignore_ascii_case("anthropic")
        || protocol.eq_ignore_ascii_case("anthropic-messages")
        || protocol.eq_ignore_ascii_case("messages")
    {
        "Ethnopic"
    } else {
        protocol
    }
}

/// Result of an API key submission attempt.
#[derive(Clone, Debug)]
pub enum SubmitResult {
    Success,
    Failed(String),
}

/// Tracks the current step in the "Add custom provider..." flow.
#[derive(Clone, Debug)]
pub enum CustomProviderStep {
    ProviderId,
    BaseUrl,
    Protocol,
    ApiKey,
}

/// Accumulates values across the 4-step custom provider flow.
#[derive(Clone, Debug)]
pub struct CustomProviderState {
    pub provider_id: String,
    pub base_url: String,
    /// Selected protocol ID (e.g., "openai", "anthropic" for Ethnopic-compatible).
    pub protocol: String,
    pub api_key: String,
    pub step: CustomProviderStep,
}

/// Pending submit payload - either known provider or custom provider.
#[derive(Clone, Debug)]
pub enum PendingSubmit {
    Known {
        provider_id: String,
        api_key: String,
    },
    Custom {
        provider_id: String,
        base_url: String,
        protocol: String,
        api_key: String,
    },
    ModelOverride {
        provider_id: String,
        model_key: String,
        model: ModelConfig,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderConnectMode {
    Known,
    Custom,
    Models,
}

#[derive(Clone, Debug)]
pub struct ProviderModelOverride {
    pub provider_id: String,
    pub model_key: String,
    pub config: ModelConfig,
}

#[derive(Clone, Debug)]
pub enum ModelOverrideStep {
    ProviderId,
    ModelKey,
    ModelId,
    Name,
    BaseUrl,
    Family,
    Flags,
    Status,
    ReleaseDate,
}

#[derive(Clone, Debug)]
pub struct ModelOverrideState {
    pub provider_id: String,
    pub model_key: String,
    pub model_id: String,
    pub name: String,
    pub base_url: String,
    pub family: String,
    pub flags: String,
    pub status: String,
    pub release_date: String,
    pub step: ModelOverrideStep,
}

impl ModelOverrideState {
    fn new() -> Self {
        Self {
            provider_id: String::new(),
            model_key: String::new(),
            model_id: String::new(),
            name: String::new(),
            base_url: String::new(),
            family: String::new(),
            flags: String::new(),
            status: String::new(),
            release_date: String::new(),
            step: ModelOverrideStep::ProviderId,
        }
    }

    fn from_override(item: &ProviderModelOverride) -> Self {
        let mut flags = Vec::new();
        if item.config.reasoning.unwrap_or(false) {
            flags.push("reasoning");
        }
        if item.config.tool_call.unwrap_or(false) {
            flags.push("tool_call");
        }
        if item.config.attachment.unwrap_or(false) {
            flags.push("attachment");
        }
        if item.config.temperature.unwrap_or(false) {
            flags.push("temperature");
        }
        if item.config.experimental.unwrap_or(false) {
            flags.push("experimental");
        }

        Self {
            provider_id: item.provider_id.clone(),
            model_key: item.model_key.clone(),
            model_id: item.config.model.clone().unwrap_or_default(),
            name: item.config.name.clone().unwrap_or_default(),
            base_url: item.config.base_url.clone().unwrap_or_default(),
            family: item.config.family.clone().unwrap_or_default(),
            flags: flags.join(","),
            status: item.config.status.clone().unwrap_or_default(),
            release_date: item.config.release_date.clone().unwrap_or_default(),
            step: ModelOverrideStep::ProviderId,
        }
    }
}

#[derive(Clone)]
pub struct ProviderDialog {
    pub providers: Vec<Provider>,
    pub model_overrides: Vec<ProviderModelOverride>,
    pub resolved_matches: Vec<Provider>,
    pub protocol_options: Vec<ConnectProtocolOption>,
    pub state: ListState,
    pub model_state: ListState,
    pub open: bool,
    pub selected_provider: Option<Provider>,
    pub api_key_input: String,
    pub input_mode: bool,
    /// Brief feedback after submitting a key.
    pub submit_result: Option<SubmitResult>,
    /// Set when the user selects "Add custom provider..." from the list.
    /// Contains the accumulated input values and current step.
    pub custom_state: Option<CustomProviderState>,
    pub model_override_state: Option<ModelOverrideState>,
    /// Index into the fixed protocol list during Protocol step.
    pub protocol_index: usize,
    pub connect_mode: ProviderConnectMode,
    pub search_query: String,
    pub resolve_error: Option<String>,
}

impl ProviderDialog {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            model_overrides: Vec::new(),
            resolved_matches: Vec::new(),
            protocol_options: Vec::new(),
            state: ListState::default(),
            model_state: ListState::default(),
            open: false,
            selected_provider: None,
            api_key_input: String::new(),
            input_mode: false,
            submit_result: None,
            custom_state: None,
            model_override_state: None,
            protocol_index: 0,
            connect_mode: ProviderConnectMode::Known,
            search_query: String::new(),
            resolve_error: None,
        }
    }

    /// Build the provider list from the set of currently connected provider IDs.
    /// Always shows all known providers; marks those in `connected` as Connected.
    /// This is the fallback when the `/provider/known` endpoint is unavailable.
    pub fn populate(&mut self, connected: &HashSet<String>) {
        self.providers
            .retain(|provider| connected.contains(&provider.id));
        for provider in &mut self.providers {
            provider.status = if connected.contains(&provider.id) {
                ProviderStatus::Connected
            } else {
                ProviderStatus::Disconnected
            };
        }
    }

    /// Build the provider list from the dynamic `models.dev` catalogue.
    /// Connected providers are sorted to the top, then alphabetically.
    pub fn populate_from_known(&mut self, entries: Vec<crate::api::KnownProviderEntry>) {
        self.providers = entries
            .into_iter()
            .map(|e| Provider {
                env_hint: e.env.first().cloned().unwrap_or_default(),
                base_url: e.base_url,
                protocol: e.protocol,
                descriptor_candidate: None,
                descriptor_candidate_error: None,
                model_count: e.model_count,
                status: if e.connected {
                    ProviderStatus::Connected
                } else {
                    ProviderStatus::Disconnected
                },
                id: e.id,
                name: e.name,
            })
            .collect();
        // Sort: connected first, then alphabetically by name
        self.providers.sort_by(|a, b| {
            let a_connected = matches!(a.status, ProviderStatus::Connected);
            let b_connected = matches!(b.status, ProviderStatus::Connected);
            b_connected
                .cmp(&a_connected)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
    }

    pub fn populate_from_connect_schema(&mut self, schema: ProviderConnectSchemaResponse) {
        self.populate_from_known(schema.providers);
        self.protocol_options = schema.protocols;
        if self.protocol_index >= self.protocol_options.len() {
            self.protocol_index = 0;
        }
        self.clear_resolution();
    }

    pub fn open(&mut self) {
        self.open = true;
        self.input_mode = false;
        self.api_key_input.clear();
        self.selected_provider = None;
        self.custom_state = None;
        self.model_override_state = None;
        self.protocol_index = 0;
        self.submit_result = None;
        self.connect_mode = ProviderConnectMode::Known;
        self.search_query.clear();
        self.clear_resolution();
        self.state
            .select((!self.visible_providers().is_empty()).then_some(0));
        self.model_state
            .select((!self.model_overrides.is_empty()).then_some(0));
    }

    pub fn close(&mut self) {
        self.open = false;
        self.input_mode = false;
        self.api_key_input.clear();
        self.selected_provider = None;
        self.custom_state = None;
        self.model_override_state = None;
        self.protocol_index = 0;
        self.submit_result = None;
        self.connect_mode = ProviderConnectMode::Known;
        self.search_query.clear();
        self.clear_resolution();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_input_mode(&self) -> bool {
        self.input_mode
    }

    pub fn accepts_text_input(&self) -> bool {
        self.input_mode
            || (self.custom_state.is_some() && !self.is_protocol_step())
            || self.model_override_state.is_some()
    }

    pub fn set_providers(&mut self, providers: Vec<Provider>) {
        self.providers = providers;
        let visible_len = self.visible_providers().len();
        if visible_len == 0 {
            self.state.select(None);
        } else if self.state.selected().is_none() {
            self.state.select(Some(0));
        } else if let Some(selected) = self.state.selected() {
            self.state
                .select(Some(selected.min(visible_len.saturating_sub(1))));
        }
    }

    pub fn set_model_overrides(&mut self, overrides: Vec<ProviderModelOverride>) {
        self.model_overrides = overrides;
        if self.model_overrides.is_empty() {
            self.model_state.select(None);
        } else if self.model_state.selected().is_none() {
            self.model_state.select(Some(0));
        } else if let Some(selected) = self.model_state.selected() {
            self.model_state.select(Some(
                selected.min(self.model_overrides.len().saturating_sub(1)),
            ));
        }
    }

    fn visible_providers(&self) -> &[Provider] {
        if self.search_query.trim().is_empty() {
            &self.providers
        } else {
            &self.resolved_matches
        }
    }

    fn sync_selection_to_visible(&mut self) {
        let visible_len = self.visible_providers().len();
        if visible_len == 0 {
            self.state.select(None);
            return;
        }
        let next = self
            .state
            .selected()
            .unwrap_or(0)
            .min(visible_len.saturating_sub(1));
        self.state.select(Some(next));
    }

    pub fn move_up(&mut self) {
        match self.connect_mode {
            ProviderConnectMode::Known => {
                if let Some(selected) = self.state.selected() {
                    let new = selected.saturating_sub(1);
                    self.state.select(Some(new));
                }
            }
            ProviderConnectMode::Models => {
                if let Some(selected) = self.model_state.selected() {
                    self.model_state.select(Some(selected.saturating_sub(1)));
                }
            }
            ProviderConnectMode::Custom => {}
        }
    }

    pub fn move_down(&mut self) {
        match self.connect_mode {
            ProviderConnectMode::Known => {
                if let Some(selected) = self.state.selected() {
                    let max = self.visible_providers().len().saturating_sub(1);
                    let new = (selected + 1).min(max);
                    self.state.select(Some(new));
                }
            }
            ProviderConnectMode::Models => {
                if let Some(selected) = self.model_state.selected() {
                    let max = self.model_overrides.len().saturating_sub(1);
                    self.model_state.select(Some((selected + 1).min(max)));
                }
            }
            ProviderConnectMode::Custom => {}
        }
    }

    pub fn selected_provider(&self) -> Option<Provider> {
        self.state
            .selected()
            .and_then(|index| self.visible_providers().get(index))
            .cloned()
    }

    /// Enter input mode for the currently highlighted provider.
    pub fn enter_input_mode(&mut self) {
        if self.connect_mode == ProviderConnectMode::Models {
            self.start_model_override_flow();
            return;
        }
        if self.connect_mode == ProviderConnectMode::Custom {
            self.start_custom_flow();
            return;
        }
        // Known provider flow
        if let Some(provider) = self.selected_provider() {
            self.enter_input_mode_for_provider(provider);
        }
    }

    pub fn enter_input_mode_for_provider(&mut self, provider: Provider) {
        self.selected_provider = Some(provider);
        self.api_key_input.clear();
        self.submit_result = None;
        self.input_mode = true;
    }

    /// Go back from input mode to the provider list.
    pub fn exit_input_mode(&mut self) {
        self.input_mode = false;
        self.api_key_input.clear();
        self.submit_result = None;
    }

    /// Exit custom flow and return to list.
    pub fn exit_custom_flow(&mut self) {
        self.custom_state = None;
        self.protocol_index = 0;
        self.submit_result = None;
    }

    pub fn toggle_mode_next(&mut self) {
        match self.connect_mode {
            ProviderConnectMode::Known => self.set_mode(ProviderConnectMode::Custom),
            ProviderConnectMode::Custom => self.set_mode(ProviderConnectMode::Models),
            ProviderConnectMode::Models => self.set_mode(ProviderConnectMode::Known),
        }
    }

    pub fn toggle_mode_prev(&mut self) {
        self.toggle_mode_next();
    }

    pub fn set_mode(&mut self, mode: ProviderConnectMode) {
        if self.connect_mode == mode {
            return;
        }
        self.connect_mode = mode;
        self.submit_result = None;
        self.custom_state = None;
        self.model_override_state = None;
        self.input_mode = false;
        if self.connect_mode == ProviderConnectMode::Known {
            self.sync_selection_to_visible();
        } else if self.connect_mode == ProviderConnectMode::Models
            && !self.model_overrides.is_empty()
        {
            self.model_state
                .select(self.model_state.selected().or(Some(0)));
        }
    }

    fn start_custom_flow(&mut self) {
        self.start_custom_flow_with_prefill(String::new(), String::new(), String::new());
    }

    fn start_model_override_flow(&mut self) {
        let default_provider = self
            .selected_provider()
            .map(|provider| provider.id)
            .or_else(|| {
                self.model_overrides
                    .first()
                    .map(|item| item.provider_id.clone())
            })
            .unwrap_or_default();
        let mut state = ModelOverrideState::new();
        state.provider_id = default_provider;
        self.model_override_state = Some(state);
        self.submit_result = None;
    }

    pub fn start_model_override_edit(&mut self) {
        let Some(item) = self.selected_model_override() else {
            return;
        };
        self.model_override_state = Some(ModelOverrideState::from_override(&item));
        self.submit_result = None;
    }

    pub fn exit_model_override_flow(&mut self) {
        self.model_override_state = None;
        self.submit_result = None;
    }

    pub fn selected_model_override(&self) -> Option<ProviderModelOverride> {
        self.model_state
            .selected()
            .and_then(|index| self.model_overrides.get(index))
            .cloned()
    }

    pub fn is_model_override_final_step(&self) -> bool {
        matches!(
            self.model_override_state.as_ref().map(|state| &state.step),
            Some(ModelOverrideStep::ReleaseDate)
        )
    }

    pub fn advance_model_override_flow(&mut self) -> bool {
        if let Some(state) = self.model_override_state.as_mut() {
            state.step = match state.step {
                ModelOverrideStep::ProviderId => ModelOverrideStep::ModelKey,
                ModelOverrideStep::ModelKey => ModelOverrideStep::ModelId,
                ModelOverrideStep::ModelId => ModelOverrideStep::Name,
                ModelOverrideStep::Name => ModelOverrideStep::BaseUrl,
                ModelOverrideStep::BaseUrl => ModelOverrideStep::Family,
                ModelOverrideStep::Family => ModelOverrideStep::Flags,
                ModelOverrideStep::Flags => ModelOverrideStep::Status,
                ModelOverrideStep::Status => ModelOverrideStep::ReleaseDate,
                ModelOverrideStep::ReleaseDate => return true,
            };
        }
        false
    }

    pub fn back_model_override_flow(&mut self) {
        let next_step = match self.model_override_state.as_ref().map(|state| &state.step) {
            Some(ModelOverrideStep::ProviderId) => {
                self.model_override_state = None;
                return;
            }
            Some(ModelOverrideStep::ModelKey) => Some(ModelOverrideStep::ProviderId),
            Some(ModelOverrideStep::ModelId) => Some(ModelOverrideStep::ModelKey),
            Some(ModelOverrideStep::Name) => Some(ModelOverrideStep::ModelId),
            Some(ModelOverrideStep::BaseUrl) => Some(ModelOverrideStep::Name),
            Some(ModelOverrideStep::Family) => Some(ModelOverrideStep::BaseUrl),
            Some(ModelOverrideStep::Flags) => Some(ModelOverrideStep::Family),
            Some(ModelOverrideStep::Status) => Some(ModelOverrideStep::Flags),
            Some(ModelOverrideStep::ReleaseDate) => Some(ModelOverrideStep::Status),
            None => None,
        };
        if let (Some(state), Some(next_step)) = (self.model_override_state.as_mut(), next_step) {
            state.step = next_step;
        }
    }

    pub fn start_custom_flow_with_prefill(
        &mut self,
        provider_id: String,
        base_url: String,
        protocol: String,
    ) {
        let protocol_index = self
            .protocol_options
            .iter()
            .position(|option| option.id == protocol)
            .unwrap_or(0);
        self.custom_state = Some(CustomProviderState {
            provider_id,
            base_url,
            protocol,
            api_key: String::new(),
            step: CustomProviderStep::ProviderId,
        });
        self.protocol_index = protocol_index;
        self.submit_result = None;
    }

    /// Go back to previous step in custom flow.
    pub fn back_custom_flow(&mut self) {
        if let Some(ref mut state) = self.custom_state {
            match state.step {
                CustomProviderStep::ProviderId => {
                    // At first step - exit custom flow entirely
                    self.exit_custom_flow();
                }
                CustomProviderStep::BaseUrl => {
                    state.step = CustomProviderStep::ProviderId;
                }
                CustomProviderStep::Protocol => {
                    state.step = CustomProviderStep::BaseUrl;
                }
                CustomProviderStep::ApiKey => {
                    state.step = CustomProviderStep::Protocol;
                }
            }
        }
    }

    /// Advance to next step in custom flow. Returns true if now at final step.
    pub fn advance_custom_flow(&mut self) -> bool {
        if let Some(ref mut state) = self.custom_state {
            match state.step {
                CustomProviderStep::ProviderId => {
                    state.step = CustomProviderStep::BaseUrl;
                    false
                }
                CustomProviderStep::BaseUrl => {
                    state.step = CustomProviderStep::Protocol;
                    false
                }
                CustomProviderStep::Protocol => {
                    // Store selected protocol before advancing
                    let protocol_id = self
                        .protocol_options
                        .get(self.protocol_index)
                        .map(|option| option.id.clone())
                        .unwrap_or_else(|| "openai".to_string());
                    state.protocol = protocol_id;
                    state.step = CustomProviderStep::ApiKey;
                    false
                }
                CustomProviderStep::ApiKey => {
                    true // Already at final step
                }
            }
        } else {
            false
        }
    }

    /// Check if currently at the final step (API key entry).
    pub fn is_final_step(&self) -> bool {
        matches!(
            self.custom_state.as_ref().map(|s| &s.step),
            Some(CustomProviderStep::ApiKey)
        )
    }

    /// Check if currently at protocol selection step.
    pub fn is_protocol_step(&self) -> bool {
        matches!(
            self.custom_state.as_ref().map(|s| &s.step),
            Some(CustomProviderStep::Protocol)
        )
    }

    /// Move protocol selection up.
    pub fn protocol_index_dec(&mut self) {
        self.protocol_index = self.protocol_index.saturating_sub(1);
    }

    /// Move protocol selection down.
    pub fn protocol_index_inc(&mut self) {
        self.protocol_index =
            (self.protocol_index + 1).min(self.protocol_options.len().saturating_sub(1));
    }

    pub fn push_char(&mut self, c: char) {
        if let Some(ref mut state) = self.model_override_state {
            match state.step {
                ModelOverrideStep::ProviderId => state.provider_id.push(c),
                ModelOverrideStep::ModelKey => state.model_key.push(c),
                ModelOverrideStep::ModelId => state.model_id.push(c),
                ModelOverrideStep::Name => state.name.push(c),
                ModelOverrideStep::BaseUrl => state.base_url.push(c),
                ModelOverrideStep::Family => state.family.push(c),
                ModelOverrideStep::Flags => state.flags.push(c),
                ModelOverrideStep::Status => state.status.push(c),
                ModelOverrideStep::ReleaseDate => state.release_date.push(c),
            }
        } else if let Some(ref mut state) = self.custom_state {
            match state.step {
                CustomProviderStep::ProviderId => state.provider_id.push(c),
                CustomProviderStep::BaseUrl => state.base_url.push(c),
                CustomProviderStep::Protocol => {}
                CustomProviderStep::ApiKey => state.api_key.push(c),
            }
        } else {
            self.api_key_input.push(c);
        }
        self.submit_result = None;
    }

    pub fn pop_char(&mut self) {
        if let Some(ref mut state) = self.model_override_state {
            match state.step {
                ModelOverrideStep::ProviderId => {
                    state.provider_id.pop();
                }
                ModelOverrideStep::ModelKey => {
                    state.model_key.pop();
                }
                ModelOverrideStep::ModelId => {
                    state.model_id.pop();
                }
                ModelOverrideStep::Name => {
                    state.name.pop();
                }
                ModelOverrideStep::BaseUrl => {
                    state.base_url.pop();
                }
                ModelOverrideStep::Family => {
                    state.family.pop();
                }
                ModelOverrideStep::Flags => {
                    state.flags.pop();
                }
                ModelOverrideStep::Status => {
                    state.status.pop();
                }
                ModelOverrideStep::ReleaseDate => {
                    state.release_date.pop();
                }
            }
        } else if let Some(ref mut state) = self.custom_state {
            match state.step {
                CustomProviderStep::ProviderId => {
                    state.provider_id.pop();
                }
                CustomProviderStep::BaseUrl => {
                    state.base_url.pop();
                }
                CustomProviderStep::Protocol => {}
                CustomProviderStep::ApiKey => {
                    state.api_key.pop();
                }
            }
        } else {
            self.api_key_input.pop();
        }
        self.submit_result = None;
    }

    /// Set the input directly (for clipboard paste).
    pub fn set_input(&mut self, text: String) {
        if let Some(ref mut state) = self.model_override_state {
            match state.step {
                ModelOverrideStep::ProviderId => state.provider_id = text,
                ModelOverrideStep::ModelKey => state.model_key = text,
                ModelOverrideStep::ModelId => state.model_id = text,
                ModelOverrideStep::Name => state.name = text,
                ModelOverrideStep::BaseUrl => state.base_url = text,
                ModelOverrideStep::Family => state.family = text,
                ModelOverrideStep::Flags => state.flags = text,
                ModelOverrideStep::Status => state.status = text,
                ModelOverrideStep::ReleaseDate => state.release_date = text,
            }
        } else if let Some(ref mut state) = self.custom_state {
            match state.step {
                CustomProviderStep::ProviderId => state.provider_id = text,
                CustomProviderStep::BaseUrl => state.base_url = text,
                CustomProviderStep::Protocol => {}
                CustomProviderStep::ApiKey => state.api_key = text,
            }
        } else {
            self.api_key_input = text;
        }
        self.submit_result = None;
    }

    pub fn push_search_char(&mut self, c: char) {
        self.search_query.push(c);
        self.state.select(Some(0));
        self.submit_result = None;
        self.resolve_error = None;
    }

    pub fn pop_search_char(&mut self) {
        self.search_query.pop();
        if self.search_query.trim().is_empty() {
            self.clear_resolution();
        } else {
            self.state.select(Some(0));
        }
        self.submit_result = None;
        self.resolve_error = None;
    }

    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.clear_resolution();
        self.submit_result = None;
    }

    pub fn clear_resolution(&mut self) {
        self.resolved_matches.clear();
        self.resolve_error = None;
        if self.providers.is_empty() {
            self.state.select(None);
        } else {
            self.state.select(Some(0));
        }
    }

    pub fn apply_resolve_response(&mut self, response: ResolveProviderConnectResponse) {
        self.resolved_matches = response
            .matches
            .into_iter()
            .map(provider_from_draft_match)
            .collect();
        self.resolve_error = None;
        if self.visible_providers().is_empty() {
            self.state.select(None);
        } else {
            self.state.select(Some(0));
        }
    }

    pub fn set_resolve_error(&mut self, error: String) {
        self.resolved_matches.clear();
        self.resolve_error = Some(error);
        self.state.select(None);
    }

    /// Returns the pending submit payload if ready.
    /// For known providers: checks input_mode and api_key_input.
    /// For custom providers: checks custom_state is at ApiKey step with non-empty key.
    pub fn pending_submit(&self) -> Option<PendingSubmit> {
        if let Some(state) = self.model_override_state.as_ref() {
            if !matches!(state.step, ModelOverrideStep::ReleaseDate) {
                return None;
            }
            let provider_id = state.provider_id.trim();
            let model_key = state.model_key.trim();
            if provider_id.is_empty() || model_key.is_empty() {
                return None;
            }

            let mut config = ModelConfig::default();
            if !state.model_id.trim().is_empty() {
                config.model = Some(state.model_id.trim().to_string());
            }
            if !state.name.trim().is_empty() {
                config.name = Some(state.name.trim().to_string());
            }
            if !state.base_url.trim().is_empty() {
                config.base_url = Some(state.base_url.trim().to_string());
            }
            if !state.family.trim().is_empty() {
                config.family = Some(state.family.trim().to_string());
            }
            if !state.status.trim().is_empty() {
                config.status = Some(state.status.trim().to_string());
            }
            if !state.release_date.trim().is_empty() {
                config.release_date = Some(state.release_date.trim().to_string());
            }
            for flag in state
                .flags
                .split(',')
                .map(str::trim)
                .filter(|flag| !flag.is_empty())
            {
                match flag.to_ascii_lowercase().replace('-', "_").as_str() {
                    "reasoning" => config.reasoning = Some(true),
                    "tool_call" | "tools" => config.tool_call = Some(true),
                    "attachment" => config.attachment = Some(true),
                    "temperature" => config.temperature = Some(true),
                    "experimental" => config.experimental = Some(true),
                    _ => {}
                }
            }
            return Some(PendingSubmit::ModelOverride {
                provider_id: provider_id.to_string(),
                model_key: model_key.to_string(),
                model: config,
            });
        }

        // Custom provider flow
        if let Some(ref state) = self.custom_state {
            if matches!(state.step, CustomProviderStep::ApiKey) && !state.api_key.trim().is_empty()
            {
                let protocol = if state.protocol.is_empty() {
                    // Use currently selected protocol
                    self.protocol_options
                        .get(self.protocol_index)
                        .map(|option| option.id.clone())
                        .unwrap_or_else(|| "openai".to_string())
                } else {
                    state.protocol.clone()
                };
                return Some(PendingSubmit::Custom {
                    provider_id: state.provider_id.clone(),
                    base_url: state.base_url.clone(),
                    protocol,
                    api_key: state.api_key.trim().to_string(),
                });
            }
            return None;
        }

        // Known provider flow
        if !self.input_mode || self.api_key_input.trim().is_empty() {
            return None;
        }
        self.selected_provider
            .as_ref()
            .map(|p| PendingSubmit::Known {
                provider_id: p.id.clone(),
                api_key: self.api_key_input.trim().to_string(),
            })
    }

    pub fn set_submit_result(&mut self, result: SubmitResult) {
        self.submit_result = Some(result);
    }
    fn render_surface<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let visible_count = match self.connect_mode {
            ProviderConnectMode::Models => self.model_overrides.len().max(1),
            _ => self.visible_providers().len().max(1),
        } as u16;
        let height = (visible_count + 8)
            .clamp(12, 22)
            .min(area.height.saturating_sub(4));
        let width = 56u16.min(area.width.saturating_sub(4));
        let popup_area = super::centered_rect(width, height, area);
        let block = Block::default()
            .title(" Connect Provider ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border));
        let content_area = super::dialog_inner(block.inner(popup_area));
        surface.render_widget(Clear, popup_area);

        if self.model_override_state.is_some() {
            self.render_model_override_input_mode(surface, popup_area, content_area, block, theme);
        } else if self.custom_state.is_some() {
            self.render_custom_input_mode(surface, popup_area, content_area, block, theme);
        } else if self.input_mode {
            self.render_input_mode(surface, popup_area, content_area, block, theme);
        } else {
            self.render_list_mode(surface, popup_area, content_area, block, theme);
        }
    }

    fn render_input_mode<S: RenderSurface>(
        &self,
        surface: &mut S,
        popup_area: Rect,
        content_area: Rect,
        block: Block,
        theme: &Theme,
    ) {
        let provider_name = self
            .selected_provider
            .as_ref()
            .map(|p| p.name.as_str())
            .unwrap_or("");
        let env_hint = self
            .selected_provider
            .as_ref()
            .and_then(|provider| provider.descriptor_candidate.as_ref())
            .map(|descriptor| descriptor.env.join(", "))
            .filter(|value| !value.is_empty())
            .or_else(|| {
                self.selected_provider
                    .as_ref()
                    .map(|provider| provider.env_hint.clone())
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_default();
        let base_url = self
            .selected_provider
            .as_ref()
            .and_then(|provider| {
                provider
                    .descriptor_candidate
                    .as_ref()
                    .and_then(|descriptor| descriptor.base_url.as_deref())
                    .or(provider.base_url.as_deref())
            })
            .unwrap_or("")
            .to_string();
        let profile = self
            .selected_provider
            .as_ref()
            .and_then(|provider| provider.descriptor_candidate.as_ref())
            .and_then(|descriptor| descriptor.profile.as_ref());
        let adapter = self
            .selected_provider
            .as_ref()
            .and_then(|provider| provider.protocol.as_deref())
            .map(display_protocol_label)
            .unwrap_or("")
            .to_string();
        let descriptor_error = self
            .selected_provider
            .as_ref()
            .and_then(|provider| provider.descriptor_candidate_error.as_deref())
            .unwrap_or("");

        // Mask the key: show first 4 chars then asterisks
        let masked = if self.api_key_input.len() > 4 {
            let (head, tail) = self.api_key_input.split_at(4);
            format!("{}{}", head, "*".repeat(tail.len()))
        } else {
            self.api_key_input.clone()
        };

        let mut lines = vec![
            Line::from(Span::styled(
                provider_name,
                Style::default().fg(theme.primary).bold(),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("Env: ", Style::default().fg(theme.text_muted)),
                Span::styled(env_hint, Style::default().fg(theme.warning)),
            ]),
            Line::from(vec![
                Span::styled("Base URL: ", Style::default().fg(theme.text_muted)),
                Span::styled(base_url.as_str(), Style::default().fg(theme.text)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Enter API Key:",
                Style::default().fg(theme.text),
            )),
            Line::from(Span::styled(
                format!("> {}█", masked),
                Style::default().fg(theme.primary),
            )),
            Line::from(""),
        ];

        if let Some(profile) = profile {
            lines.splice(
                4..4,
                [
                    Line::from(vec![
                        Span::styled("API Family: ", Style::default().fg(theme.text_muted)),
                        Span::styled(profile.api_family.as_str(), Style::default().fg(theme.text)),
                    ]),
                    Line::from(vec![
                        Span::styled("API Shape: ", Style::default().fg(theme.text_muted)),
                        Span::styled(profile.api_shape.as_str(), Style::default().fg(theme.text)),
                    ]),
                    Line::from(vec![
                        Span::styled("Transport: ", Style::default().fg(theme.text_muted)),
                        Span::styled(profile.transport.as_str(), Style::default().fg(theme.text)),
                    ]),
                ],
            );
        } else if !adapter.is_empty() {
            lines.insert(
                4,
                Line::from(vec![
                    Span::styled("Adapter: ", Style::default().fg(theme.text_muted)),
                    Span::styled(adapter.as_str(), Style::default().fg(theme.text)),
                ]),
            );
        }

        if !descriptor_error.is_empty() {
            lines.insert(
                lines.len().saturating_sub(4),
                Line::from(vec![
                    Span::styled("Descriptor: ", Style::default().fg(theme.text_muted)),
                    Span::styled(descriptor_error, Style::default().fg(theme.error)),
                ]),
            );
        }

        // Show submit result feedback
        if let Some(ref result) = self.submit_result {
            match result {
                SubmitResult::Success => {
                    lines.push(Line::from(Span::styled(
                        "✓ Connected successfully!",
                        Style::default().fg(theme.success),
                    )));
                }
                SubmitResult::Failed(msg) => {
                    let truncated = if msg.len() > 48 {
                        format!("{}...", &msg[..45])
                    } else {
                        msg.clone()
                    };
                    lines.push(Line::from(Span::styled(
                        format!("✗ {}", truncated),
                        Style::default().fg(theme.error),
                    )));
                }
            }
            lines.push(Line::from(""));
        }

        lines.push(Line::from(vec![
            Span::styled("Enter", Style::default().fg(theme.text)),
            Span::styled(" connect  ", Style::default().fg(theme.text_muted)),
            Span::styled("Esc", Style::default().fg(theme.text)),
            Span::styled(" back", Style::default().fg(theme.text_muted)),
        ]));

        surface.render_widget(
            block.style(Style::default().bg(theme.background_panel)),
            popup_area,
        );
        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(theme.background_panel));
        surface.render_widget(paragraph, content_area);
    }

    fn render_list_mode<S: RenderSurface>(
        &self,
        surface: &mut S,
        popup_area: Rect,
        content_area: Rect,
        block: Block,
        theme: &Theme,
    ) {
        surface.render_widget(
            block.style(Style::default().bg(theme.background_panel)),
            popup_area,
        );

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(3),
            ])
            .split(content_area);

        let known_style = if self.connect_mode == ProviderConnectMode::Known {
            Style::default()
                .fg(theme.primary)
                .bg(theme.background_element)
                .bold()
        } else {
            Style::default().fg(theme.text_muted)
        };
        let custom_style = if self.connect_mode == ProviderConnectMode::Custom {
            Style::default()
                .fg(theme.primary)
                .bg(theme.background_element)
                .bold()
        } else {
            Style::default().fg(theme.text_muted)
        };
        let models_style = if self.connect_mode == ProviderConnectMode::Models {
            Style::default()
                .fg(theme.primary)
                .bg(theme.background_element)
                .bold()
        } else {
            Style::default().fg(theme.text_muted)
        };

        let subtitle = match self.connect_mode {
            ProviderConnectMode::Known => {
                "Search a known provider. Enter connects quickly; A opens advanced editing."
            }
            ProviderConnectMode::Custom => {
                "Create a custom provider with provider id, base URL, protocol and API key."
            }
            ProviderConnectMode::Models => {
                "Manage configured model overrides. N adds, Enter/E edits, D deletes."
            }
        };

        surface.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("Known", known_style),
                    Span::raw("  "),
                    Span::styled("Custom", custom_style),
                    Span::raw("  "),
                    Span::styled("Models", models_style),
                ]),
                Line::from(Span::styled(
                    subtitle,
                    Style::default().fg(theme.text_muted),
                )),
                Line::from(Span::styled(
                    match self.connect_mode {
                        ProviderConnectMode::Known => format!("Search: {}", self.search_query),
                        ProviderConnectMode::Custom => "Manual provider setup wizard".to_string(),
                        ProviderConnectMode::Models => {
                            format!("Overrides: {}", self.model_overrides.len())
                        }
                    },
                    Style::default().fg(theme.text),
                )),
            ])
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(theme.background_panel)),
            sections[0],
        );

        match self.connect_mode {
            ProviderConnectMode::Known => {
                if self.providers.is_empty() {
                    surface.render_widget(
                        Paragraph::new("No known providers available. Switch to Custom to enter an endpoint manually.")
                            .wrap(Wrap { trim: false })
                            .style(Style::default().fg(theme.text_muted).bg(theme.background_panel)),
                        sections[1],
                    );
                } else if self.visible_providers().is_empty() {
                    let message = if let Some(error) = &self.resolve_error {
                        format!("Resolve failed: {}", error)
                    } else if self.search_query.trim().is_empty() {
                        "No known providers available.".to_string()
                    } else {
                        "No known match. Press Enter to use the current search text as a custom provider id."
                            .to_string()
                    };
                    surface.render_widget(
                        Paragraph::new(message).wrap(Wrap { trim: false }).style(
                            Style::default()
                                .fg(theme.text_muted)
                                .bg(theme.background_panel),
                        ),
                        sections[1],
                    );
                } else {
                    let visible = self.visible_providers();
                    let selected = self.state.selected().unwrap_or(0);
                    let list_area = Rect {
                        x: sections[1].x,
                        y: sections[1].y,
                        width: sections[1].width.saturating_sub(1),
                        height: sections[1].height,
                    };

                    if list_area.height > 0 {
                        let viewport = list_area.height as usize;
                        let mut scroll = 0usize;
                        if selected >= viewport {
                            scroll = selected.saturating_sub(viewport.saturating_sub(1));
                        }

                        for (row, (index, provider)) in visible
                            .iter()
                            .enumerate()
                            .skip(scroll)
                            .take(viewport)
                            .enumerate()
                        {
                            let is_selected = index == selected;
                            let status_icon = match provider.status {
                                ProviderStatus::Connected => "●",
                                ProviderStatus::Disconnected => "◯",
                                ProviderStatus::Error => "✗",
                            };
                            let status_color = match provider.status {
                                ProviderStatus::Connected => theme.success,
                                ProviderStatus::Disconnected => theme.text_muted,
                                ProviderStatus::Error => theme.error,
                            };
                            let row_area = Rect {
                                x: list_area.x,
                                y: list_area.y + row as u16,
                                width: list_area.width,
                                height: 1,
                            };
                            let line = Line::from(vec![
                                Span::styled(status_icon, Style::default().fg(status_color)),
                                Span::raw(" "),
                                Span::styled(
                                    &provider.name,
                                    Style::default()
                                        .fg(if is_selected {
                                            theme.primary
                                        } else {
                                            theme.text
                                        })
                                        .bg(if is_selected {
                                            theme.background_element
                                        } else {
                                            theme.background_panel
                                        }),
                                ),
                                Span::styled(
                                    format!(" · {}", provider.id),
                                    Style::default().fg(theme.text_muted).bg(if is_selected {
                                        theme.background_element
                                    } else {
                                        theme.background_panel
                                    }),
                                ),
                            ]);
                            surface.render_widget(Paragraph::new(line), row_area);
                        }

                        if visible.len() > viewport {
                            let scroll_area = Rect {
                                x: list_area.x + list_area.width,
                                y: list_area.y,
                                width: 1,
                                height: list_area.height,
                            };
                            let mut scrollbar_state = ScrollbarState::new(visible.len())
                                .position(scroll)
                                .viewport_content_length(viewport);
                            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                                .begin_symbol(None)
                                .end_symbol(None)
                                .track_symbol(Some("│"))
                                .track_style(Style::default().fg(theme.border_subtle))
                                .thumb_symbol("█")
                                .thumb_style(Style::default().fg(theme.primary));
                            surface.render_stateful_widget(
                                scrollbar,
                                scroll_area,
                                &mut scrollbar_state,
                            );
                        }
                    }
                }
            }
            ProviderConnectMode::Custom => {
                let mut lines = vec![
                    Line::from(Span::styled(
                        "Custom provider setup",
                        Style::default().fg(theme.text).bold(),
                    )),
                    Line::from(""),
                    Line::from("You will be prompted for:"),
                    Line::from("  1. Provider ID"),
                    Line::from("  2. Base URL"),
                    Line::from("  3. Protocol"),
                    Line::from("  4. API Key"),
                ];
                if let Some(result) = &self.submit_result {
                    lines.push(Line::from(""));
                    match result {
                        SubmitResult::Success => lines.push(Line::from(Span::styled(
                            "✓ Connected successfully!",
                            Style::default().fg(theme.success),
                        ))),
                        SubmitResult::Failed(msg) => lines.push(Line::from(Span::styled(
                            format!("✗ {}", msg),
                            Style::default().fg(theme.error),
                        ))),
                    }
                }

                surface.render_widget(
                    Paragraph::new(lines)
                        .wrap(Wrap { trim: false })
                        .style(Style::default().bg(theme.background_panel)),
                    sections[1],
                );
            }
            ProviderConnectMode::Models => {
                if self.model_overrides.is_empty() {
                    surface.render_widget(
                        Paragraph::new("No configured model overrides. Press N to add one.")
                            .wrap(Wrap { trim: false })
                            .style(
                                Style::default()
                                    .fg(theme.text_muted)
                                    .bg(theme.background_panel),
                            ),
                        sections[1],
                    );
                } else {
                    let selected = self.model_state.selected().unwrap_or(0);
                    let list_area = Rect {
                        x: sections[1].x,
                        y: sections[1].y,
                        width: sections[1].width.saturating_sub(1),
                        height: sections[1].height,
                    };
                    let viewport = list_area.height.max(1) as usize;
                    let scroll = if selected >= viewport {
                        selected.saturating_sub(viewport.saturating_sub(1))
                    } else {
                        0
                    };

                    for (row, (index, item)) in self
                        .model_overrides
                        .iter()
                        .enumerate()
                        .skip(scroll)
                        .take(viewport)
                        .enumerate()
                    {
                        let is_selected = index == selected;
                        let row_area = Rect {
                            x: list_area.x,
                            y: list_area.y + row as u16,
                            width: list_area.width,
                            height: 1,
                        };
                        let target = item
                            .config
                            .model
                            .as_deref()
                            .unwrap_or(item.model_key.as_str());
                        let display = item.config.name.as_deref().unwrap_or(target);
                        let line = Line::from(vec![
                            Span::styled(
                                format!("{}/{}", item.provider_id, item.model_key),
                                Style::default()
                                    .fg(if is_selected {
                                        theme.primary
                                    } else {
                                        theme.text
                                    })
                                    .bg(if is_selected {
                                        theme.background_element
                                    } else {
                                        theme.background_panel
                                    }),
                            ),
                            Span::styled(
                                format!(" -> {} ({})", target, display),
                                Style::default().fg(theme.text_muted).bg(if is_selected {
                                    theme.background_element
                                } else {
                                    theme.background_panel
                                }),
                            ),
                        ]);
                        surface.render_widget(Paragraph::new(line), row_area);
                    }

                    if self.model_overrides.len() > viewport {
                        let scroll_area = Rect {
                            x: list_area.x + list_area.width,
                            y: list_area.y,
                            width: 1,
                            height: list_area.height,
                        };
                        let mut scrollbar_state = ScrollbarState::new(self.model_overrides.len())
                            .position(scroll)
                            .viewport_content_length(viewport);
                        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                            .begin_symbol(None)
                            .end_symbol(None)
                            .track_symbol(Some("│"))
                            .track_style(Style::default().fg(theme.border_subtle))
                            .thumb_symbol("█")
                            .thumb_style(Style::default().fg(theme.primary));
                        surface.render_stateful_widget(
                            scrollbar,
                            scroll_area,
                            &mut scrollbar_state,
                        );
                    }
                }
            }
        }

        let footer = match self.connect_mode {
            ProviderConnectMode::Known => {
                let total = self.providers.len();
                let visible = self.visible_providers().len();
                let selected = self.state.selected().map(|index| index + 1).unwrap_or(0);
                vec![
                    Line::from(Span::styled(
                        if self.search_query.trim().is_empty() {
                            format!("{total} known providers · {selected}/{visible} selected")
                        } else {
                            format!(
                                "{visible} matches · {selected}/{visible} selected · {total} total known"
                            )
                        },
                        Style::default().fg(theme.text_muted),
                    )),
                    Line::from(Span::styled(
                        "Type to search  ←/→ or Tab switch mode  ↑↓ select  Enter quick connect/custom fallback  A advanced  Esc clear/close",
                        Style::default().fg(theme.text_muted),
                    )),
                ]
            }
            ProviderConnectMode::Custom => vec![
                Line::from(Span::styled(
                    "Manual provider setup",
                    Style::default().fg(theme.text_muted),
                )),
                Line::from(Span::styled(
                    "←/→ or Tab switch mode  Enter start custom setup  Esc close",
                    Style::default().fg(theme.text_muted),
                )),
            ],
            ProviderConnectMode::Models => vec![
                Line::from(Span::styled(
                    format!("{} configured overrides", self.model_overrides.len()),
                    Style::default().fg(theme.text_muted),
                )),
                Line::from(Span::styled(
                    "←/→ or Tab switch mode  ↑↓ select  N add  Enter/E edit  D delete  Esc close",
                    Style::default().fg(theme.text_muted),
                )),
            ],
        };
        surface.render_widget(
            Paragraph::new(footer).wrap(Wrap { trim: false }).style(
                Style::default()
                    .fg(theme.text_muted)
                    .bg(theme.background_panel),
            ),
            sections[2],
        );
    }

    fn render_model_override_input_mode<S: RenderSurface>(
        &self,
        surface: &mut S,
        popup_area: Rect,
        content_area: Rect,
        block: Block,
        theme: &Theme,
    ) {
        let Some(ref state) = self.model_override_state else {
            return;
        };

        let (step_label, step_num, total_steps, current_value) = match state.step {
            ModelOverrideStep::ProviderId => ("Provider ID", 1, 9, state.provider_id.as_str()),
            ModelOverrideStep::ModelKey => ("Model Key", 2, 9, state.model_key.as_str()),
            ModelOverrideStep::ModelId => ("Upstream Model ID", 3, 9, state.model_id.as_str()),
            ModelOverrideStep::Name => ("Display Name", 4, 9, state.name.as_str()),
            ModelOverrideStep::BaseUrl => ("Base URL", 5, 9, state.base_url.as_str()),
            ModelOverrideStep::Family => ("Family", 6, 9, state.family.as_str()),
            ModelOverrideStep::Flags => ("Flags (comma-separated)", 7, 9, state.flags.as_str()),
            ModelOverrideStep::Status => ("Status", 8, 9, state.status.as_str()),
            ModelOverrideStep::ReleaseDate => ("Release Date", 9, 9, state.release_date.as_str()),
        };

        let mut lines = vec![
            Line::from(Span::styled(
                format!("Model Override (Step {}/{})", step_num, total_steps),
                Style::default().fg(theme.primary).bold(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("{}:", step_label),
                Style::default().fg(theme.text),
            )),
            Line::from(Span::styled(
                format!("> {}█", current_value),
                Style::default().fg(theme.primary),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!(
                    "Provider {} / key {} / target {}",
                    if state.provider_id.is_empty() {
                        "--"
                    } else {
                        state.provider_id.as_str()
                    },
                    if state.model_key.is_empty() {
                        "--"
                    } else {
                        state.model_key.as_str()
                    },
                    if state.model_id.is_empty() {
                        "--"
                    } else {
                        state.model_id.as_str()
                    }
                ),
                Style::default().fg(theme.text_muted),
            )),
            Line::from(Span::styled(
                "Flags: reasoning, tool_call, attachment, temperature, experimental",
                Style::default().fg(theme.text_muted),
            )),
        ];

        if let Some(ref result) = self.submit_result {
            lines.push(Line::from(""));
            match result {
                SubmitResult::Success => lines.push(Line::from(Span::styled(
                    "✓ Model override saved.",
                    Style::default().fg(theme.success),
                ))),
                SubmitResult::Failed(msg) => lines.push(Line::from(Span::styled(
                    format!("✗ {}", msg),
                    Style::default().fg(theme.error),
                ))),
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Enter", Style::default().fg(theme.text)),
            Span::styled(
                if step_num == total_steps {
                    " save  "
                } else {
                    " next  "
                },
                Style::default().fg(theme.text_muted),
            ),
            Span::styled("Esc", Style::default().fg(theme.text)),
            Span::styled(" back", Style::default().fg(theme.text_muted)),
        ]));

        surface.render_widget(
            block.style(Style::default().bg(theme.background_panel)),
            popup_area,
        );
        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(theme.background_panel));
        surface.render_widget(paragraph, content_area);
    }

    fn render_custom_input_mode<S: RenderSurface>(
        &self,
        surface: &mut S,
        popup_area: Rect,
        content_area: Rect,
        block: Block,
        theme: &Theme,
    ) {
        let Some(ref state) = self.custom_state else {
            return;
        };

        // Determine step indicator text
        let (step_label, step_num, total_steps) = match state.step {
            CustomProviderStep::ProviderId => ("Provider ID", 1, 4),
            CustomProviderStep::BaseUrl => ("Base URL", 2, 4),
            CustomProviderStep::Protocol => ("Protocol", 3, 4),
            CustomProviderStep::ApiKey => ("API Key", 4, 4),
        };

        let mut lines = vec![
            Line::from(Span::styled(
                format!("Add Custom Provider (Step {}/{})", step_num, total_steps),
                Style::default().fg(theme.primary).bold(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("{}:", step_label),
                Style::default().fg(theme.text),
            )),
        ];

        // Render current input field
        match state.step {
            CustomProviderStep::ProviderId => {
                lines.push(Line::from(Span::styled(
                    format!("> {}█", state.provider_id),
                    Style::default().fg(theme.primary),
                )));
            }
            CustomProviderStep::BaseUrl => {
                lines.push(Line::from(Span::styled(
                    format!("> {}█", state.base_url),
                    Style::default().fg(theme.primary),
                )));
            }
            CustomProviderStep::Protocol => {
                // Render protocol list with selection
                lines.push(Line::from(""));
                for (i, option) in self.protocol_options.iter().enumerate() {
                    let is_selected = i == self.protocol_index;
                    let style = if is_selected {
                        Style::default()
                            .fg(theme.primary)
                            .bg(theme.background_element)
                    } else {
                        Style::default().fg(theme.text_muted)
                    };
                    let prefix = if is_selected { "› " } else { "  " };
                    lines.push(Line::from(Span::styled(
                        format!("{}{}", prefix, display_protocol_label(&option.name)),
                        style,
                    )));
                }
            }
            CustomProviderStep::ApiKey => {
                // Mask the API key
                let masked = if state.api_key.len() > 4 {
                    let (head, tail) = state.api_key.split_at(4);
                    format!("{}{}", head, "*".repeat(tail.len()))
                } else {
                    state.api_key.clone()
                };
                lines.push(Line::from(Span::styled(
                    format!("> {}█", masked),
                    Style::default().fg(theme.primary),
                )));
            }
        }

        lines.push(Line::from(""));

        // Show submit result feedback
        if let Some(ref result) = self.submit_result {
            match result {
                SubmitResult::Success => {
                    lines.push(Line::from(Span::styled(
                        "✓ Connected successfully!",
                        Style::default().fg(theme.success),
                    )));
                }
                SubmitResult::Failed(msg) => {
                    let truncated = if msg.len() > 48 {
                        format!("{}...", &msg[..45])
                    } else {
                        msg.clone()
                    };
                    lines.push(Line::from(Span::styled(
                        format!("✗ {}", truncated),
                        Style::default().fg(theme.error),
                    )));
                }
            }
            lines.push(Line::from(""));
        }

        // Navigation hints
        if matches!(state.step, CustomProviderStep::Protocol) {
            lines.push(Line::from(vec![
                Span::styled("↑↓", Style::default().fg(theme.text)),
                Span::styled(" select  ", Style::default().fg(theme.text_muted)),
                Span::styled("Enter", Style::default().fg(theme.text)),
                Span::styled(" next  ", Style::default().fg(theme.text_muted)),
                Span::styled("Esc", Style::default().fg(theme.text)),
                Span::styled(" back", Style::default().fg(theme.text_muted)),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("Enter", Style::default().fg(theme.text)),
                Span::styled(
                    if step_num == total_steps {
                        " connect  "
                    } else {
                        " next  "
                    },
                    Style::default().fg(theme.text_muted),
                ),
                Span::styled("Esc", Style::default().fg(theme.text)),
                Span::styled(" back", Style::default().fg(theme.text_muted)),
            ]));
        }

        surface.render_widget(
            block.style(Style::default().bg(theme.background_panel)),
            popup_area,
        );
        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(theme.background_panel));
        surface.render_widget(paragraph, content_area);
    }
}

impl Component for ProviderDialog {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let theme = use_context::<Theme>();
        let mut surface = BufferSurface::new(buffer);
        self.render_surface(&mut surface, area, &theme);
    }
}

impl Default for ProviderDialog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    use crate::ui::BufferSurface;

    #[test]
    fn provider_dialog_renders_to_buffer_surface() {
        let mut dialog = ProviderDialog::new();
        dialog.set_providers(vec![Provider {
            id: "openai".to_string(),
            name: "OpenAI".to_string(),
            env_hint: "OPENAI_API_KEY".to_string(),
            base_url: Some("https://api.openai.com/v1".to_string()),
            protocol: Some("openai".to_string()),
            descriptor_candidate: None,
            descriptor_candidate_error: None,
            model_count: 3,
            status: ProviderStatus::Disconnected,
        }]);
        dialog.open();

        let area = Rect::new(0, 0, 120, 32);
        let mut buffer = Buffer::empty(area);
        let mut surface = BufferSurface::new(&mut buffer);

        dialog.render_surface(&mut surface, area, &Theme::dark());

        let rendered = buffer
            .content
            .iter()
            .filter(|cell| !cell.symbol().trim().is_empty())
            .count();
        assert!(rendered > 0);
    }
}

fn provider_from_draft_match(entry: crate::api::KnownProviderEntry) -> Provider {
    Provider {
        env_hint: entry.env.first().cloned().unwrap_or_default(),
        base_url: entry.base_url,
        protocol: entry.protocol,
        descriptor_candidate: None,
        descriptor_candidate_error: None,
        model_count: entry.model_count,
        status: if entry.connected {
            ProviderStatus::Connected
        } else {
            ProviderStatus::Disconnected
        },
        id: entry.id,
        name: entry.name,
    }
}

pub fn provider_from_connect_draft(draft: &ProviderConnectDraft) -> Provider {
    Provider {
        id: draft.provider_id.clone(),
        name: draft
            .name
            .clone()
            .unwrap_or_else(|| draft.provider_id.clone()),
        env_hint: draft.env.first().cloned().unwrap_or_default(),
        base_url: draft.base_url.clone(),
        protocol: draft.protocol.clone(),
        descriptor_candidate: None,
        descriptor_candidate_error: None,
        model_count: draft.model_count,
        status: if draft.connected {
            ProviderStatus::Connected
        } else {
            ProviderStatus::Disconnected
        },
    }
}
