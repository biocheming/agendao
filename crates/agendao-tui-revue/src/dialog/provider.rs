//! 金 — Provider management dialog: list, connect, set auth.

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

#[derive(Clone)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub connected: bool,
}

pub struct ProviderDialog {
    pub visible: bool,
    pub providers: Vec<ProviderInfo>,
    pub selected: usize,
    pub editing_provider: Option<String>,
    pub edit_input: revue::widget::Input,
    pub edit_mode: EditMode,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditMode {
    None,
    ApiKey(String),
    CustomUrl(String),
}

impl ProviderDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            providers: vec![],
            selected: 0,
            editing_provider: None,
            edit_input: revue::widget::Input::new().placeholder("Enter API key..."),
            edit_mode: EditMode::None,
        }
    }

    pub fn set_providers(&mut self, providers: Vec<ProviderInfo>) {
        self.providers = providers;
        self.selected = 0;
    }

    pub fn open(&mut self) { self.visible = true; self.edit_mode = EditMode::None; }
    pub fn close(&mut self) { self.visible = false; self.edit_mode = EditMode::None; }
    pub fn is_open(&self) -> bool { self.visible }

    pub fn handle_key(&mut self, key: &Key) -> Option<ProviderAction> {
        if !self.visible { return None; }

        if self.edit_mode != EditMode::None {
            match key {
                Key::Enter => {
                    let value = self.edit_input.text().trim().to_string();
                    let action = match self.edit_mode.clone() {
                        EditMode::ApiKey(pid) => Some(ProviderAction::SetAuth(pid, value)),
                        EditMode::CustomUrl(pid) => Some(ProviderAction::RegisterCustom(pid, value)),
                        EditMode::None => None,
                    };
                    self.edit_mode = EditMode::None;
                    self.edit_input.clear();
                    return action;
                }
                Key::Escape => {
                    self.edit_mode = EditMode::None;
                    self.edit_input.clear();
                    return None;
                }
                _ => { self.edit_input.handle_key(key); return None; }
            }
        }

        match key {
            Key::Escape => { self.close(); None }
            Key::Up => { self.selected = self.selected.saturating_sub(1); None }
            Key::Down => {
                let max = self.providers.len().saturating_sub(1);
                self.selected = (self.selected + 1).min(max);
                None
            }
            Key::Enter => {
                if let Some(p) = self.providers.get(self.selected) {
                    if p.connected {
                        return Some(ProviderAction::Toggle(p.id.clone()));
                    } else {
                        self.edit_mode = EditMode::ApiKey(p.id.clone());
                        self.edit_input = revue::widget::Input::new()
                            .placeholder(format!("API key for {}...", p.name));
                        return None;
                    }
                }
                None
            }
            Key::Char('r') => {
                self.edit_mode = EditMode::CustomUrl(String::new());
                self.edit_input = revue::widget::Input::new()
                    .placeholder("https://custom-provider.com/api");
                None
            }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }

        if self.edit_mode != EditMode::None {
            let hint = match self.edit_mode {
                EditMode::ApiKey(_) => "Enter API key",
                EditMode::CustomUrl(_) => "Enter base URL",
                EditMode::None => "",
            };
            let content = vstack().gap(1)
                .child(Text::new(hint).fg(colors::FG_MUTED))
                .child(Border::rounded().fg(colors::BORDER).child(self.edit_input.clone()));
            backdrop::render_dialog("Provider Manager", colors::ACCENT_CYAN, content,
                "Enter: confirm  Esc: cancel", ctx, 54, 6);
        } else {
            if self.providers.is_empty() {
                let content = vstack().child(Text::new("No providers configured.").fg(colors::FG_MUTED));
                backdrop::render_dialog("Provider Manager", colors::ACCENT_CYAN, content,
                    "Esc: close", ctx, 54, 5);
                return;
            }
            let items: Vec<ListItem> = self.providers.iter().enumerate().take(12).map(|(i, p)| {
                let marker = if i == self.selected { "▶ " } else { "  " };
                let status = if p.connected { "●" } else { "○" };
                ListItem::Row {
                    display: format!("{}{}  {}", marker, status, p.name),
                    muted: !p.connected,
                }
            }).collect();
            backdrop::render_list_dialog(
                "Provider Manager",
                colors::ACCENT_CYAN,
                &items,
                self.selected,
                "Enter: set key  r: custom  Esc: close",
                ctx, 54, 12,
            );
        }
    }
}

#[derive(Clone, Debug)]
pub enum ProviderAction {
    Toggle(String),
    SetAuth(String, String),
    RegisterCustom(String, String),
}
