//! 土 — Task governance state dialog (Phase 5 TUI panel).
//!
//! Read-only overlay over the session task ledger: Goal, current Next,
//! live Core, Open questions, current-generation Verified checkpoints, and
//! uncovered acceptance criteria. Typed fields only — no inferred state,
//! no hidden reasoning. Toggle: Ctrl+T; Esc/q closes.

use crate::dialog::backdrop::{self, ListDialogHeading, ListItem, PromptGeom};
use crate::theme::colors;
use agendao_types::task_ledger::{
    current_checkpoints, missing_acceptance_criteria, SessionTaskLedger,
};
use revue::prelude::*;

pub struct TaskStateDialog {
    pub visible: bool,
    selected: usize,
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
        }
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.selected = 0;
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
    }

    pub fn handle_key(&mut self, key: &revue::event::Key, item_count: usize) {
        match key {
            revue::event::Key::Escape
            | revue::event::Key::Char('q')
            | revue::event::Key::Char(' ') => self.dismiss(),
            revue::event::Key::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            revue::event::Key::Down => {
                self.selected = (self.selected + 1).min(item_count.saturating_sub(1));
            }
            revue::event::Key::Home => self.selected = 0,
            revue::event::Key::End => self.selected = item_count.saturating_sub(1),
            _ => {}
        }
    }

    pub fn item_count(&self, ledger: Option<&SessionTaskLedger>) -> usize {
        task_state_items(ledger).len()
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
        let items = task_state_items(ledger);
        let selected = self.selected.min(items.len().saturating_sub(1));
        let rows = items.len().clamp(3, 16);
        backdrop::render_list_dialog_bottom(
            ListDialogHeading {
                title: "Task State",
                border_color: colors::ACCENT_CYAN(),
            },
            &items,
            selected,
            "↑/↓ navigate · Home/End · Esc/q close",
            ctx,
            geom,
            rows,
        );
    }
}

fn task_state_items(ledger: Option<&SessionTaskLedger>) -> Vec<ListItem> {
    let mut items: Vec<ListItem> = Vec::new();
    match ledger {
        None => {
            items.push(ListItem::Row {
                display: "No task ledger for this session (governance is opt-in).".to_string(),
                muted: false,
            });
        }
        Some(ledger) => {
            items.push(ListItem::Row {
                display: format!("Status: {:?}   revision {}", ledger.status, ledger.revision),
                muted: true,
            });
            if let Some(goal) = &ledger.goal {
                items.push(ListItem::Row {
                    display: format!("Goal: {}", goal.statement),
                    muted: false,
                });
                for criterion in &goal.acceptance_criteria {
                    items.push(ListItem::Row {
                        display: format!("  accept: {criterion}"),
                        muted: true,
                    });
                }
            }
            if let Some(next) = &ledger.next {
                items.push(ListItem::Row {
                    display: format!(
                        "Next: {}{}",
                        next.statement,
                        if next.provenance.pre_interrupt {
                            "  (pre-interrupt)"
                        } else {
                            ""
                        }
                    ),
                    muted: false,
                });
            }
            for entry in ledger.core.iter().filter(|entry| entry.live) {
                items.push(ListItem::Row {
                    display: format!("Core(live): {}", entry.statement),
                    muted: true,
                });
            }
            let open: Vec<_> = ledger
                .open
                .iter()
                .filter(|question| question.closed_by_checkpoint_id.is_none())
                .collect();
            if !open.is_empty() {
                items.push(ListItem::Row {
                    display: "Open:".to_string(),
                    muted: true,
                });
                for question in open.iter().take(6) {
                    items.push(ListItem::Row {
                        display: format!(
                            "  {} {} — settled by: {}",
                            question.id, question.question, question.settled_by
                        ),
                        muted: false,
                    });
                }
            }
            let checkpoints = current_checkpoints(ledger);
            if !checkpoints.is_empty() {
                items.push(ListItem::Row {
                    display: "Verified (current generation):".to_string(),
                    muted: true,
                });
                for checkpoint in checkpoints.iter().rev().take(6) {
                    let criteria = if checkpoint.covered_criteria.is_empty() {
                        "no named criteria".to_string()
                    } else {
                        checkpoint.covered_criteria.join(" | ")
                    };
                    items.push(ListItem::Row {
                        display: format!(
                            "  {} {} — by {}, scope: {}; criteria: {}",
                            checkpoint.id,
                            checkpoint.claim,
                            checkpoint.verifier.describe(),
                            checkpoint.coverage.scope,
                            criteria,
                        ),
                        muted: false,
                    });
                }
            }
            if !ledger.uncovered_criteria.is_empty() {
                items.push(ListItem::Row {
                    display: "Explicitly uncovered at completion:".to_string(),
                    muted: true,
                });
                for criterion in &ledger.uncovered_criteria {
                    items.push(ListItem::Row {
                        display: format!("  ! {criterion}"),
                        muted: false,
                    });
                }
            }
            for criterion in missing_acceptance_criteria(ledger, &ledger.uncovered_criteria) {
                items.push(ListItem::Row {
                    display: format!("? missing evidence: {criterion}"),
                    muted: false,
                });
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_types::task_ledger::TaskLedgerStatus;

    fn displays(items: &[ListItem]) -> Vec<&str> {
        items
            .iter()
            .filter_map(|item| match item {
                ListItem::Row { display, .. } => Some(display.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn distinguishes_declared_uncovered_from_missing_evidence() {
        let mut ledger = SessionTaskLedger::empty("ses_task");
        ledger.revision = 2;
        ledger.status = TaskLedgerStatus::Completed;
        ledger.goal = Some(agendao_types::task_ledger::TaskGoal {
            statement: "ship".to_string(),
            acceptance_criteria: vec!["tested".to_string(), "documented".to_string()],
            criterion_checks: vec![],
            set_by: agendao_types::task_ledger::TaskLedgerActor::User,
            set_at: 1,
        });
        ledger.uncovered_criteria = vec!["documented".to_string()];
        let items = task_state_items(Some(&ledger));
        let rows = displays(&items);
        assert!(rows.iter().any(|row| row.contains("! documented")));
        assert!(rows
            .iter()
            .any(|row| row.contains("missing evidence: tested")));
        assert!(!rows
            .iter()
            .any(|row| row.contains("missing evidence: documented")));
    }

    #[test]
    fn navigation_reaches_long_panel_tail() {
        let mut dialog = TaskStateDialog::new();
        dialog.open();
        dialog.handle_key(&revue::event::Key::End, 24);
        assert_eq!(dialog.selected, 23);
        dialog.handle_key(&revue::event::Key::Up, 24);
        assert_eq!(dialog.selected, 22);
        dialog.handle_key(&revue::event::Key::Home, 24);
        assert_eq!(dialog.selected, 0);
    }
}
