use ratatui::{buffer::Buffer, layout::Rect, style::Color};

use crate::ui::{line_from_cells, Selection};

pub fn capture_screen_lines(buffer: &Buffer, area: Rect) -> Vec<String> {
    let mut lines = Vec::new();
    for y in area.y..area.y + area.height {
        let line = line_from_cells((area.x..area.x + area.width).map(|x| buffer[(x, y)].symbol()));
        lines.push(line.trim_end().to_string());
    }
    lines
}

pub fn apply_selection_highlight(buffer: &mut Buffer, area: Rect, selection: &Selection) {
    if !selection.is_active() {
        return;
    }

    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if !selection.is_selected(y, x) {
                continue;
            }
            let cell = &buffer[(x, y)];
            let sym = cell.symbol();
            if sym.is_empty() || sym.chars().all(|c| c == ' ') {
                continue;
            }
            let cell = &mut buffer[(x, y)];
            let fg = if cell.fg == Color::Reset {
                Color::White
            } else {
                cell.fg
            };
            let bg = if cell.bg == Color::Reset {
                Color::Black
            } else {
                cell.bg
            };
            cell.fg = bg;
            cell.bg = fg;
        }
    }
}
