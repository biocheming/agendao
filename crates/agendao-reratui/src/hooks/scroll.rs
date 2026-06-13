//! Scroll hooks for managing scrollable content.
//!
//! Provides React-like hooks for building scrollable containers that can
//! handle content taller than the viewport, similar to Ink's scrolling behavior.

use crate::hooks::use_keyboard_press;

use super::state::{StateSetter, use_state};
use crossterm::event::KeyCode;

/// Scroll position and control handle
#[derive(Clone, Copy)]
pub struct ScrollHandle {
    /// Current scroll offset (lines from top)
    pub offset: usize,
    /// Total content height (in lines)
    pub content_height: usize,
    /// Visible viewport height
    pub viewport_height: usize,
    /// Setter for scroll offset
    set_offset: StateSetter<usize>,
}

impl ScrollHandle {
    /// Scroll up by the given number of lines
    pub fn scroll_up(&self, lines: usize) {
        let current = self.offset;
        let new_offset = current.saturating_sub(lines);
        self.set_offset.set(new_offset);
    }

    /// Scroll down by the given number of lines
    pub fn scroll_down(&self, lines: usize) {
        let current = self.offset;
        let max_offset = self.max_offset();
        let new_offset = (current + lines).min(max_offset);
        self.set_offset.set(new_offset);
    }

    /// Scroll to the top
    pub fn scroll_to_top(&self) {
        self.set_offset.set(0);
    }

    /// Scroll to the bottom
    pub fn scroll_to_bottom(&self) {
        self.set_offset.set(self.max_offset());
    }

    /// Page up (scroll by viewport height)
    pub fn page_up(&self) {
        self.scroll_up(self.viewport_height.saturating_sub(1));
    }

    /// Page down (scroll by viewport height)
    pub fn page_down(&self) {
        self.scroll_down(self.viewport_height.saturating_sub(1));
    }

    /// Maximum scroll offset
    pub fn max_offset(&self) -> usize {
        self.content_height.saturating_sub(self.viewport_height)
    }

    /// Whether we can scroll up
    pub fn can_scroll_up(&self) -> bool {
        self.offset > 0
    }

    /// Whether we can scroll down
    pub fn can_scroll_down(&self) -> bool {
        self.offset < self.max_offset()
    }

    /// Scroll progress as a percentage (0.0 to 1.0)
    pub fn scroll_progress(&self) -> f64 {
        let max = self.max_offset();
        if max == 0 {
            0.0
        } else {
            self.offset as f64 / max as f64
        }
    }

    /// Whether content overflows the viewport
    pub fn has_overflow(&self) -> bool {
        self.content_height > self.viewport_height
    }
}

/// Hook for managing scroll state with keyboard navigation.
///
/// Returns a `ScrollHandle` that provides scroll position and control methods.
///
/// # Arguments
///
/// * `content_height` - Total height of the content in lines
/// * `viewport_height` - Height of the visible viewport
///
/// # Example
///
/// ```rust,ignore
/// use reratui::prelude::*;
/// use reratui::hooks::use_scroll;
///
/// struct ScrollableList {
///     items: Vec<String>,
/// }
///
/// impl Component for ScrollableList {
///     fn render(&self, area: Rect, buffer: &mut Buffer) {
///         let scroll = use_scroll(self.items.len(), area.height as usize);
///         
///         // Get visible slice of items
///         let visible_items = &self.items[scroll.offset..];
///         
///         // Render items...
///     }
/// }
/// ```
pub fn use_scroll(content_height: usize, viewport_height: usize) -> ScrollHandle {
    let (offset, set_offset) = use_state(|| 0usize);

    // Clamp offset if content shrinks
    let max_offset = content_height.saturating_sub(viewport_height);
    if offset > max_offset {
        set_offset.set(max_offset);
    }

    ScrollHandle {
        offset,
        content_height,
        viewport_height,
        set_offset,
    }
}

/// Hook for scroll state with automatic keyboard bindings.
///
/// Automatically handles:
/// - `j` / `Down` - scroll down
/// - `k` / `Up` - scroll up  
/// - `g` / `Home` - scroll to top
/// - `G` / `End` - scroll to bottom
/// - `Ctrl+d` / `PageDown` - page down
/// - `Ctrl+u` / `PageUp` - page up
///
/// # Example
///
/// ```rust,ignore
/// let scroll = use_scroll_keyboard(items.len(), area.height as usize);
/// ```
pub fn use_scroll_keyboard(content_height: usize, viewport_height: usize) -> ScrollHandle {
    let scroll = use_scroll(content_height, viewport_height);

    // Handle keyboard navigation
    use_keyboard_press(move |key| match key.code {
        KeyCode::Char('j') | KeyCode::Down => scroll.scroll_down(1),
        KeyCode::Char('k') | KeyCode::Up => scroll.scroll_up(1),
        KeyCode::Char('g') | KeyCode::Home => scroll.scroll_to_top(),
        KeyCode::Char('G') | KeyCode::End => scroll.scroll_to_bottom(),
        KeyCode::PageDown => scroll.page_down(),
        KeyCode::PageUp => scroll.page_up(),
        KeyCode::Char('d')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            scroll.page_down();
        }
        KeyCode::Char('u')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            scroll.page_up();
        }
        _ => {}
    });

    scroll
}
