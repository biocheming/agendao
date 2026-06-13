//! 金 — Alert dialog using Revue Border + vstack + Text widgets.

use revue::prelude::*;
use revue::event::Key;

pub struct AlertDialog {
    pub title: String,
    pub message: String,
    pub visible: bool,
}

impl AlertDialog {
    pub fn new() -> Self { Self { title: String::new(), message: String::new(), visible: false } }

    pub fn show(&mut self, title: &str, message: &str) {
        self.title = title.to_string();
        self.message = message.to_string();
        self.visible = true;
    }

    pub fn dismiss(&mut self) { self.visible = false; }

    pub fn handle_key(&mut self, key: &Key) -> bool {
        if !self.visible { return false; }
        match key {
            Key::Enter | Key::Escape | Key::Char(' ') | Key::Char('q') => { self.dismiss(); true }
            _ => true,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }

        let mut content = vstack().gap(1);
        for line in self.message.lines() {
            content = content.child(Text::new(line.to_string()).fg(Color::rgb(169, 177, 214)));
        }
        content = content.child(Text::new("Press Enter/Esc/Space to dismiss").fg(Color::rgb(86, 95, 137)));

        let dialog = Border::rounded()
            .title(self.title.clone())
            .fg(Color::rgb(247, 118, 142))
            .child(content);

        let w = (ctx.area.width / 2).max(40).min(ctx.area.width - 4);
        let h = (self.message.lines().count() as u16 + 5).min(ctx.area.height - 4);
        let x = (ctx.area.width - w) / 2;
        let y = (ctx.area.height - h) / 2;

        // Backdrop via Layers
        let backdrop = revue::widget::layers()
            .child(dialog);

        // Render centered
        let pos = revue::widget::positioned(backdrop)
            .x(x as i16).y(y as i16).width(w).height(h);
        pos.render(ctx);
    }
}
