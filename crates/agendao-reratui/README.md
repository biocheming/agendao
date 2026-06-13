# Reratui

A React-inspired fiber-based TUI framework for Rust, built on top of [ratatui](https://github.com/ratatui-org/ratatui).

[![Crates.io](https://img.shields.io/crates/v/reratui.svg)](https://crates.io/crates/reratui)
[![Documentation](https://docs.rs/reratui/badge.svg)](https://docs.rs/reratui)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](#license)

## Overview

Reratui brings React's component model and hooks system to terminal user interfaces. It features a fiber-based architecture that enables efficient rendering, proper state management, and a familiar development experience for those coming from React.

### Key Features

- **React-like Component Model** - Define components using the `Component` trait
- **Fiber Architecture** - Efficient reconciliation and rendering pipeline
- **Comprehensive Hooks System** - State, effects, context, refs, memoization, and more
- **Async Support** - First-class async effects, queries, and mutations
- **Event Handling** - Keyboard, mouse, and terminal resize events
- **Built on ratatui** - Full access to ratatui's powerful widget system

## Quick Start

Add reratui to your `Cargo.toml`:

```toml
[dependencies]
reratui = "0.2"
tokio = { version = "1", features = ["full"] }
```

### Hello World

```rust
use reratui::prelude::*;

struct App;

impl Component for App {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (count, set_count) = use_state(|| 0);

        // Handle keyboard events
        use_keyboard_press(move |key| {
            match key.code {
                KeyCode::Char(' ') => set_count.update(|c| c + 1),
                KeyCode::Char('q') => request_exit(),
                _ => {}
            }
        });

        // Render UI
        let text = format!("Count: {} (Space to increment, Q to quit)", count);
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .render(area, buffer);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    render(|| App).await?;
    Ok(())
}
```

## Architecture

Reratui uses a 5-phase render pipeline inspired by React's fiber architecture:

1. **Poll** - Wait for terminal events or scheduled updates
2. **Render** - Execute component render functions, collecting state changes
3. **Commit** - Apply batched state updates to the fiber tree
4. **Event** - Process terminal events (keyboard, mouse, resize)
5. **Effect** - Run effects and cleanup functions

### Fiber System

Each component instance is represented by a `Fiber` node that maintains:

- Hook state (useState, useEffect, etc.)
- Pending effects and cleanup functions
- Context values
- Parent/child relationships
- Dirty flags for re-rendering

## Hooks Reference

### State Management

| Hook          | Description                                |
| ------------- | ------------------------------------------ |
| `use_state`   | Local component state with batched updates |
| `use_reducer` | Complex state with reducer pattern         |
| `use_ref`     | Mutable reference without re-renders       |
| `use_history` | State with undo/redo support               |

### Effects

| Hook                    | Description                           |
| ----------------------- | ------------------------------------- |
| `use_effect`            | Side effects with dependency tracking |
| `use_effect_once`       | Effect that runs only on mount        |
| `use_async_effect`      | Async effects with cleanup            |
| `use_async_effect_once` | Async effect that runs only on mount  |

### Context

| Hook                   | Description                  |
| ---------------------- | ---------------------------- |
| `use_context`          | Consume context from parent  |
| `use_context_provider` | Provide context to children  |
| `try_use_context`      | Optional context consumption |

### Memoization

| Hook           | Description                    |
| -------------- | ------------------------------ |
| `use_memo`     | Memoize expensive computations |
| `use_callback` | Memoize callback functions     |

### Async Data

| Hook           | Description                |
| -------------- | -------------------------- |
| `use_future`   | Track async task state     |
| `use_query`    | Data fetching with caching |
| `use_mutation` | Mutation state tracking    |

### Events

| Hook                    | Description                      |
| ----------------------- | -------------------------------- |
| `use_event`             | Access current terminal event    |
| `use_keyboard`          | Handle all keyboard events       |
| `use_keyboard_press`    | Handle key press events only     |
| `use_keyboard_shortcut` | Handle specific key combinations |
| `use_mouse`             | Handle all mouse events          |
| `use_mouse_click`       | Handle mouse clicks              |
| `use_mouse_hover`       | Track hover state over area      |
| `use_mouse_drag`        | Track drag operations            |

### Timing

| Hook           | Description                  |
| -------------- | ---------------------------- |
| `use_timeout`  | Execute callback after delay |
| `use_interval` | Execute callback repeatedly  |

### Layout

| Hook              | Description                 |
| ----------------- | --------------------------- |
| `use_area`        | Get component's render area |
| `use_frame`       | Access frame context        |
| `use_resize`      | Track terminal dimensions   |
| `use_media_query` | Responsive breakpoints      |

### Forms

| Hook               | Description               |
| ------------------ | ------------------------- |
| `use_form`         | Form state and validation |
| `use_form_context` | Access form from children |
| `use_watch`        | Watch form field changes  |

## State Setter API

The `StateSetter` returned by `use_state` provides several methods:

```rust
let (count, set_count) = use_state(|| 0);

// Direct set
set_count.set(5);

// Update with function
set_count.update(|c| c + 1);

// Conditional updates (only trigger re-render if value changes)
set_count.set_if_changed(5);
set_count.update_if_changed(|c| c + 1);
```

## Examples

The repository includes several examples demonstrating various features:

- **counter_fiber** - Basic counter with state
- **effect_timing** - Effect lifecycle demonstration
- **async_fetch_example** - Async data fetching
- **query_example** - Data queries with caching
- **mutation_example** - Mutations with reducer pattern
- **events_showcase** - Keyboard and mouse events
- **command_palette** - Complex UI with multiple components
- **data_fetcher** - Multiple async data sources

Run an example:

```bash
cargo run --example counter_fiber
```

## Runtime Functions

```rust
// Start the application
render(|| App).await?;

// With options
render_with_options(|| App, RenderOptions {
    frame_interval_ms: 16,  // ~60 FPS
    strict_mode: false,
}).await?;

// Exit control
request_exit();      // Request graceful exit
should_exit();       // Check if exit requested
reset_exit();        // Cancel exit request
```

## Strict Mode

Enable strict mode for development to catch common issues:

```rust
render_with_options(|| App, RenderOptions {
    strict_mode: true,
    ..Default::default()
}).await?;
```

Strict mode helps detect:

- Conditional hook calls
- Hook order changes between renders
- Missing effect dependencies

See [STRICT_MODE.md](STRICT_MODE.md) for details.

## Requirements

- Rust 1.85.0 or later (edition 2024)
- Tokio runtime for async support

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](../../LICENSE-APACHE))
- MIT license ([LICENSE-MIT](../../LICENSE-MIT))

at your option.

## Contributing

See [CONTRIBUTING.md](../../CONTRIBUTING.md) for guidelines.
