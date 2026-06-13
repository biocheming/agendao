# Async Patterns Guide

This guide covers async programming patterns in Reratui, including data fetching, mutations, and async effects.

## Table of Contents

- [Overview](#overview)
- [use_future](#use_future)
- [use_query](#use_query)
- [use_mutation](#use_mutation)
- [Async Effects](#async-effects)
- [Error Handling](#error-handling)
- [Caching Strategies](#caching-strategies)
- [Best Practices](#best-practices)

## Overview

Reratui provides several hooks for async operations:

| Hook               | Use Case                        |
| ------------------ | ------------------------------- |
| `use_future`       | Simple async tasks              |
| `use_query`        | Data fetching with caching      |
| `use_mutation`     | Create/Update/Delete operations |
| `use_async_effect` | Async side effects              |

## use_future

For simple async operations without caching:

```rust
use reratui::hooks::{use_future, FutureState};

async fn fetch_greeting() -> Result<String, String> {
    // Simulate API call
    tokio::time::sleep(Duration::from_secs(1)).await;
    Ok("Hello from the server!".to_string())
}

impl Component for GreetingComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        // Fetch on mount (None deps = run once)
        let future = use_future(fetch_greeting, None::<()>);

        let text = match future.state() {
            FutureState::Idle => "Not started",
            FutureState::Pending => "Loading...",
            FutureState::Resolved(msg) => msg.as_str(),
            FutureState::Error(err) => err.as_str(),
        };

        Paragraph::new(text).render(area, buffer);
    }
}
```

### Refetching with Dependencies

```rust
impl Component for UserProfile {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (user_id, set_user_id) = use_state(|| 1);

        // Refetch when user_id changes
        let user = use_future(
            move || async move { fetch_user(user_id).await },
            Some(user_id),
        );

        // Change user triggers refetch
        use_keyboard_press(move |key| {
            match key.code {
                KeyCode::Char('1') => set_user_id.set(1),
                KeyCode::Char('2') => set_user_id.set(2),
                KeyCode::Char('3') => set_user_id.set(3),
                _ => {}
            }
        });

        // Render based on state...
    }
}
```

### Manual Refetch

```rust
let future = use_future(fetch_data, Some(()));

use_keyboard_press(move |key| {
    if key.code == KeyCode::Char('r') {
        future.refetch();
    }
});
```

## use_query

For data fetching with caching, stale-while-revalidate, and retry logic:

```rust
use reratui::hooks::{use_query, QueryOptions, QueryStatus};

impl Component for UserList {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let query = use_query(
            "users", // Cache key
            || async { api::fetch_users().await },
            Some(QueryOptions {
                enabled: true,
                stale_time: Duration::from_secs(30),    // Fresh for 30s
                cache_time: Duration::from_secs(300),   // Cache for 5min
                retry: true,
                retry_attempts: 3,
                retry_delay: Duration::from_millis(500),
                retry_exponential_backoff: true,
            }),
        );

        match query.status {
            QueryStatus::Idle => {
                // Query disabled or not started
            }
            QueryStatus::Loading => {
                render_loading_spinner(area, buffer);
            }
            QueryStatus::Refreshing => {
                // Show cached data with refresh indicator
                if let Some(users) = &query.data {
                    render_users(users, area, buffer);
                    render_refresh_indicator(area, buffer);
                }
            }
            QueryStatus::Success => {
                if let Some(users) = &query.data {
                    render_users(users, area, buffer);
                }
            }
            QueryStatus::Error => {
                if let Some(error) = &query.error {
                    render_error(error, area, buffer);
                }
            }
        }
    }
}
```

### Query with Dynamic Keys

```rust
impl Component for UserDetail {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (user_id, set_user_id) = use_state(|| 1);

        // Different cache entry for each user_id
        let query = use_query(
            format!("user-{}", user_id), // Dynamic key
            move || async move { api::fetch_user(user_id).await },
            None,
        );

        // Switching users uses cached data if available
        use_keyboard_press(move |key| {
            match key.code {
                KeyCode::Left => set_user_id.update(|id| id.saturating_sub(1)),
                KeyCode::Right => set_user_id.update(|id| id + 1),
                _ => {}
            }
        });

        // Render...
    }
}
```

### Cache Invalidation

```rust
let query = use_query("data", fetch_data, None);

use_keyboard_press(move |key| {
    match key.code {
        KeyCode::Char('r') => query.refetch(),      // Refetch, keep cache
        KeyCode::Char('c') => query.invalidate(),   // Clear cache, refetch
        _ => {}
    }
});
```

### Conditional Queries

```rust
let (search_term, set_search_term) = use_state(|| String::new());

let query = use_query(
    format!("search-{}", search_term),
    move || async move { api::search(&search_term).await },
    Some(QueryOptions {
        enabled: search_term.len() >= 3, // Only search with 3+ chars
        ..Default::default()
    }),
);
```

## use_mutation

For create, update, and delete operations:

```rust
use reratui::hooks::{use_mutation, MutationOptions, MutationStatus};

#[derive(Clone)]
struct CreateUserRequest {
    name: String,
    email: String,
}

impl Component for CreateUserForm {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (name, set_name) = use_state(|| String::new());
        let (email, set_email) = use_state(|| String::new());

        let create_mutation = use_mutation(
            |request: CreateUserRequest| async move {
                api::create_user(request).await
            },
            Some(MutationOptions {
                retry: true,
                retry_attempts: 3,
                retry_delay: Duration::from_millis(500),
                retry_exponential_backoff: true,
            }),
        );

        let state = create_mutation.state();

        // Handle form submission
        use_keyboard_press(move |key| {
            if key.code == KeyCode::Enter && !state.is_pending {
                create_mutation.mutate(CreateUserRequest {
                    name: name.clone(),
                    email: email.clone(),
                });
            }
        });

        // Render form
        let status_text = match state.status {
            MutationStatus::Idle => "Press Enter to submit",
            MutationStatus::Pending => "Creating user...",
            MutationStatus::Success => "User created!",
            MutationStatus::Error => "Failed to create user",
        };

        // Render form fields and status...
    }
}
```

### Mutation with Callbacks

Handle success/error in effects:

```rust
impl Component for UserManager {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (notification, set_notification) = use_state(|| None::<String>);

        let delete_mutation = use_mutation(
            |user_id: u64| async move { api::delete_user(user_id).await },
            None,
        );

        let state = delete_mutation.state();

        // React to mutation state changes
        use_effect(
            move || {
                if state.is_success {
                    set_notification.set(Some("User deleted!".to_string()));
                } else if state.is_error {
                    if let Some(err) = &state.error {
                        set_notification.set(Some(format!("Error: {}", err)));
                    }
                }
                None
            },
            (state.is_success, state.is_error),
        );

        // Render...
    }
}
```

### Optimistic Updates

```rust
impl Component for TodoList {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (todos, set_todos) = use_state(|| vec![]);

        let toggle_mutation = use_mutation(
            |todo_id: u64| async move { api::toggle_todo(todo_id).await },
            None,
        );

        let handle_toggle = move |todo_id: u64| {
            // Optimistic update
            set_todos.update(|todos| {
                todos.iter().map(|t| {
                    if t.id == todo_id {
                        Todo { completed: !t.completed, ..t.clone() }
                    } else {
                        t.clone()
                    }
                }).collect()
            });

            // Actual mutation
            toggle_mutation.mutate(todo_id);
        };

        // If mutation fails, you'd want to rollback
        let state = toggle_mutation.state();
        use_effect(
            move || {
                if state.is_error {
                    // Rollback: refetch todos
                    // In a real app, you'd track the original state
                }
                None
            },
            state.is_error,
        );

        // Render todos...
    }
}
```

## Async Effects

For async side effects that aren't data fetching:

```rust
impl Component for WebSocketComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (messages, set_messages) = use_state(|| vec![]);

        use_async_effect_once(move || {
            let set_msgs = set_messages.clone();
            async move {
                let ws = connect_websocket().await;

                // Spawn message handler
                let handle = tokio::spawn(async move {
                    while let Some(msg) = ws.recv().await {
                        set_msgs.update(|msgs| {
                            let mut new_msgs = msgs.clone();
                            new_msgs.push(msg);
                            new_msgs
                        });
                    }
                });

                // Cleanup: close connection
                Some(Box::new(move || {
                    Box::pin(async move {
                        handle.abort();
                    })
                }) as AsyncCleanupFn)
            }
        });

        // Render messages...
    }
}
```

### Async Effect with Dependencies

```rust
impl Component for SubscriptionComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (channel, set_channel) = use_state(|| "general".to_string());

        use_async_effect(
            move || {
                let ch = channel.clone();
                async move {
                    let subscription = subscribe_to_channel(&ch).await;

                    Some(Box::new(move || {
                        Box::pin(async move {
                            subscription.unsubscribe().await;
                        })
                    }) as AsyncCleanupFn)
                }
            },
            channel.clone(), // Re-subscribe when channel changes
        );

        // Render...
    }
}
```

## Error Handling

### Displaying Errors

```rust
impl Component for ErrorHandlingDemo {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let query = use_query("data", fetch_data, None);

        if let Some(error) = &query.error {
            let error_block = Block::default()
                .title("Error")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red));

            let error_text = Paragraph::new(vec![
                Line::from(Span::styled(
                    "Failed to load data",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(error.to_string()),
                Line::from(""),
                Line::from(Span::styled(
                    "Press 'r' to retry",
                    Style::default().fg(Color::Yellow),
                )),
            ])
            .block(error_block)
            .alignment(Alignment::Center);

            error_text.render(area, buffer);
            return;
        }

        // Render normal content...
    }
}
```

### Retry Logic

```rust
let query = use_query(
    "data",
    fetch_data,
    Some(QueryOptions {
        retry: true,
        retry_attempts: 3,
        retry_delay: Duration::from_millis(1000),
        retry_exponential_backoff: true, // 1s, 2s, 4s
        ..Default::default()
    }),
);
```

### Error Boundaries

Create a wrapper component for error handling:

```rust
struct ErrorBoundary<C: Component> {
    child: C,
}

impl<C: Component> Component for ErrorBoundary<C> {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (has_error, set_has_error) = use_state(|| false);

        if has_error {
            Paragraph::new("Something went wrong")
                .style(Style::default().fg(Color::Red))
                .render(area, buffer);
            return;
        }

        // In a real implementation, you'd catch panics
        self.child.render(area, buffer);
    }
}
```

## Caching Strategies

### Stale-While-Revalidate

Show cached data immediately, refresh in background:

```rust
let query = use_query(
    "data",
    fetch_data,
    Some(QueryOptions {
        stale_time: Duration::from_secs(30),  // Fresh for 30s
        cache_time: Duration::from_secs(300), // Keep in cache 5min
        ..Default::default()
    }),
);

// query.is_stale indicates background refresh
if query.is_stale {
    render_stale_indicator(area, buffer);
}
```

### Cache-First

Prefer cached data, only fetch if missing:

```rust
let query = use_query(
    "data",
    fetch_data,
    Some(QueryOptions {
        stale_time: Duration::from_secs(3600), // Fresh for 1 hour
        cache_time: Duration::from_secs(86400), // Cache for 24 hours
        ..Default::default()
    }),
);
```

### Network-First

Always fetch fresh data:

```rust
let query = use_query(
    "data",
    fetch_data,
    Some(QueryOptions {
        stale_time: Duration::ZERO, // Always stale
        ..Default::default()
    }),
);
```

## Best Practices

### 1. Use Appropriate Hooks

| Scenario                 | Hook               |
| ------------------------ | ------------------ |
| Simple one-time fetch    | `use_future_once`  |
| Fetch with refetch       | `use_future`       |
| Cached data fetching     | `use_query`        |
| Create/Update/Delete     | `use_mutation`     |
| Subscriptions/WebSockets | `use_async_effect` |

### 2. Handle All States

Always handle loading, error, and success states:

```rust
match query.status {
    QueryStatus::Idle => { /* Not started */ }
    QueryStatus::Loading => { /* Show spinner */ }
    QueryStatus::Refreshing => { /* Show data + indicator */ }
    QueryStatus::Success => { /* Show data */ }
    QueryStatus::Error => { /* Show error */ }
}
```

### 3. Use Meaningful Cache Keys

```rust
// Good: Descriptive, unique keys
use_query(format!("user-{}", user_id), ...);
use_query(format!("posts-page-{}", page), ...);
use_query(format!("search-{}-{}", query, filters), ...);

// Bad: Generic keys that might collide
use_query("data", ...);
use_query("fetch", ...);
```

### 4. Configure Retry Appropriately

```rust
// For critical operations
MutationOptions {
    retry: true,
    retry_attempts: 5,
    retry_exponential_backoff: true,
    ..Default::default()
}

// For non-critical operations
MutationOptions {
    retry: false,
    ..Default::default()
}
```

### 5. Clean Up Async Operations

Always provide cleanup for subscriptions and long-running operations:

```rust
use_async_effect(
    || async {
        let subscription = subscribe().await;

        // IMPORTANT: Clean up on unmount or deps change
        Some(Box::new(|| Box::pin(async {
            subscription.unsubscribe().await;
        })))
    },
    deps,
);
```

### 6. Avoid Waterfalls

Fetch data in parallel when possible:

```rust
// Good: Parallel fetches
let users = use_query("users", fetch_users, None);
let posts = use_query("posts", fetch_posts, None);
let comments = use_query("comments", fetch_comments, None);

// Bad: Sequential fetches (waterfall)
// Don't make one query depend on another unless necessary
```

### 7. Debounce Search Queries

```rust
let (search_term, set_search_term) = use_state(|| String::new());
let (debounced_term, set_debounced) = use_state(|| String::new());

// Debounce the search term
use_effect(
    move || {
        let term = search_term.clone();
        let set_debounced = set_debounced.clone();

        // In a real app, use a proper debounce mechanism
        set_debounced.set(term);
        None
    },
    search_term.clone(),
);

// Query uses debounced term
let results = use_query(
    format!("search-{}", debounced_term),
    move || async move { api::search(&debounced_term).await },
    Some(QueryOptions {
        enabled: debounced_term.len() >= 2,
        ..Default::default()
    }),
);
```
