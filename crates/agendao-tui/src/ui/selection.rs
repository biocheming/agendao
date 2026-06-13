use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

/// Terminal text selection — tracks a rectangular region in screen coordinates
/// and provides hit-testing + text extraction.
///
/// Selection follows standard terminal behavior:
/// - First row: from start column to end of line
/// - Middle rows: entire line
/// - Last row: from start of line to end column
/// - Single row: from start column to end column
#[derive(Clone)]
pub struct Selection {
    /// Anchor point (where mouse-down happened).
    anchor: Option<(u16, u16)>,
    /// Current drag endpoint.
    cursor: Option<(u16, u16)>,
    /// Optional clipping region for highlight and copy.
    scope: Option<Rect>,
    /// True while the mouse button is held down.
    dragging: bool,
}

impl Selection {
    pub fn new() -> Self {
        Self {
            anchor: None,
            cursor: None,
            scope: None,
            dragging: false,
        }
    }

    /// Begin a new selection at (row, col).
    pub fn start(&mut self, row: u16, col: u16) {
        self.start_scoped(row, col, None);
    }

    /// Begin a new selection constrained to a given rectangle.
    pub fn start_scoped(&mut self, row: u16, col: u16, scope: Option<Rect>) {
        let (row, col) = clamp_point_to_scope(scope, row, col);
        self.anchor = Some((row, col));
        self.cursor = Some((row, col));
        self.scope = scope;
        self.dragging = true;
    }

    /// Update the drag endpoint.
    pub fn update(&mut self, row: u16, col: u16) {
        if self.dragging {
            let (row, col) = clamp_point_to_scope(self.scope, row, col);
            self.cursor = Some((row, col));
        }
    }

    /// Mouse button released — keep the selection visible but stop tracking.
    pub fn finalize(&mut self) {
        self.dragging = false;
    }

    /// Dismiss the selection entirely.
    pub fn clear(&mut self) {
        self.anchor = None;
        self.cursor = None;
        self.scope = None;
        self.dragging = false;
    }

    /// True if there is a visible selection (dragging or finalized).
    pub fn is_active(&self) -> bool {
        self.anchor.is_some() && self.cursor.is_some()
    }

    /// True if the user is currently dragging.
    pub fn is_selecting(&self) -> bool {
        self.dragging
    }

    /// Returns the normalized range: (top-left, bottom-right) in reading order.
    fn range(&self) -> Option<((u16, u16), (u16, u16))> {
        match (self.anchor, self.cursor) {
            (Some(a), Some(b)) => {
                if a.0 < b.0 || (a.0 == b.0 && a.1 <= b.1) {
                    Some((a, b))
                } else {
                    Some((b, a))
                }
            }
            _ => None,
        }
    }

    /// Test whether a specific cell is inside the selection.
    pub fn is_selected(&self, row: u16, col: u16) -> bool {
        if !point_in_scope(self.scope, row, col) {
            return false;
        }
        let ((r0, c0), (r1, c1)) = match self.range() {
            Some(r) => r,
            None => return false,
        };

        if row < r0 || row > r1 {
            return false;
        }

        if r0 == r1 {
            // Single-line selection
            return col >= c0 && col <= c1;
        }

        if row == r0 {
            // First row: from start col to end of line
            return col >= c0;
        }

        if row == r1 {
            // Last row: from start of line to end col
            return col <= c1;
        }

        // Middle rows: entire line
        true
    }

    /// Extract the selected text using a callback that returns the full line
    /// content for a given row number.
    pub fn get_selected_text<F>(&self, get_line: F) -> String
    where
        F: Fn(u16) -> Option<String>,
    {
        let ((r0, c0), (r1, c1)) = match self.range() {
            Some(r) => r,
            None => return String::new(),
        };
        let (row_start, row_end) = match intersect_row_range(self.scope, r0, r1) {
            Some(range) => range,
            None => return String::new(),
        };

        let mut result = String::new();

        for row in row_start..=row_end {
            let line = match get_line(row) {
                Some(l) => l,
                None => continue,
            };

            let Some((start_col, end_col_exclusive)) =
                selected_columns_for_row(self.scope, r0, c0, r1, c1, row)
            else {
                continue;
            };
            let selected = slice_by_columns(&line, start_col as usize, end_col_exclusive as usize);

            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(selected.trim_end());
        }

        result
    }
}

fn clamp_point_to_scope(scope: Option<Rect>, row: u16, col: u16) -> (u16, u16) {
    let Some(scope) = scope else {
        return (row, col);
    };
    if scope.width == 0 || scope.height == 0 {
        return (row, col);
    }
    let max_row = scope.y.saturating_add(scope.height.saturating_sub(1));
    let max_col = scope.x.saturating_add(scope.width.saturating_sub(1));
    (row.clamp(scope.y, max_row), col.clamp(scope.x, max_col))
}

fn point_in_scope(scope: Option<Rect>, row: u16, col: u16) -> bool {
    let Some(scope) = scope else {
        return true;
    };
    if scope.width == 0 || scope.height == 0 {
        return false;
    }
    let max_row = scope.y.saturating_add(scope.height);
    let max_col = scope.x.saturating_add(scope.width);
    row >= scope.y && row < max_row && col >= scope.x && col < max_col
}

fn intersect_row_range(scope: Option<Rect>, r0: u16, r1: u16) -> Option<(u16, u16)> {
    let Some(scope) = scope else {
        return Some((r0, r1));
    };
    if scope.height == 0 {
        return None;
    }
    let scope_end = scope.y.saturating_add(scope.height.saturating_sub(1));
    let start = r0.max(scope.y);
    let end = r1.min(scope_end);
    (start <= end).then_some((start, end))
}

fn selected_columns_for_row(
    scope: Option<Rect>,
    r0: u16,
    c0: u16,
    r1: u16,
    c1: u16,
    row: u16,
) -> Option<(u16, u16)> {
    let (mut start, mut end_exclusive) = if r0 == r1 {
        (c0, c1.saturating_add(1))
    } else if row == r0 {
        (c0, u16::MAX)
    } else if row == r1 {
        (0, c1.saturating_add(1))
    } else {
        (0, u16::MAX)
    };

    if let Some(scope) = scope {
        if scope.width == 0 {
            return None;
        }
        let scope_start = scope.x;
        let scope_end = scope.x.saturating_add(scope.width);
        start = start.max(scope_start);
        end_exclusive = end_exclusive.min(scope_end);
    }

    (start < end_exclusive).then_some((start, end_exclusive))
}

fn slice_by_columns(line: &str, start_col: usize, end_col_exclusive: usize) -> String {
    if start_col >= end_col_exclusive {
        return String::new();
    }
    let start = byte_index_for_column_start(line, start_col);
    let end = byte_index_for_column_end(line, end_col_exclusive);
    if start >= end || start >= line.len() {
        return String::new();
    }
    line[start..end].to_string()
}

fn byte_index_for_column_start(line: &str, target_col: usize) -> usize {
    if target_col == 0 {
        return 0;
    }

    let mut col = 0usize;
    for (idx, ch) in line.char_indices() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
        if target_col <= col {
            return idx;
        }
        if target_col < col + width {
            // Selection starts inside a glyph cell: snap to glyph start.
            return idx;
        }
        col += width;
    }
    line.len()
}

fn byte_index_for_column_end(line: &str, target_col: usize) -> usize {
    if target_col == 0 {
        return 0;
    }

    let mut col = 0usize;
    for (idx, ch) in line.char_indices() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
        let end_idx = idx + ch.len_utf8();
        if target_col <= col {
            return idx;
        }
        if target_col < col + width {
            // Selection ends inside a glyph cell: include the whole glyph.
            return end_idx;
        }
        if target_col == col + width {
            return end_idx;
        }
        col += width;
    }
    line.len()
}

impl Default for Selection {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Selection;
    use ratatui::layout::Rect;

    #[test]
    fn utf8_glyph_boundary_selection_is_safe() {
        let line = "  ││ Provider error";
        let mut selection = Selection::new();
        selection.start(0, 2);
        selection.update(0, 4);
        selection.finalize();

        let selected = selection.get_selected_text(|row| {
            if row == 0 {
                Some(line.to_string())
            } else {
                None
            }
        });

        assert_eq!(selected, "││");
    }

    #[test]
    fn unicode_wide_char_selection_is_safe() {
        let line = "A你B";
        let mut selection = Selection::new();
        // Select only the wide glyph cell range.
        selection.start(0, 1);
        selection.update(0, 2);
        selection.finalize();

        let selected = selection.get_selected_text(|row| {
            if row == 0 {
                Some(line.to_string())
            } else {
                None
            }
        });

        assert_eq!(selected, "你");
    }

    #[test]
    fn scoped_selection_does_not_copy_outside_rect() {
        let mut selection = Selection::new();
        selection.start_scoped(5, 10, Some(Rect::new(10, 5, 8, 3)));
        selection.update(8, 40);
        selection.finalize();

        let selected = selection.get_selected_text(|row| match row {
            5 => Some("0123456789abcdefghij".to_string()),
            6 => Some("0123456789klmnopqrst".to_string()),
            7 => Some("0123456789uvwxyzABCD".to_string()),
            _ => None,
        });

        assert_eq!(selected, "abcdefgh\nklmnopqr\nuvwxyzAB");
    }
}
