//! 金 — Model selection dialog: Provider → Model → Variant.
//!
//! Uses shared dialog backdrop for consistent visual identity.

use crate::dialog::backdrop::{self, ListItem};
use crate::theme::colors;
use revue::event::Key;
use revue::prelude::*;

#[derive(Clone)]
pub struct ModelEntry {
    /// Provider registry id (e.g. "aihubmix") — what the server uses to
    /// resolve `provider_id/model_id` in PromptRequest.
    pub provider: String,
    /// Human-friendly provider label (e.g. "AIHubMix") used for the group
    /// header. Falls back to `provider` when not set, so older callers
    /// constructing `ModelEntry { ... }` literally still compile.
    pub provider_display: String,
    pub model_id: String,
    pub display: String,
    pub variants: Vec<String>,
    pub available: bool,
}

/// Result of `ModelSelectDialog::handle_key`.
///
/// Using a 3-arm enum (instead of `Option<ModelEntry>`) lets the dialog
/// surface "your Enter didn't work because…" reasons without a side
/// channel — the host then routes it to a toast.
pub enum ModelDialogOutcome {
    /// Dialog is still consuming keys (navigation, filtering, etc.).
    None,
    /// User pressed Enter but the selection was invalid (header row,
    /// model unavailable in the current runtime). Caller should show this
    /// string as a toast.
    Notice(String),
    /// User picked a usable model; dialog has already closed.
    Selected(ModelEntry),
}

#[derive(Clone)]
pub struct ProviderGroup {
    /// Provider 注册表 id（server 解析 `provider_id/model_id` 用）。
    pub provider_id: String,
    /// Human-friendly provider label（e.g. "AIHubMix"）用作组头。
    pub name: String,
    pub models: Vec<ModelEntry>,
}

pub struct ModelSelectDialog {
    pub visible: bool,
    groups: Vec<ProviderGroup>,
    flat: Vec<FlatRow>,
    /// All flat rows that match the current query (or all rows when empty).
    /// `selected` indexes into `flat` after filtering is rebuilt.
    selected: usize,
    variant_idx: usize,
    recent: Vec<(String, String)>,
    /// Effective model for the active session, rendered independently from
    /// the recent-model history.
    current: Option<String>,
    /// Live search query — type to filter, Backspace to delete.
    query: String,
}

#[derive(Clone)]
enum FlatRow {
    Header(String),
    Model(usize, usize),
} // group_idx, model_idx

impl Default for ModelSelectDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelSelectDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            groups: vec![],
            flat: vec![],
            selected: 0,
            variant_idx: 0,
            recent: vec![],
            current: None,
            query: String::new(),
        }
    }

    pub fn set_models(&mut self, models: Vec<ModelEntry>) {
        self.groups.clear();
        // Group by provider id, but capture each group's display label
        // from the first ModelEntry we see. Without this, the group header
        // shows "aihubmix" instead of "AIHubMix" — visually it looked like
        // a regression after we switched provider field from name → id.
        let mut providers: std::collections::BTreeMap<String, (String, Vec<ModelEntry>)> =
            std::collections::BTreeMap::new();
        for m in models {
            let key = m.provider.clone();
            let display = if m.provider_display.is_empty() {
                m.provider.clone()
            } else {
                m.provider_display.clone()
            };
            providers
                .entry(key)
                .or_insert_with(|| (display, Vec::new()))
                .1
                .push(m);
        }
        for (id, (display, models)) in providers {
            self.groups.push(ProviderGroup {
                provider_id: id,
                name: display,
                models,
            });
        }
        self.rebuild_flat();
    }

    pub fn set_recent(&mut self, recent: Vec<(String, String)>) {
        self.recent = recent;
        self.rebuild_flat();
    }

    pub fn set_current(&mut self, current: Option<String>) {
        self.current = current.filter(|value| !value.trim().is_empty());
        self.rebuild_flat();
    }

    fn is_current(&self, model: &ModelEntry) -> bool {
        self.current.as_deref().is_some_and(|current| {
            current == format!("{}/{}", model.provider, model.model_id)
                || (!current.contains('/') && current == model.model_id)
        })
    }

    /// 记录一次模型选择：置顶、按 `(provider, model)` 去重、cap 到 8。
    /// 返回新列表供调用方 `put_recent_models` 持久化（选中即回写权威）。
    pub fn record_recent(&mut self, provider: &str, model: &str) -> Vec<(String, String)> {
        self.recent.retain(|(p, m)| p != provider || m != model);
        self.recent
            .insert(0, (provider.to_string(), model.to_string()));
        self.recent.truncate(8);
        self.rebuild_flat();
        self.recent.clone()
    }

    fn rebuild_flat(&mut self) {
        self.flat.clear();
        let q = self.query.to_lowercase();
        let matches = |provider: &str, m: &ModelEntry| -> bool {
            if q.is_empty() {
                return true;
            }
            // Case-insensitive substring match against provider, model id, and display name
            provider.to_lowercase().contains(&q)
                || m.model_id.to_lowercase().contains(&q)
                || m.display.to_lowercase().contains(&q)
        };

        if q.is_empty() && !self.recent.is_empty() {
            self.flat.push(FlatRow::Header("★ Recent".into()));
            for (provider, model_id) in &self.recent {
                if let Some((gi, mi)) = self.find_model(provider, model_id) {
                    self.flat.push(FlatRow::Model(gi, mi));
                }
            }
        }
        for (gi, group) in self.groups.iter().enumerate() {
            // Pre-compute matching models for this group; skip the header
            // when nothing matches so the user sees a tight result list.
            let mut matched: Vec<usize> = Vec::new();
            for (mi, m) in group.models.iter().enumerate() {
                if matches(&group.name, m) {
                    matched.push(mi);
                }
            }
            if matched.is_empty() {
                continue;
            }
            self.flat.push(FlatRow::Header(format!("▸ {}", group.name)));
            for mi in matched {
                self.flat.push(FlatRow::Model(gi, mi));
            }
        }
        // Reset selection to first selectable row (skip headers)
        self.selected = self
            .flat
            .iter()
            .position(|row| match row {
                FlatRow::Model(gi, mi) => self
                    .groups
                    .get(*gi)
                    .and_then(|group| group.models.get(*mi))
                    .is_some_and(|model| self.is_current(model)),
                FlatRow::Header(_) => false,
            })
            .or_else(|| {
                self.flat
                    .iter()
                    .position(|r| matches!(r, FlatRow::Model(_, _)))
            })
            .unwrap_or(0);
    }

    fn find_model(&self, provider: &str, model_id: &str) -> Option<(usize, usize)> {
        for (gi, g) in self.groups.iter().enumerate() {
            // ★ Recent 行存的是 provider id；`g.name` 是 display label
            //（e.g. "AIHubMix"）——只比 name 会让 recent 行永远匹配不上、
            // 区块形同虚设。优先按 provider_id，兜底按 display 名（老数据）。
            if g.provider_id == provider || g.name == provider {
                for (mi, m) in g.models.iter().enumerate() {
                    if m.model_id == model_id {
                        return Some((gi, mi));
                    }
                }
            }
        }
        None
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.selected = 0;
        self.query.clear();
        self.rebuild_flat();
    }
    pub fn close(&mut self) {
        self.visible = false;
        self.query.clear();
    }
    pub fn is_open(&self) -> bool {
        self.visible
    }

    /// Outcome of a key press on the model dialog.
    /// `Selected` carries the chosen model and closes the dialog.
    /// `Notice` is a soft signal to the host so it can surface a toast
    /// (e.g. "provider is not connected") without abusing exception flow.
    /// `None` means the key was navigation/filter and the dialog stays open.
    pub fn handle_key(&mut self, key: &Key) -> ModelDialogOutcome {
        if !self.visible {
            return ModelDialogOutcome::None;
        }
        match key {
            Key::Up => {
                // Skip headers when navigating up.
                let mut i = self.selected;
                while i > 0 {
                    i -= 1;
                    if matches!(self.flat.get(i), Some(FlatRow::Model(_, _))) {
                        self.selected = i;
                        break;
                    }
                }
                ModelDialogOutcome::None
            }
            Key::Down => {
                let mut i = self.selected;
                let max = self.flat.len();
                while i + 1 < max {
                    i += 1;
                    if matches!(self.flat.get(i), Some(FlatRow::Model(_, _))) {
                        self.selected = i;
                        break;
                    }
                }
                ModelDialogOutcome::None
            }
            Key::Tab => {
                if let Some(model) = self.selected_model() {
                    if !model.variants.is_empty() {
                        self.variant_idx = (self.variant_idx + 1) % model.variants.len();
                    }
                }
                ModelDialogOutcome::None
            }
            Key::Enter => {
                // The current selection might be a group header (no
                // selectable model) or a muted/unavailable row whose
                // provider isn't connected. Give the host a reason to
                // show via toast instead of silently swallowing the key.
                let Some(m) = self.selected_model().cloned() else {
                    return ModelDialogOutcome::Notice(
                        "Move to a model row before pressing Enter.".to_string(),
                    );
                };
                if !m.available {
                    return ModelDialogOutcome::Notice(format!(
                        "Model '{}' is listed in the catalogue but is not registered in the current '{}' runtime. Add it in Settings or choose an available model.",
                        m.model_id, m.provider_display,
                    ));
                }
                self.close();
                ModelDialogOutcome::Selected(m)
            }
            Key::Escape => {
                self.close();
                ModelDialogOutcome::None
            }
            // Live filter — type characters to narrow, Backspace to delete.
            Key::Backspace => {
                if self.query.pop().is_some() {
                    self.rebuild_flat();
                }
                ModelDialogOutcome::None
            }
            Key::Char(c) if c.is_ascii_graphic() && *c != ' ' => {
                self.query.push(*c);
                self.rebuild_flat();
                ModelDialogOutcome::None
            }
            Key::Char(' ') => {
                self.query.push(' ');
                self.rebuild_flat();
                ModelDialogOutcome::None
            }
            _ => ModelDialogOutcome::None,
        }
    }

    fn selected_model(&self) -> Option<&ModelEntry> {
        match self.flat.get(self.selected) {
            Some(FlatRow::Model(gi, mi)) => self.groups.get(*gi)?.models.get(*mi),
            _ => None,
        }
    }

    /// 粘贴 → 追加到实时过滤 query（剥离控制字符；' ' 与 Key::Space 输入同效）。
    pub fn paste_query(&mut self, text: &str) -> bool {
        if !self.visible {
            return false;
        }
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        if !clean.is_empty() {
            self.query.push_str(&clean);
            self.selected = 0;
        }
        true
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom) {
        if !self.visible {
            return;
        }

        // U16：无 provider/无模型 → 空态明示 + 下一步指引（原渲染零行
        // 空框 + 过滤 hint，用户无从下手的死端）。
        if self.groups.is_empty() {
            let items = vec![ListItem::Row {
                display: "  (No models configured — add a provider in Settings)".to_string(),
                muted: true,
            }];
            backdrop::render_list_dialog_bottom(
                backdrop::ListDialogHeading {
                    title: "Select Model",
                    border_color: colors::ACCENT_CYAN(),
                },
                &items,
                0,
                "Esc: close",
                ctx,
                geom,
                3,
            );
            return;
        }

        // Build all items (no truncation — backdrop scrolls). Without query
        // filtering, 5,140 models exhaust the user's patience; once we add
        // a search box this becomes `flat.iter().filter(matches_query)`.
        let mut items: Vec<ListItem> = self
            .flat
            .iter()
            .enumerate()
            .map(|(i, row)| match row {
                FlatRow::Header(label) => ListItem::Header(label.clone()),
                FlatRow::Model(gi, mi) => {
                    let model = &self.groups[*gi].models[*mi];
                    let variant = if !model.variants.is_empty() && i == self.selected {
                        format!(
                            " [{}]",
                            model.variants[self.variant_idx % model.variants.len()]
                        )
                    } else {
                        String::new()
                    };
                    let current = if self.is_current(model) {
                        "  [current]"
                    } else {
                        ""
                    };
                    ListItem::Row {
                        display: format!("{}{}{}", model.display, variant, current),
                        muted: !model.available,
                    }
                }
            })
            .collect();

        // U17①：过滤无命中 → 明示行（原渲染零行空框 + 过滤 hint，用户
        // 分不清是没匹配还是没数据）。
        if items.is_empty() {
            items.push(ListItem::Row {
                display: format!("  No matches for '{}'", self.query),
                muted: true,
            });
        }

        let title = if self.query.is_empty() {
            "Select Model".to_string()
        } else {
            format!("Select Model — query: {}", self.query)
        };

        backdrop::render_list_dialog_bottom(
            backdrop::ListDialogHeading {
                title: &title,
                border_color: colors::ACCENT_CYAN(),
            },
            &items,
            self.selected,
            "type to filter  ⌫ erase  ↑↓ navigate  Tab: variant  Enter: select  Esc: close",
            ctx,
            geom,
            18,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, available: bool) -> ModelEntry {
        ModelEntry {
            provider: "deepseek".to_string(),
            provider_display: "DeepSeek".to_string(),
            model_id: id.to_string(),
            display: id.to_string(),
            variants: Vec::new(),
            available,
        }
    }

    #[test]
    fn unavailable_catalog_model_cannot_be_selected() {
        let mut dialog = ModelSelectDialog::new();
        dialog.set_models(vec![model("deepseek-v4-pro", false)]);
        dialog.open();

        let ModelDialogOutcome::Notice(message) = dialog.handle_key(&Key::Enter) else {
            panic!("unavailable model must produce a notice");
        };
        assert!(message.contains("deepseek-v4-pro"));
        assert!(dialog.is_open());
    }

    #[test]
    fn registered_model_can_be_selected() {
        let mut dialog = ModelSelectDialog::new();
        dialog.set_models(vec![model("deepseek-v4-flash", true)]);
        dialog.open();

        let ModelDialogOutcome::Selected(selected) = dialog.handle_key(&Key::Enter) else {
            panic!("registered model must be selectable");
        };
        assert_eq!(selected.model_id, "deepseek-v4-flash");
        assert!(!dialog.is_open());
    }

    #[test]
    fn current_model_is_distinct_from_recent_and_selected_on_open() {
        let mut dialog = ModelSelectDialog::new();
        dialog.set_models(vec![
            model("deepseek-v4-flash", true),
            model("deepseek-v4-pro", true),
        ]);
        dialog.set_recent(vec![("deepseek".into(), "deepseek-v4-flash".into())]);
        dialog.set_current(Some("deepseek/deepseek-v4-pro".into()));
        dialog.open();

        let selected = dialog.selected_model().expect("current model selected");
        assert_eq!(selected.model_id, "deepseek-v4-pro");
        assert!(dialog.is_current(selected));
    }
}
