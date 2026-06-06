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
    collect_assistant_tool_results, compose_assistant_segments, is_tool_result_carrier,
    TerminalAssistantSegment, TerminalMessage, TerminalMessagePart, TerminalMessageRole,
    TerminalToolResultInfo,
};
use agendao_command_render::terminal_tool_block_display::{build_file_items, build_image_items};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
};
use reratui::element::Element;
use reratui::fiber_tree::with_current_fiber;
use reratui::hooks::{use_context, use_memo, use_state, StateSetter};
use reratui::{Buffer, Component};

use super::message_palette;
use super::shared_block_items::render_shared_message_block_items;
use super::sidebar::SidebarRenderState;
use crate::bridge::{ReactiveAppContextHandle, ReactiveSessionContext};
use crate::components::{Prompt, Sidebar};
use crate::context::{
    AppContext, Message, MessagePart, MessageRole, RevertInfo, SidebarLifecycleState, SidebarMode,
};
use crate::ui::{BufferSurface, RenderSurface};

include!("session/state.rs");
include!("session/render.rs");
include!("session/view.rs");

#[cfg(test)]
mod tests {
    include!("session/tests.rs");
}
