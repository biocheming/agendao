//! 金 — 通知中心（U7③）。
//!
//! 回看 `AppStore.toast_history`（最近 50 条，含已过期/已 dismiss 的——
//! 历史的意义就是回看「错过的」）。**只读视图**：数据真相留在 store
//! signal（土律·单一权威），dialog 只持有 visible + selected，不复制
//! 条目；render/handle_key 以 `&[ToastMsg]` 参数读历史。
//!
//! 入口：`/notifications` slash + status bar 🔔 角标点击。Esc 关闭。

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};
use crate::store::types::{ToastMsg, ToastMsgVariant};

pub struct NotificationDialog {
    pub visible: bool,
    selected: usize,
}

impl Default for NotificationDialog {
    fn default() -> Self {
        Self::new()
    }
}

/// 相对时刻（"3s ago"/"2m ago"/"1h ago"/"2d ago"）——历史列表行首时间戳。
fn fmt_age(created_at: u64, now_ms: u64) -> String {
    let secs = now_ms.saturating_sub(created_at) / 1000;
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

impl NotificationDialog {
    pub fn new() -> Self {
        Self { visible: false, selected: 0 }
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.selected = 0;
    }
    pub fn close(&mut self) {
        self.visible = false;
    }
    pub fn is_open(&self) -> bool {
        self.visible
    }

    /// 只读导航：↑/↓/Home/End 移动、Esc 关闭。无条目级动作（通知是
    /// 已发生事件的记录，无可执行语义——道纪第十条：不伪可操作）。
    /// 返回 true = 键被消费。
    pub fn handle_key(&mut self, key: &Key, entry_count: usize) -> bool {
        if !self.visible {
            return false;
        }
        if entry_count == 0 {
            if matches!(key, Key::Escape) {
                self.close();
            }
            return true;
        }
        let len = entry_count;
        match key {
            Key::Up => self.selected = (self.selected + len - 1) % len,
            Key::Down => self.selected = (self.selected + 1) % len,
            Key::Home => self.selected = 0,
            Key::End => self.selected = len - 1,
            Key::Escape => self.close(),
            _ => {}
        }
        true
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom, history: &[ToastMsg]) {
        if !self.visible {
            return;
        }
        if history.is_empty() {
            let items = vec![ListItem::Row {
                display: "  (No notifications yet)".to_string(),
                muted: true,
            }];
            backdrop::render_list_dialog_bottom(
                "Notifications",
                colors::ACCENT_CYAN(),
                &items,
                0,
                "Esc: close",
                ctx, geom, 3,
            );
            return;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        // 最新在上（回看场景：最近的错过最相关）。selected 以该序索引，
        // handle_key 的 entry_count 同源（同一 history 切片）。
        let items: Vec<ListItem> = history
            .iter()
            .rev()
            .enumerate()
            .map(|(i, t)| {
                let icon = match t.variant {
                    ToastMsgVariant::Success => "✓",
                    ToastMsgVariant::Error => "✕",
                    ToastMsgVariant::Warning => "⚠",
                    ToastMsgVariant::Info => "•",
                };
                let marker = if i == self.selected { "▶ " } else { "  " };
                ListItem::Row {
                    display: format!(
                        "{}{} {}  {}",
                        marker,
                        icon,
                        t.text,
                        fmt_age(t.created_at, now_ms)
                    ),
                    muted: false,
                }
            })
            .collect();
        backdrop::render_list_dialog_bottom(
            "Notifications",
            colors::ACCENT_CYAN(),
            &items,
            self.selected,
            "↑↓ navigate  Esc: close",
            ctx, geom, 12,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_age_buckets() {
        let now = 1_000_000_000u64;
        assert_eq!(fmt_age(now - 3_000, now), "3s ago");
        assert_eq!(fmt_age(now - 120_000, now), "2m ago");
        assert_eq!(fmt_age(now - 3_600_000, now), "1h ago");
        assert_eq!(fmt_age(now - 172_800_000, now), "2d ago");
        // 时钟回拨/同刻不 panic（saturating_sub）。
        assert_eq!(fmt_age(now + 5_000, now), "0s ago");
    }

    #[test]
    fn navigation_wraps_and_esc_closes() {
        let mut d = NotificationDialog::new();
        d.open();
        assert!(d.handle_key(&Key::Up, 3));
        assert_eq!(d.selected, 2, "↑ 从 0 回绕到末条");
        assert!(d.handle_key(&Key::Down, 3));
        assert_eq!(d.selected, 0);
        assert!(d.handle_key(&Key::Escape, 3));
        assert!(!d.is_open(), "Esc 关闭");
    }

    #[test]
    fn empty_history_consumes_but_only_esc_acts() {
        let mut d = NotificationDialog::new();
        d.open();
        assert!(d.handle_key(&Key::Up, 0), "空态键被消费不越界");
        assert_eq!(d.selected, 0);
        assert!(d.handle_key(&Key::Escape, 0));
        assert!(!d.is_open());
    }
}
