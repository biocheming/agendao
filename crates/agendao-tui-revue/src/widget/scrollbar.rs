//! 金 / 木 — Interactive scrollbar overlay.

use revue::layout::Rect;
use revue::prelude::Color;
use revue::render::{Cell, Modifier};
use revue::widget::traits::RenderContext;

use crate::theme::colors;

#[derive(Clone, Copy, Debug)]
pub struct Scrollbar {
    pub area: Rect,
    pub content_height: u16,
    pub viewport_height: u16,
    pub offset: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarHit {
    ArrowUp,
    ArrowDown,
    PageUp,
    PageDown,
    BeginDrag(ScrollbarDrag),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollbarDrag {
    pub origin_y: u16,
    pub origin_offset: u16,
}

impl Scrollbar {
    pub fn new(area: Rect, content_height: u16, viewport_height: u16, offset: u16) -> Self {
        Self { area, content_height, viewport_height, offset }
    }

    pub fn is_scrollable(&self) -> bool {
        self.content_height > self.viewport_height
    }

    pub fn max_offset(&self) -> u16 {
        self.content_height.saturating_sub(self.viewport_height)
    }

    fn arrow_up_y(&self) -> u16 { self.area.y }
    fn arrow_down_y(&self) -> u16 { self.area.y + self.area.height.saturating_sub(1) }
    fn track_height(&self) -> u16 { self.area.height.saturating_sub(2) }
    fn track_y(&self) -> u16 { self.area.y + 1 }

    fn thumb_height(&self) -> u16 {
        let track = self.track_height();
        if track == 0 || self.content_height == 0 { return 1; }
        let raw = (self.viewport_height as f32 / self.content_height as f32) * track as f32;
        (raw.max(1.0) as u16).clamp(1, track)
    }

    fn thumb_top_y(&self) -> u16 {
        let track = self.track_height();
        let thumb = self.thumb_height();
        if track <= thumb || self.max_offset() == 0 { return self.track_y(); }
        let ratio = self.offset as f32 / self.max_offset() as f32;
        let off = ((track - thumb) as f32 * ratio).round() as u16;
        self.track_y() + off
    }

    fn is_in_thumb(&self, _x: u16, y: u16) -> bool {
        let top = self.thumb_top_y();
        let bottom = top + self.thumb_height();
        y >= top && y < bottom
    }

    pub fn hit_test(&self, x: u16, y: u16) -> Option<ScrollbarHit> {
        if !self.is_scrollable() { return None; }
        if x != self.area.x { return None; }
        if y < self.area.y || y >= self.area.y + self.area.height { return None; }
        if y == self.arrow_up_y() { return Some(ScrollbarHit::ArrowUp); }
        if y == self.arrow_down_y() { return Some(ScrollbarHit::ArrowDown); }
        if self.is_in_thumb(x, y) {
            return Some(ScrollbarHit::BeginDrag(ScrollbarDrag {
                origin_y: y, origin_offset: self.offset,
            }));
        }
        if y < self.thumb_top_y() { Some(ScrollbarHit::PageUp) } else { Some(ScrollbarHit::PageDown) }
    }

    pub fn drag_to_offset(&self, drag: ScrollbarDrag, current_y: u16) -> u16 {
        let track = self.track_height();
        let thumb = self.thumb_height();
        if track <= thumb { return self.offset; }
        let dy = current_y as i32 - drag.origin_y as i32;
        let usable = (track - thumb) as i32;
        if usable == 0 { return drag.origin_offset; }
        let max = self.max_offset() as i32;
        let new_offset_i = drag.origin_offset as i32 + (dy * max + usable / 2) / usable;
        new_offset_i.clamp(0, max) as u16
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if self.area.width == 0 || self.area.height < 3 { return; }
        if !self.is_scrollable() {
            for dy in 0..self.area.height {
                self.put(ctx, self.area.y + dy, ' ', None, None);
            }
            return;
        }
        let track_color = Some(colors::FG_MUTED);
        let thumb_color = Some(colors::ACCENT_CYAN);
        let arrow_color = Some(colors::FG_PRIMARY);
        self.put(ctx, self.arrow_up_y(), '▲', arrow_color, None);
        self.put(ctx, self.arrow_down_y(), '▼', arrow_color, None);
        let track_top = self.track_y();
        let track_bot = track_top + self.track_height();
        for y in track_top..track_bot { self.put(ctx, y, '│', track_color, None); }
        let thumb_top = self.thumb_top_y();
        let thumb_bot = thumb_top + self.thumb_height();
        for y in thumb_top..thumb_bot.min(track_bot) { self.put_bold(ctx, y, '█', thumb_color, None); }
    }

    fn put(&self, ctx: &mut RenderContext, y: u16, ch: char, fg: Option<Color>, bg: Option<Color>) {
        let rel_x = self.area.x.saturating_sub(ctx.area.x);
        let rel_y = y.saturating_sub(ctx.area.y);
        let mut cell = Cell::new(ch);
        cell.fg = fg; cell.bg = bg;
        ctx.set(rel_x, rel_y, cell);
    }

    fn put_bold(&self, ctx: &mut RenderContext, y: u16, ch: char, fg: Option<Color>, bg: Option<Color>) {
        let rel_x = self.area.x.saturating_sub(ctx.area.x);
        let rel_y = y.saturating_sub(ctx.area.y);
        let mut cell = Cell::new(ch);
        cell.fg = fg; cell.bg = bg;
        cell.modifier |= Modifier::BOLD;
        ctx.set(rel_x, rel_y, cell);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sb(content: u16, viewport: u16, offset: u16) -> Scrollbar {
        Scrollbar::new(Rect::new(10, 2, 1, 20), content, viewport, offset)
    }

    #[test]
    fn hit_test_outside_column_is_none() {
        let s = sb(100, 20, 0);
        assert_eq!(s.hit_test(9, 2), None);
        assert_eq!(s.hit_test(11, 2), None);
    }

    #[test]
    fn hit_test_above_or_below_area_is_none() {
        let s = sb(100, 20, 0);
        assert_eq!(s.hit_test(10, 1), None);
        assert_eq!(s.hit_test(10, 22), None);
    }

    #[test]
    fn hit_test_arrow_rows() {
        let s = sb(100, 20, 0);
        assert_eq!(s.hit_test(10, 2), Some(ScrollbarHit::ArrowUp));
        assert_eq!(s.hit_test(10, 21), Some(ScrollbarHit::ArrowDown));
    }

    #[test]
    fn hit_test_thumb_returns_drag_with_origin() {
        let s = sb(100, 20, 0);
        match s.hit_test(10, 3) {
            Some(ScrollbarHit::BeginDrag(d)) => {
                assert_eq!(d.origin_y, 3);
                assert_eq!(d.origin_offset, 0);
            }
            other => panic!("expected BeginDrag, got {:?}", other),
        }
    }

    #[test]
    fn hit_test_track_above_thumb_is_pageup() {
        let s = sb(200, 20, 180);
        let hit = s.hit_test(10, 4).unwrap();
        assert!(matches!(hit, ScrollbarHit::PageUp), "got {:?}", hit);
    }

    #[test]
    fn hit_test_track_below_thumb_is_pagedown() {
        let s = sb(200, 20, 0);
        let hit = s.hit_test(10, 18).unwrap();
        assert!(matches!(hit, ScrollbarHit::PageDown), "got {:?}", hit);
    }

    #[test]
    fn hit_test_when_not_scrollable_is_none() {
        let s = sb(15, 20, 0);
        assert_eq!(s.hit_test(10, 10), None);
    }

    #[test]
    fn drag_translates_y_delta_to_offset() {
        let s = sb(200, 20, 0);
        let drag = ScrollbarDrag { origin_y: 3, origin_offset: 0 };
        let new_off = s.drag_to_offset(drag, 11);
        assert_eq!(new_off, 85);
    }

    #[test]
    fn drag_clamps_to_max_offset() {
        let s = sb(100, 20, 0);
        let drag = ScrollbarDrag { origin_y: 3, origin_offset: 0 };
        let new_off = s.drag_to_offset(drag, 999);
        assert_eq!(new_off, s.max_offset());
    }

    #[test]
    fn drag_clamps_to_zero() {
        let s = sb(100, 20, 50);
        let drag = ScrollbarDrag { origin_y: 10, origin_offset: 50 };
        let new_off = s.drag_to_offset(drag, 0);
        assert_eq!(new_off, 0);
    }

    #[test]
    fn thumb_top_at_max_offset_aligns_with_bottom_of_track() {
        let s = sb(200, 20, 180);
        let track_top = s.track_y();
        let track_h = s.track_height();
        let thumb_h = s.thumb_height();
        let thumb_top = s.thumb_top_y();
        assert_eq!(thumb_top + thumb_h, track_top + track_h);
    }
}
