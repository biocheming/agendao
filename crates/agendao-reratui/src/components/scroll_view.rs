//! ScrollView component for scrollable content.
//!
//! Provides a container that can scroll any content (widgets or text) taller than
//! the viewport. Uses a virtual buffer approach to render content off-screen and
//! then copy the visible portion.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget, Widget},
};

use crate::Component;
use crate::hooks::{use_keyboard, use_state};
use crossterm::event::KeyCode;

/// Configuration for scroll indicators
#[derive(Clone, Debug)]
pub struct ScrollIndicator {
    /// Show scrollbar
    pub show_scrollbar: bool,
    /// Show "more above" indicator
    pub show_more_above: bool,
    /// Show "more below" indicator  
    pub show_more_below: bool,
    /// Scrollbar track color
    pub track_color: Color,
    /// Scrollbar thumb color
    pub thumb_color: Color,
}

impl Default for ScrollIndicator {
    fn default() -> Self {
        Self {
            show_scrollbar: true,
            show_more_above: true,
            show_more_below: true,
            track_color: Color::DarkGray,
            thumb_color: Color::Gray,
        }
    }
}

// ============================================================================
// Virtual Buffer ScrollView - Generic for any widget
// ============================================================================

/// A virtual buffer that content can be rendered into.
///
/// This buffer can be larger than the viewport, allowing content to be
/// rendered at its full size and then scrolled.
pub struct VirtualBuffer {
    buffer: Buffer,
    /// The full content area (may be larger than viewport)
    pub area: Rect,
}

impl VirtualBuffer {
    /// Create a new virtual buffer with the given dimensions
    pub fn new(width: u16, height: u16) -> Self {
        let area = Rect::new(0, 0, width, height);
        Self {
            buffer: Buffer::empty(area),
            area,
        }
    }

    /// Get a mutable reference to the underlying buffer
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffer
    }

    /// Get a reference to the underlying buffer
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// Copy a portion of this virtual buffer to a target buffer
    ///
    /// # Arguments
    /// * `target` - The buffer to copy to
    /// * `target_area` - The area in the target buffer to copy to
    /// * `scroll_offset` - Vertical scroll offset (lines from top)
    pub fn copy_to(&self, target: &mut Buffer, target_area: Rect, scroll_offset: u16) {
        let src_height = self.area.height;
        let dst_height = target_area.height;

        for dy in 0..dst_height {
            let src_y = scroll_offset + dy;
            if src_y >= src_height {
                break;
            }

            for dx in 0..target_area.width.min(self.area.width) {
                let src_cell = self.buffer.cell((dx, src_y));
                if let Some(cell) = src_cell {
                    if let Some(dst_cell) =
                        target.cell_mut((target_area.x + dx, target_area.y + dy))
                    {
                        *dst_cell = cell.clone();
                    }
                }
            }
        }
    }
}

/// Props for the generic ScrollView component
pub struct ScrollViewProps<F>
where
    F: Fn(Rect, &mut Buffer),
{
    /// Total content height (in lines/rows)
    pub content_height: u16,
    /// Render function that receives the full content area and buffer
    pub render_content: F,
    /// Optional block wrapper
    pub block: Option<Block<'static>>,
    /// Scroll indicator configuration
    pub indicators: ScrollIndicator,
    /// Enable keyboard navigation
    pub keyboard_nav: bool,
    /// Lines to scroll per key press
    pub scroll_step: u16,
    /// Initial scroll offset
    pub initial_offset: u16,
}

impl<F> ScrollViewProps<F>
where
    F: Fn(Rect, &mut Buffer),
{
    /// Create new ScrollViewProps with content height and render function
    ///
    /// The render function receives the full content area (which may be taller
    /// than the viewport) and should render all content as if there's no scrolling.
    pub fn new(content_height: u16, render_content: F) -> Self {
        Self {
            content_height,
            render_content,
            block: None,
            indicators: ScrollIndicator::default(),
            keyboard_nav: true,
            scroll_step: 1,
            initial_offset: 0,
        }
    }

    /// Add a block wrapper
    pub fn block(mut self, block: Block<'static>) -> Self {
        self.block = Some(block);
        self
    }

    /// Configure scroll indicators
    pub fn indicators(mut self, indicators: ScrollIndicator) -> Self {
        self.indicators = indicators;
        self
    }

    /// Enable/disable keyboard navigation
    pub fn keyboard_nav(mut self, enabled: bool) -> Self {
        self.keyboard_nav = enabled;
        self
    }

    /// Set scroll step (lines per key press)
    pub fn scroll_step(mut self, step: u16) -> Self {
        self.scroll_step = step;
        self
    }

    /// Set initial scroll offset
    pub fn initial_offset(mut self, offset: u16) -> Self {
        self.initial_offset = offset;
        self
    }
}

/// A scrollable container that works with any widget or content.
///
/// Unlike simple text scrolling, this component uses a virtual buffer approach:
/// 1. Creates an off-screen buffer sized to fit all content
/// 2. Renders your content to this virtual buffer
/// 3. Copies only the visible portion to the actual screen
///
/// This means ANY widget can be scrolled, not just text.
///
/// # Example
///
/// ```rust,ignore
/// use reratui::prelude::*;
/// use reratui::components::{ScrollView, ScrollViewProps};
///
/// struct MyScrollableContent;
///
/// impl Component for MyScrollableContent {
///     fn render(&self, area: Rect, buffer: &mut Buffer) {
///         let content_height = 100; // Total content is 100 lines tall
///         
///         let props = ScrollViewProps::new(content_height, |content_area, buf| {
///             // Render ANY widgets here - they'll be scrollable!
///             
///             // Example: render multiple paragraphs
///             for i in 0..10 {
///                 let para_area = Rect::new(
///                     content_area.x,
///                     content_area.y + i * 10,
///                     content_area.width,
///                     10,
///                 );
///                 Paragraph::new(format!("Section {}", i + 1))
///                     .block(Block::default().borders(Borders::ALL))
///                     .render(para_area, buf);
///             }
///             
///             // Or render a table, list, chart - anything!
///         })
///         .block(Block::default().borders(Borders::ALL).title("Scrollable"));
///
///         ScrollView::new(props).render(area, buffer);
///     }
/// }
/// ```
pub struct ScrollView<F>
where
    F: Fn(Rect, &mut Buffer),
{
    props: ScrollViewProps<F>,
}

impl<F> ScrollView<F>
where
    F: Fn(Rect, &mut Buffer),
{
    /// Create a new ScrollView with the given props
    pub fn new(props: ScrollViewProps<F>) -> Self {
        Self { props }
    }
}

impl<F> Component for ScrollView<F>
where
    F: Fn(Rect, &mut Buffer) + 'static,
{
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        // Calculate inner area (accounting for block borders)
        let inner_area = if let Some(ref block) = self.props.block {
            block.clone().render(area, buffer);
            block.inner(area)
        } else {
            area
        };

        // Reserve space for scrollbar if enabled
        let content_area = if self.props.indicators.show_scrollbar {
            Rect {
                width: inner_area.width.saturating_sub(1),
                ..inner_area
            }
        } else {
            inner_area
        };

        let viewport_height = content_area.height;
        let content_height = self.props.content_height;

        // Scroll state
        let (offset, set_offset) = use_state(|| self.props.initial_offset as usize);

        // Calculate max offset
        let max_offset = (content_height as usize).saturating_sub(viewport_height as usize);

        // Clamp offset if content shrinks
        let clamped_offset = offset.min(max_offset);
        if clamped_offset != offset {
            set_offset.set(clamped_offset);
        }

        // Keyboard navigation
        if self.props.keyboard_nav {
            let step = self.props.scroll_step as usize;

            use_keyboard(move |key| {
                let current = offset;
                let new_offset = match key.code {
                    KeyCode::Char('j') | KeyCode::Down => (current + step).min(max_offset),
                    KeyCode::Char('k') | KeyCode::Up => current.saturating_sub(step),
                    KeyCode::Char('g') | KeyCode::Home => 0,
                    KeyCode::Char('G') | KeyCode::End => max_offset,
                    KeyCode::PageDown => {
                        (current + (viewport_height as usize).saturating_sub(1)).min(max_offset)
                    }
                    KeyCode::PageUp => {
                        current.saturating_sub((viewport_height as usize).saturating_sub(1))
                    }
                    KeyCode::Char('d')
                        if key
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL) =>
                    {
                        (current + viewport_height as usize / 2).min(max_offset)
                    }
                    KeyCode::Char('u')
                        if key
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL) =>
                    {
                        current.saturating_sub(viewport_height as usize / 2)
                    }
                    _ => return,
                };
                set_offset.set(new_offset);
            });
        }

        // Create virtual buffer for full content
        let mut virtual_buf = VirtualBuffer::new(content_area.width, content_height);

        // Create the content area for rendering (starts at 0,0 in virtual buffer)
        let virtual_content_area = Rect::new(0, 0, content_area.width, content_height);

        // Render content to virtual buffer
        (self.props.render_content)(virtual_content_area, virtual_buf.buffer_mut());

        // Copy visible portion to actual buffer
        virtual_buf.copy_to(buffer, content_area, clamped_offset as u16);

        // Render scrollbar
        if self.props.indicators.show_scrollbar && content_height > viewport_height {
            let scrollbar_area = Rect {
                x: inner_area.x + inner_area.width.saturating_sub(1),
                y: inner_area.y,
                width: 1,
                height: inner_area.height,
            };

            // ScrollbarState: position goes from 0 to max_offset
            let mut scrollbar_state =
                ScrollbarState::new(max_offset.max(1)).position(clamped_offset);

            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .track_style(Style::default().fg(self.props.indicators.track_color))
                .thumb_style(Style::default().fg(self.props.indicators.thumb_color));

            StatefulWidget::render(scrollbar, scrollbar_area, buffer, &mut scrollbar_state);
        }

        // Render "more above" indicator
        if self.props.indicators.show_more_above && clamped_offset > 0 {
            let indicator = "▲";
            let x = content_area.x + content_area.width.saturating_sub(2);
            buffer.set_string(
                x,
                content_area.y,
                indicator,
                Style::default().fg(Color::Yellow),
            );
        }

        // Render "more below" indicator
        if self.props.indicators.show_more_below && clamped_offset < max_offset {
            let indicator = "▼";
            let y = content_area.y + content_area.height.saturating_sub(1);
            let x = content_area.x + content_area.width.saturating_sub(2);
            buffer.set_string(x, y, indicator, Style::default().fg(Color::Yellow));
        }
    }
}

// ============================================================================
// Callback-based ScrollView (simpler, for item lists)
// ============================================================================

/// Props for callback-based ScrollView (for item lists)
pub struct ScrollViewItemProps<F>
where
    F: Fn(Rect, &mut Buffer, usize, usize),
{
    /// Total number of items/lines in the content
    pub item_count: usize,
    /// Render function that receives (area, buffer, start_index, visible_count)
    pub render_items: F,
    /// Optional block wrapper
    pub block: Option<Block<'static>>,
    /// Scroll indicator configuration
    pub indicators: ScrollIndicator,
    /// Enable keyboard navigation
    pub keyboard_nav: bool,
    /// Lines to scroll per key press
    pub scroll_step: usize,
    /// Optional externally controlled scroll offset.
    ///
    /// When set, the component becomes read-only with respect to scroll
    /// position and uses this offset instead of its internal `use_state`.
    pub controlled_offset: Option<usize>,
}

impl<F> ScrollViewItemProps<F>
where
    F: Fn(Rect, &mut Buffer, usize, usize),
{
    /// Create new props for item-based scrolling
    pub fn new(item_count: usize, render_items: F) -> Self {
        Self {
            item_count,
            render_items,
            block: None,
            indicators: ScrollIndicator::default(),
            keyboard_nav: true,
            scroll_step: 1,
            controlled_offset: None,
        }
    }

    /// Add a block wrapper
    pub fn block(mut self, block: Block<'static>) -> Self {
        self.block = Some(block);
        self
    }

    /// Configure scroll indicators
    pub fn indicators(mut self, indicators: ScrollIndicator) -> Self {
        self.indicators = indicators;
        self
    }

    /// Enable/disable keyboard navigation
    pub fn keyboard_nav(mut self, enabled: bool) -> Self {
        self.keyboard_nav = enabled;
        self
    }

    /// Set scroll step
    pub fn scroll_step(mut self, step: usize) -> Self {
        self.scroll_step = step;
        self
    }

    /// Supply an external scroll offset instead of using internal state.
    pub fn controlled_offset(mut self, offset: usize) -> Self {
        self.controlled_offset = Some(offset);
        self
    }
}

/// Item-based scrollable container (simpler API for lists).
///
/// Use this when you have a list of items where each item is one line.
/// For more complex layouts with multi-line widgets, use `ScrollView`.
///
/// # Example
///
/// ```rust,ignore
/// let props = ScrollViewItemProps::new(items.len(), |area, buf, start, count| {
///     for (i, item) in items.iter().skip(start).take(count).enumerate() {
///         let y = area.y + i as u16;
///         buf.set_string(area.x, y, item, Style::default());
///     }
/// });
/// ScrollViewItems::new(props).render(area, buffer);
/// ```
pub struct ScrollViewItems<F>
where
    F: Fn(Rect, &mut Buffer, usize, usize),
{
    props: ScrollViewItemProps<F>,
}

impl<F> ScrollViewItems<F>
where
    F: Fn(Rect, &mut Buffer, usize, usize),
{
    /// Create a new ScrollViewItems
    pub fn new(props: ScrollViewItemProps<F>) -> Self {
        Self { props }
    }
}

impl<F> Component for ScrollViewItems<F>
where
    F: Fn(Rect, &mut Buffer, usize, usize) + 'static,
{
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let inner_area = if let Some(ref block) = self.props.block {
            block.clone().render(area, buffer);
            block.inner(area)
        } else {
            area
        };

        let content_area = if self.props.indicators.show_scrollbar {
            Rect {
                width: inner_area.width.saturating_sub(1),
                ..inner_area
            }
        } else {
            inner_area
        };

        let viewport_height = content_area.height as usize;
        let content_height = self.props.item_count;

        let controlled_offset = self.props.controlled_offset;
        let (offset, set_offset) = use_state(|| controlled_offset.unwrap_or(0usize));
        let max_offset = content_height.saturating_sub(viewport_height);

        let raw_offset = controlled_offset.unwrap_or(offset);
        let clamped_offset = raw_offset.min(max_offset);
        if controlled_offset.is_none() && clamped_offset != offset {
            set_offset.set(clamped_offset);
        }

        if self.props.keyboard_nav && controlled_offset.is_none() {
            let step = self.props.scroll_step;

            use_keyboard(move |key| {
                let current = offset;
                let new_offset = match key.code {
                    KeyCode::Char('j') | KeyCode::Down => (current + step).min(max_offset),
                    KeyCode::Char('k') | KeyCode::Up => current.saturating_sub(step),
                    KeyCode::Char('g') | KeyCode::Home => 0,
                    KeyCode::Char('G') | KeyCode::End => max_offset,
                    KeyCode::PageDown => {
                        (current + viewport_height.saturating_sub(1)).min(max_offset)
                    }
                    KeyCode::PageUp => current.saturating_sub(viewport_height.saturating_sub(1)),
                    _ => return,
                };
                set_offset.set(new_offset);
            });
        }

        let visible_count = viewport_height.min(content_height.saturating_sub(clamped_offset));
        (self.props.render_items)(content_area, buffer, clamped_offset, visible_count);

        // Scrollbar
        if self.props.indicators.show_scrollbar && content_height > viewport_height {
            let scrollbar_area = Rect {
                x: inner_area.x + inner_area.width.saturating_sub(1),
                y: inner_area.y,
                width: 1,
                height: inner_area.height,
            };

            let mut scrollbar_state =
                ScrollbarState::new(max_offset.max(1)).position(clamped_offset);

            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .track_style(Style::default().fg(self.props.indicators.track_color))
                .thumb_style(Style::default().fg(self.props.indicators.thumb_color));

            StatefulWidget::render(scrollbar, scrollbar_area, buffer, &mut scrollbar_state);
        }

        // Indicators
        if self.props.indicators.show_more_above && clamped_offset > 0 {
            buffer.set_string(
                content_area.x + content_area.width.saturating_sub(2),
                content_area.y,
                "▲",
                Style::default().fg(Color::Yellow),
            );
        }

        if self.props.indicators.show_more_below && clamped_offset < max_offset {
            buffer.set_string(
                content_area.x + content_area.width.saturating_sub(2),
                content_area.y + content_area.height.saturating_sub(1),
                "▼",
                Style::default().fg(Color::Yellow),
            );
        }
    }
}
