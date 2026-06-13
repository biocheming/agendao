# Examples Guide

This guide provides an overview of the example applications included with Reratui.

## Running Examples

All examples can be run from the repository root:

```bash
cargo run --example <example_name>
```

## Example Overview

| Example               | Description           | Key Concepts                    |
| --------------------- | --------------------- | ------------------------------- |
| `counter_fiber`       | Basic counter         | State, keyboard events          |
| `effect_timing`       | Effect lifecycle      | Effects, cleanup                |
| `async_fetch_example` | Async data fetching   | `use_future`, async             |
| `query_example`       | Data queries          | `use_query`, caching            |
| `mutation_example`    | Mutations             | `use_mutation`, `use_reducer`   |
| `data_fetcher`        | Multiple data sources | Multiple futures, refresh       |
| `events_showcase`     | Event handling        | Keyboard, mouse events          |
| `command_palette`     | Complex UI            | Context, intervals, composition |

---

## counter_fiber

A simple counter demonstrating basic state management.

```bash
cargo run --example counter_fiber
```

**Key Concepts:**

- `use_state` for state management
- `use_keyboard_press` for input handling
- Basic component structure

**Controls:**

- `Space` - Increment counter
- `q` - Quit

**Code Highlights:**

```rust
struct Counter;

impl Component for Counter {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (count, set_count) = use_state(|| 0);

        use_keyboard_press(move |key| {
            match key.code {
                KeyCode::Char(' ') => set_count.update(|c| c + 1),
                KeyCode::Char('q') => request_exit(),
                _ => {}
            }
        });

        Paragraph::new(format!("Count: {}", count))
            .alignment(Alignment::Center)
            .render(area, buffer);
    }
}
```

---

## effect_timing

Demonstrates effect lifecycle and cleanup.

```bash
cargo run --example effect_timing
```

**Key Concepts:**

- `use_effect` with dependencies
- `use_effect_once` for mount effects
- Effect cleanup functions
- Effect execution order

**What to Observe:**

- Effects run after render
- Cleanup runs before next effect
- Dependency changes trigger re-run

---

## async_fetch_example

Shows async data fetching patterns.

```bash
cargo run --example async_fetch_example
```

**Key Concepts:**

- `use_future` for async operations
- Loading states
- Error handling
- Refetching data

**Controls:**

- `r` - Refresh data
- `q` - Quit

---

## query_example

GitHub repository search with caching.

```bash
cargo run --example query_example
```

**Key Concepts:**

- `use_query` for data fetching
- Query caching and stale-while-revalidate
- Retry logic with exponential backoff
- Cache invalidation

**Controls:**

- `1-4` - Change search language
- `r` - Refetch
- `c` - Clear cache
- `Ctrl+Q` - Quit

**Code Highlights:**

```rust
let query_options = QueryOptions {
    enabled: true,
    stale_time: Duration::from_secs(30),
    cache_time: Duration::from_secs(300),
    retry: true,
    retry_attempts: 3,
    ..Default::default()
};

let query_result = use_query(
    current_query.clone(),
    move || async move { search_github_repos(&query).await },
    Some(query_options),
);
```

---

## mutation_example

User management with mutations and reducer.

```bash
cargo run --example mutation_example
```

**Key Concepts:**

- `use_mutation` for CRUD operations
- `use_reducer` for complex state
- Mutation status tracking
- Success/error handling

**Controls:**

- `n` - New user (opens form)
- `d` - Delete selected user
- `↑/↓` - Navigate list
- `x` - Reset mutations
- `Ctrl+Q` - Quit

**In Form:**

- Type to enter name
- `1` - Toggle role
- `2` - Auto-generate email
- `c` - Create user
- `Esc` - Cancel

**Code Highlights:**

```rust
// Reducer for form state
fn form_reducer(state: &FormState, action: FormAction) -> FormState {
    match action {
        FormAction::Open => FormState { is_open: true, ..state.clone() },
        FormAction::SetName(name) => FormState { name, ..state.clone() },
        FormAction::Submit => FormState::default(),
        // ...
    }
}

let (form_state, form_dispatch) = use_reducer(form_reducer, FormState::default());

// Mutation with retry
let create_mutation = use_mutation(
    |request: CreateUserRequest| async move { create_user_api(request).await },
    Some(MutationOptions {
        retry: true,
        retry_attempts: 3,
        retry_exponential_backoff: true,
        ..Default::default()
    }),
);
```

---

## data_fetcher

Multiple async data sources with individual refresh.

```bash
cargo run --example data_fetcher
```

**Key Concepts:**

- Multiple `use_future` hooks
- Individual and global refresh
- Progress tracking
- Parallel data fetching

**Controls:**

- `r` - Refresh all
- `1-4` - Refresh individual sources
- `q` - Quit

**Code Highlights:**

```rust
// Multiple independent futures
let user_data = use_future(fetch_user_data, Some((refresh_count, user_refresh)));
let weather_data = use_future(fetch_weather_data, Some((refresh_count, weather_refresh)));
let stats_data = use_future(fetch_stats, Some((refresh_count, stats_refresh)));
let notifications = use_future(fetch_notifications, Some((refresh_count, notif_refresh)));

// Track overall progress
let completed = [&user_data, &weather_data, &stats_data, &notifications]
    .iter()
    .filter(|h| matches!(h.state(), FutureState::Resolved(_)))
    .count();
```

---

## events_showcase

Comprehensive event handling demonstration.

```bash
cargo run --example events_showcase
```

**Key Concepts:**

- `use_event` for raw events
- Keyboard event details
- Mouse tracking
- Terminal resize handling

**Controls:**

- Any key - Shows key info
- Mouse - Shows position and events
- `q` or `Esc` - Quit

**Code Highlights:**

```rust
if let Some(event) = use_event() {
    match event {
        Event::Key(KeyEvent { code, modifiers, kind, .. }) => {
            if kind == KeyEventKind::Press {
                // Handle key press
            }
        }
        Event::Mouse(mouse) => {
            set_mouse_pos.set((mouse.column, mouse.row));
            match mouse.kind {
                MouseEventKind::Down(button) => { /* click */ }
                MouseEventKind::Moved => { /* move */ }
                MouseEventKind::ScrollUp => { /* scroll */ }
                _ => {}
            }
        }
        Event::Resize(w, h) => {
            // Terminal resized
        }
        _ => {}
    }
}
```

---

## command_palette

Complex application with multiple components.

```bash
cargo run --example command_palette
```

**Key Concepts:**

- Component composition
- Context providers (theme)
- `use_interval` for periodic updates
- Command registration pattern
- Keyboard shortcuts

**Controls:**

- `Ctrl+P` - Open command palette
- `Ctrl+E` - Toggle edit mode
- `Ctrl+V` - Toggle view mode
- `Ctrl+C` - Quit

**In Command Palette:**

- Type to filter
- `↑/↓` - Navigate
- `Enter` - Execute
- `Esc` - Close

**Architecture:**

```
CommandPaletteApp
├── Header
│   ├── MenuBar
│   ├── ConnectionStatus
│   └── Marquee
├── MessageList
├── DebugPanel
├── HelpBar
└── CommandPaletteComponent (overlay)
```

**Code Highlights:**

```rust
// Theme context
use_context_provider(|| theme.clone());

// Command registration
palette.register("greet", "👋 Display greeting", move || {
    set_messages.update(|msgs| {
        msgs.push(Message::new("Hello!"));
        msgs
    });
});

// Periodic status updates
use_interval(
    move || {
        set_connection_status.update(|status| match status {
            ConnectionStatus::Connected => ConnectionStatus::Connecting,
            ConnectionStatus::Connecting => ConnectionStatus::Disconnected,
            ConnectionStatus::Disconnected => ConnectionStatus::Connected,
        });
    },
    5000,
);
```

---

## Creating Your Own Examples

### Minimal Template

```rust
use reratui::prelude::*;

struct App;

impl Component for App {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        // Your component logic here
        Paragraph::new("Hello, Reratui!")
            .render(area, buffer);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    render(|| App).await?;
    Ok(())
}
```

### With State and Events

```rust
use reratui::prelude::*;

struct App;

impl Component for App {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (state, set_state) = use_state(|| "Initial".to_string());

        use_keyboard_press(move |key| {
            match key.code {
                KeyCode::Char('1') => set_state.set("State 1".to_string()),
                KeyCode::Char('2') => set_state.set("State 2".to_string()),
                KeyCode::Char('q') => request_exit(),
                _ => {}
            }
        });

        let block = Block::default()
            .title("My App")
            .borders(Borders::ALL);

        Paragraph::new(state)
            .block(block)
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

### With Async Data

```rust
use reratui::prelude::*;
use reratui::hooks::{use_query, QueryStatus};

struct App;

impl Component for App {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let query = use_query(
            "data",
            || async { fetch_data().await },
            None,
        );

        let content = match query.status {
            QueryStatus::Loading => "Loading...".to_string(),
            QueryStatus::Success => format!("Data: {:?}", query.data),
            QueryStatus::Error => format!("Error: {:?}", query.error),
            _ => "Idle".to_string(),
        };

        Paragraph::new(content).render(area, buffer);
    }
}

async fn fetch_data() -> Result<String, String> {
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    Ok("Hello from async!".to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    render(|| App).await?;
    Ok(())
}
```

---

## Tips for Learning

1. **Start Simple** - Begin with `counter_fiber` to understand basics
2. **Add Complexity Gradually** - Move to `effect_timing` for effects
3. **Explore Async** - Try `async_fetch_example` then `query_example`
4. **Study Complex Apps** - Examine `command_palette` for architecture patterns
5. **Experiment** - Modify examples to test your understanding
