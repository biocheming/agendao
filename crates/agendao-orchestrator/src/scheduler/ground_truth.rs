use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

const BOULDER_RELATIVE_PATH: &str = ".sisyphus/boulder.json";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct BoulderState {
    active_plan: String,
    started_at: String,
    session_ids: Vec<String>,
    plan_name: String,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    worktree_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SchedulerGroundTruthContext {
    pub(super) plan_path: String,
    pub(super) plan_snapshot: Option<String>,
    pub(super) boulder_path: Option<String>,
    pub(super) plan_name: Option<String>,
    pub(super) worktree_path: Option<String>,
    pub(super) session_count: Option<usize>,
    pub(super) started_at: Option<String>,
    pub(super) agent: Option<String>,
}

fn read_trimmed_file(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|content| content.trim().to_string())
        .filter(|content| !content.is_empty())
}

fn boulder_path(workdir: &str) -> PathBuf {
    Path::new(workdir).join(BOULDER_RELATIVE_PATH)
}

fn relativize_display_path(workdir: &str, path: &Path) -> String {
    path.strip_prefix(workdir)
        .ok()
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn read_boulder_state(workdir: &str) -> Option<(BoulderState, String)> {
    let path = boulder_path(workdir);
    let raw = fs::read_to_string(&path).ok()?;
    let state = serde_json::from_str::<BoulderState>(&raw).ok()?;
    Some((state, relativize_display_path(workdir, &path)))
}

pub(super) fn load_scheduler_ground_truth(
    workdir: &str,
    preferred_plan_path: Option<&str>,
) -> Option<SchedulerGroundTruthContext> {
    let boulder = read_boulder_state(workdir);
    let plan_path = preferred_plan_path
        .map(PathBuf::from)
        .filter(|path| path.exists() || Path::new(workdir).join(path).exists())
        .or_else(|| {
            boulder
                .as_ref()
                .map(|(state, _)| PathBuf::from(&state.active_plan))
                .filter(|path| path.exists())
        })?;

    let absolute_plan_path = if plan_path.is_absolute() {
        plan_path
    } else {
        Path::new(workdir).join(plan_path)
    };

    let displayed_plan_path = relativize_display_path(workdir, &absolute_plan_path);
    let plan_snapshot = read_trimmed_file(&absolute_plan_path);

    let (boulder_path, plan_name, worktree_path, session_count, started_at, agent) = boulder
        .map(|(state, path)| {
            (
                Some(path),
                Some(state.plan_name),
                state.worktree_path,
                Some(state.session_ids.len()),
                Some(state.started_at),
                state.agent,
            )
        })
        .unwrap_or((None, None, None, None, None, None));

    Some(SchedulerGroundTruthContext {
        plan_path: displayed_plan_path,
        plan_snapshot,
        boulder_path,
        plan_name,
        worktree_path,
        session_count,
        started_at,
        agent,
    })
}

pub(super) fn render_scheduler_ground_truth(
    context: &SchedulerGroundTruthContext,
) -> Option<String> {
    let mut sections = Vec::new();
    sections.push(format!("authoritative_plan_path: `{}`", context.plan_path));
    if let Some(plan_name) = context.plan_name.as_deref() {
        sections.push(format!("plan_name: `{plan_name}`"));
    }
    if let Some(boulder_path) = context.boulder_path.as_deref() {
        sections.push(format!("boulder_state_path: `{boulder_path}`"));
    }
    if let Some(agent) = context.agent.as_deref() {
        sections.push(format!("active_agent: `{agent}`"));
    }
    if let Some(worktree_path) = context.worktree_path.as_deref() {
        sections.push(format!("worktree_path: `{worktree_path}`"));
    }
    if let Some(session_count) = context.session_count {
        sections.push(format!("tracked_sessions: `{session_count}`"));
    }
    if let Some(started_at) = context.started_at.as_deref() {
        sections.push(format!("started_at: `{started_at}`"));
    }
    if let Some(plan_snapshot) = context.plan_snapshot.as_deref() {
        sections.push(format!("authoritative_plan_snapshot:\n{plan_snapshot}"));
    }
    (!sections.is_empty()).then(|| sections.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("agendao-orchestrator-{name}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn loads_ground_truth_from_boulder_state_when_preferred_path_missing() {
        let workdir = temp_dir("ground-truth");
        let plan_path = workdir.join(".sisyphus/plans/demo.md");
        fs::create_dir_all(plan_path.parent().unwrap()).unwrap();
        fs::write(&plan_path, "- [ ] task A\n- [x] task B\n").unwrap();
        fs::write(
            workdir.join(BOULDER_RELATIVE_PATH),
            format!(
                r#"{{
  "active_plan": "{}",
  "started_at": "2026-03-09T00:00:00Z",
  "session_ids": ["ses-1", "ses-2"],
  "plan_name": "demo",
  "agent": "atlas",
  "worktree_path": "/tmp/worktree-demo"
}}"#,
                plan_path.display()
            ),
        )
        .unwrap();

        let context = load_scheduler_ground_truth(workdir.to_string_lossy().as_ref(), None)
            .expect("ground truth should load");
        assert_eq!(context.plan_name.as_deref(), Some("demo"));
        assert_eq!(context.session_count, Some(2));
        assert!(context.plan_snapshot.unwrap().contains("- [ ] task A"));
    }
}
