# Hooks Reference

Complete API reference for all hooks in Reratui.

## Table of Contents

- [State Hooks](#state-hooks)
  - [use_state](#use_state)
  - [use_reducer](#use_reducer)
  - [use_ref](#use_ref)
  - [use_history](#use_history)
- [Effect Hooks](#effect-hooks)
  - [use_effect](#use_effect)
  - [use_effect_once](#use_effect_once)
  - [use_async_effect](#use_async_effect)
  - [use_async_effect_once](#use_async_effect_once)
- [Context Hooks](#context-hooks)
  - [use_context](#use_context)
  - [use_context_provider](#use_context_provider)
  - [try_use_context](#try_use_context)
- [Memoization Hooks](#memoization-hooks)
  - [use_memo](#use_memo)
  - [use_callback](#use_callback)
- [Async Hooks](#async-hooks)
  - [use_future](#use_future)
  - [use_future_once](#use_future_once)
  - [use_query](#use_query)
  - [use_mutation](#use_mutation)
- [Event Hooks](#event-hooks)
  - [use_event](#use_event)
  - [use_keyboard](#use_keyboard)
  - [use_keyboard_press](#use_keyboard_press)
  - [use_keyboard_shortcut](#use_keyboard_shortcut)
  - [use_mouse](#use_mouse)
  - [use_mouse_click](#use_mouse_click)
  - [use_mouse_hover](#use_mouse_hover)
  - [use_mouse_drag](#use_mouse_drag)
  - [use_mouse_position](#use_mouse_position)
  - [use_double_click](#use_double_click)
- [Timing Hooks](#timing-hooks)
  - [use_timeout](#use_timeout)
  - [use_interval](#use_interval)
- [Layout Hooks](#layout-hooks)
  - [use_area](#use_area)
  - [use_frame](#use_frame)
  - [use_resize](#use_resize)
  - [use_media_query](#use_media_query)
- [Form Hooks](#form-hooks)
  - [use_form](#use_form)
  - [use_form_context](#use_form_context)
  - [use_watch](#use_watch)
- [Utility Hooks](#utility-hooks)
  - [use_id](#use_id)

---

## State Hooks

### use_state

Manages local component state with batched updates.

```rust
fn use_state<T, F>(initializer: F) -> (T, StateSetter<T>)
where
    T: Clone + Send + Sync + PartialEq + 'static,
    F: FnOnce() -> T;
```

**Returns:** A tuple of `(current_value, setter)`

**StateSetter Methods:**

| Method                  | Description                   |
| ----------------------- | ----------------------------- |
| `set(value)`            | Set to a new value            |
| `update(fn)`            | Update using a function       |
| `set_if_changed(value)` | Set only if value differs     |
| `update_if_changed(fn)` | Update only if result differs |

**Example:**

```rust
let (count, set_count) = use_state(|| 0);

// Direct set
set_count.set(5);

// Functional update
set_count.update(|c| c + 1);

// Conditional update (avoids unnecessary re-renders)
set_count.set_if_changed(count);
```

---

### use_reducer

Manages complex state with a reducer function, similar to React's useReducer.

```rust
fn use_reducer<S, A, R>(reducer: R, initial_state: S) -> (S, Dispatch<A>)
where
    S: Clone + Send + Sync + 'static,
    A: Clone + Send + Sync + 'static,
    R: Fn(&S, A) -> S + Send + Sync + 'static;
```

**Example:**

```rust
#[derive(Clone)]
enum Action {
    Increment,
    Decrement,
    Reset,
}

fn reducer(state: &i32, action: Action) -> i32 {
    match action {
        Action::Increment => state + 1,
        Action::Decrement => state - 1,
        Action::Reset => 0,
    }
}

let (count, dispatch) = use_reducer(reducer, 0);

dispatch.dispatch(Action::Increment);
```

---

### use_ref

Creates a mutable reference that persists across renders without causing re-renders.

```rust
fn use_ref<T, F>(initializer: F) -> Ref<T>
where
    T: Clone + Send + Sync + 'static,
    F: FnOnce() -> T;
```

**Ref Methods:**

| Method       | Description                         |
| ------------ | ----------------------------------- |
| `get()`      | Get current value                   |
| `set(value)` | Set new value (no re-render)        |
| `update(fn)` | Update with function (no re-render) |

**Example:**

```rust
let render_count = use_ref(|| 0);
render_count.update(|c| c + 1);

let previous_value = use_ref(|| None);
previous_value.set(Some(current_value.clone()));
```

---

### use_history

Tracks value history with undo/redo support.

```rust
fn use_history<T, F>(initializer: F) -> HistoryHandle<T>
where
    T: Clone + Send + Sync + 'static,
    F: FnOnce() -> T;
```

**HistoryHandle Methods:**

| Method            | Description                       |
| ----------------- | --------------------------------- |
| `current()`       | Get current value                 |
| `set(value)`      | Set new value (pushes to history) |
| `undo()`          | Restore previous value            |
| `redo()`          | Restore next value                |
| `can_undo()`      | Check if undo is available        |
| `can_redo()`      | Check if redo is available        |
| `past_count()`    | Number of undo steps              |
| `future_count()`  | Number of redo steps              |
| `clear_history()` | Clear all history                 |
| `go_back(n)`      | Go back n steps                   |
| `go_forward(n)`   | Go forward n steps                |

**Example:**

```rust
let history = use_history(|| String::new());

history.set("Hello".to_string());
history.set("Hello World".to_string());

history.undo(); // Back to "Hello"
history.redo(); // Forward to "Hello World"
```

---

## Effect Hooks

### use_effect

Runs side effects after render with dependency tracking.

```rust
fn use_effect<D, F>(effect: F, deps: D)
where
    D: PartialEq + Clone + Send + Sync + 'static,
    F: FnOnce() -> Option<Box<dyn FnOnce() + Send>> + Send + 'static;
```

**Example:**

```rust
let (count, _) = use_state(|| 0);

// Effect runs when count changes
use_effect(
    move || {
        println!("Count changed to: {}", count);

        // Optional cleanup function
        Some(Box::new(|| {
            println!("Cleaning up previous effect");
        }))
    },
    count, // Dependencies
);
```

---

### use_effect_once

Runs an effect only once on mount.

```rust
fn use_effect_once<F>(effect: F)
where
    F: FnOnce() -> Option<Box<dyn FnOnce() + Send>> + Send + 'static;
```

**Example:**

```rust
use_effect_once(|| {
    println!("Component mounted!");

    Some(Box::new(|| {
        println!("Component unmounting!");
    }))
});
```

---

### use_async_effect

Runs async side effects with dependency tracking.

```rust
fn use_async_effect<D, F, Fut>(effect: F, deps: D)
where
    D: PartialEq + Clone + Send + Sync + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Option<AsyncCleanupFn>> + Send + 'static;
```

**Example:**

```rust
let (user_id, _) = use_state(|| 1);

use_async_effect(
    move || async move {
        let user = fetch_user(user_id).await;
        println!("Fetched user: {:?}", user);

        // Optional async cleanup
        Some(Box::new(|| Box::pin(async {
            println!("Cleaning up...");
        })))
    },
    user_id,
);
```

---

### use_async_effect_once

Runs an async effect only once on mount.

```rust
fn use_async_effect_once<F, Fut>(effect: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Option<AsyncCleanupFn>> + Send + 'static;
```

---

## Context Hooks

### use_context

Consumes a context value from a parent provider.

```rust
fn use_context<T>() -> T
where
    T: Clone + Send + Sync + 'static;
```

**Panics:** If no provider exists for type `T`.

---

### use_context_provider

Provides a context value to child components.

```rust
fn use_context_provider<T, F>(initializer: F)
where
    T: Clone + Send + Sync + 'static,
    F: FnOnce() -> T;
```

**Example:**

```rust
// In parent component
#[derive(Clone)]
struct Theme {
    primary: Color,
    secondary: Color,
}

use_context_provider(|| Theme {
    primary: Color::Blue,
    secondary: Color::Gray,
});

// In child component
let theme = use_context::<Theme>();
```

---

### try_use_context

Attempts to consume a context value, returning `None` if not available.

```rust
fn try_use_context<T>() -> Option<T>
where
    T: Clone + Send + Sync + 'static;
```

---

## Memoization Hooks

### use_memo

Memoizes an expensive computation.

```rust
fn use_memo<T, D, F>(compute: F, deps: D) -> T
where
    T: Clone + Send + Sync + 'static,
    D: PartialEq + Clone + Send + Sync + 'static,
    F: FnOnce() -> T;
```

**Example:**

```rust
let (items, _) = use_state(|| vec![1, 2, 3, 4, 5]);

let sum = use_memo(
    || items.iter().sum::<i32>(),
    items.clone(),
);
```

---

### use_callback

Memoizes a callback function.

```rust
fn use_callback<F, D>(callback: F, deps: D) -> Callback<F>
where
    F: Clone + Send + Sync + 'static,
    D: PartialEq + Clone + Send + Sync + 'static;
```

---

## Async Hooks

### use_future

Tracks the state of an async task.

```rust
fn use_future<T, E, F, Fut, D>(
    future_fn: F,
    deps: Option<D>,
) -> FutureHandle<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<T, E>> + Send + 'static,
    D: PartialEq + Clone + Send + Sync + 'static;
```

**FutureState:**

```rust
enum FutureState<T, E> {
    Idle,
    Pending,
    Resolved(T),
    Error(E),
}
```

**FutureHandle Methods:**

| Method          | Description             |
| --------------- | ----------------------- |
| `state()`       | Get current FutureState |
| `is_pending()`  | Check if loading        |
| `is_resolved()` | Check if completed      |
| `data()`        | Get resolved data       |
| `error()`       | Get error               |
| `refetch()`     | Trigger refetch         |

**Example:**

```rust
async fn fetch_data() -> Result<String, String> {
    Ok("Hello".to_string())
}

let handle = use_future(fetch_data, Some(()));

match handle.state() {
    FutureState::Idle => println!("Not started"),
    FutureState::Pending => println!("Loading..."),
    FutureState::Resolved(data) => println!("Data: {}", data),
    FutureState::Error(err) => println!("Error: {}", err),
}
```

---

### use_future_once

Runs an async task only once on mount.

```rust
fn use_future_once<T, E, F, Fut>(future_fn: F) -> FutureHandle<T, E>
```

---

### use_query

Data fetching with caching, stale-while-revalidate, and retry logic.

```rust
fn use_query<K, T, E, F, Fut>(
    key: K,
    query_fn: F,
    options: Option<QueryOptions>,
) -> QueryResult<T, E>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<T, E>> + Send + 'static;
```

**QueryOptions:**

```rust
struct QueryOptions {
    pub enabled: bool,           // Whether to run the query
    pub stale_time: Duration,    // Time before data is stale
    pub cache_time: Duration,    // Time to keep in cache
    pub retry: bool,             // Enable retry on failure
    pub retry_attempts: u32,     // Number of retry attempts
    pub retry_delay: Duration,   // Delay between retries
    pub retry_exponential_backoff: bool,
}
```

**QueryStatus:**

```rust
enum QueryStatus {
    Idle,
    Loading,
    Refreshing,
    Success,
    Error,
}
```

**QueryResult Fields:**

| Field      | Type          | Description           |
| ---------- | ------------- | --------------------- |
| `status`   | `QueryStatus` | Current status        |
| `data`     | `Option<T>`   | Cached data           |
| `error`    | `Option<E>`   | Error if failed       |
| `is_stale` | `bool`        | Whether data is stale |

**QueryResult Methods:**

| Method         | Description             |
| -------------- | ----------------------- |
| `refetch()`    | Force refetch           |
| `invalidate()` | Clear cache and refetch |

**Example:**

```rust
let query = use_query(
    "users",
    || async { fetch_users().await },
    Some(QueryOptions {
        stale_time: Duration::from_secs(30),
        cache_time: Duration::from_secs(300),
        retry: true,
        retry_attempts: 3,
        ..Default::default()
    }),
);

match query.status {
    QueryStatus::Loading => render_loading(),
    QueryStatus::Success => render_data(&query.data.unwrap()),
    QueryStatus::Error => render_error(&query.error.unwrap()),
    _ => {}
}
```

---

### use_mutation

Tracks mutation state for create/update/delete operations.

```rust
fn use_mutation<T, E, A, F, Fut>(
    mutation_fn: F,
    options: Option<MutationOptions>,
) -> MutationHandle<T, E, A>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    A: Clone + Send + Sync + 'static,
    F: Fn(A) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<T, E>> + Send + 'static;
```

**MutationOptions:**

```rust
struct MutationOptions {
    pub retry: bool,
    pub retry_attempts: u32,
    pub retry_delay: Duration,
    pub retry_exponential_backoff: bool,
}
```

**MutationStatus:**

```rust
enum MutationStatus {
    Idle,
    Pending,
    Success,
    Error,
}
```

**MutationHandle Methods:**

| Method         | Description               |
| -------------- | ------------------------- |
| `mutate(args)` | Execute mutation          |
| `state()`      | Get current MutationState |
| `reset()`      | Reset to idle state       |
| `cancel()`     | Cancel pending mutation   |

**Example:**

```rust
let create_user = use_mutation(
    |user: CreateUserRequest| async move {
        api::create_user(user).await
    },
    Some(MutationOptions {
        retry: true,
        retry_attempts: 3,
        ..Default::default()
    }),
);

// Execute mutation
create_user.mutate(CreateUserRequest {
    name: "John".to_string(),
    email: "john@example.com".to_string(),
});

// Check state
let state = create_user.state();
if state.is_success {
    println!("User created: {:?}", state.data);
}
```

---

## Event Hooks

### use_event

Returns the current terminal event, if any.

```rust
fn use_event() -> Option<Event>
```

**Example:**

```rust
if let Some(Event::Key(key)) = use_event() {
    if key.code == KeyCode::Char('q') {
        request_exit();
    }
}
```

---

### use_keyboard

Handles all keyboard events.

```rust
fn use_keyboard<F>(handler: F)
where
    F: Fn(KeyEvent) + Send + Sync + 'static;
```

---

### use_keyboard_press

Handles only key press events (filters out release/repeat).

```rust
fn use_keyboard_press<F>(handler: F)
where
    F: Fn(KeyEvent) + Send + Sync + 'static;
```

**Example:**

```rust
use_keyboard_press(move |key| {
    match key.code {
        KeyCode::Up => set_selected.update(|s| s.saturating_sub(1)),
        KeyCode::Down => set_selected.update(|s| s + 1),
        KeyCode::Enter => handle_select(),
        KeyCode::Char('q') => request_exit(),
        _ => {}
    }
});
```

---

### use_keyboard_shortcut

Handles specific key combinations.

```rust
fn use_keyboard_shortcut<F>(
    key_code: KeyCode,
    modifiers: KeyModifiers,
    handler: F,
)
where
    F: Fn() + Send + Sync + 'static;
```

**Example:**

```rust
// Ctrl+S to save
use_keyboard_shortcut(
    KeyCode::Char('s'),
    KeyModifiers::CONTROL,
    || save_document(),
);

// Ctrl+Shift+P for command palette
use_keyboard_shortcut(
    KeyCode::Char('p'),
    KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    || open_command_palette(),
);
```

---

### use_mouse

Handles all mouse events.

```rust
fn use_mouse<F>(handler: F)
where
    F: Fn(MouseEvent) + Send + Sync + 'static;
```

---

### use_mouse_click

Handles mouse click events.

```rust
fn use_mouse_click<F>(handler: F)
where
    F: Fn(MouseButton, u16, u16) + Send + Sync + 'static;
```

**Example:**

```rust
use_mouse_click(move |button, x, y| {
    if button == MouseButton::Left {
        handle_click(x, y);
    }
});
```

---

### use_mouse_hover

Tracks hover state over a rectangular area.

```rust
fn use_mouse_hover(area: Rect) -> bool
```

**Example:**

```rust
let button_area = Rect::new(10, 5, 20, 3);
let is_hovering = use_mouse_hover(button_area);

let style = if is_hovering {
    Style::default().bg(Color::Blue)
} else {
    Style::default()
};
```

---

### use_mouse_drag

Tracks drag operations.

```rust
fn use_mouse_drag() -> (DragInfo, impl Fn() + Clone)
```

**DragInfo:**

```rust
struct DragInfo {
    pub button: Option<MouseButton>,
    pub start: (u16, u16),
    pub current: (u16, u16),
    pub is_dragging: bool,
    pub is_start: bool,
    pub is_end: bool,
}
```

---

### use_mouse_position

Returns current mouse position.

```rust
fn use_mouse_position() -> (u16, u16)
```

---

### use_double_click

Detects double-click events.

```rust
fn use_double_click<F>(max_delay: Duration, handler: F)
where
    F: Fn(MouseButton, u16, u16) + Send + Sync + 'static;
```

---

## Timing Hooks

### use_timeout

Executes a callback after a delay.

```rust
fn use_timeout<F>(callback: F, delay_ms: u64) -> TimeoutHandle
where
    F: Fn() + Send + Sync + 'static;
```

**TimeoutHandle Methods:**

| Method         | Description            |
| -------------- | ---------------------- |
| `cancel()`     | Cancel the timeout     |
| `reset()`      | Reset the timer        |
| `is_pending()` | Check if still pending |

**Example:**

```rust
let timeout = use_timeout(
    || println!("Timeout fired!"),
    5000, // 5 seconds
);

// Cancel if needed
timeout.cancel();
```

---

### use_interval

Executes a callback repeatedly at an interval.

```rust
fn use_interval<F>(callback: F, interval_ms: u64) -> IntervalHandle
where
    F: Fn() + Send + Sync + 'static;
```

**IntervalHandle Methods:**

| Method         | Description         |
| -------------- | ------------------- |
| `pause()`      | Pause the interval  |
| `resume()`     | Resume the interval |
| `is_running()` | Check if running    |

**Example:**

```rust
let interval = use_interval(
    move || set_time.set(get_current_time()),
    1000, // Every second
);
```

---

## Layout Hooks

### use_area

Returns the component's render area.

```rust
fn use_area() -> ComponentArea
```

`ComponentArea` implements `Deref<Target = Rect>`.

---

### use_frame

Returns the current frame context.

```rust
fn use_frame() -> FrameContext
```

**FrameContext Methods:**

| Method             | Description           |
| ------------------ | --------------------- |
| `count()`          | Frame number          |
| `delta()`          | Time since last frame |
| `fps()`            | Current FPS           |
| `is_first_frame()` | Check if first frame  |

---

### use_resize

Returns current terminal dimensions.

```rust
fn use_resize() -> (u16, u16)
```

---

### use_media_query

Evaluates a predicate against terminal dimensions.

```rust
fn use_media_query<F>(predicate: F) -> bool
where
    F: Fn((u16, u16)) -> bool + Send + Sync + 'static;
```

**Example:**

```rust
let is_narrow = use_media_query(|(w, _)| w < 80);
let is_mobile = use_media_query(|(w, _)| w < 60);
let is_desktop = use_media_query(|(w, _)| w >= 120);
```

---

## Form Hooks

### use_form

Creates a form with validation.

```rust
fn use_form(config: FormConfig) -> FormHandle
```

**FormConfig Builder:**

```rust
let form = use_form(
    FormConfig::builder()
        .field("email", "")
        .field("password", "")
        .validator("email", Validator::required("Email is required"))
        .validator("email", Validator::email("Invalid email"))
        .validator("password", Validator::min_length(8, "Min 8 characters"))
        .on_submit(|values| {
            println!("Submitted: {:?}", values);
        })
        .build()
);
```

**Validator Methods:**

| Method                 | Description             |
| ---------------------- | ----------------------- |
| `required(msg)`        | Field must not be empty |
| `min_length(n, msg)`   | Minimum length          |
| `max_length(n, msg)`   | Maximum length          |
| `email(msg)`           | Email format            |
| `url(msg)`             | URL format              |
| `numeric(msg)`         | Numeric value           |
| `integer(msg)`         | Integer value           |
| `pattern(regex, msg)`  | Regex pattern           |
| `min(n, msg)`          | Minimum numeric value   |
| `max(n, msg)`          | Maximum numeric value   |
| `range(min, max, msg)` | Numeric range           |
| `alphanumeric(msg)`    | Alphanumeric only       |
| `alpha(msg)`           | Letters only            |
| `custom(fn)`           | Custom validation       |

**FormHandle Methods:**

| Method                        | Description                 |
| ----------------------------- | --------------------------- |
| `register(name)`              | Get field registration      |
| `get_value(name)`             | Get field value             |
| `set_value(name, value)`      | Set field value             |
| `get_error(name)`             | Get field error             |
| `is_touched(name)`            | Check if field touched      |
| `validate_field(name, value)` | Validate single field       |
| `validate_all()`              | Validate all fields         |
| `submit()`                    | Submit the form             |
| `reset(initial)`              | Reset to initial values     |
| `is_valid()`                  | Check if form is valid      |
| `has_errors()`                | Check for any errors        |
| `is_dirty()`                  | Check if any field modified |

---

### use_form_context

Accesses form from child components.

```rust
fn use_form_context() -> FormHandle
```

---

### use_watch

Watches a form field value.

```rust
fn use_watch(form: &FormHandle, field_name: &str) -> String
```

---

## Utility Hooks

### use_id

Generates a unique ID for the component instance.

```rust
fn use_id() -> String
```

**Example:**

```rust
let id = use_id();
// Returns something like "reratui-1-0"
```

---

## Hook Rules

1. **Call hooks at the top level** - Don't call hooks inside loops, conditions, or nested functions
2. **Call hooks in the same order** - Hooks must be called in the same order on every render
3. **Only call hooks from components** - Hooks can only be called within `Component::render()`

**Bad:**

```rust
// DON'T DO THIS
if some_condition {
    let (state, _) = use_state(|| 0); // Conditional hook call!
}
```

**Good:**

```rust
// DO THIS
let (state, set_state) = use_state(|| 0);
if some_condition {
    // Use state here
}
```
