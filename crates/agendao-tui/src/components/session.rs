use parking_lot::Mutex;
#[cfg(test)]
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use agendao_command_render::terminal_presentation::{
    compose_assistant_segments, TerminalAssistantSegment, TerminalMessage, TerminalMessagePart,
    TerminalMessageRole, TerminalToolResultInfo,
};
use agendao_command_render::terminal_tool_block_display::{build_file_items, build_image_items};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Widget,
    },
};
use reratui::element::Element;
use reratui::fiber_tree::with_current_fiber;
use reratui::hooks::{
    stop_propagation, use_context, use_keyboard_press, use_memo, use_mouse, use_ref, use_state,
    StateSetter,
};
use reratui::components::VirtualBuffer;
use reratui::{Buffer, Component};

use super::message_palette;
use super::shared_block_items::render_shared_message_block_items;
use super::sidebar::SidebarRenderState;
use crate::components::{
    Prompt, Sidebar, SidebarChromeMode, SidebarChromeProps, SidebarRenderInputs,
};
use crate::context::{
    AppContext, Message, MessagePart, MessageRole, RevertInfo, SidebarLifecycleState, SidebarMode,
};
use crate::ui::{BufferSurface, RenderSurface};
use crossterm::event::{MouseButton, MouseEventKind};

include!("session/state.rs");
include!("session/render.rs");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionLeftMouseDownOutcome {
    Consumed,
    BeginSelection { area: Rect },
    ClearSelection,
}

include!("session/view.rs");

#[cfg(test)]
mod tests {
    include!("session/tests.rs");
}
