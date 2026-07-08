//! 金 — Prompt stash: save/load prompt drafts. 持久化到
//! `<data_dir>/agendao/prompt-stash.json`，启动载入、push/delete 落盘
//! （水律：回流落盘，下一轮启动可复用）。

use revue::prelude::*;
use revue::event::Key;
use serde::{Serialize, Deserialize};
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

#[derive(Clone, Serialize, Deserialize)]
pub struct StashEntry {
    pub text: String,
    pub created_at: i64,
}

/// Stash 持久化路径：与 prompt-history.json 同目录（data_dir/agendao）。
fn default_stash_path() -> std::path::PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("agendao").join("prompt-stash.json")
}

/// 启动期载入 stash（文件缺失/损坏时返回空，不阻断启动）。
pub fn load_stash() -> Vec<StashEntry> {
    std::fs::read_to_string(default_stash_path()).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 落盘 stash（0o600 权限，与 prompt-history.json 同口径）。
pub fn save_stash(entries: &[StashEntry]) {
    let path = default_stash_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(entries) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .write(true).create(true).truncate(true)
                .mode(0o600)
                .open(&path)
            {
                let _ = f.write_all(json.as_bytes());
            }
        }
        #[cfg(not(unix))]
        {
            let _ = std::fs::write(&path, &json);
        }
    }
}

pub struct StashDialog {
    pub visible: bool,
    pub entries: Vec<StashEntry>,
    pub selected: usize,
}

impl StashDialog {
    pub fn new() -> Self {
        Self { visible: false, entries: vec![], selected: 0 }
    }

    pub fn open(&mut self) { self.visible = true; self.selected = 0; }
    pub fn close(&mut self) { self.visible = false; }
    pub fn is_open(&self) -> bool { self.visible }

    pub fn set_entries(&mut self, entries: Vec<StashEntry>) {
        self.entries = entries;
        self.selected = 0;
    }

    /// 当前条目（供 dispatcher 在 delete 后同步回权威 self.stash_entries）。
    pub fn entries(&self) -> &[StashEntry] { &self.entries }

    pub fn handle_key(&mut self, key: &Key) -> Option<String> {
        if !self.visible { return None; }
        match key {
            Key::Escape => { self.close(); None }
            Key::Up => { self.selected = self.selected.saturating_sub(1); None }
            Key::Down => {
                let max = self.entries.len().saturating_sub(1);
                self.selected = (self.selected + 1).min(max);
                None
            }
            Key::Enter => {
                let text = self.entries.get(self.selected).map(|e| e.text.clone());
                self.close();
                text
            }
            Key::Delete | Key::Char('d') => {
                if self.selected < self.entries.len() {
                    self.entries.remove(self.selected);
                    if self.selected >= self.entries.len().saturating_sub(1) {
                        self.selected = self.entries.len().saturating_sub(1);
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom) {
        if !self.visible { return; }

        if self.entries.is_empty() {
            let content = vstack().child(Text::new("(empty stash)").fg(colors::FG_MUTED));
            backdrop::render_dialog_bottom("Prompt Stash", colors::ACCENT_PURPLE, content,
                "Esc: close", ctx, geom, 5);
            return;
        }

        // backdrop sliding viewport 自动接管;此处不再 .take(N)(否则选中超出 N 视野不跟随)。
        let items: Vec<ListItem> = self.entries.iter().enumerate().map(|(i, entry)| {
            let preview: String = entry.text.chars().take(60).collect();
            let marker = if i == self.selected { "▶ " } else { "  " };
            ListItem::Row {
                display: format!("{}{}", marker, preview),
                muted: false,
            }
        }).collect();

        backdrop::render_list_dialog_bottom(
            "Prompt Stash",
            colors::ACCENT_PURPLE,
            &items,
            self.selected,
            "↑↓ navigate  Enter: restore  d: delete  Esc: close",
            ctx, geom, 10,
        );
    }
}
