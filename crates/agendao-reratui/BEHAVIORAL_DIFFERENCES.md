# Behavioral Differences: React vs Reratui

This document outlines the key differences between React and Reratui for developers familiar with React.

## Overview

Reratui is inspired by React but adapted for terminal user interfaces in Rust. While the mental model is similar, there are important differences due to the different runtime environment and language.

## Component Model

### React

```jsx
function Counter({ initialCount }) {
  const [count, setCount] = useState(initialCount);
  return <div>Count: {count}</div>;
}
```

### Reratui

```rust
struct Counter {
    initial_count: i32,
}

impl Component for Counter {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (count, set_count) = use_state(|| self.initial_count);
        Paragraph::new(format!("Count: {}", count))
            .render(area, buffer);
    }
}
```

**Key Differences:**

- Components are structs implementing `Component` trait
- Props are struct fields
- Render receives `area` and `buffer` instead of returning JSX
- Direct rendering to buffer instead of virtual DOM

## State Management

### State Updates

| React                        | Reratui                           |
| ---------------------------- | --------------------------------- |
| `setState(value)`            | `set_state.set(value)`            |
| `setState(prev => prev + 1)` | `set_state.update(\|c\| c + 1)`   |
| Automatic batching           | Explicit batching in render cycle |

### Reratui-Specific Methods

```rust
// Only update if value changed (avoids unnecessary re-renders)
set_count.set_if_changed(new_value);
set_count.update_if_changed(|c| c + 1);
```

### State Type Requirements

React: Any JavaScript value
Reratui: `T: Clone + Send + Sync + PartialEq + 'static`

## Effects

### React

```jsx
useEffect(() => {
  console.log("Effect ran");
  return () => console.log("Cleanup");
}, [dep]);
```

### Reratui

```rust
use_effect(
    move || {
        println!("Effect ran");
        Some(Box::new(|| println!("Cleanup")))
    },
    dep,
);
```

**Key Differences:**

- Cleanup is `Option<Box<dyn FnOnce()>>` instead of optional return
- Dependencies are a single value (use tuples for multiple)
- No dependency array - single dependency or tuple

### Dependency Comparison

| React                  | Reratui                         |
| ---------------------- | ------------------------------- |
| `[]` (empty array)     | `use_effect_once`               |
| `[dep]`                | `use_effect(..., dep)`          |
| `[dep1, dep2]`         | `use_effect(..., (dep1, dep2))` |
| No deps (every render) | Not directly supported          |

## Context

### React

```jsx
// Provider
<ThemeContext.Provider value={theme}>
  <App />
</ThemeContext.Provider>;

// Consumer
const theme = useContext(ThemeContext);
```

### Reratui

```rust
// Provider (inside component)
use_context_provider(|| theme.clone());

// Consumer
let theme = use_context::<Theme>();
```

**Key Differences:**

- No separate Context object creation
- Provider is a hook, not a component wrapper
- Type-based lookup instead of context object
- `try_use_context` for optional context

## Refs

### React

```jsx
const ref = useRef(initialValue);
ref.current = newValue;
console.log(ref.current);
```

### Reratui

```rust
let ref_handle = use_ref(|| initial_value);
ref_handle.set(new_value);
let value = ref_handle.get();
```

**Key Differences:**

- Methods instead of `.current` property
- `get()` returns cloned value
- `update(fn)` for functional updates

## Memoization

### React

```jsx
const memoized = useMemo(() => expensive(), [dep]);
const callback = useCallback(() => doSomething(), [dep]);
```

### Reratui

```rust
let memoized = use_memo(|| expensive(), dep);
let callback = use_callback(|| do_something(), dep);
```

**Key Differences:**

- Single dependency value (use tuples for multiple)
- Callback returns `Callback<F>` wrapper

## Event Handling

### React

```jsx
<button onClick={(e) => handleClick(e)}>Click</button>
```

### Reratui

```rust
use_keyboard_press(move |key| {
    if key.code == KeyCode::Enter {
        handle_click();
    }
});

use_mouse_click(move |button, x, y| {
    if button == MouseButton::Left {
        handle_click();
    }
});
```

**Key Differences:**

- No JSX event props
- Hooks for event handling
- Separate hooks for keyboard and mouse
- Terminal events instead of DOM events

## Event Propagation

Reratui follows React's event propagation model:

### React

```jsx
function Parent() {
  const handleClick = (e) => {
    console.log("Parent received event");
  };

  return (
    <div onClick={handleClick}>
      <Child />
    </div>
  );
}

function Child() {
  const handleClick = (e) => {
    console.log("Child received event");
    e.stopPropagation(); // Prevents parent from receiving
  };

  return <button onClick={handleClick}>Click</button>;
}
```

### Reratui

```rust
fn parent_component() {
    // Parent can read the event
    if let Some(Event::Key(key)) = use_event() {
        println!("Parent received event");
    }

    // Render child...
}

fn child_component() {
    // Child can also read the SAME event
    if let Some(Event::Key(key)) = use_event() {
        if key.code == KeyCode::Enter {
            println!("Child received event");
            stop_propagation(); // Prevents other fibers from receiving
        }
    }
}
```

**Key Similarities:**

- Events are available to ALL components during a render frame
- Multiple components can read the same event
- `stop_propagation()` prevents other components from receiving the event
- The component that called `stop_propagation()` can still read the event

**Available Functions:**

| Function             | Description                                      |
| -------------------- | ------------------------------------------------ |
| `use_event()`        | Returns the current event (respects propagation) |
| `stop_propagation()` | Prevents other fibers from receiving the event   |
| `peek_event()`       | Returns the event without respecting propagation |

**Example: Nested Event Handling**

```rust
fn scroll_container() {
    // ScrollView handles scroll events
    if let Some(Event::Key(key)) = use_event() {
        match key.code {
            KeyCode::Up | KeyCode::Down => {
                // Handle scroll
                stop_propagation(); // Don't let parent handle these
            }
            _ => {} // Let other keys propagate
        }
    }
}

fn app() {
    // App can handle events that weren't stopped
    if let Some(Event::Key(key)) = use_event() {
        if key.code == KeyCode::Char('q') {
            // Quit app - this will work because scroll didn't stop it
        }
    }

    // Render scroll_container...
}
```

## Async Data Fetching

### React (with React Query)

```jsx
const { data, isLoading, error, refetch } = useQuery({
  queryKey: ["users"],
  queryFn: fetchUsers,
  staleTime: 30000,
});
```

### Reratui

```rust
let query = use_query(
    "users",
    || async { fetch_users().await },
    Some(QueryOptions {
        stale_time: Duration::from_secs(30),
        ..Default::default()
    }),
);

// Access: query.data, query.status, query.error, query.refetch()
```

**Key Differences:**

- Built-in (no separate library)
- `QueryStatus` enum instead of boolean flags
- Duration types instead of milliseconds

## Rendering

### React

- Virtual DOM diffing
- Reconciliation algorithm
- Automatic re-renders on state change

### Reratui

- Direct buffer rendering
- Fiber-based architecture
- 5-phase render pipeline:
  1. Poll (wait for events)
  2. Render (execute components)
  3. Commit (apply state updates)
  4. Event (process terminal events)
  5. Effect (run effects)

## Lifecycle

### React Lifecycle

```
Mount → Update → Unmount
```

### Reratui Lifecycle

```
Mount → Render Loop → Unmount
         ↓
    Poll → Render → Commit → Event → Effect
         ↑___________________________|
```

## No Direct Equivalents

### React Features Not in Reratui

| React               | Reratui Alternative                     |
| ------------------- | --------------------------------------- |
| JSX                 | Direct widget rendering                 |
| Suspense            | Manual loading states                   |
| Error Boundaries    | Manual error handling                   |
| Portals             | Not applicable (single buffer)          |
| Fragments           | Not needed                              |
| forwardRef          | Not applicable                          |
| useImperativeHandle | Not applicable                          |
| useLayoutEffect     | `use_effect` (all effects are "layout") |
| useDeferredValue    | Not available                           |
| useTransition       | Not available                           |
| Server Components   | Not applicable                          |

### Reratui-Specific Features

| Feature           | Description               |
| ----------------- | ------------------------- |
| `use_keyboard`    | Terminal keyboard events  |
| `use_mouse`       | Terminal mouse events     |
| `use_area`        | Component render area     |
| `use_frame`       | Frame timing info         |
| `use_resize`      | Terminal resize events    |
| `use_media_query` | Terminal size breakpoints |
| `use_history`     | Undo/redo state           |
| `use_form`        | Form validation           |
| `use_timeout`     | Timeout with handle       |
| `use_interval`    | Interval with handle      |

## Threading Model

### React

- Single-threaded (main thread)
- Concurrent features for interruptible rendering

### Reratui

- Async runtime (Tokio)
- State must be `Send + Sync`
- Effects can spawn async tasks

## Type Safety

### React

- Runtime prop validation (PropTypes) or TypeScript
- Hooks can return any type

### Reratui

- Compile-time type checking
- Strict type requirements on hooks
- Generic constraints enforced

## Performance Considerations

### React

- Virtual DOM overhead
- Reconciliation cost
- Memoization for optimization

### Reratui

- Direct buffer rendering (no VDOM)
- Fiber-based dirty tracking
- State batching
- `set_if_changed` / `update_if_changed` for optimization

## Migration Tips

1. **Think in traits** - Components are trait implementations, not functions
2. **Embrace ownership** - Clone when needed, use `Arc` for shared state
3. **Single dependency** - Use tuples for multiple effect dependencies
4. **Direct rendering** - No JSX, render directly to buffer
5. **Terminal events** - Use keyboard/mouse hooks instead of event props
6. **Type constraints** - Ensure state types meet trait bounds
7. **Async runtime** - Tokio is required for async features
