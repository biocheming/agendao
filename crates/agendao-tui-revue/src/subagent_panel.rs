//! M9 read-only Subagent transcript/details projection and panel.
use crate::dialog::backdrop::{self, ListItem};
use crate::theme::colors;
use agendao_api::{ExecutionKind, ExecutionStatus, SessionExecutionNode, SessionExecutionTopology};
use revue::event::Key;
use revue::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubagentEntry {
    pub id: String,
    pub label: String,
    pub status: ExecutionStatus,
    pub recent_event: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubagentPanelProjection {
    pub session_id: String,
    pub topology_updated_at: Option<i64>,
    pub topology_fingerprint: String,
    pub entries: Vec<SubagentEntry>,
}

impl SubagentPanelProjection {
    pub fn from_topology(topology: &SessionExecutionTopology) -> Self {
        fn walk(nodes: &[SessionExecutionNode], out: &mut Vec<SubagentEntry>) {
            for node in nodes {
                let explicit = node.metadata.as_ref().is_some_and(|m| {
                    m.get("subagent").and_then(|v| v.as_bool()) == Some(true)
                        || m.get("role").and_then(|v| v.as_str()) == Some("subagent")
                });
                if explicit
                    && matches!(node.kind, ExecutionKind::SchedulerNode)
                    && !matches!(node.status, ExecutionStatus::Done)
                {
                    out.push(SubagentEntry {
                        id: node.id.clone(),
                        label: node.label.clone().unwrap_or_else(|| node.id.clone()),
                        status: node.status.clone(),
                        recent_event: node.recent_event.clone(),
                    });
                }
                walk(&node.children, out);
            }
        }
        let mut entries = Vec::new();
        walk(&topology.roots, &mut entries);
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            session_id: topology.session_id.clone(),
            topology_updated_at: topology.updated_at,
            topology_fingerprint: serde_json::to_string(topology).unwrap_or_default(),
            entries,
        }
    }
}

pub struct SubagentPanel {
    pub visible: bool,
    selected: usize,
}
impl Default for SubagentPanel {
    fn default() -> Self {
        Self::new()
    }
}
impl SubagentPanel {
    pub fn new() -> Self {
        Self {
            visible: false,
            selected: 0,
        }
    }
    pub fn open(&mut self) {
        self.visible = true;
        self.selected = 0;
    }
    pub fn close(&mut self) {
        self.visible = false;
    }
    pub fn handle_key(&mut self, key: &Key, count: usize) -> bool {
        if !self.visible {
            return false;
        }
        if count == 0 {
            if matches!(key, Key::Escape) {
                self.close();
            }
            return true;
        }
        match key {
            Key::Up => self.selected = (self.selected + count - 1) % count,
            Key::Down => self.selected = (self.selected + 1) % count,
            Key::Home => self.selected = 0,
            Key::End => self.selected = count - 1,
            Key::Escape => self.close(),
            _ => {}
        }
        true
    }
    pub fn render(
        &self,
        ctx: &mut RenderContext,
        geom: backdrop::PromptGeom,
        projection: Option<&SubagentPanelProjection>,
    ) {
        if !self.visible {
            return;
        }
        let Some(p) = projection else {
            backdrop::render_list_dialog_bottom(
                backdrop::ListDialogHeading {
                    title: "Subagents",
                    border_color: colors::ACCENT_CYAN(),
                },
                &[ListItem::Row {
                    display: "  (No authoritative subagent topology)".into(),
                    muted: true,
                }],
                0,
                "Esc: close",
                ctx,
                geom,
                3,
            );
            return;
        };
        if p.entries.is_empty() {
            backdrop::render_list_dialog_bottom(
                backdrop::ListDialogHeading {
                    title: "Subagents",
                    border_color: colors::ACCENT_CYAN(),
                },
                &[ListItem::Row {
                    display: "  (No active subagents)".into(),
                    muted: true,
                }],
                0,
                "Esc: close",
                ctx,
                geom,
                3,
            );
            return;
        }
        let items: Vec<ListItem> = p
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| ListItem::Row {
                display: format!(
                    "{}{} [{:?}] {}{}",
                    if i == self.selected { "▶ " } else { "  " },
                    e.id,
                    e.status,
                    e.label,
                    e.recent_event
                        .as_deref()
                        .map(|s| format!(" — {s}"))
                        .unwrap_or_default()
                ),
                muted: false,
            })
            .collect();
        backdrop::render_list_dialog_bottom(
            backdrop::ListDialogHeading {
                title: "Subagents (read-only latest details)",
                border_color: colors::ACCENT_CYAN(),
            },
            &items,
            self.selected.min(items.len() - 1),
            "Latest event only · ↑↓ navigate  Esc: close",
            ctx,
            geom,
            12,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_explicit_running_subagents_project() {
        let t = SessionExecutionTopology {
            session_id: "s".into(),
            active_count: 2,
            done_count: 1,
            running_count: 2,
            waiting_count: 0,
            cancelling_count: 0,
            retry_count: 0,
            updated_at: Some(2),
            roots: vec![
                SessionExecutionNode {
                    id: "ordinary".into(),
                    kind: ExecutionKind::SchedulerNode,
                    status: ExecutionStatus::Running,
                    label: None,
                    parent_id: None,
                    waiting_on: None,
                    recent_event: None,
                    started_at: 0,
                    updated_at: 0,
                    metadata: None,
                    children: vec![],
                },
                SessionExecutionNode {
                    id: "sa".into(),
                    kind: ExecutionKind::SchedulerNode,
                    status: ExecutionStatus::Waiting,
                    label: Some("agent".into()),
                    parent_id: None,
                    waiting_on: None,
                    recent_event: Some("working".into()),
                    started_at: 0,
                    updated_at: 0,
                    metadata: Some(serde_json::json!({"role":"subagent"})),
                    children: vec![],
                },
                SessionExecutionNode {
                    id: "done".into(),
                    kind: ExecutionKind::SchedulerNode,
                    status: ExecutionStatus::Done,
                    label: None,
                    parent_id: None,
                    waiting_on: None,
                    recent_event: None,
                    started_at: 0,
                    updated_at: 0,
                    metadata: Some(serde_json::json!({"subagent":true})),
                    children: vec![],
                },
            ],
        };
        let p = SubagentPanelProjection::from_topology(&t);
        assert_eq!(
            p.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["sa"]
        );
    }
}
