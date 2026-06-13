//! # Reratui - A Modern, Reactive TUI Framework for Rust
//!
//! Reratui brings React-inspired component architecture and hooks to terminal user interfaces,
//! enabling developers to build complex, interactive TUI applications with clean, maintainable code.
//!
//! ## Features
//!
//! - **Fiber Architecture** - React-like fiber system with proper effect timing and state batching
//! - **Component-Based Architecture** - Build modular UIs with reusable components
//! - **Hooks System** - Manage state and side effects with React-like hooks
//! - **Async-First** - Built on Tokio with first-class async/await support
//! - **Cross-Thread State Updates** - Background tasks can safely update UI state
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use reratui::prelude::*;
//!
//! struct Counter;
//!
//! impl Component for Counter {
//!     fn render(&self, area: Rect, buffer: &mut Buffer) {
//!         let (count, set_count) = use_state(|| 0);
//!
//!         if let Some(Event::Key(KeyEvent { code, kind: KeyEventKind::Press, .. })) = use_event() {
//!             match code {
//!                 KeyCode::Char('j') => set_count.update(|n| n + 1),
//!                 KeyCode::Char('k') => set_count.update(|n| n - 1),
//!                 KeyCode::Char('q') => request_exit(),
//!                 _ => {}
//!             }
//!         }
//!
//!         let block = Block::default()
//!             .title("Counter")
//!             .borders(Borders::ALL);
//!         let paragraph = Paragraph::new(format!("Count: {}", count))
//!             .alignment(Alignment::Center)
//!             .block(block);
//!         paragraph.render(area, buffer);
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     render(|| Counter).await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Available Hooks
//!
//! - [`use_state`](hooks::use_state) - Local component state with batching
//! - [`use_reducer`](hooks::use_reducer) - Complex state with actions
//! - [`use_effect`](hooks::use_effect) - Side effects with proper post-commit timing
//! - [`use_context`](hooks::use_context) - Share data across components
//! - [`use_ref`](hooks::use_ref) - Mutable references
//! - [`use_callback`](hooks::use_callback) - Memoized callbacks
//! - [`use_memo`](hooks::use_memo) - Memoized values
//! - [`use_event`](hooks::use_event) - Terminal event handling
//! - [`use_interval`](hooks::use_interval) - Periodic callbacks
//! - [`use_timeout`](hooks::use_timeout) - Delayed callbacks
//!
//! ## Component Pattern
//!
//! Implement the `Component` trait for your components:
//!
//! ```rust,no_run
//! use reratui::prelude::*;
//!
//! struct MyComponent {
//!     title: String,
//! }
//!
//! impl Component for MyComponent {
//!     fn render(&self, area: Rect, buffer: &mut Buffer) {
//!         let (state, set_state) = use_state(|| 0);
//!         
//!         // Custom layout logic
//!         let chunks = Layout::default()
//!             .direction(Direction::Vertical)
//!             .constraints([Constraint::Length(3), Constraint::Min(0)])
//!             .split(area);
//!         
//!         // Render widgets directly
//!         let paragraph = Paragraph::new(self.title.clone());
//!         paragraph.render(chunks[0], buffer);
//!     }
//! }
//! ```
//!
//! ## Key Types
//!
//! - [`FiberId`] - Unique identifier for a component instance
//! - [`Fiber`] - A mounted component instance with its own hook state
//! - [`FiberTree`] - Global fiber tree tracking all mounted components
//! - [`Component`] - Trait for implementing components
//!
//! ## Examples
//!
//! See the [`examples/`](https://github.com/sabry-awad97/reratui/tree/main/examples) directory for:
//!
//! - **counter_fiber** - Basic state management and event handling
//! - **command_palette** - Complex UI with animations and keyboard navigation

// Core fiber types
mod fiber;
pub mod fiber_tree;

// Consolidated render context
pub mod render_context;

// Element types (virtual DOM)
pub mod element;

// Event system
pub mod event;

// Global event handlers
pub mod global_events;

// Panic handler
pub mod panic_handler;

// Component trait and related types
mod component;

// Context management
pub mod context_stack;

// Hooks (React-like APIs)
pub mod hooks;

// Scheduler (batching, effects, reconciliation)
pub mod scheduler;

// Runtime (render loop)
mod runtime;

// Strict mode for development
mod strict_mode;

// Built-in components
pub mod components;

// Re-exports for public API
pub use component::{Component, ComponentArea, reset_component_position_counter};
pub use context_stack::ContextStack;
pub use element::{Element, RenderableComponent};
pub use event::{
    clear_current_event, get_current_event, peek_current_event, set_current_event,
    stop_event_propagation,
};
pub use fiber::{
    AsyncCleanupFn, AsyncEffectFn, AsyncEffectFuture, AsyncPendingEffect, CleanupFn, Fiber,
    FiberId, PendingEffect,
};
pub use fiber_tree::FiberTree;
pub use global_events::{clear_global_handlers, on_global_event, process_global_event};
pub use panic_handler::setup_panic_handler;
pub use render_context::{
    RenderContext, clear_render_context, init_render_context, is_render_context_initialized,
    with_render_context, with_render_context_mut,
};
pub use runtime::{
    RenderOptions, is_in_render_phase, render, render_with_options, request_exit, reset_exit,
    should_exit, warn_if_effect_during_render,
};
pub use strict_mode::{StrictMode, is_strict_mode_enabled, set_strict_mode_enabled};

// Re-export crossterm event types for convenience
pub use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

// Re-export ratatui for standalone usage (no need for separate ratatui dependency)
pub use ratatui;

// Re-export commonly used ratatui types at crate root for convenience
pub use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Padding, Paragraph, Widget, Wrap},
};

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::component::{Component, ComponentArea};
    pub use crate::context_stack::ContextStack;
    pub use crate::element::{Element, RenderableComponent};
    pub use crate::fiber::{Fiber, FiberId};
    pub use crate::fiber_tree::FiberTree;
    pub use crate::render_context::{
        RenderContext, clear_render_context, init_render_context, is_render_context_initialized,
        with_render_context, with_render_context_mut,
    };
    pub use crate::runtime::{
        RenderOptions, is_in_render_phase, render, render_with_options, request_exit, should_exit,
    };
    pub use crate::strict_mode::{StrictMode, is_strict_mode_enabled, set_strict_mode_enabled};

    // Re-export hooks
    pub use crate::hooks::{
        Dispatch, EffectEvent, HistoryHandle, IntervalHandle, Ref, ScrollHandle, StateSetter,
        TimeoutHandle, peek_event, stop_propagation, try_use_context, use_async_effect,
        use_async_effect_once, use_callback, use_context, use_context_provider, use_effect,
        use_effect_event, use_effect_once, use_event, use_history, use_id, use_interval, use_memo,
        use_reducer, use_ref, use_scroll, use_scroll_keyboard, use_state, use_timeout,
    };

    // Re-export components
    pub use crate::components::{ScrollIndicator, ScrollView, ScrollViewProps};

    // Re-export crossterm event types
    pub use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    // Re-export ratatui types for standalone usage
    pub use ratatui::{
        self,
        buffer::Buffer,
        layout::{Alignment, Constraint, Direction, Layout, Rect},
        style::{Color, Modifier, Style, Stylize},
        text::{Line, Span, Text},
        widgets::{Block, Borders, Padding, Paragraph, Widget, Wrap},
    };
}
