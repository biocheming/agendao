# Architecture Overview

This document explains the internal architecture of Reratui, including the fiber system, render pipeline, and hook implementation.

## Table of Contents

- [Overview](#overview)
- [Fiber Architecture](#fiber-architecture)
- [Render Pipeline](#render-pipeline)
- [Hook System](#hook-system)
- [State Management](#state-management)
- [Effect System](#effect-system)
- [Context System](#context-system)
- [Event Handling](#event-handling)

## Overview

Reratui is built on a fiber-based architecture inspired by React Fiber. The key components are:

```
┌────────────────────────────────────────────────────────────┐
│                        Runtime                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │                   Render Loop                       │   │
│  │  ┌─────┐  ┌────────┐  ┌────────┐  ┌───────┐  ┌─────┐│   │
│  │  │Poll │→ │ Render │→ │ Commit │→ │ Event │ →│Effect│   │
│  │  └─────┘  └────────┘  └────────┘  └───────┘  └─────┘│   │
│  └─────────────────────────────────────────────────────┘   │
│                            ↓                               │
│  ┌─────────────────────────────────────────────────────┐   │
│  │                    Fiber Tree                       │   │
│  │  ┌────────┐    ┌────────┐    ┌────────┐             │   │
│  │  │ Fiber  │───→│ Fiber  │───→│ Fiber  │             │   │
│  │  │ (Root) │    │(Child) │    │(Child) │             │   │
│  │  └────────┘    └────────┘    └────────┘             │   │
│  └─────────────────────────────────────────────────────┘   │
│                            ↓                               │
│  ┌─────────────────────────────────────────────────────┐   │
│  │                   Scheduler                         │   │
│  │  ┌──────────────┐  ┌──────────────┐                 │   │
│  │  │ State Batch  │  │ Effect Queue │                 │   │
│  │  └──────────────┘  └──────────────┘                 │   │
│  └─────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────┘
```

## Fiber Architecture

### What is a Fiber?

A Fiber is a unit of work that represents a component instance. Each fiber maintains:

```rust
pub struct Fiber {
    pub id: FiberId,                    // Unique identifier
    pub hooks: Vec<Box<dyn Any>>,       // Hook state storage
    pub hook_index: usize,              // Current hook position
    pub pending_effects: Vec<PendingEffect>,
    pub cleanup_by_hook: HashMap<usize, CleanupFn>,
    pub async_cleanup_by_hook: HashMap<usize, AsyncCleanupFn>,
    pub provided_contexts: Vec<TypeId>,
    pub parent: Option<FiberId>,
    pub children: Vec<FiberId>,
    pub dirty: bool,                    // Needs re-render
    pub key: Option<String>,            // For reconciliation
}
```

### Fiber Tree

The `FiberTree` manages all fibers:

```rust
pub struct FiberTree {
    fibers: HashMap<FiberId, Fiber>,
    root: Option<FiberId>,
    render_stack: Vec<FiberId>,         // Currently rendering
    pending_unmount: Vec<FiberId>,      // Scheduled for cleanup
}
```

### Fiber Lifecycle

```
Mount → Render → Update → Unmount
  │        │        │        │
  │        │        │        └─ Run cleanups, remove fiber
  │        │        └─ Re-render on state change
  │        └─ Execute render, call hooks
  └─ Create fiber, initialize hooks
```

## Render Pipeline

The render loop consists of 5 phases:

### 1. Poll Phase

Wait for terminal events or scheduled updates:

```rust
// Simplified
loop {
    let event = poll_event(frame_interval)?;
    if event.is_some() || has_dirty_fibers() {
        // Continue to render
    }
}
```

### 2. Render Phase

Execute component render functions:

```rust
// For each dirty fiber
fiber_tree.begin_render(fiber_id);
component.render(area, buffer);
fiber_tree.end_render();
```

During render:

- Hook index is reset to 0
- Hooks are called in order
- State reads return current values
- State writes are batched

### 3. Commit Phase

Apply batched state updates:

```rust
// Process state batch
for update in state_batch.drain() {
    let fiber = tree.get_mut(update.fiber_id);
    fiber.set_hook(update.hook_index, update.new_value);
    fiber.dirty = true;
}
```

### 4. Event Phase

Process terminal events:

```rust
// Set current event for hooks to read
set_current_event(event);

// Re-render to process event
// (hooks like use_keyboard read the event)
```

### 5. Effect Phase

Run effects and cleanups:

```rust
// Run cleanups first (LIFO order)
for cleanup in cleanups.drain().rev() {
    cleanup();
}

// Run new effects
for effect in pending_effects.drain() {
    let cleanup = (effect.effect)();
    if let Some(cleanup) = cleanup {
        fiber.cleanup_by_hook.insert(effect.hook_index, cleanup);
    }
}
```

## Hook System

### Hook Storage

Hooks store state in the fiber's `hooks` vector:

```rust
// Each hook gets an index
let hook_index = fiber.next_hook_index();

// State is stored at that index
fiber.set_hook(hook_index, initial_value);

// Retrieved on subsequent renders
let value = fiber.get_hook::<T>(hook_index);
```

### Hook Index Management

```rust
impl Fiber {
    pub fn next_hook_index(&mut self) -> usize {
        let index = self.hook_index;
        self.hook_index += 1;
        index
    }

    pub fn reset_hook_index(&mut self) {
        self.hook_index = 0;
    }
}
```

This is why hooks must be called in the same order - the index determines which state belongs to which hook.

### Hook Implementation Pattern

```rust
pub fn use_state<T, F>(initializer: F) -> (T, StateSetter<T>)
where
    T: Clone + Send + Sync + PartialEq + 'static,
    F: FnOnce() -> T,
{
    with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();
        let fiber_id = fiber.id;

        // Initialize on first render
        let value = fiber.get_or_init_hook(hook_index, initializer);

        // Create setter
        let setter = StateSetter {
            fiber_id,
            hook_index,
            _marker: PhantomData,
        };

        (value, setter)
    })
}
```

## State Management

### State Batching

State updates are batched during render:

```rust
pub struct StateBatch {
    updates: Vec<StateUpdate>,
    is_batching: bool,
}

pub struct StateUpdate {
    pub fiber_id: FiberId,
    pub hook_index: usize,
    pub update: StateUpdateKind,
}

pub enum StateUpdateKind {
    Value(Box<dyn Any + Send>),
    Updater(Box<dyn FnOnce(Box<dyn Any>) -> Box<dyn Any> + Send>),
}
```

### Update Flow

```
set_count.set(5)
    │
    ▼
queue_update(fiber_id, StateUpdate { ... })
    │
    ▼
StateBatch.updates.push(update)
    │
    ▼ (end of render phase)
    │
StateBatch.end_batch(tree)
    │
    ▼
Apply updates to fibers
Mark fibers dirty
```

## Effect System

### Effect Types

```rust
// Sync effect
pub struct PendingEffect {
    pub effect: Box<dyn FnOnce() -> Option<CleanupFn>>,
    pub hook_index: usize,
}

// Async effect
pub struct AsyncPendingEffect {
    pub effect: AsyncEffectFn,
    pub hook_index: usize,
}
```

### Effect Queue

```rust
pub struct EffectQueue {
    sync_effects: Vec<PendingEffect>,
    async_effects: Vec<AsyncPendingEffect>,
    sync_cleanups: Vec<CleanupFn>,
    async_cleanups: Vec<AsyncCleanupFn>,
}
```

### Effect Execution Order

1. Run sync cleanups (LIFO)
2. Run async cleanups (LIFO)
3. Run sync effects
4. Spawn async effects

### Dependency Tracking

Effects track dependencies to determine when to re-run:

```rust
pub fn use_effect<D, F>(effect: F, deps: D)
where
    D: PartialEq + Clone + Send + Sync + 'static,
{
    with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();

        // Get previous deps
        let prev_deps = fiber.get_hook::<D>(hook_index);

        // Check if deps changed
        let should_run = prev_deps.map_or(true, |prev| prev != deps);

        if should_run {
            // Store new deps
            fiber.set_hook(hook_index, deps.clone());

            // Queue effect
            fiber.pending_effects.push(PendingEffect {
                effect: Box::new(effect),
                hook_index,
            });
        }
    });
}
```

## Context System

### Context Stack

Context uses a thread-local stack:

```rust
thread_local! {
    static CONTEXT_STACK: RefCell<ContextStack> = RefCell::new(ContextStack::new());
}

pub struct ContextStack {
    // TypeId -> Vec<(FiberId, Value)>
    contexts: HashMap<TypeId, Vec<(FiberId, Box<dyn Any + Send + Sync>)>>,
}
```

### Provider

```rust
pub fn use_context_provider<T, F>(initializer: F)
where
    T: Clone + Send + Sync + 'static,
    F: FnOnce() -> T,
{
    with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();
        let value = fiber.get_or_init_hook(hook_index, initializer);

        // Push to context stack
        push_context(fiber.id, value);

        // Track for cleanup
        fiber.provided_contexts.push(TypeId::of::<T>());
    });
}
```

### Consumer

```rust
pub fn use_context<T>() -> T
where
    T: Clone + Send + Sync + 'static,
{
    get_context::<T>().expect("Context not found")
}

fn get_context<T: Clone + 'static>() -> Option<T> {
    CONTEXT_STACK.with(|stack| {
        let stack = stack.borrow();
        let type_id = TypeId::of::<T>();

        stack.contexts.get(&type_id)
            .and_then(|vec| vec.last())
            .and_then(|(_, value)| value.downcast_ref::<T>())
            .cloned()
    })
}
```

## Event Handling

### Event Storage

```rust
thread_local! {
    static CURRENT_EVENT: RefCell<Option<Arc<Event>>> = RefCell::new(None);
}

pub fn set_current_event(event: Option<Arc<Event>>) {
    CURRENT_EVENT.with(|e| *e.borrow_mut() = event);
}

pub fn use_event() -> Option<Event> {
    CURRENT_EVENT.with(|e| e.borrow().as_ref().map(|arc| (**arc).clone()))
}
```

### Event Hooks

Event hooks use the effect event pattern for stable callbacks:

```rust
pub fn use_keyboard<F>(handler: F)
where
    F: Fn(KeyEvent) + Send + Sync + 'static,
{
    // Create stable callback
    let stable_handler = use_effect_event(move |key_event: KeyEvent| {
        handler(key_event);
    });

    // Check for keyboard events
    if let Some(Event::Key(key_event)) = use_event() {
        stable_handler.call(key_event);
    }
}
```

## Thread Safety

### Requirements

All hook state must be `Send + Sync` because:

- State may be accessed from async effects
- The runtime uses async/await

### Patterns

```rust
// Use Arc for shared state
let shared = Arc::new(Mutex::new(data));

// Use parking_lot for better performance
use parking_lot::Mutex;
let shared = Arc::new(Mutex::new(data));
```

## Performance Considerations

### Dirty Tracking

Only dirty fibers are re-rendered:

```rust
impl FiberTree {
    pub fn dirty_fibers(&self) -> HashSet<FiberId> {
        self.fibers
            .iter()
            .filter(|(_, fiber)| fiber.dirty)
            .map(|(id, _)| *id)
            .collect()
    }
}
```

### Conditional Updates

Use `set_if_changed` to avoid unnecessary re-renders:

```rust
impl<T: PartialEq> StateSetter<T> {
    pub fn set_if_changed(&self, value: T) {
        // Only queue update if value differs
        if current_value != value {
            self.set(value);
        }
    }
}
```

### Memoization

`use_memo` caches expensive computations:

```rust
pub fn use_memo<T, D, F>(compute: F, deps: D) -> T {
    with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();

        // Check if deps changed
        let prev = fiber.get_hook::<(D, T)>(hook_index);

        if let Some((prev_deps, cached)) = prev {
            if prev_deps == deps {
                return cached; // Return cached value
            }
        }

        // Recompute
        let value = compute();
        fiber.set_hook(hook_index, (deps, value.clone()));
        value
    })
}
```

## Debugging

### Tracing

Enable tracing for detailed logs:

```rust
tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .init();
```

### Strict Mode

Enable strict mode to catch hook violations:

```rust
render_with_options(|| App, RenderOptions {
    strict_mode: true,
    ..Default::default()
}).await?;
```

### Fiber Inspection

In debug builds, fibers track hook types:

```rust
#[cfg(debug_assertions)]
pub previous_hook_types: Vec<&'static str>,
#[cfg(debug_assertions)]
pub current_hook_types: Vec<&'static str>,
```
