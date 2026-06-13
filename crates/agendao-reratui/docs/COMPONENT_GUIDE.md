# Component Guide

This guide covers how to create and compose components in Reratui.

## Table of Contents

- [Basic Components](#basic-components)
- [Component with State](#component-with-state)
- [Component with Props](#component-with-props)
- [Composing Components](#composing-components)
- [Context and Providers](#context-and-providers)
- [Event Handling](#event-handling)
- [Lifecycle and Effects](#lifecycle-and-effects)
- [Best Practices](#best-practices)

## Basic Components

Components in Reratui implement the `Component` trait:

```rust
use reratui::prelude::*;

struct MyComponent;

impl Component for MyComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        Paragraph::new("Hello, World!")
            .render(area, buffer);
    }
}
```

The `render` method receives:

- `area: Rect` - The rectangular area where the component should render
- `buffer: &mut Buffer` - The terminal buffer to draw into

## Component with State

Use hooks to add state to your components:

```rust
struct Counter;

impl Component for Counter {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        // State hook - persists across renders
        let (count, set_count) = use_state(|| 0);

        // Handle keyboard input
        use_keyboard_press(move |key| {
            match key.code {
                KeyCode::Up => set_count.update(|c| c + 1),
                KeyCode::Down => set_count.update(|c| c.saturating_sub(1)),
                KeyCode::Char('q') => request_exit(),
                _ => {}
            }
        });

        // Render
        let text = format!("Count: {}", count);
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .render(area, buffer);
    }
}
```

## Component with Props

Components can hold data as fields:

```rust
struct Greeting {
    name: String,
    show_emoji: bool,
}

impl Greeting {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            show_emoji: true,
        }
    }

    fn with_emoji(mut self, show: bool) -> Self {
        self.show_emoji = show;
        self
    }
}

impl Component for Greeting {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let text = if self.show_emoji {
            format!("👋 Hello, {}!", self.name)
        } else {
            format!("Hello, {}!", self.name)
        };

        Paragraph::new(text).render(area, buffer);
    }
}

// Usage
Greeting::new("Alice")
    .with_emoji(true)
    .render(area, buffer);
```

## Composing Components

### Manual Composition

Compose components by calling their render methods:

```rust
struct App;

impl Component for App {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        // Split the area
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(3),
            ])
            .split(area);

        // Render child components
        Header { title: "My App".to_string() }.render(chunks[0], buffer);
        Content.render(chunks[1], buffer);
        Footer.render(chunks[2], buffer);
    }
}

struct Header {
    title: String,
}

impl Component for Header {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let block = Block::default()
            .title(self.title.as_str())
            .borders(Borders::ALL);
        block.render(area, buffer);
    }
}

struct Content;

impl Component for Content {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        Paragraph::new("Main content here")
            .block(Block::default().borders(Borders::ALL))
            .render(area, buffer);
    }
}

struct Footer;

impl Component for Footer {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        Paragraph::new("Press 'q' to quit")
            .alignment(Alignment::Center)
            .render(area, buffer);
    }
}
```

### Component Lists

Render lists of components:

```rust
struct ItemList {
    items: Vec<String>,
}

impl Component for ItemList {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (selected, set_selected) = use_state(|| 0usize);

        use_keyboard_press(move |key| {
            match key.code {
                KeyCode::Up => set_selected.update(|s| s.saturating_sub(1)),
                KeyCode::Down => set_selected.update(|s| (s + 1).min(self.items.len() - 1)),
                _ => {}
            }
        });

        let items: Vec<ListItem> = self.items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let style = if i == selected {
                    Style::default().bg(Color::Blue).fg(Color::White)
                } else {
                    Style::default()
                };
                ListItem::new(item.as_str()).style(style)
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().title("Items").borders(Borders::ALL));

        list.render(area, buffer);
    }
}
```

## Context and Providers

Share data across the component tree using context:

### Creating a Theme Provider

```rust
#[derive(Clone)]
struct Theme {
    primary: Color,
    secondary: Color,
    background: Color,
    text: Color,
}

impl Theme {
    fn dark() -> Self {
        Self {
            primary: Color::Cyan,
            secondary: Color::Blue,
            background: Color::Black,
            text: Color::White,
        }
    }

    fn light() -> Self {
        Self {
            primary: Color::Blue,
            secondary: Color::Cyan,
            background: Color::White,
            text: Color::Black,
        }
    }
}

struct ThemeProvider {
    theme: Theme,
}

impl Component for ThemeProvider {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        // Provide theme to all children
        use_context_provider(|| self.theme.clone());

        // Render children (in real app, you'd pass children somehow)
        ThemedContent.render(area, buffer);
    }
}

struct ThemedContent;

impl Component for ThemedContent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        // Consume theme from context
        let theme = use_context::<Theme>();

        let block = Block::default()
            .title("Themed Content")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.primary));

        Paragraph::new("This uses the theme!")
            .style(Style::default().fg(theme.text))
            .block(block)
            .render(area, buffer);
    }
}
```

### Optional Context

Use `try_use_context` when context might not be available:

```rust
impl Component for OptionalThemedComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let theme = try_use_context::<Theme>()
            .unwrap_or_else(Theme::dark);

        // Use theme...
    }
}
```

## Event Handling

### Keyboard Events

```rust
impl Component for KeyboardDemo {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (last_key, set_last_key) = use_state(|| "None".to_string());

        // Handle all key presses
        use_keyboard_press(move |key| {
            let key_str = match key.code {
                KeyCode::Char(c) => format!("'{}'", c),
                KeyCode::Enter => "Enter".to_string(),
                KeyCode::Esc => "Escape".to_string(),
                KeyCode::Up => "Up".to_string(),
                KeyCode::Down => "Down".to_string(),
                _ => format!("{:?}", key.code),
            };

            let mods = if key.modifiers.contains(KeyModifiers::CONTROL) {
                "Ctrl+"
            } else if key.modifiers.contains(KeyModifiers::ALT) {
                "Alt+"
            } else {
                ""
            };

            set_last_key.set(format!("{}{}", mods, key_str));
        });

        // Handle specific shortcuts
        use_keyboard_shortcut(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
            || request_exit(),
        );

        Paragraph::new(format!("Last key: {}", last_key))
            .render(area, buffer);
    }
}
```

### Mouse Events

```rust
impl Component for MouseDemo {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (click_pos, set_click_pos) = use_state(|| None::<(u16, u16)>);

        // Track clicks
        use_mouse_click(move |button, x, y| {
            if button == MouseButton::Left {
                set_click_pos.set(Some((x, y)));
            }
        });

        // Track hover over a specific area
        let button_area = Rect::new(10, 5, 20, 3);
        let is_hovering = use_mouse_hover(button_area);

        let button_style = if is_hovering {
            Style::default().bg(Color::Blue)
        } else {
            Style::default().bg(Color::DarkGray)
        };

        // Render button
        let button = Paragraph::new("Click Me")
            .alignment(Alignment::Center)
            .style(button_style);
        button.render(button_area, buffer);

        // Show click position
        if let Some((x, y)) = click_pos {
            let info = format!("Last click: ({}, {})", x, y);
            Paragraph::new(info).render(
                Rect::new(0, area.height - 1, area.width, 1),
                buffer,
            );
        }
    }
}
```

## Lifecycle and Effects

### Mount/Unmount Effects

```rust
impl Component for LifecycleDemo {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        // Run once on mount
        use_effect_once(|| {
            println!("Component mounted!");

            // Cleanup on unmount
            Some(Box::new(|| {
                println!("Component unmounting!");
            }))
        });

        Paragraph::new("Check console for lifecycle events")
            .render(area, buffer);
    }
}
```

### Effects with Dependencies

```rust
impl Component for EffectDemo {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (count, set_count) = use_state(|| 0);
        let (message, set_message) = use_state(|| String::new());

        // Effect runs when count changes
        use_effect(
            move || {
                set_message.set(format!("Count is now: {}", count));

                // Optional cleanup
                Some(Box::new(|| {
                    // Cleanup before next effect run
                }))
            },
            count, // Dependency
        );

        use_keyboard_press(move |key| {
            if key.code == KeyCode::Char(' ') {
                set_count.update(|c| c + 1);
            }
        });

        Paragraph::new(message).render(area, buffer);
    }
}
```

### Async Effects

```rust
impl Component for AsyncEffectDemo {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (user_id, set_user_id) = use_state(|| 1);
        let (user_name, set_user_name) = use_state(|| "Loading...".to_string());

        // Async effect runs when user_id changes
        use_async_effect(
            move || {
                let set_name = set_user_name.clone();
                async move {
                    let name = fetch_user_name(user_id).await;
                    set_name.set(name);
                    None // No cleanup needed
                }
            },
            user_id,
        );

        Paragraph::new(format!("User: {}", user_name))
            .render(area, buffer);
    }
}
```

## Best Practices

### 1. Keep Components Focused

Each component should have a single responsibility:

```rust
// Good: Focused components
struct UserAvatar { user_id: u64 }
struct UserName { name: String }
struct UserBio { bio: String }

// Bad: Monolithic component
struct UserEverything {
    user_id: u64,
    name: String,
    bio: String,
    avatar_url: String,
    // ... many more fields
}
```

### 2. Lift State Up

Share state by lifting it to a common ancestor:

```rust
struct Parent;

impl Component for Parent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        // State lives in parent
        let (selected, set_selected) = use_state(|| 0);

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        // Pass state down to children
        Sidebar { selected, on_select: set_selected.clone() }
            .render(chunks[0], buffer);
        Content { selected }
            .render(chunks[1], buffer);
    }
}
```

### 3. Use Context for Deep Props

Avoid prop drilling with context:

```rust
// Instead of passing theme through every component...
struct App;

impl Component for App {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        // Provide once at the top
        use_context_provider(|| Theme::dark());

        // Children can access directly
        DeepNestedComponent.render(area, buffer);
    }
}
```

### 4. Memoize Expensive Computations

```rust
impl Component for ExpensiveComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let (items, _) = use_state(|| load_items());

        // Memoize expensive computation
        let stats = use_memo(
            || calculate_statistics(&items),
            items.clone(),
        );

        // Use stats...
    }
}
```

### 5. Handle Loading States

```rust
impl Component for DataComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let query = use_query(
            "data",
            || async { fetch_data().await },
            None,
        );

        match query.status {
            QueryStatus::Loading => {
                Paragraph::new("Loading...")
                    .alignment(Alignment::Center)
                    .render(area, buffer);
            }
            QueryStatus::Error => {
                let error = query.error.as_ref().unwrap();
                Paragraph::new(format!("Error: {}", error))
                    .style(Style::default().fg(Color::Red))
                    .render(area, buffer);
            }
            QueryStatus::Success => {
                let data = query.data.as_ref().unwrap();
                render_data(data, area, buffer);
            }
            _ => {}
        }
    }
}
```

### 6. Clean Up Resources

Always clean up in effects:

```rust
use_effect(
    move || {
        let subscription = subscribe_to_events();

        Some(Box::new(move || {
            subscription.unsubscribe();
        }))
    },
    (),
);
```

### 7. Use Conditional Updates

Avoid unnecessary re-renders:

```rust
// Only update if value actually changed
set_value.set_if_changed(new_value);

// Or with update function
set_value.update_if_changed(|v| compute_new_value(v));
```
