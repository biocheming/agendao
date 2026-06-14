//! 金 — Session list dialog: browse and switch sessions.

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

#[derive(Clone, Debug)]
pub struct SessionEntry {
    pub id: String,
    pub title: String,
    pub status_hint: String,
}

pub struct SessionListDialog {
    pub visible: bool,
    pub sessions: Vec<SessionEntry>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    /// Live search query — type to narrow the visible list. Matches
    /// either the title or the session id (case-insensitive substring).
    pub query: String,
}

impl SessionListDialog {
    pub fn new() -> Self {
        Self { visible: false, sessions: vec![], selected: 0, loading: false, error: None, query: String::new() }
    }

    pub fn open(&mut self) { self.visible = true; self.selected = 0; self.query.clear(); }

    pub fn close(&mut self) {
        self.visible = false;
        self.sessions.clear();
        self.error = None;
        self.loading = false;
        self.query.clear();
    }

    pub fn is_open(&self) -> bool { self.visible }

    pub fn set_sessions(&mut self, sessions: Vec<SessionEntry>) {
        self.sessions = sessions;
        self.loading = false;
        self.error = None;
        self.selected = 0;
    }

    pub fn set_error(&mut self, err: String) {
        self.error = Some(err);
        self.loading = false;
        self.sessions.clear();
    }

    /// Return the currently filtered session list (indexes into self.sessions).
    fn filtered_indices(&self) -> Vec<usize> {
        let q = self.query.to_lowercase();
        if q.is_empty() {
            return (0..self.sessions.len()).collect();
        }
        self.sessions.iter().enumerate()
            .filter(|(_, s)| s.title.to_lowercase().contains(&q) || s.id.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn handle_key(&mut self, key: &Key) -> Option<SessionEntry> {
        if !self.visible { return None; }
        match key {
            Key::Up => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            Key::Down => {
                let max = self.filtered_indices().len().saturating_sub(1);
                self.selected = (self.selected + 1).min(max);
                None
            }
            Key::Enter => {
                let filtered = self.filtered_indices();
                let s = filtered.get(self.selected)
                    .and_then(|&i| self.sessions.get(i))
                    .cloned();
                self.close();
                s
            }
            Key::Escape => { self.close(); None }
            Key::Backspace => {
                if self.query.pop().is_some() { self.selected = 0; }
                None
            }
            // Allow alphanumeric + space + dash/underscore/dot for filtering
            Key::Char(c) if c.is_ascii_graphic() || *c == ' ' => {
                self.query.push(*c);
                self.selected = 0;
                None
            }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }

        if self.loading {
            let content = vstack().child(Text::new("Loading sessions...").fg(colors::FG_MUTED));
            backdrop::render_dialog("Sessions", colors::ACCENT_CYAN, content,
                "Loading...", ctx, 70, 5);
        } else if let Some(ref err) = self.error {
            let content = vstack().child(Text::new(&format!("Error: {}", err)).fg(colors::ACCENT_RED));
            backdrop::render_dialog("Sessions", colors::ACCENT_RED, content,
                "Esc: close", ctx, 70, 5);
        } else if self.sessions.is_empty() {
            let content = vstack().child(Text::new("No sessions found.").fg(colors::FG_MUTED));
            backdrop::render_dialog("Sessions", colors::ACCENT_CYAN, content,
                "Esc: close", ctx, 70, 5);
        } else {
            let filtered = self.filtered_indices();
            let items: Vec<ListItem> = filtered.iter().map(|&i| {
                let s = &self.sessions[i];
                let status = if s.status_hint.is_empty() { String::new() } else { format!(" [{}]", s.status_hint) };
                ListItem::Row {
                    display: format!("{}{}", s.title, status),
                    muted: false,
                }
            }).collect();
            let title = if self.query.is_empty() {
                "Sessions".to_string()
            } else {
                format!("Sessions — query: {}", self.query)
            };
            backdrop::render_list_dialog(
                &title,
                colors::ACCENT_CYAN,
                &items,
                self.selected,
                "type to filter  ↑↓ navigate  Enter: open  Esc: close",
                ctx, 80, 18,
            );
        }
    }
}
