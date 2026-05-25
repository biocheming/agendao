use super::{
    AgentRegistry, CliExecutionRuntime, CliFrontendProjection, CliStyle, Config, Path,
    PromptCompletion, PromptFrame, ProviderRegistry, cli_available_presets, cli_mode_label,
};
use crossterm::terminal;
use std::sync::{Arc, Mutex};

const CLI_PROMPT_SUGGESTION_LIMIT: usize = 6;
const CLI_PROMPT_STABLE_LANE_ROWS: usize = 4;
const CLI_PROMPT_SCREEN_ROWS_MIN: usize = 4;
const CLI_PROMPT_SCREEN_ROWS_MAX: usize = 8;
const CLI_PROMPT_SCREEN_ROWS_FALLBACK: usize = 6;

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
    show_transcript_tail: bool,
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
            show_transcript_tail: true,
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

    pub(super) fn snapshot_labels(&self) -> (String, String) {
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
        (mode, model)
    }

    #[cfg(test)]
    pub(super) fn from_labels(
        mode_label: &str,
        model_label: &str,
        frontend_projection: Arc<Mutex<CliFrontendProjection>>,
        style: &CliStyle,
    ) -> Self {
        Self {
            mode_label: Mutex::new(mode_label.to_string()),
            model_label: Mutex::new(model_label.to_string()),
            selection: Mutex::new(CliPromptSelectionState::default()),
            catalog: Mutex::new(CliPromptCatalog::default()),
            frontend_projection,
            show_transcript_tail: true,
            style: style.clone(),
        }
    }

    pub(super) fn set_show_transcript_tail(&mut self, show_transcript_tail: bool) {
        self.show_transcript_tail = show_transcript_tail;
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
                " Session idle  •  Alt+Enter/Ctrl+J newline  •  /help  •  Ctrl+D exit ".to_string()
            });
        PromptFrame::boxed_with_footer(&mode, &model, &footer, &self.style)
            .with_screen_lines(self.screen_lines(line, cursor_pos))
    }

    fn screen_lines(&self, line: &str, cursor_pos: usize) -> Vec<String> {
        let assist = self.assist(line, cursor_pos);
        let mut screen_lines = self
            .frontend_projection
            .lock()
            .map(|projection| {
                if self.show_transcript_tail {
                    cli_prompt_lane_screen_lines_from_projection(&projection)
                } else {
                    projection.prompt_lane_lines_stable(CLI_PROMPT_STABLE_LANE_ROWS)
                }
            })
            .unwrap_or_default();
        screen_lines.extend(assist.screen_lines);
        screen_lines
    }

    #[cfg(test)]
    pub(super) fn screen_lines_for_test(&self, line: &str, cursor_pos: usize) -> Vec<String> {
        self.screen_lines(line, cursor_pos)
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

pub(super) fn cli_prompt_lane_screen_lines_from_projection(
    projection: &CliFrontendProjection,
) -> Vec<String> {
    let content_width = usize::from(CliStyle::detect().width.saturating_sub(5)).max(20);
    cli_prompt_screen_lines_with_budget(projection, content_width, cli_prompt_screen_line_budget())
}

pub(super) fn cli_prompt_screen_lines_with_budget(
    projection: &CliFrontendProjection,
    content_width: usize,
    max_rows: usize,
) -> Vec<String> {
    if max_rows == 0 {
        return Vec::new();
    }

    let lane_lines = projection.prompt_lane_lines_stable(CLI_PROMPT_STABLE_LANE_ROWS);
    let transcript_budget = max_rows.saturating_sub(lane_lines.len());
    let transcript_lines = if projection.transcript.rendered_text().trim().is_empty() {
        Vec::new()
    } else {
        projection
            .transcript
            .viewport_lines(content_width, transcript_budget.saturating_sub(1), 0)
    };

    let mut lines = Vec::new();
    lines.extend(transcript_lines.iter().cloned());
    if !transcript_lines.is_empty() && !lane_lines.is_empty() {
        lines.push(cli_prompt_lane_separator(content_width));
    }
    lines.extend(lane_lines);
    lines
}

fn cli_prompt_screen_line_budget() -> usize {
    match terminal::size() {
        Ok((_, rows)) => usize::from(rows.saturating_sub(14))
            .clamp(CLI_PROMPT_SCREEN_ROWS_MIN, CLI_PROMPT_SCREEN_ROWS_MAX),
        Err(_) => CLI_PROMPT_SCREEN_ROWS_FALLBACK,
    }
}

fn cli_prompt_lane_separator(width: usize) -> String {
    CliStyle::detect().dim(&"─".repeat(width.min(72).max(16)))
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

#[cfg(test)]
mod tests {
    use super::{
        CLI_PROMPT_STABLE_LANE_ROWS, CliFrontendProjection, CliPromptChrome,
        cli_prompt_lane_screen_lines_from_projection, cli_prompt_lane_separator,
        cli_prompt_screen_lines_with_budget,
    };
    use crate::run::CliStyle;
    use crate::run::frontend_state_types::CliRunTailState;
    use rocode_util::util::color::strip_ansi;
    use std::sync::{Arc, Mutex};

    #[test]
    fn active_prompt_screen_lines_show_lane_when_transcript_is_empty() {
        let projection = CliFrontendProjection {
            active_label: Some("Skill SkillsList".to_string()),
            run_tail: Some(CliRunTailState {
                status: "running".to_string(),
                detail: Some("Current stage: Research".to_string()),
            }),
            ..CliFrontendProjection::default()
        };

        let lane_lines = projection.prompt_lane_lines_stable(CLI_PROMPT_STABLE_LANE_ROWS);
        let screen_lines = cli_prompt_screen_lines_with_budget(&projection, 72, 8);

        assert_eq!(screen_lines, lane_lines);
    }

    #[test]
    fn transcript_tail_and_active_lane_share_prompt_screen_budget() {
        let mut projection = CliFrontendProjection {
            active_label: Some("Tool SkillsList".to_string()),
            run_tail: Some(CliRunTailState {
                status: "running".to_string(),
                detail: Some("Current stage: Research".to_string()),
            }),
            ..CliFrontendProjection::default()
        };
        projection
            .transcript
            .append_committed("alpha\nbeta\ngamma\ndelta\n");

        let lane_lines = projection.prompt_lane_lines_stable(CLI_PROMPT_STABLE_LANE_ROWS);
        let screen_lines = cli_prompt_screen_lines_with_budget(&projection, 72, 4);
        let plain_lines = screen_lines
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>();

        assert_eq!(
            plain_lines,
            vec![
                "delta".to_string(),
                strip_ansi(&cli_prompt_lane_separator(72)),
                strip_ansi(&lane_lines[0]),
                strip_ansi(&lane_lines[1]),
            ]
        );
        assert!(!plain_lines.iter().any(|line| line == "alpha"));
        assert!(!plain_lines.iter().any(|line| line == "beta"));
        assert!(!plain_lines.iter().any(|line| line == "gamma"));
    }

    #[test]
    fn default_prompt_screen_lines_stay_empty_without_transcript_or_lane() {
        let projection = CliFrontendProjection::default();

        assert!(cli_prompt_lane_screen_lines_from_projection(&projection).is_empty());
    }

    #[test]
    fn rich_prompt_frame_can_hide_transcript_tail() {
        let projection = Arc::new(Mutex::new(CliFrontendProjection {
            active_label: Some("Tool SkillView".to_string()),
            run_tail: Some(CliRunTailState {
                status: "running".to_string(),
                detail: Some("Using Skill SkillView".to_string()),
            }),
            ..CliFrontendProjection::default()
        }));
        projection
            .lock()
            .expect("projection")
            .transcript
            .append_committed("alpha\nbeta\ngamma\ndelta\n");

        let style = CliStyle::plain();
        let mut chrome =
            CliPromptChrome::from_labels("Agent build", "Model x", projection.clone(), &style);
        chrome.set_show_transcript_tail(false);

        let plain_lines = chrome
            .screen_lines_for_test("", 0)
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>();

        assert!(
            !plain_lines.iter().any(|line| line == "alpha"),
            "{plain_lines:?}"
        );
        assert!(
            !plain_lines.iter().any(|line| line == "delta"),
            "{plain_lines:?}"
        );
        assert!(
            plain_lines
                .iter()
                .any(|line| line.contains("Tool SkillView") || line.contains("running")),
            "{plain_lines:?}"
        );
    }
}
