use super::{
    cli_available_presets, cli_mode_label, AgentRegistry, CliExecutionRuntime,
    CliFrontendProjection, CliStyle, Config, Path, PromptCompletion, PromptFrame, ProviderRegistry,
};
use std::sync::{Arc, Mutex};

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
        name: "validation",
        takes_value: None,
        description: "show config validation",
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
        name: "attached",
        takes_value: None,
        description: "list or focus attached sessions",
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
pub(super) struct CliPromptCatalog {
    pub(super) models: Vec<String>,
    pub(super) agents: Vec<String>,
    pub(super) presets: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct CliPromptSelectionState {
    pub(super) model: String,
    pub(super) agent: String,
    pub(super) preset: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct CliPromptAssistView {
    pub(super) screen_lines: Vec<String>,
    pub(super) completion: Option<PromptCompletion>,
}

#[derive(Debug)]
pub(super) struct CliPromptChrome {
    mode_label: Mutex<String>,
    model_label: Mutex<String>,
    selection: Mutex<CliPromptSelectionState>,
    catalog: Mutex<CliPromptCatalog>,
    frontend_projection: Arc<Mutex<CliFrontendProjection>>,
    style: CliStyle,
}

impl CliPromptChrome {
    pub(super) fn new(
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
                preset: runtime.scheduler_profile_name.clone(),
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

    pub(super) fn update_from_runtime(&self, runtime: &CliExecutionRuntime) {
        if let Ok(mut mode) = self.mode_label.lock() {
            *mode = cli_mode_label(runtime);
        }
        if let Ok(mut model) = self.model_label.lock() {
            *model = format!("Model {}", runtime.resolved_model_label);
        }
        if let Ok(mut selection) = self.selection.lock() {
            selection.model = runtime.resolved_model_label.clone();
            selection.agent = runtime.resolved_agent_name.clone();
            selection.preset = runtime.scheduler_profile_name.clone();
        }
    }

    pub(super) fn update_model_catalog(&self, models: Vec<String>) {
        if let Ok(mut catalog) = self.catalog.lock() {
            catalog.models = models;
        }
    }

    pub(super) fn assist(&self, line: &str, cursor_pos: usize) -> CliPromptAssistView {
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

    pub(super) fn frame(&self, line: &str, cursor_pos: usize) -> PromptFrame {
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

pub(super) fn cli_prompt_assist_view(
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

pub(super) fn cli_prompt_screen_lines() -> Vec<String> {
    Vec::new()
}
