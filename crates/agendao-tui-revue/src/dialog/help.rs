//! 金 — Help dialog using Revue Border + vstack + Text widgets.

use revue::prelude::*;
use revue::event::Key;

pub struct HelpDialog {
    pub visible: bool,
}

impl HelpDialog {
    pub fn new() -> Self { Self { visible: false } }
    pub fn toggle(&mut self) { self.visible = !self.visible; }
    pub fn dismiss(&mut self) { self.visible = false; }

    pub fn handle_key(&mut self, key: &Key) -> bool {
        if !self.visible { return false; }
        match key {
            Key::Escape | Key::Char('q') | Key::Char('h') | Key::Char('?') => { self.dismiss(); true }
            _ => true,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }

        let bindings: &[(&str, &str)] = &[
            ("Enter", "Send prompt"),
            ("Esc/q", "Quit"),
            ("h", "Home screen"),
            ("?", "Toggle help"),
            ("Up/Down", "Prompt history"),
            ("Tab", "Autocomplete"),
            ("Ctrl+C", "Force quit"),
        ];

        let mut content = vstack().gap(1);
        for (key, desc) in bindings {
            content = content.child(
                hstack().gap(2)
                    .child(Text::new(format!("{:>10}", key)).fg(Color::rgb(125, 207, 255)))
                    .child(Text::new(*desc).fg(Color::rgb(169, 177, 214)))
            );
        }
        content = content.child(Text::new("Press Esc/q/h/? to close").fg(Color::rgb(86, 95, 137)));

        let dialog = Border::rounded()
            .title(" Help — Keybindings ")
            .fg(Color::rgb(137, 180, 250))
            .child(content);

        let w = 54u16.min(ctx.area.width - 4);
        let h = (bindings.len() as u16 + 4).min(ctx.area.height - 4);
        let x = (ctx.area.width - w) / 2;
        let y = (ctx.area.height - h) / 2;

        let pos = revue::widget::positioned(dialog)
            .x(x as i16).y(y as i16).width(w).height(h);
        pos.render(ctx);
    }
}
