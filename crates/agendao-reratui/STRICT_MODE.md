# Strict Mode

Strict Mode is a development feature that helps catch common mistakes and enforce best practices in your Reratui applications.

## Enabling Strict Mode

```rust
use reratui::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    render_with_options(|| App, RenderOptions {
        strict_mode: true,
        ..Default::default()
    }).await?;
    Ok(())
}
```

## What Strict Mode Detects

### 1. Hook Order Changes

Hooks must be called in the same order on every render. Strict mode warns when hook order changes between renders.

**Bad:**

```rust
impl Component for BadComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (show_extra, _) = use_state(|| false);

        // DON'T DO THIS - conditional hook call
        if show_extra {
            let (extra, _) = use_state(|| 0); // Hook order changes!
        }

        let (count, _) = use_state(|| 0);
    }
}
```

**Good:**

```rust
impl Component for GoodComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (show_extra, _) = use_state(|| false);
        let (extra, _) = use_state(|| 0);      // Always called
        let (count, _) = use_state(|| 0);      // Always called

        // Use the values conditionally instead
        if show_extra {
            // Use extra here
        }
    }
}
```

### 2. Hook Count Changes

The number of hooks called must be consistent across renders.

**Bad:**

```rust
impl Component for BadComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (items, _) = use_state(|| vec![1, 2, 3]);

        // DON'T DO THIS - variable number of hooks
        for item in &items {
            let (item_state, _) = use_state(|| 0); // Hook count varies!
        }
    }
}
```

**Good:**

```rust
impl Component for GoodComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (items, _) = use_state(|| vec![1, 2, 3]);

        // Use a single state for all items
        let (item_states, set_item_states) = use_state(|| {
            items.iter().map(|_| 0).collect::<Vec<_>>()
        });
    }
}
```

### 3. Hook Type Changes

The type of hook at each position must remain consistent.

**Bad:**

```rust
impl Component for BadComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (mode, _) = use_state(|| "state");

        // DON'T DO THIS - different hook types at same position
        if mode == "state" {
            let (value, _) = use_state(|| 0);
        } else {
            let value = use_memo(|| 0, ()); // Different hook type!
        }
    }
}
```

## Strict Mode Warnings

When strict mode detects an issue, it logs a warning:

```
WARN fiber_id=FiberId(1) previous_count=2 current_count=3
     Hook count changed between renders. This may indicate conditional hook calls.

WARN fiber_id=FiberId(1) hook_index=1 previous_hook="use_state" current_hook="use_effect"
     Hook order changed between renders. Hooks must be called in the same order.
```

## Rules of Hooks

### 1. Only Call Hooks at the Top Level

Don't call hooks inside loops, conditions, or nested functions.

```rust
// ✅ Good
let (count, set_count) = use_state(|| 0);
let (name, set_name) = use_state(|| String::new());

// ❌ Bad
if some_condition {
    let (count, _) = use_state(|| 0);
}

// ❌ Bad
for i in 0..n {
    let (item, _) = use_state(|| 0);
}
```

### 2. Only Call Hooks from Components

Hooks can only be called within `Component::render()`.

```rust
// ✅ Good
impl Component for MyComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (state, _) = use_state(|| 0); // Inside render
    }
}

// ❌ Bad
fn helper_function() {
    let (state, _) = use_state(|| 0); // Outside component!
}
```

### 3. Call Hooks in the Same Order

Hooks must be called in the same order on every render.

```rust
// ✅ Good - same order every time
let (a, _) = use_state(|| 0);
let (b, _) = use_state(|| 0);
let (c, _) = use_state(|| 0);

// ❌ Bad - order depends on condition
if condition {
    let (a, _) = use_state(|| 0);
    let (b, _) = use_state(|| 0);
} else {
    let (b, _) = use_state(|| 0);
    let (a, _) = use_state(|| 0);
}
```

## Why These Rules Matter

Reratui relies on the order of hook calls to associate state with the correct hook. Each hook call is identified by its position in the call sequence, not by a name or key.

When you call hooks conditionally or in loops, the order can change between renders, causing:

- State to be associated with the wrong hook
- Unexpected behavior
- Hard-to-debug issues

## Debugging Hook Issues

### Enable Tracing

Add tracing to see detailed hook information:

```rust
use tracing_subscriber;

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    // ... run app
}
```

### Check Hook Order

If you see hook order warnings:

1. Look for conditional hook calls
2. Look for hooks in loops
3. Look for early returns before all hooks are called
4. Ensure all code paths call the same hooks

### Common Patterns

**Pattern: Conditional Logic with Hooks**

```rust
// ❌ Bad
if show_details {
    let (details, _) = use_state(|| load_details());
    render_details(details);
}

// ✅ Good
let (details, _) = use_state(|| None);
if show_details {
    if let Some(d) = &details {
        render_details(d);
    }
}
```

**Pattern: Dynamic Lists**

```rust
// ❌ Bad
for item in items {
    let (state, _) = use_state(|| item.default());
}

// ✅ Good
let (states, set_states) = use_state(|| {
    items.iter().map(|i| i.default()).collect::<HashMap<_, _>>()
});
```

**Pattern: Early Returns**

```rust
// ❌ Bad
if loading {
    return render_loading();
    // Hooks below are skipped!
}
let (data, _) = use_state(|| None);

// ✅ Good
let (data, _) = use_state(|| None);
if loading {
    return render_loading();
}
```

## Performance Considerations

Strict mode adds overhead for tracking hook calls. It's recommended to:

- Enable strict mode during development
- Disable strict mode in production builds

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = RenderOptions {
        strict_mode: cfg!(debug_assertions), // Only in debug builds
        ..Default::default()
    };

    render_with_options(|| App, options).await?;
    Ok(())
}
```

## Summary

Strict mode helps you:

- Catch hook rule violations early
- Understand why your component behaves unexpectedly
- Write more maintainable code

Always develop with strict mode enabled, and fix any warnings before they become bugs.
