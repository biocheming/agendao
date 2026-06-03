use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use crate::CommandContext;

const MODEL_DECIDES_WORKTREE_BLOCK: &str = r#"
## Worktree Setup Required

No worktree specified. Before starting work, you MUST choose or create one:

1. `git worktree list --porcelain` — list existing worktrees
2. Create if needed: `git worktree add <absolute-path> <branch-or-HEAD>`
3. Update `.sisyphus/boulder.json` — add `"worktree_path": "<absolute-path>"`
4. Work exclusively inside that worktree directory"#;

const KEYWORD_PATTERN: &[&str] = &["ultrawork", "ulw"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUserRequest {
    pub plan_name: Option<String>,
    pub explicit_worktree_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct BoulderState {
    active_plan: String,
    started_at: String,
    session_ids: Vec<String>,
    plan_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlanProgress {
    total: usize,
    completed: usize,
    is_complete: bool,
}

pub fn render(base_prompt: String, ctx: &CommandContext) -> anyhow::Result<String> {
    let session_id = ctx
        .variables
        .get("SESSION_ID")
        .cloned()
        .unwrap_or_else(|| "unknown-session".to_string());
    let timestamp = ctx
        .variables
        .get("TIMESTAMP")
        .cloned()
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    let context_info = build_context_info(ctx, &session_id, &timestamp)?;
    Ok(format!("{base_prompt}\n\n---\n{context_info}"))
}

fn build_context_info(
    ctx: &CommandContext,
    session_id: &str,
    timestamp: &str,
) -> anyhow::Result<String> {
    let directory = ctx.working_directory.as_path();
    let existing_state = read_boulder_state(directory);
    let parsed = parse_user_request(&ctx.arguments.join(" "));
    let (worktree_path, worktree_block) =
        resolve_worktree_context(parsed.explicit_worktree_path.as_deref());
    let mut context_info = String::new();

    if let Some(explicit_plan_name) = parsed.plan_name.as_deref() {
        let all_plans = find_prometheus_plans(directory);
        let matched_plan = find_plan_by_name(&all_plans, explicit_plan_name);

        if let Some(plan_path) = matched_plan {
            let progress = get_plan_progress(&plan_path);
            if progress.is_complete {
                context_info = format!(
                    "## Plan Already Complete\n\nThe requested plan \"{}\" has been completed.\nAll {} tasks are done. Create a new plan with: /plan \"your task\"",
                    get_plan_name(&plan_path),
                    progress.total
                );
            } else {
                if existing_state.is_some() {
                    clear_boulder_state(directory);
                }
                let new_state = create_boulder_state(
                    &plan_path,
                    session_id,
                    Some("atlas".to_string()),
                    worktree_path.clone(),
                    Some(timestamp.to_string()),
                );
                write_boulder_state(directory, &new_state)?;
                context_info = format!(
                    "## Auto-Selected Plan\n\n**Plan**: {}\n**Path**: {}\n**Progress**: {}/{} tasks\n**Session ID**: {}\n**Started**: {}\n{}\n\nboulder.json has been created. Read the plan and begin execution.",
                    get_plan_name(&plan_path),
                    plan_path.display(),
                    progress.completed,
                    progress.total,
                    session_id,
                    timestamp,
                    worktree_block,
                );
            }
        } else {
            let incomplete_plans: Vec<PathBuf> = all_plans
                .into_iter()
                .filter(|path| !get_plan_progress(path).is_complete)
                .collect();
            if incomplete_plans.is_empty() {
                context_info = format!(
                    "## Plan Not Found\n\nCould not find a plan matching \"{}\".\nNo incomplete plans available. Create a new plan with: /plan \"your task\"",
                    explicit_plan_name
                );
            } else {
                let plan_list = incomplete_plans
                    .iter()
                    .enumerate()
                    .map(|(idx, plan_path)| {
                        let progress = get_plan_progress(plan_path);
                        format!(
                            "{}. [{}] - Progress: {}/{}",
                            idx + 1,
                            get_plan_name(plan_path),
                            progress.completed,
                            progress.total
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                context_info = format!(
                    "## Plan Not Found\n\nCould not find a plan matching \"{}\".\n\nAvailable incomplete plans:\n{}\n\nAsk the user which plan to work on.",
                    explicit_plan_name,
                    plan_list
                );
            }
        }
    } else if let Some(existing_state) = existing_state.clone() {
        let progress = get_plan_progress(Path::new(&existing_state.active_plan));
        if !progress.is_complete {
            let effective_worktree = worktree_path
                .clone()
                .or(existing_state.worktree_path.clone());
            if let Some(path) = worktree_path.clone() {
                let mut updated_sessions = existing_state.session_ids.clone();
                if !updated_sessions.iter().any(|value| value == session_id) {
                    updated_sessions.push(session_id.to_string());
                }
                let updated_state = BoulderState {
                    worktree_path: Some(path),
                    session_ids: updated_sessions,
                    ..existing_state.clone()
                };
                write_boulder_state(directory, &updated_state)?;
            } else {
                append_session_id(directory, session_id)?;
            }
            let worktree_display = effective_worktree
                .map(|path| format!("\n**Worktree**: {path}"))
                .unwrap_or_else(|| worktree_block.clone());
            context_info = format!(
                "## Active Work Session Found\n\n**Status**: RESUMING existing work\n**Plan**: {}\n**Path**: {}\n**Progress**: {}/{} tasks completed\n**Sessions**: {} (current session appended)\n**Started**: {}{}\n\nThe current session ({}) has been added to session_ids.\nRead the plan file and continue from the first unchecked task.",
                existing_state.plan_name,
                existing_state.active_plan,
                progress.completed,
                progress.total,
                existing_state.session_ids.len() + 1,
                existing_state.started_at,
                worktree_display,
                session_id,
            );
        } else {
            context_info = format!(
                "## Previous Work Complete\n\nThe previous plan ({}) has been completed.\nLooking for new plans...",
                existing_state.plan_name
            );
        }
    }

    let should_search_for_plans = match (&existing_state, parsed.plan_name.as_deref()) {
        (None, None) => true,
        (Some(state), None) => get_plan_progress(Path::new(&state.active_plan)).is_complete,
        _ => false,
    };

    if should_search_for_plans {
        let plans = find_prometheus_plans(directory);
        let incomplete_plans: Vec<PathBuf> = plans
            .iter()
            .filter(|path| !get_plan_progress(path).is_complete)
            .cloned()
            .collect();

        if plans.is_empty() {
            if !context_info.is_empty() {
                context_info.push_str("\n\n");
            }
            context_info.push_str(
                "## No Plans Found\n\nNo Prometheus plan files found at .sisyphus/plans/\nUse Prometheus to create a work plan first: /plan \"your task\"",
            );
        } else if incomplete_plans.is_empty() {
            if !context_info.is_empty() {
                context_info.push_str("\n\n");
            }
            context_info.push_str(&format!(
                "## All Plans Complete\n\nAll {} plan(s) are complete. Create a new plan with: /plan \"your task\"",
                plans.len()
            ));
        } else if incomplete_plans.len() == 1 {
            let plan_path = &incomplete_plans[0];
            let progress = get_plan_progress(plan_path);
            let new_state = create_boulder_state(
                plan_path,
                session_id,
                Some("atlas".to_string()),
                worktree_path.clone(),
                Some(timestamp.to_string()),
            );
            write_boulder_state(directory, &new_state)?;
            if !context_info.is_empty() {
                context_info.push_str("\n\n");
            }
            context_info.push_str(&format!(
                "## Auto-Selected Plan\n\n**Plan**: {}\n**Path**: {}\n**Progress**: {}/{} tasks\n**Session ID**: {}\n**Started**: {}\n{}\n\nboulder.json has been created. Read the plan and begin execution.",
                get_plan_name(plan_path),
                plan_path.display(),
                progress.completed,
                progress.total,
                session_id,
                timestamp,
                worktree_block,
            ));
        } else {
            let plan_list = incomplete_plans
                .iter()
                .enumerate()
                .map(|(idx, plan_path)| {
                    let progress = get_plan_progress(plan_path);
                    let modified = fs::metadata(plan_path)
                        .and_then(|meta| meta.modified())
                        .map(|time| chrono::DateTime::<Utc>::from(time).to_rfc3339())
                        .unwrap_or_else(|_| timestamp.to_string());
                    format!(
                        "{}. [{}] - Modified: {} - Progress: {}/{}",
                        idx + 1,
                        get_plan_name(plan_path),
                        modified,
                        progress.completed,
                        progress.total
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            if !context_info.is_empty() {
                context_info.push_str("\n\n");
            }
            context_info.push_str(&format!(
                "<system-reminder>\n## Multiple Plans Found\n\nCurrent Time: {}\nSession ID: {}\n\n{}\n\nAsk the user which plan to work on. Present the options above and wait for their response.\n{}\n</system-reminder>",
                timestamp,
                session_id,
                plan_list,
                worktree_block,
            ));
        }
    }

    Ok(context_info)
}

pub fn parse_user_request(raw: &str) -> ParsedUserRequest {
    let mut remainder = raw.trim().to_string();
    if remainder.is_empty() {
        return ParsedUserRequest {
            plan_name: None,
            explicit_worktree_path: None,
        };
    }

    let parts = remainder
        .split_whitespace()
        .map(|part| part.to_string())
        .collect::<Vec<_>>();
    let mut explicit_worktree_path = None;
    let mut retained = Vec::new();
    let mut idx = 0;
    while idx < parts.len() {
        if parts[idx] == "--worktree" {
            explicit_worktree_path = parts.get(idx + 1).cloned();
            idx += if explicit_worktree_path.is_some() {
                2
            } else {
                1
            };
            continue;
        }
        retained.push(parts[idx].clone());
        idx += 1;
    }

    remainder = retained
        .into_iter()
        .filter(|item| {
            !KEYWORD_PATTERN
                .iter()
                .any(|keyword| item.eq_ignore_ascii_case(keyword))
        })
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();

    ParsedUserRequest {
        plan_name: (!remainder.is_empty()).then_some(remainder),
        explicit_worktree_path,
    }
}

fn resolve_worktree_context(explicit_worktree_path: Option<&str>) -> (Option<String>, String) {
    match explicit_worktree_path {
        None => (None, MODEL_DECIDES_WORKTREE_BLOCK.to_string()),
        Some(path) => {
            if let Some(validated) = detect_worktree_path(path) {
                (
                    Some(validated.clone()),
                    format!("\n**Worktree**: {validated}"),
                )
            } else {
                (
                    None,
                    format!(
                        "\n**Worktree** (needs setup): `git worktree add {} <branch>`, then add `\"worktree_path\"` to boulder.json",
                        path
                    ),
                )
            }
        }
    }
}

fn detect_worktree_path(directory: &str) -> Option<String> {
    let output = ProcessCommand::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(directory)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn boulder_file_path(directory: &Path) -> PathBuf {
    directory.join(".sisyphus").join("boulder.json")
}

fn read_boulder_state(directory: &Path) -> Option<BoulderState> {
    let file_path = boulder_file_path(directory);
    let content = fs::read_to_string(file_path).ok()?;
    let mut state = serde_json::from_str::<BoulderState>(&content).ok()?;
    if state.session_ids.is_empty() {
        state.session_ids = Vec::new();
    }
    Some(state)
}

fn write_boulder_state(directory: &Path, state: &BoulderState) -> anyhow::Result<()> {
    let file_path = boulder_file_path(directory);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(file_path, serde_json::to_string_pretty(state)?)?;
    Ok(())
}

fn append_session_id(directory: &Path, session_id: &str) -> anyhow::Result<Option<BoulderState>> {
    let Some(mut state) = read_boulder_state(directory) else {
        return Ok(None);
    };
    if !state.session_ids.iter().any(|value| value == session_id) {
        state.session_ids.push(session_id.to_string());
        write_boulder_state(directory, &state)?;
    }
    Ok(Some(state))
}

fn clear_boulder_state(directory: &Path) {
    let file_path = boulder_file_path(directory);
    if let Err(error) = fs::remove_file(&file_path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(
                path = %file_path.display(),
                %error,
                "failed to clear boulder state file"
            );
        }
    }
}

fn find_prometheus_plans(directory: &Path) -> Vec<PathBuf> {
    let plans_dir = directory.join(".sisyphus").join("plans");
    let mut plans = match fs::read_dir(plans_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("md"))
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    plans.sort_by(|left, right| {
        let left_mtime = fs::metadata(left).and_then(|meta| meta.modified()).ok();
        let right_mtime = fs::metadata(right).and_then(|meta| meta.modified()).ok();
        right_mtime.cmp(&left_mtime)
    });
    plans
}

fn get_plan_progress(plan_path: &Path) -> PlanProgress {
    let Ok(content) = fs::read_to_string(plan_path) else {
        return PlanProgress {
            total: 0,
            completed: 0,
            is_complete: true,
        };
    };

    let mut unchecked = 0usize;
    let mut checked = 0usize;
    for line in content.lines().map(str::trim_start) {
        if !(line.starts_with("- [") || line.starts_with("* [")) {
            continue;
        }
        if line.starts_with("- [ ]") || line.starts_with("* [ ]") {
            unchecked += 1;
        } else if line.starts_with("- [x]")
            || line.starts_with("* [x]")
            || line.starts_with("- [X]")
            || line.starts_with("* [X]")
        {
            checked += 1;
        }
    }
    let total = unchecked + checked;
    PlanProgress {
        total,
        completed: checked,
        is_complete: total == 0 || total == checked,
    }
}

fn get_plan_name(plan_path: &Path) -> String {
    plan_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("plan")
        .to_string()
}

fn create_boulder_state(
    plan_path: &Path,
    session_id: &str,
    agent: Option<String>,
    worktree_path: Option<String>,
    timestamp: Option<String>,
) -> BoulderState {
    BoulderState {
        active_plan: plan_path.to_string_lossy().to_string(),
        started_at: timestamp.unwrap_or_else(|| Utc::now().to_rfc3339()),
        session_ids: vec![session_id.to_string()],
        plan_name: get_plan_name(plan_path),
        agent,
        worktree_path,
    }
}

fn find_plan_by_name(plans: &[PathBuf], requested_name: &str) -> Option<PathBuf> {
    let requested = requested_name.to_ascii_lowercase();
    plans
        .iter()
        .find(|plan| get_plan_name(plan).to_ascii_lowercase() == requested)
        .cloned()
        .or_else(|| {
            plans
                .iter()
                .find(|plan| {
                    get_plan_name(plan)
                        .to_ascii_lowercase()
                        .contains(&requested)
                })
                .cloned()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("agendao-command-{label}-{suffix}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_user_request_with_worktree_and_keyword() {
        let parsed = parse_user_request("my-plan ultrawork --worktree /tmp/wt");
        assert_eq!(parsed.plan_name.as_deref(), Some("my-plan"));
        assert_eq!(parsed.explicit_worktree_path.as_deref(), Some("/tmp/wt"));
    }

    #[test]
    fn renders_plan_selection_context_and_persists_boulder() {
        let directory = temp_dir("start-work");
        let plan_dir = directory.join(".sisyphus/plans");
        fs::create_dir_all(&plan_dir).unwrap();
        fs::write(
            plan_dir.join("demo.md"),
            "# Demo\n\n- [ ] first\n- [x] second\n",
        )
        .unwrap();

        let ctx = CommandContext::new(directory.clone())
            .with_arguments(vec!["demo".to_string()])
            .with_variable("SESSION_ID".to_string(), "session-1".to_string())
            .with_variable("TIMESTAMP".to_string(), "2026-03-08T00:00:00Z".to_string());

        let rendered = render("<session-context></session-context>".to_string(), &ctx).unwrap();
        assert!(rendered.contains("## Auto-Selected Plan"));
        assert!(rendered.contains("demo"));

        let boulder = read_boulder_state(&directory).unwrap();
        assert_eq!(boulder.plan_name, "demo");
        assert_eq!(boulder.session_ids, vec!["session-1".to_string()]);

        let _ = fs::remove_dir_all(directory);
    }
}
