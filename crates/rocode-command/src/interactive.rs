use crate::{ui_command_argument_kind, ResolvedUiCommand, UiActionId};

pub const EVENTS_BROWSER_DEFAULT_PAGE_SIZE: usize = 24;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractiveEventsQuery {
    pub stage_id: Option<String>,
    pub execution_id: Option<String>,
    pub event_type: Option<String>,
    pub since: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractiveEventsCommand {
    ShowCurrent,
    ShowFiltered {
        filter: InteractiveEventsQuery,
        page: usize,
    },
    JumpPage(usize),
    NextPage,
    PreviousPage,
    FirstPage,
    Clear,
}

pub fn default_events_browser_query() -> InteractiveEventsQuery {
    InteractiveEventsQuery {
        limit: Some(EVENTS_BROWSER_DEFAULT_PAGE_SIZE),
        ..Default::default()
    }
}

pub fn parse_events_browser_command(raw: Option<&str>) -> InteractiveEventsCommand {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return InteractiveEventsCommand::ShowCurrent;
    };

    let mut parts = raw.split_whitespace();
    let head = parts.next().unwrap_or_default().to_ascii_lowercase();
    let tail = parts.collect::<Vec<_>>().join(" ");
    if tail.is_empty() {
        match head.as_str() {
            "next" | "more" => return InteractiveEventsCommand::NextPage,
            "prev" | "previous" | "back" => return InteractiveEventsCommand::PreviousPage,
            "first" | "head" => return InteractiveEventsCommand::FirstPage,
            "clear" | "reset" => return InteractiveEventsCommand::Clear,
            _ => {}
        }
    }

    if head == "page" {
        if let Ok(page) = tail.parse::<usize>() {
            return InteractiveEventsCommand::JumpPage(page.max(1));
        }
    }

    let (filter, page) = parse_events_browser_query_spec(Some(raw));
    InteractiveEventsCommand::ShowFiltered { filter, page }
}

pub fn parse_events_browser_query(raw: Option<&str>) -> InteractiveEventsQuery {
    parse_events_browser_query_spec(raw).0
}

pub fn parse_events_browser_query_spec(raw: Option<&str>) -> (InteractiveEventsQuery, usize) {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return (default_events_browser_query(), 1);
    };

    if !raw.contains('=') {
        return (
            InteractiveEventsQuery {
                stage_id: Some(raw.to_string()),
                ..default_events_browser_query()
            },
            1,
        );
    }

    let mut query = default_events_browser_query();
    let mut page = 1usize;
    for token in raw.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key {
            "stage" | "stage_id" => query.stage_id = Some(value.to_string()),
            "exec" | "execution" | "execution_id" => query.execution_id = Some(value.to_string()),
            "type" | "event" | "event_type" => query.event_type = Some(value.to_string()),
            "since" => query.since = value.parse::<i64>().ok(),
            "limit" => query.limit = value.parse::<usize>().ok(),
            "page" => {
                if let Ok(parsed) = value.parse::<usize>() {
                    page = parsed.max(1);
                }
            }
            _ => {}
        }
    }

    (query, page)
}

pub fn events_browser_page_size(input: &InteractiveEventsQuery) -> usize {
    input
        .limit
        .unwrap_or(EVENTS_BROWSER_DEFAULT_PAGE_SIZE)
        .max(1)
}

pub fn events_browser_offset_for_page(input: &InteractiveEventsQuery, page: usize) -> usize {
    events_browser_page_size(input).saturating_mul(page.saturating_sub(1))
}

pub fn events_browser_page_for_offset(input: &InteractiveEventsQuery, offset: usize) -> usize {
    (offset / events_browser_page_size(input)) + 1
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractiveMemoryRuleHitQuery {
    pub run_id: Option<String>,
    pub record_id: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractiveMemoryConsolidationRequest {
    pub include_candidates: bool,
    pub limit: Option<usize>,
}

pub fn parse_memory_rule_hit_query(raw: Option<&str>) -> InteractiveMemoryRuleHitQuery {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return InteractiveMemoryRuleHitQuery {
            limit: Some(50),
            ..Default::default()
        };
    };

    let mut query = InteractiveMemoryRuleHitQuery {
        limit: Some(50),
        ..Default::default()
    };
    for token in raw.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key {
            "run" | "run_id" => query.run_id = Some(value.to_string()),
            "record" | "id" | "memory" | "memory_id" => query.record_id = Some(value.to_string()),
            "limit" => query.limit = value.parse::<usize>().ok(),
            _ => {}
        }
    }
    query
}

pub fn parse_memory_consolidation_request(
    raw: Option<&str>,
) -> InteractiveMemoryConsolidationRequest {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return InteractiveMemoryConsolidationRequest::default();
    };

    let mut request = InteractiveMemoryConsolidationRequest::default();
    for token in raw.split_whitespace() {
        let normalized = token.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "candidate" | "candidates" | "include-candidates" | "all" => {
                request.include_candidates = true;
                continue;
            }
            _ => {}
        }

        if let Some((key, value)) = token.split_once('=') {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match key {
                "limit" => request.limit = value.parse::<usize>().ok(),
                "include_candidates" | "include-candidates" => {
                    request.include_candidates =
                        matches!(value, "1" | "true" | "yes" | "on" | "all" | "candidate");
                }
                _ => {}
            }
        }
    }
    request
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractiveCommand {
    Exit,
    ShowHelp,
    Abort,
    ShowRecovery,
    ExecuteRecovery(String),
    NewSession,
    ClearScreen,
    ShowStatus,
    ListModels,
    SelectModel(String),
    ListProviders,
    ConnectProvider(Option<String>),
    ListThemes,
    ListPresets,
    SelectPreset(String),
    ListSessions,
    ParentSession,
    ListAttachedSessions,
    FocusAttachedSession(String),
    FocusNextAttachedSession,
    FocusPreviousAttachedSession,
    BackToRootSession,
    ListTasks,
    ShowTask(String),
    KillTask(String),
    Compact(Option<String>),
    Copy,
    ListAgents,
    SelectAgent(String),
    ToggleSidebar,
    ToggleActive,
    ScrollUp,
    ScrollDown,
    ScrollBottom,
    ShowRuntime,
    ShowUsage,
    ShowInsights,
    ShowConfigValidation,
    ShowEvents(Option<String>),
    ShowMemory(Option<String>),
    ShowMemoryPreview(Option<String>),
    ShowMemoryDetail(String),
    ShowMemoryValidation(String),
    ShowMemoryConflicts(String),
    ShowMemoryRulePacks,
    ShowMemoryRuleHits(Option<String>),
    ShowMemoryConsolidationRuns,
    RunMemoryConsolidation(Option<String>),
    /// `/inspect [stage_id]` — show stage event log for current session.
    InspectStage(Option<String>),
    /// User typed an unknown /command — we should warn, not treat as prompt.
    Unknown(String),
}

impl InteractiveCommand {
    pub fn ui_action_invocation(&self) -> Option<ResolvedUiCommand> {
        let (action_id, argument) = match self {
            Self::Exit => (UiActionId::Exit, None),
            Self::ShowHelp => (UiActionId::ShowHelp, None),
            Self::Abort => (UiActionId::AbortExecution, None),
            Self::ShowRecovery => (UiActionId::OpenRecoveryList, None),
            Self::NewSession => (UiActionId::NewSession, None),
            Self::ShowStatus => (UiActionId::ShowStatus, None),
            Self::ListModels => (UiActionId::OpenModelList, None),
            Self::SelectModel(model_ref) => (UiActionId::OpenModelList, Some(model_ref.clone())),
            Self::ListProviders => (UiActionId::ConnectProvider, None),
            Self::ConnectProvider(query) => (UiActionId::ConnectProvider, query.clone()),
            Self::ListThemes => (UiActionId::OpenThemeList, None),
            Self::ListPresets => (UiActionId::OpenPresetList, None),
            Self::SelectPreset(name) => (UiActionId::OpenPresetList, Some(name.clone())),
            Self::ListSessions => (UiActionId::OpenSessionList, None),
            Self::ParentSession => (UiActionId::NavigateParentSession, None),
            Self::ListTasks => (UiActionId::ListTasks, None),
            Self::Compact(focus) => (UiActionId::CompactSession, focus.clone()),
            Self::Copy => (UiActionId::CopySession, None),
            Self::ListAgents => (UiActionId::OpenAgentList, None),
            Self::SelectAgent(name) => (UiActionId::OpenAgentList, Some(name.clone())),
            Self::ToggleSidebar => (UiActionId::ToggleSidebar, None),
            _ => return None,
        };

        Some(ResolvedUiCommand {
            action_id,
            argument_kind: ui_command_argument_kind(action_id),
            argument,
        })
    }

    pub fn ui_action_id(&self) -> Option<UiActionId> {
        self.ui_action_invocation()
            .map(|invocation| invocation.action_id)
    }
}

pub fn parse_interactive_command(input: &str) -> Option<InteractiveCommand> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let plain = trimmed.to_ascii_lowercase();
    match plain.as_str() {
        "exit" | "quit" => return Some(InteractiveCommand::Exit),
        "help" => return Some(InteractiveCommand::ShowHelp),
        "clear" => return Some(InteractiveCommand::ClearScreen),
        "stats" => return Some(InteractiveCommand::ShowStatus),
        "models" => return Some(InteractiveCommand::ListModels),
        "providers" => return Some(InteractiveCommand::ListProviders),
        _ => {}
    }

    if !trimmed.starts_with('/') {
        return None;
    }

    let body = trimmed[1..].trim();
    if body.is_empty() {
        return None;
    }

    let mut parts = body.split_whitespace();
    let name = parts.next()?.to_ascii_lowercase();
    let arg = parts.collect::<Vec<_>>().join(" ");

    match name.as_str() {
        "help" | "commands" => Some(InteractiveCommand::ShowHelp),
        "exit" | "quit" | "q" => Some(InteractiveCommand::Exit),
        "abort" => Some(InteractiveCommand::Abort),
        "recover" | "recovery" => {
            if arg.is_empty() {
                Some(InteractiveCommand::ShowRecovery)
            } else {
                Some(InteractiveCommand::ExecuteRecovery(arg))
            }
        }
        "new" => Some(InteractiveCommand::NewSession),
        "clear" => Some(InteractiveCommand::ClearScreen),
        "status" | "stats" => Some(InteractiveCommand::ShowStatus),
        "runtime" => Some(InteractiveCommand::ShowRuntime),
        "usage" => Some(InteractiveCommand::ShowUsage),
        "insights" | "insight" => Some(InteractiveCommand::ShowInsights),
        "validation" | "validate" => Some(InteractiveCommand::ShowConfigValidation),
        "events" | "event" => Some(InteractiveCommand::ShowEvents(
            (!arg.is_empty()).then_some(arg),
        )),
        "config" => {
            let mut sub_parts = arg.split_whitespace();
            let sub_cmd = sub_parts.next().unwrap_or_default();
            match sub_cmd {
                "validation" | "validate" => Some(InteractiveCommand::ShowConfigValidation),
                _ => Some(InteractiveCommand::Unknown("config".to_string())),
            }
        }
        "memory" | "memories" => {
            if arg.is_empty() {
                Some(InteractiveCommand::ShowMemory(None))
            } else {
                let mut sub_parts = arg.split_whitespace();
                let sub_cmd = sub_parts.next().unwrap_or_default();
                let sub_arg = sub_parts.collect::<Vec<_>>().join(" ");
                match sub_cmd {
                    "show" | "detail" if !sub_arg.is_empty() => {
                        Some(InteractiveCommand::ShowMemoryDetail(sub_arg))
                    }
                    "preview" => Some(InteractiveCommand::ShowMemoryPreview(
                        (!sub_arg.is_empty()).then_some(sub_arg),
                    )),
                    "validation" | "report" if !sub_arg.is_empty() => {
                        Some(InteractiveCommand::ShowMemoryValidation(sub_arg))
                    }
                    "conflicts" | "conflict" if !sub_arg.is_empty() => {
                        Some(InteractiveCommand::ShowMemoryConflicts(sub_arg))
                    }
                    "rules" | "rule-packs" | "rulepacks" => {
                        Some(InteractiveCommand::ShowMemoryRulePacks)
                    }
                    "hits" | "rule-hits" | "rulehits" => {
                        Some(InteractiveCommand::ShowMemoryRuleHits(
                            (!sub_arg.is_empty()).then_some(sub_arg),
                        ))
                    }
                    "runs" | "run" => Some(InteractiveCommand::ShowMemoryConsolidationRuns),
                    "consolidate" | "reflect" => Some(InteractiveCommand::RunMemoryConsolidation(
                        (!sub_arg.is_empty()).then_some(sub_arg),
                    )),
                    _ => Some(InteractiveCommand::ShowMemory(Some(arg))),
                }
            }
        }
        "models" => Some(InteractiveCommand::ListModels),
        "model" => {
            if arg.is_empty() {
                Some(InteractiveCommand::ListModels)
            } else {
                Some(InteractiveCommand::SelectModel(arg))
            }
        }
        "providers" => Some(InteractiveCommand::ListProviders),
        "provider" | "connect" => Some(InteractiveCommand::ConnectProvider(
            (!arg.is_empty()).then_some(arg),
        )),
        "theme" | "themes" => Some(InteractiveCommand::ListThemes),
        "preset" | "presets" => {
            if arg.is_empty() {
                Some(InteractiveCommand::ListPresets)
            } else {
                Some(InteractiveCommand::SelectPreset(arg))
            }
        }
        "session" | "sessions" | "resume" | "continue" => Some(InteractiveCommand::ListSessions),
        "parent" | "back" => Some(InteractiveCommand::ParentSession),
        "attached" | "attachments" => {
            if arg.is_empty() {
                Some(InteractiveCommand::ListAttachedSessions)
            } else {
                let mut sub_parts = arg.split_whitespace();
                let sub_cmd = sub_parts.next().unwrap_or("");
                let sub_arg = sub_parts.collect::<Vec<_>>().join(" ");
                match sub_cmd {
                    "list" => Some(InteractiveCommand::ListAttachedSessions),
                    "focus" if !sub_arg.is_empty() => {
                        Some(InteractiveCommand::FocusAttachedSession(sub_arg))
                    }
                    "focus" => Some(InteractiveCommand::ListAttachedSessions),
                    "next" => Some(InteractiveCommand::FocusNextAttachedSession),
                    "prev" | "previous" => Some(InteractiveCommand::FocusPreviousAttachedSession),
                    "back" | "root" => Some(InteractiveCommand::BackToRootSession),
                    _ => Some(InteractiveCommand::ListAttachedSessions),
                }
            }
        }
        "compact" => {
            if arg.is_empty() {
                Some(InteractiveCommand::Compact(None))
            } else {
                Some(InteractiveCommand::Compact(Some(arg)))
            }
        }
        "copy" => Some(InteractiveCommand::Copy),
        "agent" | "agents" => {
            if arg.is_empty() {
                Some(InteractiveCommand::ListAgents)
            } else {
                Some(InteractiveCommand::SelectAgent(arg))
            }
        }
        "sidebar" => Some(InteractiveCommand::ToggleSidebar),
        "active" => Some(InteractiveCommand::ToggleActive),
        "inspect" | "stage" | "stages" => {
            if arg.is_empty() {
                Some(InteractiveCommand::InspectStage(None))
            } else {
                Some(InteractiveCommand::InspectStage(Some(arg)))
            }
        }
        "up" | "pageup" => Some(InteractiveCommand::ScrollUp),
        "down" | "pagedown" => Some(InteractiveCommand::ScrollDown),
        "bottom" | "end" => Some(InteractiveCommand::ScrollBottom),
        "tasks" | "task" => {
            if arg.is_empty() {
                Some(InteractiveCommand::ListTasks)
            } else {
                let mut sub_parts = arg.split_whitespace();
                let sub_cmd = sub_parts.next().unwrap_or("");
                let sub_arg = sub_parts.collect::<Vec<_>>().join(" ");
                match sub_cmd {
                    "show" if !sub_arg.is_empty() => Some(InteractiveCommand::ShowTask(sub_arg)),
                    "kill" | "cancel" if !sub_arg.is_empty() => {
                        Some(InteractiveCommand::KillTask(sub_arg))
                    }
                    _ => Some(InteractiveCommand::ListTasks),
                }
            }
        }
        _ => Some(InteractiveCommand::Unknown(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        default_events_browser_query, parse_events_browser_command, parse_events_browser_query,
        parse_interactive_command, parse_memory_consolidation_request, parse_memory_rule_hit_query,
        InteractiveCommand, InteractiveEventsCommand, InteractiveEventsQuery,
        InteractiveMemoryConsolidationRequest, InteractiveMemoryRuleHitQuery,
        EVENTS_BROWSER_DEFAULT_PAGE_SIZE,
    };
    use crate::{ResolvedUiCommand, UiActionId, UiCommandArgumentKind};

    #[test]
    fn parses_plain_commands() {
        assert_eq!(
            parse_interactive_command("help"),
            Some(InteractiveCommand::ShowHelp)
        );
        assert_eq!(
            parse_interactive_command("models"),
            Some(InteractiveCommand::ListModels)
        );
        assert_eq!(
            parse_interactive_command("providers"),
            Some(InteractiveCommand::ListProviders)
        );
        assert_eq!(
            parse_interactive_command("clear"),
            Some(InteractiveCommand::ClearScreen)
        );
    }

    #[test]
    fn parses_slash_commands() {
        assert_eq!(
            parse_interactive_command("/help"),
            Some(InteractiveCommand::ShowHelp)
        );
        assert_eq!(
            parse_interactive_command("/abort"),
            Some(InteractiveCommand::Abort)
        );
        assert_eq!(
            parse_interactive_command("/recover"),
            Some(InteractiveCommand::ShowRecovery)
        );
        assert_eq!(
            parse_interactive_command("/recover retry"),
            Some(InteractiveCommand::ExecuteRecovery("retry".to_string()))
        );
        assert_eq!(
            parse_interactive_command("/themes"),
            Some(InteractiveCommand::ListThemes)
        );
        assert_eq!(
            parse_interactive_command("/connect"),
            Some(InteractiveCommand::ConnectProvider(None))
        );
        assert_eq!(
            parse_interactive_command("/connect openrouter"),
            Some(InteractiveCommand::ConnectProvider(Some(
                "openrouter".to_string()
            )))
        );
        assert_eq!(
            parse_interactive_command("/preset"),
            Some(InteractiveCommand::ListPresets)
        );
        assert_eq!(
            parse_interactive_command("/session"),
            Some(InteractiveCommand::ListSessions)
        );
        assert_eq!(
            parse_interactive_command("/parent"),
            Some(InteractiveCommand::ParentSession)
        );
        assert_eq!(
            parse_interactive_command("/back"),
            Some(InteractiveCommand::ParentSession)
        );
        assert_eq!(
            parse_interactive_command("/attached"),
            Some(InteractiveCommand::ListAttachedSessions)
        );
        assert_eq!(
            parse_interactive_command("/attachments"),
            Some(InteractiveCommand::ListAttachedSessions)
        );
        assert_eq!(
            parse_interactive_command("/compact"),
            Some(InteractiveCommand::Compact(None))
        );
        assert_eq!(
            parse_interactive_command("/compact grep tool results"),
            Some(InteractiveCommand::Compact(Some(
                "grep tool results".to_string()
            )))
        );
        assert_eq!(
            parse_interactive_command("/copy"),
            Some(InteractiveCommand::Copy)
        );
        assert_eq!(
            parse_interactive_command("/new"),
            Some(InteractiveCommand::NewSession)
        );
        assert_eq!(
            parse_interactive_command("/clear"),
            Some(InteractiveCommand::ClearScreen)
        );
        assert_eq!(
            parse_interactive_command("/runtime"),
            Some(InteractiveCommand::ShowRuntime)
        );
        assert_eq!(
            parse_interactive_command("/usage"),
            Some(InteractiveCommand::ShowUsage)
        );
        assert_eq!(
            parse_interactive_command("/insights"),
            Some(InteractiveCommand::ShowInsights)
        );
        assert_eq!(
            parse_interactive_command("/validation"),
            Some(InteractiveCommand::ShowConfigValidation)
        );
        assert_eq!(
            parse_interactive_command("/config validation"),
            Some(InteractiveCommand::ShowConfigValidation)
        );
        assert_eq!(
            parse_interactive_command("/events"),
            Some(InteractiveCommand::ShowEvents(None))
        );
        assert_eq!(
            parse_interactive_command("/events stage=stg_1 limit=10"),
            Some(InteractiveCommand::ShowEvents(Some(
                "stage=stg_1 limit=10".to_string()
            )))
        );
        assert_eq!(
            parse_interactive_command("/memory"),
            Some(InteractiveCommand::ShowMemory(None))
        );
        assert_eq!(
            parse_interactive_command("/memory cargo"),
            Some(InteractiveCommand::ShowMemory(Some("cargo".to_string())))
        );
        assert_eq!(
            parse_interactive_command("/memory show mem_1"),
            Some(InteractiveCommand::ShowMemoryDetail("mem_1".to_string()))
        );
        assert_eq!(
            parse_interactive_command("/memory preview cargo test"),
            Some(InteractiveCommand::ShowMemoryPreview(Some(
                "cargo test".to_string()
            )))
        );
        assert_eq!(
            parse_interactive_command("/memory validation mem_1"),
            Some(InteractiveCommand::ShowMemoryValidation(
                "mem_1".to_string()
            ))
        );
        assert_eq!(
            parse_interactive_command("/memory conflicts mem_1"),
            Some(InteractiveCommand::ShowMemoryConflicts("mem_1".to_string()))
        );
        assert_eq!(
            parse_interactive_command("/memory rules"),
            Some(InteractiveCommand::ShowMemoryRulePacks)
        );
        assert_eq!(
            parse_interactive_command("/memory hits run=mem_run_1"),
            Some(InteractiveCommand::ShowMemoryRuleHits(Some(
                "run=mem_run_1".to_string()
            )))
        );
        assert_eq!(
            parse_interactive_command("/memory runs"),
            Some(InteractiveCommand::ShowMemoryConsolidationRuns)
        );
        assert_eq!(
            parse_interactive_command("/memory consolidate candidates limit=10"),
            Some(InteractiveCommand::RunMemoryConsolidation(Some(
                "candidates limit=10".to_string()
            )))
        );
    }

    #[test]
    fn parses_model_selection() {
        assert_eq!(
            parse_interactive_command("/model openai/gpt-4.1"),
            Some(InteractiveCommand::SelectModel(
                "openai/gpt-4.1".to_string()
            ))
        );
        assert_eq!(
            parse_interactive_command("/model"),
            Some(InteractiveCommand::ListModels)
        );
    }

    #[test]
    fn parses_agent_commands() {
        assert_eq!(
            parse_interactive_command("/agent"),
            Some(InteractiveCommand::ListAgents)
        );
        assert_eq!(
            parse_interactive_command("/agents"),
            Some(InteractiveCommand::ListAgents)
        );
        assert_eq!(
            parse_interactive_command("/agent build"),
            Some(InteractiveCommand::SelectAgent("build".to_string()))
        );
        assert_eq!(
            parse_interactive_command("/preset prometheus"),
            Some(InteractiveCommand::SelectPreset("prometheus".to_string()))
        );
    }

    #[test]
    fn unknown_slash_command_returns_unknown() {
        assert_eq!(
            parse_interactive_command("/foo"),
            Some(InteractiveCommand::Unknown("foo".to_string()))
        );
        assert_eq!(
            parse_interactive_command("/nonexistent"),
            Some(InteractiveCommand::Unknown("nonexistent".to_string()))
        );
    }

    #[test]
    fn parses_toggle_commands() {
        assert_eq!(
            parse_interactive_command("/sidebar"),
            Some(InteractiveCommand::ToggleSidebar)
        );
        assert_eq!(
            parse_interactive_command("/active"),
            Some(InteractiveCommand::ToggleActive)
        );
    }

    #[test]
    fn parses_scroll_commands() {
        assert_eq!(
            parse_interactive_command("/up"),
            Some(InteractiveCommand::ScrollUp)
        );
        assert_eq!(
            parse_interactive_command("/down"),
            Some(InteractiveCommand::ScrollDown)
        );
        assert_eq!(
            parse_interactive_command("/bottom"),
            Some(InteractiveCommand::ScrollBottom)
        );
        assert_eq!(
            parse_interactive_command("/pageup"),
            Some(InteractiveCommand::ScrollUp)
        );
        assert_eq!(
            parse_interactive_command("/end"),
            Some(InteractiveCommand::ScrollBottom)
        );
    }

    #[test]
    fn ignores_non_commands() {
        assert_eq!(parse_interactive_command(""), None);
        assert_eq!(parse_interactive_command("hello rocode"), None);
    }

    #[test]
    fn parses_tasks_commands() {
        assert_eq!(
            parse_interactive_command("/tasks"),
            Some(InteractiveCommand::ListTasks)
        );
        assert_eq!(
            parse_interactive_command("/task"),
            Some(InteractiveCommand::ListTasks)
        );
        assert_eq!(
            parse_interactive_command("/tasks show a1"),
            Some(InteractiveCommand::ShowTask("a1".to_string()))
        );
        assert_eq!(
            parse_interactive_command("/tasks kill a1"),
            Some(InteractiveCommand::KillTask("a1".to_string()))
        );
        assert_eq!(
            parse_interactive_command("/tasks cancel a2"),
            Some(InteractiveCommand::KillTask("a2".to_string()))
        );
    }

    #[test]
    fn parses_attached_session_commands() {
        assert_eq!(
            parse_interactive_command("/attached list"),
            Some(InteractiveCommand::ListAttachedSessions)
        );
        assert_eq!(
            parse_interactive_command("/attached focus attached-42"),
            Some(InteractiveCommand::FocusAttachedSession(
                "attached-42".to_string()
            ))
        );
        assert_eq!(
            parse_interactive_command("/attached focus"),
            Some(InteractiveCommand::ListAttachedSessions)
        );
        assert_eq!(
            parse_interactive_command("/attached next"),
            Some(InteractiveCommand::FocusNextAttachedSession)
        );
        assert_eq!(
            parse_interactive_command("/attached prev"),
            Some(InteractiveCommand::FocusPreviousAttachedSession)
        );
        assert_eq!(
            parse_interactive_command("/attached previous"),
            Some(InteractiveCommand::FocusPreviousAttachedSession)
        );
        assert_eq!(
            parse_interactive_command("/attached back"),
            Some(InteractiveCommand::BackToRootSession)
        );
        assert_eq!(
            parse_interactive_command("/attached root"),
            Some(InteractiveCommand::BackToRootSession)
        );
    }

    #[test]
    fn parses_inspect_commands() {
        assert_eq!(
            parse_interactive_command("/inspect"),
            Some(InteractiveCommand::InspectStage(None))
        );
        assert_eq!(
            parse_interactive_command("/inspect stg_abc"),
            Some(InteractiveCommand::InspectStage(Some(
                "stg_abc".to_string()
            )))
        );
        assert_eq!(
            parse_interactive_command("/stage"),
            Some(InteractiveCommand::InspectStage(None))
        );
        assert_eq!(
            parse_interactive_command("/stages"),
            Some(InteractiveCommand::InspectStage(None))
        );
    }

    #[test]
    fn parses_default_events_browser_query() {
        assert_eq!(
            parse_events_browser_query(None),
            default_events_browser_query()
        );
    }

    #[test]
    fn parses_stage_alias_events_browser_query() {
        assert_eq!(
            parse_events_browser_query(Some("stg_123")),
            InteractiveEventsQuery {
                stage_id: Some("stg_123".to_string()),
                limit: Some(EVENTS_BROWSER_DEFAULT_PAGE_SIZE),
                ..Default::default()
            }
        );
    }

    #[test]
    fn parses_structured_events_browser_query() {
        assert_eq!(
            parse_events_browser_query(Some(
                "stage=stg_1 exec=exe_2 type=session.updated limit=10 since=42"
            )),
            InteractiveEventsQuery {
                stage_id: Some("stg_1".to_string()),
                execution_id: Some("exe_2".to_string()),
                event_type: Some("session.updated".to_string()),
                since: Some(42),
                limit: Some(10),
            }
        );
    }

    #[test]
    fn parses_events_browser_navigation_commands() {
        assert_eq!(
            parse_events_browser_command(Some("next")),
            InteractiveEventsCommand::NextPage
        );
        assert_eq!(
            parse_events_browser_command(Some("prev")),
            InteractiveEventsCommand::PreviousPage
        );
        assert_eq!(
            parse_events_browser_command(Some("clear")),
            InteractiveEventsCommand::Clear
        );
        assert_eq!(
            parse_events_browser_command(Some("first")),
            InteractiveEventsCommand::FirstPage
        );
        assert_eq!(
            parse_events_browser_command(Some("page 3")),
            InteractiveEventsCommand::JumpPage(3)
        );
        assert_eq!(
            parse_events_browser_command(Some("stage=stg_1 limit=10")),
            InteractiveEventsCommand::ShowFiltered {
                filter: InteractiveEventsQuery {
                    stage_id: Some("stg_1".to_string()),
                    limit: Some(10),
                    ..Default::default()
                },
                page: 1,
            }
        );
        assert_eq!(
            parse_events_browser_command(Some("stage=stg_1 limit=10 page=2")),
            InteractiveEventsCommand::ShowFiltered {
                filter: InteractiveEventsQuery {
                    stage_id: Some("stg_1".to_string()),
                    limit: Some(10),
                    ..Default::default()
                },
                page: 2,
            }
        );
    }

    #[test]
    fn maps_non_parameterized_commands_to_ui_actions() {
        assert_eq!(
            InteractiveCommand::Exit.ui_action_id(),
            Some(UiActionId::Exit)
        );
        assert_eq!(
            InteractiveCommand::ListModels.ui_action_id(),
            Some(UiActionId::OpenModelList)
        );
        assert_eq!(
            InteractiveCommand::ToggleSidebar.ui_action_id(),
            Some(UiActionId::ToggleSidebar)
        );
        assert_eq!(
            InteractiveCommand::Compact(None).ui_action_id(),
            Some(UiActionId::CompactSession)
        );
        assert_eq!(
            InteractiveCommand::ListPresets.ui_action_id(),
            Some(UiActionId::OpenPresetList)
        );
        assert_eq!(
            InteractiveCommand::Copy.ui_action_id(),
            Some(UiActionId::CopySession)
        );
        assert_eq!(
            InteractiveCommand::ListAttachedSessions.ui_action_id(),
            None
        );
        assert_eq!(
            InteractiveCommand::Abort.ui_action_id(),
            Some(UiActionId::AbortExecution)
        );
        assert_eq!(
            InteractiveCommand::SelectModel("foo".to_string()).ui_action_id(),
            Some(UiActionId::OpenModelList)
        );
    }

    #[test]
    fn maps_parameterized_commands_to_ui_action_invocations() {
        assert_eq!(
            InteractiveCommand::SelectModel("openai/gpt-5".to_string()).ui_action_invocation(),
            Some(ResolvedUiCommand {
                action_id: UiActionId::OpenModelList,
                argument_kind: UiCommandArgumentKind::ModelRef,
                argument: Some("openai/gpt-5".to_string()),
            })
        );
        assert_eq!(
            InteractiveCommand::SelectAgent("build".to_string()).ui_action_invocation(),
            Some(ResolvedUiCommand {
                action_id: UiActionId::OpenAgentList,
                argument_kind: UiCommandArgumentKind::AgentRef,
                argument: Some("build".to_string()),
            })
        );
        assert_eq!(
            InteractiveCommand::SelectPreset("atlas".to_string()).ui_action_invocation(),
            Some(ResolvedUiCommand {
                action_id: UiActionId::OpenPresetList,
                argument_kind: UiCommandArgumentKind::PresetRef,
                argument: Some("atlas".to_string()),
            })
        );
        assert_eq!(
            InteractiveCommand::ConnectProvider(Some("glm".to_string())).ui_action_invocation(),
            Some(ResolvedUiCommand {
                action_id: UiActionId::ConnectProvider,
                argument_kind: UiCommandArgumentKind::Text,
                argument: Some("glm".to_string()),
            })
        );
    }

    #[test]
    fn parses_memory_rule_hit_query_spec() {
        assert_eq!(
            parse_memory_rule_hit_query(Some("run=run_1 record=mem_1 limit=10")),
            InteractiveMemoryRuleHitQuery {
                run_id: Some("run_1".to_string()),
                record_id: Some("mem_1".to_string()),
                limit: Some(10),
            }
        );
    }

    #[test]
    fn parses_memory_consolidation_request_spec() {
        assert_eq!(
            parse_memory_consolidation_request(Some("candidates limit=12")),
            InteractiveMemoryConsolidationRequest {
                include_candidates: true,
                limit: Some(12),
            }
        );
    }
}
