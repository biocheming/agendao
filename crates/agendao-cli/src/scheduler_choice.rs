use agendao_orchestrator::selector::SchedulerChoice;
use agendao_orchestrator::templates::TemplateId;
use jsonc_parser::{parse_to_serde_value, ParseOptions};

pub(super) fn parse_scheduler_choice(raw: Option<&str>) -> anyhow::Result<Option<SchedulerChoice>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let choice = match raw {
        "auto" => SchedulerChoice::Auto,
        "direct" => SchedulerChoice::Template {
            template: TemplateId::Direct,
        },
        "plan" => SchedulerChoice::Template {
            template: TemplateId::Plan,
        },
        "coordinate" => SchedulerChoice::Template {
            template: TemplateId::Coordinate,
        },
        "verify" => SchedulerChoice::Template {
            template: TemplateId::Verify,
        },
        "autoresearch" => SchedulerChoice::Template {
            template: TemplateId::Autoresearch,
        },
        path if path.starts_with('@') => SchedulerChoice::Blueprint {
            blueprint: parse_blueprint_file(std::path::Path::new(&path[1..]))?,
        },
        json if json.starts_with('{') => SchedulerChoice::Blueprint {
            blueprint: parse_blueprint_document(json)?,
        },
        other => anyhow::bail!(
            "unknown scheduler '{other}'; expected auto, direct, plan, coordinate, verify, autoresearch, @FILE, or a SchedulerBlueprint JSON object"
        ),
    };
    Ok(Some(choice))
}

pub(super) fn parse_blueprint_file(
    path: &std::path::Path,
) -> anyhow::Result<agendao_orchestrator::blueprint::SchedulerBlueprint> {
    let content = std::fs::read_to_string(path).map_err(|error| {
        anyhow::anyhow!("failed to read Blueprint '{}': {error}", path.display())
    })?;
    parse_blueprint_document(&content)
}

fn parse_blueprint_document(
    content: &str,
) -> anyhow::Result<agendao_orchestrator::blueprint::SchedulerBlueprint> {
    let options = ParseOptions {
        allow_trailing_commas: true,
        ..ParseOptions::default()
    };
    let value = parse_to_serde_value(content, &options)
        .map_err(|error| anyhow::anyhow!("invalid scheduler Blueprint JSONC: {error:?}"))?
        .ok_or_else(|| anyhow::anyhow!("scheduler Blueprint document is empty"))?;
    serde_json::from_value(value)
        .map_err(|error| anyhow::anyhow!("invalid scheduler Blueprint: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_legacy_profile_names() {
        assert!(parse_scheduler_choice(Some("sisyphus")).is_err());
    }

    #[test]
    fn parses_typed_template() {
        assert!(matches!(
            parse_scheduler_choice(Some("verify")).unwrap(),
            Some(SchedulerChoice::Template {
                template: TemplateId::Verify
            })
        ));
    }

    #[test]
    fn parses_blueprint_jsonc_document() {
        let document = r#"
        {
          // User-authored scheduler data.
          "schema": "v1",
          "name": "direct",
          "entry": "done",
          "nodes": {
            "done": { "kind": "end", "result": "last-node" },
          },
          "limits": {
            "max_model_calls": 1,
            "max_tool_calls": 1,
            "max_total_tokens": 1,
            "max_wall_time_ms": 1,
            "max_parallelism": 1,
            "max_graph_nodes": 1,
            "max_graph_depth": 1,
            "max_loop_iterations": 1,
            "max_agent_steps": 1
          },
          "output": {
            "format": "text",
            "include_usage": true,
            "include_artifact_refs": false
          }
        }
        "#;
        let blueprint = parse_blueprint_document(document).expect("valid JSONC Blueprint");
        assert_eq!(blueprint.name.as_str(), "direct");
    }
}
