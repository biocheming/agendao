//! 木+金 — Home Screen with visual hierarchy.
//!
//! Layout: centered column (max_width 72) holding
//!   1. Logo block (left-padded, no border)
//!   2. Quick Actions panel  (Border::rounded, 2-col grid via child_sized)
//!   3. Tip panel             (Border::rounded, single rotating tip)
//!   4. Environment panel     (Border::rounded, dir + version)
//!
//! Why panels instead of `─── Section ───` rule lines:
//!   horizontal-rule strings have a fixed character length that doesn't
//!   adapt to terminal width — Border widgets size with the parent stack,
//!   so the visual hierarchy stays consistent at any size.

use revue::prelude::*;
use revue::prelude::{Border, Stack};
use crate::store::app_store::AppStore;
use crate::theme::colors;

const HOME_TIPS: &[&str] = &[
    "Press {highlight}Tab{/highlight} to cycle modes",
    "Press {highlight}Ctrl+P{/highlight} to open the command palette",
    "Type {highlight}/help{/highlight} to browse all commands",
    "Use {highlight}/sessions{/highlight} to resume older work",
    "Use {highlight}/timeline{/highlight} to jump to any message",
    "Use {highlight}/new{/highlight} to start a clean session",
    "Use {highlight}/compact{/highlight} when context grows long",
    "Use {highlight}/copy{/highlight} to copy current session summary",
    "Use {highlight}/fork{/highlight} to branch from a message",
    "Use {highlight}/rename{/highlight} to rename this session",
    "Use {highlight}/export{/highlight} to export the transcript",
    "Use {highlight}Esc{/highlight} twice to interrupt a running task",
    "Use {highlight}Alt+Up{/highlight} / {highlight}Alt+Down{/highlight} for prompt history",
    "Use {highlight}@path{/highlight} to reference files in prompt",
    "Use {highlight}/models{/highlight} to switch the active model",
    "Use {highlight}/agents{/highlight} to switch the active agent",
];

const TIP_ROTATE_SECONDS: i64 = 12;
/// Outer width of the centered column. Wider terminals show side gutters,
/// narrower terminals fall through to the natural single-column flow.
const CONTENT_MAX_WIDTH: u16 = 72;
/// Width of the key-label slot inside one grid column. Must accommodate
/// the longest label including brackets, e.g. `[/sessions]` = 11 chars.
const KEY_SLOT_WIDTH: u16 = 12;

pub struct HomeLayout {
    pub store: AppStore,
}

impl View for HomeLayout {
    fn render(&self, ctx: &mut RenderContext) {
        let area = ctx.area;
        let is_narrow = area.width < 60;
        let is_short = area.height < 18;

        // Logo source-of-truth — count lines so we can size the slot exactly.
        // Without `child_sized`, vstack distributes height EQUALLY across all
        // children (Auto), which compresses the logo to ~2 rows and leaves
        // huge empty gaps elsewhere. Fixed sizing keeps each section its
        // natural footprint.
        let logo_lines = agendao_command_render::branding::logo_lines("  ");
        let logo_h = logo_lines.len() as u16;

        // Each panel's natural height = border (2) + content rows
        // 2x2 grid: 2 rows × (4-row card + 1-row gap) = 9 rows
        // Card = border(2) + label(1) + key_line(1) = 4 rows
        let card_h: u16 = 4;
        let actions_h: u16 = if is_narrow { 4 * card_h + 3 } else { 2 * card_h + 1 };
        let tip_h: u16 = 1 + 2;
        let footer_h: u16 = 2 + 2;  // 2 content rows (dir/version + model/agent) + border
        let subtitle_h: u16 = 1;

        let mut column = vstack().gap(1).max_width(CONTENT_MAX_WIDTH);

        // 1. Logo
        column = column.child_sized(self.logo_block(&logo_lines), logo_h);

        // 2. Subtitle / call-to-action
        column = column.child_sized(
            Text::new("  Type below and press Enter to start.")
                .fg(colors::FG_MUTED),
            subtitle_h,
        );

        // 3. Quick Actions panel
        if !is_short {
            column = column.child_sized(self.actions_panel(is_narrow), actions_h);
        }

        // 4. Tip panel (only on wide+tall terminals)
        if !is_short && !is_narrow {
            column = column.child_sized(self.tip_panel(), tip_h);
        }

        // 5. Environment footer
        column = column.child_sized(self.footer_panel(), footer_h);

        column.render(ctx);
    }
}

impl HomeLayout {
    /// Render the brand logo (purple ASCII art).
    fn logo_block(&self, lines: &[String]) -> Stack {
        let mut s = vstack();
        for line in lines {
            // child_sized(_, 1) — each line is exactly 1 row, no Auto compression
            s = s.child_sized(Text::new(line.as_str()).fg(colors::ACCENT_PURPLE), 1);
        }
        s
    }

    /// Quick Actions grid — 2x2 cards in the E-mockup style.
    ///
    /// Each card is a small bordered tile with an UPPERCASE category
    /// label (`START` / `RESUME` / `CONFIG` / `HELP`) and the actual
    /// command underneath. This reads as 4 distinct buckets the user
    /// can scan in one glance, instead of the previous "key + desc"
    /// list which mashed all 6 actions into the same density.
    ///
    /// The grid wrapper itself stays unbordered — only the inner cards
    /// have borders, so the section reads as a panel of tiles rather
    /// than a panel-with-text.
    fn actions_panel(&self, is_narrow: bool) -> Stack {
        let model = self.store.selected_model.get();
        let sel_model_short = model
            .map(|m| {
                // Strip provider prefix for compactness in the tile.
                m.split_once('/').map(|(_, t)| t.to_string()).unwrap_or(m)
            })
            .unwrap_or_else(|| "default".to_string());

        let cards: [(&str, &str, &str, Color); 4] = [
            ("START",  "Enter",     "新会话",                 colors::ACCENT_GREEN),
            ("RESUME", "/sessions", "历史会话",               colors::ACCENT_CYAN),
            ("CONFIG", "/models",   sel_model_short.as_str(), colors::ACCENT_BLUE),
            ("HELP",   "Ctrl+P",    "命令面板",               colors::ACCENT_PURPLE),
        ];

        let mut grid = vstack().gap(1);
        if is_narrow {
            // Single column on narrow terminals
            for (cat, key, hint, color) in cards.iter() {
                grid = grid.child_sized(self.action_card(cat, key, hint, *color), 4);
            }
        } else {
            // 2x2 grid: rows of two cards each
            for chunk in cards.chunks(2) {
                let row = hstack().gap(1)
                    .child_flex(self.action_card(chunk[0].0, chunk[0].1, chunk[0].2, chunk[0].3), 1.0)
                    .child_flex(self.action_card(chunk[1].0, chunk[1].1, chunk[1].2, chunk[1].3), 1.0);
                grid = grid.child_sized(row, 4);
            }
        }
        grid
    }

    /// Render a single action tile: small label header + key + hint.
    /// Uses Border::rounded for the frame so each card reads as a
    /// distinct surface — the visual language of E-mockup tiles.
    fn action_card(&self, label: &str, key: &str, hint: &str, accent: Color) -> Border {
        let body = vstack().gap(0)
            .child_sized(
                Text::new(format!(" {} ", label)).fg(accent).bold(),
                1,
            )
            .child_sized(
                hstack().gap(1)
                    .child(Text::new(key).fg(colors::FG_PRIMARY).bold())
                    .child(Text::new(format!("· {}", hint)).fg(colors::FG_MUTED)),
                1,
            );
        Border::rounded()
            .fg(colors::BORDER)
            .child(body)
    }

    /// One key + description, aligned via fixed-width key column.
    /// Used by the narrow-mode fallback when the grid collapses to
    /// a single column.
    #[allow(dead_code)]
    fn key_action(&self, key: &str, desc: &str) -> Stack {
        hstack().gap(1)
            .child_sized(
                Text::new(format!("[{}]", key)).fg(colors::ACCENT_CYAN),
                KEY_SLOT_WIDTH,
            )
            .child_flex(
                Text::new(desc).fg(colors::FG_SECONDARY),
                1.0,
            )
    }

    /// Single rotating tip in a panel — replaces the old "─── Tips ───" rule.
    fn tip_panel(&self) -> Border {
        let slot = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0) as i64
            / TIP_ROTATE_SECONDS;
        let tip_idx = (slot as usize) % HOME_TIPS.len();
        let tip = HOME_TIPS[tip_idx];

        let body = hstack().gap(1)
            .child_sized(Text::new("💡").fg(colors::ACCENT_ORANGE), 2)
            .child_flex(self.parse_tip_highlights(tip), 1.0);

        Border::rounded()
            .title(" Tip ".to_string())
            .fg(colors::BORDER)
            .child(body)
    }

    /// Replace `{highlight}...{/highlight}` markup with bold spans.
    /// Each segment is `child_sized` to its exact char count so hstack's
    /// Auto distribution doesn't smear them across the panel width.
    fn parse_tip_highlights(&self, tip: &str) -> Stack {
        let mut stack = hstack().gap(0);
        let mut remaining = tip;

        loop {
            if let Some(start_idx) = remaining.find("{highlight}") {
                let (plain, after) = remaining.split_at(start_idx);
                if !plain.is_empty() {
                    let w = plain.chars().count() as u16;
                    stack = stack.child_sized(Text::new(plain).fg(colors::FG_MUTED), w);
                }
                let highlighted = &after["{highlight}".len()..];
                if let Some(end_idx) = highlighted.find("{/highlight}") {
                    let (hl, rest) = highlighted.split_at(end_idx);
                    let w = hl.chars().count() as u16;
                    stack = stack.child_sized(
                        Text::new(hl).fg(colors::FG_PRIMARY).bold(),
                        w,
                    );
                    remaining = &rest["{/highlight}".len()..];
                } else {
                    break;
                }
            } else {
                if !remaining.is_empty() {
                    let w = remaining.chars().count() as u16;
                    stack = stack.child_sized(Text::new(remaining).fg(colors::FG_MUTED), w);
                }
                break;
            }
        }

        stack
    }

    /// Environment panel — directory left, version right, padded to fill width.
    /// Environment / Selection panel.
    ///
    /// Shows the working directory, current model & agent picks, and
    /// the agendao version. Putting the active model on the home page
    /// answers the "did /models actually take effect?" question without
    /// requiring the user to start a session first.
    fn footer_panel(&self) -> Border {
        let dir = self.store.working_dir.get();
        let dir_display = shorten_path(&dir);
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        let model = self.store.selected_model.get();
        let agent = self.store.selected_agent.get();

        // Two-line body: row 1 = directory · version, row 2 = model · agent.
        let version_w = version.chars().count() as u16;
        let row1 = hstack().gap(2)
            .child_flex(
                Text::new(format!("📁  {}", dir_display)).fg(colors::FG_MUTED),
                1.0,
            )
            .child_sized(Text::new(version).fg(colors::FG_MUTED), version_w);

        let model_label = match &model {
            Some(m) => format!("🤖  {}", m),
            None    => "🤖  (default — type /models to pick)".to_string(),
        };
        let agent_label = match &agent {
            Some(a) => format!("🛠  {}", a),
            None    => "🛠  build".to_string(),
        };
        let model_color = if model.is_some() { colors::ACCENT_CYAN } else { colors::FG_MUTED };
        let agent_color = if agent.is_some() { colors::ACCENT_PURPLE } else { colors::FG_MUTED };
        let agent_w = (agent_label.chars().count() as u16).saturating_add(2);
        let row2 = hstack().gap(2)
            .child_flex(Text::new(model_label).fg(model_color), 1.0)
            .child_sized(Text::new(agent_label).fg(agent_color), agent_w);

        let body = vstack().gap(0)
            .child_sized(row1, 1)
            .child_sized(row2, 1);

        Border::rounded()
            .title(" Environment ".to_string())
            .fg(colors::BORDER)
            .child(body)
    }
}

/// Replace the user's home prefix with `~` and truncate long middle segments
/// so the path fits inside the panel without wrapping.
fn shorten_path(path: &str) -> String {
    let with_tilde = if let Some(home) = std::env::var_os("HOME") {
        let home_str = home.to_string_lossy();
        if path.starts_with(home_str.as_ref()) {
            format!("~{}", &path[home_str.len()..])
        } else {
            path.to_string()
        }
    } else {
        path.to_string()
    };

    const MAX: usize = 48;
    if with_tilde.chars().count() <= MAX {
        with_tilde
    } else {
        // Keep prefix + last 2 segments
        let parts: Vec<&str> = with_tilde.split('/').collect();
        if parts.len() >= 4 {
            format!("{}/.../{}/{}",
                parts[0],
                parts[parts.len() - 2],
                parts[parts.len() - 1])
        } else {
            with_tilde
        }
    }
}
