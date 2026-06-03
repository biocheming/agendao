use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{StatefulWidget, Widget},
    Frame,
};

pub trait RenderSurface {
    fn render_widget<W: Widget>(&mut self, widget: W, area: Rect);

    fn render_stateful_widget<W: StatefulWidget>(
        &mut self,
        widget: W,
        area: Rect,
        state: &mut W::State,
    );

    // Cursor placement is an optional capability: interactive terminal
    // surfaces can honor it, while offscreen/test surfaces may safely ignore
    // it and still reuse the same rendering code.
    fn set_cursor_position(&mut self, _x: u16, _y: u16) {}
}

impl RenderSurface for Frame<'_> {
    fn render_widget<W: Widget>(&mut self, widget: W, area: Rect) {
        Frame::render_widget(self, widget, area);
    }

    fn render_stateful_widget<W: StatefulWidget>(
        &mut self,
        widget: W,
        area: Rect,
        state: &mut W::State,
    ) {
        Frame::render_stateful_widget(self, widget, area, state);
    }

    fn set_cursor_position(&mut self, x: u16, y: u16) {
        Frame::set_cursor_position(self, (x, y));
    }
}

pub struct BufferSurface<'a> {
    buffer: &'a mut Buffer,
    cursor_position: Option<(u16, u16)>,
}

impl<'a> BufferSurface<'a> {
    pub fn new(buffer: &'a mut Buffer) -> Self {
        Self {
            buffer,
            cursor_position: None,
        }
    }

    pub fn buffer_mut(&mut self) -> &mut Buffer {
        self.buffer
    }

    pub fn cursor_position(&self) -> Option<(u16, u16)> {
        self.cursor_position
    }
}

impl RenderSurface for BufferSurface<'_> {
    fn render_widget<W: Widget>(&mut self, widget: W, area: Rect) {
        widget.render(area, self.buffer);
    }

    fn render_stateful_widget<W: StatefulWidget>(
        &mut self,
        widget: W,
        area: Rect,
        state: &mut W::State,
    ) {
        widget.render(area, self.buffer, state);
    }

    fn set_cursor_position(&mut self, x: u16, y: u16) {
        self.cursor_position = Some((x, y));
    }
}
