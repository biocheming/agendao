use super::CliStyle;
use crate::util::truncate_text;
use agendao_stage_protocol::StageEvent;

pub(super) type CliEventsQueryInput = agendao_command_runtime::interactive::InteractiveEventsQuery;
pub(super) type CliEventsCommandInput =
    agendao_command_runtime::interactive::InteractiveEventsCommand;

#[derive(Debug, Clone, Default)]
pub(super) struct CliEventsBrowserState {
    pub(super) session_id: String,
    pub(super) filter: CliEventsQueryInput,
    pub(super) offset: usize,
}

#[cfg(test)]
pub(super) const CLI_EVENTS_DEFAULT_PAGE_SIZE: usize =
    agendao_command_runtime::interactive::EVENTS_BROWSER_DEFAULT_PAGE_SIZE;

pub(super) fn cli_default_events_query_input() -> CliEventsQueryInput {
    agendao_command_runtime::interactive::default_events_browser_query()
}

pub(super) fn cli_parse_events_command_input(raw: Option<&str>) -> CliEventsCommandInput {
    agendao_command_runtime::interactive::parse_events_browser_command(raw)
}

#[cfg(test)]
pub(super) fn cli_parse_events_query_input(raw: Option<&str>) -> CliEventsQueryInput {
    agendao_command_runtime::interactive::parse_events_browser_query(raw)
}

pub(super) fn cli_events_query(
    input: &CliEventsQueryInput,
    offset: usize,
) -> crate::api_client::SessionEventsQuery {
    crate::api_client::SessionEventsQuery {
        stage_id: input.stage_id.clone(),
        execution_id: input.execution_id.clone(),
        event_type: input.event_type.clone(),
        since: input.since,
        limit: input.limit,
        offset: Some(offset),
    }
}

pub(super) fn cli_events_page_size(input: &CliEventsQueryInput) -> usize {
    agendao_command_runtime::interactive::events_browser_page_size(input)
}

pub(super) fn cli_events_offset_for_page(input: &CliEventsQueryInput, page: usize) -> usize {
    agendao_command_runtime::interactive::events_browser_offset_for_page(input, page)
}

pub(super) fn cli_events_page_for_offset(input: &CliEventsQueryInput, offset: usize) -> usize {
    agendao_command_runtime::interactive::events_browser_page_for_offset(input, offset)
}

pub(super) fn cli_events_filter_label(input: &CliEventsQueryInput) -> String {
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
    parts.push(format!("limit={}", cli_events_page_size(input)));
    parts.join(" · ")
}

pub(super) fn cli_events_window_label(offset: usize, count: usize) -> String {
    if count == 0 {
        return "items 0".to_string();
    }
    format!("items {}-{}", offset + 1, offset + count)
}

fn cli_event_payload_summary(payload: &serde_json::Value) -> Option<String> {
    match payload {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(text.trim().to_string()),
        value => serde_json::to_string(value).ok(),
    }
    .filter(|text| !text.is_empty())
    .map(|text| truncate_text(&text.replace('\n', " "), 120))
}

pub(super) fn cli_event_lines(events: &[StageEvent], style: &CliStyle) -> Vec<String> {
    if events.is_empty() {
        return vec![style.dim("no matching events")];
    }

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
        lines.push(headline);
        if let Some(payload) = cli_event_payload_summary(&event.payload) {
            lines.push(format!("  {}", payload));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::{
        cli_default_events_query_input, cli_parse_events_command_input,
        cli_parse_events_query_input, CliEventsCommandInput, CliEventsQueryInput,
        CLI_EVENTS_DEFAULT_PAGE_SIZE,
    };

    #[test]
    fn parses_default_events_query_input() {
        assert_eq!(
            cli_parse_events_query_input(None),
            cli_default_events_query_input()
        );
    }

    #[test]
    fn parses_stage_alias_events_query_input() {
        assert_eq!(
            cli_parse_events_query_input(Some("stg_123")),
            CliEventsQueryInput {
                stage_id: Some("stg_123".to_string()),
                limit: Some(CLI_EVENTS_DEFAULT_PAGE_SIZE),
                ..Default::default()
            }
        );
    }

    #[test]
    fn parses_structured_events_query_input() {
        assert_eq!(
            cli_parse_events_query_input(Some(
                "stage=stg_1 exec=exe_2 type=session.updated limit=10 since=42"
            )),
            CliEventsQueryInput {
                stage_id: Some("stg_1".to_string()),
                execution_id: Some("exe_2".to_string()),
                event_type: Some("session.updated".to_string()),
                since: Some(42),
                limit: Some(10),
            }
        );
    }

    #[test]
    fn parses_events_navigation_commands() {
        assert_eq!(
            cli_parse_events_command_input(Some("next")),
            CliEventsCommandInput::NextPage
        );
        assert_eq!(
            cli_parse_events_command_input(Some("prev")),
            CliEventsCommandInput::PreviousPage
        );
        assert_eq!(
            cli_parse_events_command_input(Some("clear")),
            CliEventsCommandInput::Clear
        );
        assert_eq!(
            cli_parse_events_command_input(Some("first")),
            CliEventsCommandInput::FirstPage
        );
        assert_eq!(
            cli_parse_events_command_input(Some("page 3")),
            CliEventsCommandInput::JumpPage(3)
        );
        assert_eq!(
            cli_parse_events_command_input(Some("stage=stg_1 limit=10")),
            CliEventsCommandInput::ShowFiltered {
                filter: CliEventsQueryInput {
                    stage_id: Some("stg_1".to_string()),
                    limit: Some(10),
                    ..Default::default()
                },
                page: 1,
            }
        );
        assert_eq!(
            cli_parse_events_command_input(Some("stage=stg_1 limit=10 page=2")),
            CliEventsCommandInput::ShowFiltered {
                filter: CliEventsQueryInput {
                    stage_id: Some("stg_1".to_string()),
                    limit: Some(10),
                    ..Default::default()
                },
                page: 2,
            }
        );
    }
}
