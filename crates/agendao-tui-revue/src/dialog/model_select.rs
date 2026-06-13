//! 金 — Model selection dialog: Provider → Model → Variant.
//!
//! Old TUI: ratatui Paragraph + hand-drawn groups + recent models.
//! New: Revue Border::rounded() + vstack() + Text::new().

use revue::prelude::*;
use revue::event::Key;

#[derive(Clone)]
pub struct ModelEntry {
    pub provider: String, pub model_id: String, pub display: String,
    pub variants: Vec<String>, pub available: bool,
}

#[derive(Clone)]
pub struct ProviderGroup {
    pub name: String, pub models: Vec<ModelEntry>,
}

pub struct ModelSelectDialog {
    pub visible: bool,
    groups: Vec<ProviderGroup>,
    flat: Vec<FlatRow>,
    selected: usize,
    variant_idx: usize,
    recent: Vec<(String, String)>,
}

#[derive(Clone)]
enum FlatRow { Header(String), Model(usize, usize) } // group_idx, model_idx

impl ModelSelectDialog {
    pub fn new() -> Self { Self { visible: false, groups: vec![], flat: vec![], selected: 0, variant_idx: 0, recent: vec![] } }

    pub fn set_models(&mut self, models: Vec<ModelEntry>) {
        self.groups.clear();
        let mut providers: std::collections::BTreeMap<String, Vec<ModelEntry>> = std::collections::BTreeMap::new();
        for m in models {
            providers.entry(m.provider.clone()).or_default().push(m);
        }
        for (name, models) in providers {
            self.groups.push(ProviderGroup { name, models });
        }
        self.rebuild_flat();
    }

    pub fn set_recent(&mut self, recent: Vec<(String, String)>) { self.recent = recent; self.rebuild_flat(); }

    fn rebuild_flat(&mut self) {
        self.flat.clear();
        // Recent section
        if !self.recent.is_empty() {
            self.flat.push(FlatRow::Header("★ Recent".into()));
            for (provider, model_id) in &self.recent {
                if let Some((gi, mi)) = self.find_model(provider, model_id) {
                    self.flat.push(FlatRow::Model(gi, mi));
                }
            }
        }
        // Provider groups
        for (gi, group) in self.groups.iter().enumerate() {
            self.flat.push(FlatRow::Header(format!("▸ {}", group.name)));
            for mi in 0..group.models.len() {
                self.flat.push(FlatRow::Model(gi, mi));
            }
        }
        self.selected = 0;
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

    pub fn open(&mut self) { self.visible = true; self.selected = 0; }
    pub fn close(&mut self) { self.visible = false; }
    pub fn is_open(&self) -> bool { self.visible }

    pub fn handle_key(&mut self, key: &Key) -> Option<ModelEntry> {
        if !self.visible { return None; }
        match key {
            Key::Up => { self.selected = self.selected.saturating_sub(1); None }
            Key::Down => { let max = self.flat.len().saturating_sub(1); self.selected = (self.selected + 1).min(max); None }
            Key::Tab => {
                // Cycle variant
                if let Some(model) = self.selected_model() {
                    if !model.variants.is_empty() {
                        self.variant_idx = (self.variant_idx + 1) % model.variants.len();
                    }
                }
                None
            }
            Key::Enter => { self.close(); self.selected_model().cloned() }
            Key::Escape => { self.close(); None }
            _ => None,
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
        let mut content = vstack().gap(0);
        for (i, row) in self.flat.iter().enumerate().take(14) {
            match row {
                FlatRow::Header(label) => {
                    content = content.child(Text::new(label.as_str()).bold().fg(Color::rgb(137, 180, 250)));
                }
                FlatRow::Model(gi, mi) => {
                    let model = &self.groups[*gi].models[*mi];
                    let marker = if i == self.selected { "▶" } else { " " };
                    let variant = if !model.variants.is_empty() && i == self.selected {
                        format!(" [{}]", model.variants[self.variant_idx % model.variants.len()])
                    } else { String::new() };
                    let line = format!("{} {}{}", marker, model.display, variant);
                    let color = if i == self.selected { Color::rgb(125, 207, 255) } else { Color::rgb(169, 177, 214) };
                    if !model.available { content = content.child(Text::new(line).fg(Color::rgb(86, 95, 137))); }
                    else { content = content.child(Text::new(line).fg(color)); }
                }
            }
        }
        let dialog = Border::rounded().title(" Select Model (Tab: variant) ").fg(Color::rgb(125, 207, 255)).child(content);
        let w = 50u16.min(ctx.area.width - 4);
        let h = 16u16.min(ctx.area.height - 4);
        let x = (ctx.area.width - w) / 2; let y = (ctx.area.height - h) / 2;
        revue::widget::positioned(dialog).x(x as i16).y(y as i16).width(w).height(h).render(ctx);
    }
}
