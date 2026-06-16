//! 金 — Model selection dialog: Provider → Model → Variant.
//!
//! Uses shared dialog backdrop for consistent visual identity.

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

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
    /// disconnected provider). Caller should show this string as a toast.
    Notice(String),
    /// User picked a usable model; dialog has already closed.
    Selected(ModelEntry),
}

#[derive(Clone)]
pub struct ProviderGroup {
    pub name: String, pub models: Vec<ModelEntry>,
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
    /// Live search query — type to filter, Backspace to delete.
    query: String,
}

#[derive(Clone)]
enum FlatRow { Header(String), Model(usize, usize) } // group_idx, model_idx

impl ModelSelectDialog {
    pub fn new() -> Self { Self { visible: false, groups: vec![], flat: vec![], selected: 0, variant_idx: 0, recent: vec![], query: String::new() } }

    pub fn set_models(&mut self, models: Vec<ModelEntry>) {
        self.groups.clear();
        // Group by provider id, but capture each group's display label
        // from the first ModelEntry we see. Without this, the group header
        // shows "aihubmix" instead of "AIHubMix" — visually it looked like
        // a regression after we switched provider field from name → id.
        let mut providers: std::collections::BTreeMap<String, (String, Vec<ModelEntry>)> = std::collections::BTreeMap::new();
        for m in models {
            let key = m.provider.clone();
            let display = if m.provider_display.is_empty() { m.provider.clone() } else { m.provider_display.clone() };
            providers.entry(key).or_insert_with(|| (display, Vec::new())).1.push(m);
        }
        for (_id, (display, models)) in providers {
            self.groups.push(ProviderGroup { name: display, models });
        }
        self.rebuild_flat();
    }

    pub fn set_recent(&mut self, recent: Vec<(String, String)>) { self.recent = recent; self.rebuild_flat(); }

    fn rebuild_flat(&mut self) {
        self.flat.clear();
        let q = self.query.to_lowercase();
        let matches = |provider: &str, m: &ModelEntry| -> bool {
            if q.is_empty() { return true; }
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
            if matched.is_empty() { continue; }
            self.flat.push(FlatRow::Header(format!("▸ {}", group.name)));
            for mi in matched {
                self.flat.push(FlatRow::Model(gi, mi));
            }
        }
        // Reset selection to first selectable row (skip headers)
        self.selected = self.flat.iter().position(|r| matches!(r, FlatRow::Model(_, _))).unwrap_or(0);
    }

    fn find_model(&self, provider: &str, model_id: &str) -> Option<(usize, usize)> {
        for (gi, g) in self.groups.iter().enumerate() {
            if g.name == provider {
                for (mi, m) in g.models.iter().enumerate() {
                    if m.model_id == model_id { return Some((gi, mi)); }
                }
            }
        }
        None
    }

    pub fn open(&mut self) { self.visible = true; self.selected = 0; self.query.clear(); self.rebuild_flat(); }
    pub fn close(&mut self) { self.visible = false; self.query.clear(); }
    pub fn is_open(&self) -> bool { self.visible }

    /// Outcome of a key press on the model dialog.
    /// `Selected` carries the chosen model and closes the dialog.
    /// `Notice` is a soft signal to the host so it can surface a toast
    /// (e.g. "provider is not connected") without abusing exception flow.
    /// `None` means the key was navigation/filter and the dialog stays open.
    pub fn handle_key(&mut self, key: &Key) -> ModelDialogOutcome {
        if !self.visible { return ModelDialogOutcome::None; }
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
                        "Provider '{}' is not connected — pick a model from a connected provider, or use /providers to authenticate first.",
                        m.provider_display,
                    ));
                }
                self.close();
                ModelDialogOutcome::Selected(m)
            }
            Key::Escape => { self.close(); ModelDialogOutcome::None }
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

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }

        // Build all items (no truncation — backdrop scrolls). Without query
        // filtering, 5,140 models exhaust the user's patience; once we add
        // a search box this becomes `flat.iter().filter(matches_query)`.
        let items: Vec<ListItem> = self.flat.iter().enumerate().map(|(i, row)| {
            match row {
                FlatRow::Header(label) => ListItem::Header(label.clone()),
                FlatRow::Model(gi, mi) => {
                    let model = &self.groups[*gi].models[*mi];
                    let variant = if !model.variants.is_empty() && i == self.selected {
                        format!(" [{}]", model.variants[self.variant_idx % model.variants.len()])
                    } else { String::new() };
                    ListItem::Row {
                        display: format!("{}{}", model.display, variant),
                        muted: !model.available,
                    }
                }
            }
        }).collect();

        let title = if self.query.is_empty() {
            "Select Model".to_string()
        } else {
            format!("Select Model — query: {}", self.query)
        };

        backdrop::render_list_dialog_bottom(
            &title,
            colors::ACCENT_CYAN,
            &items,
            self.selected,
            "type to filter  ↑↓ navigate  Tab: variant  Enter: select  Esc: close",
            ctx,
            64,
            18,
        );
    }
}
