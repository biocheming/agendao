//! 土 — Task governance state dialog.
//!
//! The list is a projection of the server-authoritative ledger. Edits emit
//! typed CAS operations; the app bridge commits them through HTTP, Unix RPC,
//! or Direct mode and then applies the returned replacement snapshot.

use crate::dialog::backdrop::{self, ListDialogHeading, ListItem, PromptGeom};
use crate::theme::colors;
use agendao_types::task_ledger::{
    current_checkpoints, missing_acceptance_criteria, SessionTaskLedger, TaskGoal, TaskLedgerActor,
    TaskLedgerOp, VerificationCoverage, VerifierRef,
};
use revue::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub enum TaskStateAction {
    Apply(TaskLedgerOp),
    NavigateEvidence(String),
}

#[derive(Clone, Debug)]
enum RowAction {
    EditGoal,
    EditNext,
    EditCore { slot: u8 },
    CloseOpen { open_id: String },
    Evidence(String),
}

struct TaskStateRow {
    item: ListItem,
    action: Option<RowAction>,
}

enum EditMode {
    Goal,
    Next,
    Core { slot: Option<u8> },
    CloseOpen { open_id: String },
}

struct TaskStateEditor {
    mode: EditMode,
    primary: revue::widget::Input,
    secondary: revue::widget::Input,
    secondary_active: bool,
}

pub struct TaskStateDialog {
    pub visible: bool,
    selected: usize,
    editor: Option<TaskStateEditor>,
}

impl Default for TaskStateDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskStateDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            selected: 0,
            editor: None,
        }
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.selected = 0;
        self.editor = None;
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
        self.editor = None;
    }

    pub fn handle_key(
        &mut self,
        key: &revue::event::Key,
        ledger: Option<&SessionTaskLedger>,
    ) -> Option<TaskStateAction> {
        if self.editor.is_some() {
            return self.handle_editor_key(key, ledger);
        }
        let rows = task_state_rows(ledger);
        match key {
            revue::event::Key::Escape
            | revue::event::Key::Char('q')
            | revue::event::Key::Char(' ') => self.dismiss(),
            revue::event::Key::Up => self.selected = self.selected.saturating_sub(1),
            revue::event::Key::Down => {
                self.selected = (self.selected + 1).min(rows.len().saturating_sub(1));
            }
            revue::event::Key::Home => self.selected = 0,
            revue::event::Key::End => self.selected = rows.len().saturating_sub(1),
            revue::event::Key::Char('a') => self.open_core_editor(None, ""),
            revue::event::Key::Char('e') => {
                if let Some(action) = rows.get(self.selected).and_then(|row| row.action.clone()) {
                    self.open_row_editor(action, ledger);
                }
            }
            revue::event::Key::Char('c') => {
                if let Some(RowAction::CloseOpen { open_id }) =
                    rows.get(self.selected).and_then(|row| row.action.clone())
                {
                    self.open_close_editor(open_id);
                }
            }
            revue::event::Key::Enter => {
                if let Some(RowAction::Evidence(reference)) =
                    rows.get(self.selected).and_then(|row| row.action.clone())
                {
                    return Some(TaskStateAction::NavigateEvidence(reference));
                }
            }
            _ => {}
        }
        None
    }

    fn open_row_editor(&mut self, action: RowAction, ledger: Option<&SessionTaskLedger>) {
        let Some(ledger) = ledger else { return };
        match action {
            RowAction::EditGoal => {
                let Some(goal) = ledger.goal.as_ref() else {
                    return;
                };
                self.editor = Some(TaskStateEditor {
                    mode: EditMode::Goal,
                    primary: revue::widget::Input::new().value(&goal.statement),
                    secondary: revue::widget::Input::new()
                        .value(goal.acceptance_criteria.join("; ")),
                    secondary_active: false,
                });
            }
            RowAction::EditNext => {
                let Some(next) = ledger.next.as_ref() else {
                    return;
                };
                self.editor = Some(TaskStateEditor {
                    mode: EditMode::Next,
                    primary: revue::widget::Input::new().value(&next.statement),
                    secondary: revue::widget::Input::new(),
                    secondary_active: false,
                });
            }
            RowAction::EditCore { slot } => {
                let statement = ledger
                    .core
                    .iter()
                    .filter(|entry| entry.live)
                    .nth(slot.saturating_sub(1) as usize)
                    .map(|entry| entry.statement.as_str())
                    .unwrap_or_default();
                self.open_core_editor(Some(slot), statement);
            }
            RowAction::CloseOpen { open_id } => self.open_close_editor(open_id),
            RowAction::Evidence(_) => {}
        }
    }

    fn open_core_editor(&mut self, slot: Option<u8>, statement: &str) {
        self.editor = Some(TaskStateEditor {
            mode: EditMode::Core { slot },
            primary: revue::widget::Input::new().value(statement),
            secondary: revue::widget::Input::new(),
            secondary_active: false,
        });
    }

    fn open_close_editor(&mut self, open_id: String) {
        self.editor = Some(TaskStateEditor {
            mode: EditMode::CloseOpen { open_id },
            primary: revue::widget::Input::new().placeholder("Verified claim"),
            secondary: revue::widget::Input::new().placeholder("Coverage scope"),
            secondary_active: false,
        });
    }

    fn handle_editor_key(
        &mut self,
        key: &revue::event::Key,
        ledger: Option<&SessionTaskLedger>,
    ) -> Option<TaskStateAction> {
        match key {
            revue::event::Key::Escape => {
                self.editor = None;
                return None;
            }
            revue::event::Key::Tab | revue::event::Key::BackTab => {
                if matches!(
                    self.editor.as_ref().map(|editor| &editor.mode),
                    Some(EditMode::Goal | EditMode::CloseOpen { .. })
                ) {
                    if let Some(editor) = self.editor.as_mut() {
                        editor.secondary_active = !editor.secondary_active;
                    }
                }
                return None;
            }
            revue::event::Key::Enter => return self.finish_editor(ledger),
            _ => {}
        }
        let editor = self.editor.as_mut()?;
        if editor.secondary_active {
            editor.secondary.handle_key(key);
        } else {
            editor.primary.handle_key(key);
        }
        None
    }

    fn finish_editor(&mut self, ledger: Option<&SessionTaskLedger>) -> Option<TaskStateAction> {
        let ledger = ledger?;
        let editor = self.editor.as_ref()?;
        let primary = editor.primary.text().trim().to_string();
        let secondary = editor.secondary.text().trim().to_string();
        if primary.is_empty()
            || (matches!(editor.mode, EditMode::CloseOpen { .. }) && secondary.is_empty())
        {
            return None;
        }
        let op = match &editor.mode {
            EditMode::Goal => {
                let criteria = secondary
                    .split(';')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>();
                let checks = ledger
                    .goal
                    .as_ref()
                    .map(|goal| {
                        goal.criterion_checks
                            .iter()
                            .filter(|check| criteria.contains(&check.criterion))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                TaskLedgerOp::SetGoal {
                    goal: TaskGoal {
                        statement: primary,
                        acceptance_criteria: criteria,
                        criterion_checks: checks,
                        set_by: TaskLedgerActor::User,
                        set_at: now_ms(),
                    },
                }
            }
            EditMode::Next => TaskLedgerOp::SetNext {
                statement: primary,
                actor: Some(TaskLedgerActor::User),
            },
            EditMode::Core { slot: Some(slot) } => TaskLedgerOp::SwapCoreLive {
                slot: *slot,
                statement: primary,
                actor: Some(TaskLedgerActor::User),
            },
            EditMode::Core { slot: None } => TaskLedgerOp::AddCore {
                statement: primary,
                live: true,
                actor: Some(TaskLedgerActor::User),
            },
            EditMode::CloseOpen { open_id } => TaskLedgerOp::CloseOpenWithCheckpoint {
                open_id: open_id.clone(),
                claim: primary,
                verifier: VerifierRef::UserConfirmation {
                    actor: "user".to_string(),
                },
                coverage: VerificationCoverage { scope: secondary },
                covered_criteria: Vec::new(),
                evidence_artifact_ids: Vec::new(),
                source_stage_id: None,
            },
        };
        self.editor = None;
        Some(TaskStateAction::Apply(op))
    }

    pub fn render(
        &self,
        ctx: &mut RenderContext,
        geom: PromptGeom,
        ledger: Option<&SessionTaskLedger>,
    ) {
        if !self.visible {
            return;
        }
        if let Some(editor) = &self.editor {
            let (title, first_label, second_label) = match editor.mode {
                EditMode::Goal => ("Edit Goal", "Statement", Some("Criteria (; separated)")),
                EditMode::Next => ("Edit Next", "Statement", None),
                EditMode::Core { slot: Some(_) } => ("Replace Core", "Statement", None),
                EditMode::Core { slot: None } => ("Add Core", "Statement", None),
                EditMode::CloseOpen { .. } => {
                    ("Close Open", "Verified claim", Some("Coverage scope"))
                }
            };
            let mut content = vstack().gap(1).child(Text::new(first_label)).child_sized(
                Border::rounded()
                    .fg(if editor.secondary_active {
                        colors::BORDER()
                    } else {
                        colors::ACCENT_CYAN()
                    })
                    .child(editor.primary.clone()),
                3,
            );
            if let Some(label) = second_label {
                content = content.child(Text::new(label)).child_sized(
                    Border::rounded()
                        .fg(if editor.secondary_active {
                            colors::ACCENT_CYAN()
                        } else {
                            colors::BORDER()
                        })
                        .child(editor.secondary.clone()),
                    3,
                );
            }
            backdrop::render_dialog_bottom(
                title,
                colors::ACCENT_CYAN(),
                content,
                "Tab: field · Enter: save · Esc: cancel",
                ctx,
                geom,
                if second_label.is_some() { 12 } else { 8 },
            );
            return;
        }

        let rows = task_state_rows(ledger);
        let items = rows.into_iter().map(|row| row.item).collect::<Vec<_>>();
        let selected = self.selected.min(items.len().saturating_sub(1));
        let visible_rows = items.len().clamp(3, 18);
        backdrop::render_list_dialog_bottom(
            ListDialogHeading {
                title: "Task State",
                border_color: colors::ACCENT_CYAN(),
            },
            &items,
            selected,
            "e: edit · a: add Core · c: close Open · Enter: evidence · Esc: close",
            ctx,
            geom,
            visible_rows,
        );
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn row(display: impl Into<String>, muted: bool, action: Option<RowAction>) -> TaskStateRow {
    TaskStateRow {
        item: ListItem::Row {
            display: display.into(),
            muted,
        },
        action,
    }
}

fn wrapped_rows(
    prefix: &str,
    text: &str,
    muted: bool,
    action: Option<RowAction>,
) -> Vec<TaskStateRow> {
    const WIDTH: usize = 28;
    let chars = text.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return vec![row(prefix, muted, action)];
    }
    chars
        .chunks(WIDTH)
        .enumerate()
        .map(|(index, chunk)| {
            let label = if index == 0 { prefix } else { "  " };
            row(
                format!("{label}{}", chunk.iter().collect::<String>()),
                muted,
                if index == 0 { action.clone() } else { None },
            )
        })
        .collect()
}

fn task_state_rows(ledger: Option<&SessionTaskLedger>) -> Vec<TaskStateRow> {
    let mut rows = Vec::new();
    let Some(ledger) = ledger else {
        return vec![row(
            "No task ledger for this session (governance is opt-in).",
            false,
            None,
        )];
    };
    rows.push(row(
        format!("Status: {:?}   revision {}", ledger.status, ledger.revision),
        true,
        None,
    ));
    if let Some(goal) = &ledger.goal {
        rows.extend(wrapped_rows(
            "Goal: ",
            &format!("{}  [by {:?}]", goal.statement, goal.set_by),
            false,
            Some(RowAction::EditGoal),
        ));
        for criterion in &goal.acceptance_criteria {
            rows.extend(wrapped_rows("  accept: ", criterion, true, None));
        }
    }
    if let Some(next) = &ledger.next {
        rows.extend(wrapped_rows(
            "Next: ",
            &format!(
                "{}  [by {:?}]{}",
                next.statement,
                next.provenance.actor,
                if next.provenance.pre_interrupt {
                    " (pre-interrupt)"
                } else {
                    ""
                }
            ),
            false,
            Some(RowAction::EditNext),
        ));
    }
    for (index, entry) in ledger.core.iter().filter(|entry| entry.live).enumerate() {
        rows.extend(wrapped_rows(
            &format!("{}: ", entry.id),
            &format!("{}  [by {:?}]", entry.statement, entry.set_by),
            true,
            Some(RowAction::EditCore {
                slot: index as u8 + 1,
            }),
        ));
    }
    for question in ledger
        .open
        .iter()
        .filter(|question| question.closed_by_checkpoint_id.is_none())
    {
        rows.extend(wrapped_rows(
            &format!("{}: ", question.id),
            &format!(
                "{} — settled by: {}",
                question.question, question.settled_by
            ),
            false,
            Some(RowAction::CloseOpen {
                open_id: question.id.clone(),
            }),
        ));
    }
    for checkpoint in current_checkpoints(ledger).into_iter().rev().take(6) {
        rows.extend(wrapped_rows(
            &format!("{}: ", checkpoint.id),
            &format!(
                "{} — by {}, scope: {}",
                checkpoint.claim,
                checkpoint.verifier.describe(),
                checkpoint.coverage.scope
            ),
            false,
            None,
        ));
        if let Some(stage_id) = &checkpoint.source_stage_id {
            rows.push(row(
                format!("  evidence stage: {stage_id}"),
                true,
                Some(RowAction::Evidence(stage_id.clone())),
            ));
        }
        for artifact_id in &checkpoint.evidence_artifact_ids {
            rows.extend(wrapped_rows(
                "  evidence: ",
                artifact_id,
                true,
                Some(RowAction::Evidence(artifact_id.clone())),
            ));
        }
    }
    for criterion in &ledger.uncovered_criteria {
        rows.extend(wrapped_rows("! uncovered: ", criterion, false, None));
    }
    for criterion in missing_acceptance_criteria(ledger, &ledger.uncovered_criteria) {
        rows.extend(wrapped_rows(
            "? missing evidence: ",
            &criterion,
            false,
            None,
        ));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_types::task_ledger::{OpenQuestion, TaskLedgerStatus};

    fn ledger() -> SessionTaskLedger {
        let mut ledger = SessionTaskLedger::empty("ses_task");
        ledger.revision = 2;
        ledger.status = TaskLedgerStatus::Active;
        ledger.goal = Some(TaskGoal {
            statement: "ship".to_string(),
            acceptance_criteria: vec!["tested".to_string(), "documented".to_string()],
            criterion_checks: vec![],
            set_by: TaskLedgerActor::Model,
            set_at: 1,
        });
        ledger.next = Some(agendao_types::task_ledger::NextAction {
            statement: "test".to_string(),
            provenance: agendao_types::task_ledger::NextActionProvenance {
                actor: TaskLedgerActor::Model,
                pre_interrupt: false,
                set_at: 1,
            },
        });
        ledger
    }

    fn displays(rows: &[TaskStateRow]) -> Vec<&str> {
        rows.iter()
            .filter_map(|row| match &row.item {
                ListItem::Row { display, .. } => Some(display.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn distinguishes_declared_uncovered_from_missing_evidence() {
        let mut ledger = ledger();
        ledger.status = TaskLedgerStatus::Completed;
        ledger.uncovered_criteria = vec!["documented".to_string()];
        let rows = task_state_rows(Some(&ledger));
        let displays = displays(&rows);
        assert!(displays
            .iter()
            .any(|value| value.contains("! uncovered: documented")));
        assert!(displays
            .iter()
            .any(|value| value.contains("missing evidence: tested")));
        assert!(!displays
            .iter()
            .any(|value| value.contains("missing evidence: documented")));
    }

    #[test]
    fn editor_emits_user_provenance_and_close_checkpoint() {
        let mut ledger = ledger();
        ledger.open.push(OpenQuestion {
            id: "open-01".to_string(),
            question: "reviewed?".to_string(),
            settled_by: "manual review".to_string(),
            opened_at: 1,
            closed_by_checkpoint_id: None,
        });
        let mut dialog = TaskStateDialog::new();
        dialog.open();
        dialog.open_close_editor("open-01".to_string());
        let editor = dialog.editor.as_mut().unwrap();
        editor.primary = revue::widget::Input::new().value("reviewed");
        editor.secondary = revue::widget::Input::new().value("desktop");
        let action = dialog
            .handle_key(&revue::event::Key::Enter, Some(&ledger))
            .expect("submit action");
        assert!(matches!(
            action,
            TaskStateAction::Apply(TaskLedgerOp::CloseOpenWithCheckpoint {
                open_id,
                verifier: VerifierRef::UserConfirmation { .. },
                ..
            }) if open_id == "open-01"
        ));
    }

    #[test]
    fn evidence_row_returns_navigation_action() {
        let mut ledger = ledger();
        ledger
            .verified
            .push(agendao_types::task_ledger::VerifiedCheckpoint {
                id: "chk-01".to_string(),
                claim: "checked".to_string(),
                verifier: VerifierRef::UserConfirmation {
                    actor: "user".to_string(),
                },
                coverage: VerificationCoverage {
                    scope: "stage".to_string(),
                },
                goal_generation: 0,
                evidence_artifact_ids: vec![],
                source_stage_id: Some("stage-1".to_string()),
                covered_criteria: vec![],
                supersedes: None,
                superseded_by: None,
                created_at: 1,
            });
        let rows = task_state_rows(Some(&ledger));
        let evidence_index = rows
            .iter()
            .position(|row| matches!(row.action, Some(RowAction::Evidence(_))))
            .unwrap();
        let mut dialog = TaskStateDialog::new();
        dialog.open();
        dialog.selected = evidence_index;
        assert_eq!(
            dialog.handle_key(&revue::event::Key::Enter, Some(&ledger)),
            Some(TaskStateAction::NavigateEvidence("stage-1".to_string()))
        );
    }
}
