//! 金 — SessionHeader: top bar with session info.
//!
//! Shows working directory, agent/model badges via Revue hstack + Text widgets.

use revue::prelude::*;

pub fn render_header(
    working_dir: &str,
    session_title: &str,
    agent: Option<&str>,
    model: Option<&str>,
    ctx: &mut RenderContext,
) {
    let short_dir = if working_dir.len() > 35 {
        format!("...{}", &working_dir[working_dir.len().saturating_sub(32)..])
    } else { working_dir.to_string() };

    // Background
    for x in 0..ctx.area.width {
        ctx.draw_text(x, ctx.area.y, " ", Color::rgb(30, 32, 44));
        ctx.draw_text(x, ctx.area.y + 1, "─", Color::rgb(59, 66, 97));
    }

    // Left: working directory
    ctx.draw_text(1, ctx.area.y, &format!("📁 {}", short_dir), Color::rgb(86, 95, 137));

    // Agent badge
    let mut col = short_dir.len() as u16 + 6;
    if let Some(a) = agent {
        ctx.draw_text(col, ctx.area.y, &format!("🤖 {}", a), Color::rgb(187, 154, 247));
        col += a.len() as u16 + 5;
    }

    // Model badge
    if let Some(m) = model {
        ctx.draw_text(col, ctx.area.y, &format!("🧠 {}", m), Color::rgb(125, 207, 255));
    }

    // Session title (right side)
    if !session_title.is_empty() && session_title != "New Session" {
        let title_x = ctx.area.width.saturating_sub(session_title.len() as u16 + 2);
        ctx.draw_text(title_x, ctx.area.y, session_title, Color::rgb(169, 177, 214));
    }
}
